//! The Socket.IO v5 packet codec, equal to `socket.io-parser` 4.2.7 where it is well defined.
//!
//! A packet is `<type>[<attachments>-][<nsp>,][<ackId>][JSON]` with the types `0` CONNECT to `6` BINARY_ACK; the main
//! namespace `/` is omitted. A binary event is a text packet whose JSON holds placeholders
//! `{"_placeholder":true,"num":n}`, followed by one binary message per attachment.
//!
//! The decoder reads untrusted bytes, so it fails closed: everything that is not exactly a packet of the protocol is
//! a [`DecodeError`] and the connection is closed. It never indexes or slices at a position that is not an ASCII
//! delimiter, never allocates from a declared count, bounds the JSON nesting and the memory a packet may cost, and
//! reads JSON with a private visitor instead of `serde_json::Value`'s deserializer, which depends on which features
//! other crates of the build enable (`raw_value` makes `Value` re-enter the parser for a magic key).

use std::cell::Cell;
use std::io::Write as _;

use bytes::Bytes;
use indexmap::IndexMap;
use serde::de::{self, DeserializeSeed, Visitor};

use super::options::IoOptions;
use super::value::{Binary, ENTRY_COST, IoValue, MAX_DEPTH, NODE_COST};

/// The event names a client may not send and the server may not emit (`RESERVED_EVENTS` of `socket.io-parser`).
pub(crate) const RESERVED_EVENTS: [&str; 6] = ["connect", "connect_error", "disconnect", "disconnecting", "newListener", "removeListener"];

/// The largest acknowledgement id (`Number.MAX_SAFE_INTEGER`): beyond it a JavaScript client's id is not exact.
pub(crate) const MAX_ACK_ID: u64 = (1 << 53) - 1;

/// The longest namespace a packet may carry; registered namespaces are at most this long.
pub(crate) const MAX_NAMESPACE_LEN: usize = 256;

/// What a decoded packet costs besides its values: the packet, the namespace and the event name headers.
const PACKET_COST: usize = 128;

pub(crate) fn is_reserved(name: &str) -> bool {
    RESERVED_EVENTS.contains(&name)
}

/// One Socket.IO packet, binary attachments already restored in place of their placeholders.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Packet {
    pub(crate) nsp: String,
    pub(crate) body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Body {
    /// CONNECT: `auth` from a client, `{"sid"}` from the server.
    Connect(Option<IoValue>),
    Disconnect,
    Event {
        id: Option<u64>,
        name: String,
        args: Vec<IoValue>,
    },
    Ack {
        id: u64,
        args: Vec<IoValue>,
    },
    /// CONNECT_ERROR: a string or an object.
    ConnectError(IoValue),
}

impl Packet {
    /// What the decoded packet costs in memory (the model of `IoOptions::max_decoded_bytes`).
    pub(crate) fn cost(&self) -> usize {
        let values = match &self.body {
            Body::Connect(v) => v.as_ref().map_or(0, IoValue::cost),
            Body::Disconnect => 0,
            Body::Event { name, args, .. } => name.len() + args.iter().map(IoValue::cost).fold(0usize, usize::saturating_add),
            Body::Ack { args, .. } => args.iter().map(IoValue::cost).fold(0usize, usize::saturating_add),
            Body::ConnectError(v) => v.cost(),
        };
        PACKET_COST.saturating_add(self.nsp.len()).saturating_add(values)
    }
}

/// Why a packet could not be decoded. Every one closes the connection; none echoes data of the packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum DecodeError {
    #[error("unknown packet type")]
    UnknownType,
    #[error("illegal attachments")]
    IllegalAttachments,
    #[error("too many attachments")]
    TooManyAttachments,
    #[error("invalid namespace")]
    InvalidNamespace,
    #[error("invalid acknowledgement id")]
    InvalidAckId,
    #[error("invalid payload")]
    InvalidPayload,
    #[error("event name too long")]
    EventNameTooLong,
    #[error("too many arguments")]
    TooManyArguments,
    #[error("JSON nested too deeply")]
    TooDeep,
    #[error("decoded packet too large")]
    TooLarge,
    #[error("got plaintext data when reconstructing a packet")]
    UnexpectedText,
    #[error("got binary data when not reconstructing a packet")]
    UnexpectedBinary,
    #[error("illegal attachments")]
    IllegalPlaceholder,
}

/// The limits a decoder enforces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_attachments: usize,
    pub(crate) max_decoded_bytes: usize,
    pub(crate) max_auth_bytes: usize,
    pub(crate) max_event_name_len: usize,
    pub(crate) max_args: usize,
}

impl Limits {
    pub(crate) fn from_options(options: &IoOptions) -> Limits {
        Limits {
            max_attachments: options.max_attachments,
            max_decoded_bytes: options.max_decoded_bytes,
            max_auth_bytes: options.max_auth_bytes,
            max_event_name_len: options.max_event_name_len,
            max_args: options.max_args,
        }
    }
}

// --------------------------------------------------------------------------------------------------- encoding

/// An encoded packet: one text message, then its attachments (each a binary message).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Encoded {
    pub(crate) text: String,
    pub(crate) attachments: Vec<Binary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EncodeError {
    TooManyAttachments,
    TooDeep,
}

struct Writer {
    out: Vec<u8>,
    attachments: Vec<Binary>,
    max_attachments: usize,
    binary: bool,
}

impl Writer {
    fn string(&mut self, s: &str) {
        serde_json::to_writer(&mut self.out, s).expect("writing to a Vec cannot fail");
    }

    fn value(&mut self, v: &IoValue, depth: usize) -> Result<(), EncodeError> {
        match v {
            IoValue::Null => self.out.extend_from_slice(b"null"),
            IoValue::Bool(true) => self.out.extend_from_slice(b"true"),
            IoValue::Bool(false) => self.out.extend_from_slice(b"false"),
            IoValue::Number(n) => {
                write!(self.out, "{n}").expect("writing to a Vec cannot fail");
            }
            IoValue::String(s) => self.string(s),
            IoValue::Binary(b) => {
                // Only reached when the packet is binary: the flag is computed from the same arguments.
                if !self.binary || self.attachments.len() >= self.max_attachments {
                    return Err(EncodeError::TooManyAttachments);
                }
                let num = self.attachments.len();
                self.attachments.push(b.clone());
                write!(self.out, r#"{{"_placeholder":true,"num":{num}}}"#).expect("writing to a Vec cannot fail");
            }
            IoValue::Array(items) => {
                if depth + 1 > MAX_DEPTH {
                    return Err(EncodeError::TooDeep);
                }
                self.out.push(b'[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push(b',');
                    }
                    self.value(item, depth + 1)?;
                }
                self.out.push(b']');
            }
            IoValue::Object(entries) => {
                if depth + 1 > MAX_DEPTH {
                    return Err(EncodeError::TooDeep);
                }
                self.out.push(b'{');
                for (i, (k, item)) in entries.iter().enumerate() {
                    if i > 0 {
                        self.out.push(b',');
                    }
                    self.string(k);
                    self.out.push(b':');
                    self.value(item, depth + 1)?;
                }
                self.out.push(b'}');
            }
        }
        Ok(())
    }
}

/// Encodes a packet. A packet whose arguments hold binary data at any depth becomes BINARY_EVENT / BINARY_ACK, with
/// the placeholders numbered depth first, as `deconstructPacket` does; more than `max_attachments` of them fail.
pub(crate) fn encode(packet: &Packet, max_attachments: usize) -> Result<Encoded, EncodeError> {
    let binary = match &packet.body {
        Body::Event { args, .. } | Body::Ack { args, .. } => args.iter().any(IoValue::contains_binary),
        _ => false,
    };
    let mut w = Writer { out: Vec::new(), attachments: Vec::new(), max_attachments, binary };
    let (kind, id) = match &packet.body {
        Body::Connect(_) => (0u8, None),
        Body::Disconnect => (1, None),
        Body::Event { id, .. } => (if binary { 5 } else { 2 }, *id),
        Body::Ack { id, .. } => (if binary { 6 } else { 3 }, Some(*id)),
        Body::ConnectError(_) => (4, None),
    };
    // The JSON first: the attachment count goes in front of it.
    match &packet.body {
        Body::Connect(Some(v)) | Body::ConnectError(v) => w.value(v, 0)?,
        Body::Connect(None) | Body::Disconnect => {}
        Body::Event { name, args, .. } => {
            w.out.push(b'[');
            w.string(name);
            for item in args {
                w.out.push(b',');
                w.value(item, 1)?;
            }
            w.out.push(b']');
        }
        Body::Ack { args, .. } => {
            w.out.push(b'[');
            for (i, item) in args.iter().enumerate() {
                if i > 0 {
                    w.out.push(b',');
                }
                w.value(item, 1)?;
            }
            w.out.push(b']');
        }
    }
    let Writer { out: json, attachments, .. } = w;
    let mut head = String::with_capacity(32 + packet.nsp.len());
    head.push((b'0' + kind) as char);
    if binary {
        head.push_str(&attachments.len().to_string());
        head.push('-');
    }
    if packet.nsp != "/" && !packet.nsp.is_empty() {
        head.push_str(&packet.nsp);
        head.push(',');
    }
    if let Some(id) = id {
        head.push_str(&id.to_string());
    }
    let mut text = head.into_bytes();
    text.extend_from_slice(&json);
    // The JSON is UTF-8 by construction (serde_json writes valid UTF-8 and the rest is ASCII).
    let text = String::from_utf8(text).expect("an encoded packet is valid UTF-8");
    Ok(Encoded { text, attachments })
}

// --------------------------------------------------------------------------------------------------- decoding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    Size,
    Depth,
}

struct Budget {
    remaining: Cell<usize>,
    fail: Cell<Option<Fail>>,
}

impl Budget {
    fn charge<E: de::Error>(&self, cost: usize) -> Result<(), E> {
        let left = self.remaining.get();
        if cost > left {
            self.fail.set(Some(Fail::Size));
            return Err(E::custom("decoded packet too large"));
        }
        self.remaining.set(left - cost);
        Ok(())
    }
}

struct Seed<'a> {
    budget: &'a Budget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = IoValue;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<IoValue, D::Error> {
        deserializer.deserialize_any(Visit { budget: self.budget, depth: self.depth })
    }
}

struct KeySeed<'a> {
    budget: &'a Budget,
}

impl<'de> DeserializeSeed<'de> for KeySeed<'_> {
    type Value = String;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<String, D::Error> {
        struct K<'a>(&'a Budget);
        impl Visitor<'_> for K<'_> {
            type Value = String;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a string key")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
                self.0.charge::<E>(ENTRY_COST.saturating_add(v.len()))?;
                Ok(v.to_owned())
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<String, E> {
                self.0.charge::<E>(ENTRY_COST.saturating_add(v.len()))?;
                Ok(v)
            }
        }
        deserializer.deserialize_str(K(self.budget))
    }
}

struct Visit<'a> {
    budget: &'a Budget,
    depth: usize,
}

impl Visit<'_> {
    fn container<E: de::Error>(&self) -> Result<(), E> {
        if self.depth + 1 > MAX_DEPTH {
            self.budget.fail.set(Some(Fail::Depth));
            return Err(E::custom("JSON nested too deeply"));
        }
        self.budget.charge::<E>(NODE_COST)
    }
}

impl<'de> Visitor<'de> for Visit<'_> {
    type Value = IoValue;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST)?;
        Ok(IoValue::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST)?;
        Ok(IoValue::from(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST)?;
        Ok(IoValue::from(v))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST)?;
        Ok(IoValue::from(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST.saturating_add(v.len()))?;
        Ok(IoValue::String(v.to_owned()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST.saturating_add(v.len()))?;
        Ok(IoValue::String(v))
    }
    fn visit_unit<E: de::Error>(self) -> Result<IoValue, E> {
        self.budget.charge::<E>(NODE_COST)?;
        Ok(IoValue::Null)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<IoValue, A::Error> {
        self.container::<A::Error>()?;
        // No capacity from a size hint: the vector grows as items are read and paid for.
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Seed { budget: self.budget, depth: self.depth + 1 })? {
            items.push(item);
        }
        Ok(IoValue::Array(items))
    }
    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<IoValue, A::Error> {
        self.container::<A::Error>()?;
        let mut entries = IndexMap::new();
        while let Some(key) = map.next_key_seed(KeySeed { budget: self.budget })? {
            let value = map.next_value_seed(Seed { budget: self.budget, depth: self.depth + 1 })?;
            entries.insert(key, value);
        }
        Ok(IoValue::Object(Box::new(entries)))
    }
}

/// Parses one JSON document, whole, spending at most `budget` of the cost model.
fn parse_json(text: &str, budget: usize) -> Result<IoValue, DecodeError> {
    let state = Budget { remaining: Cell::new(budget), fail: Cell::new(None) };
    let mut de = serde_json::Deserializer::from_str(text);
    let result = Seed { budget: &state, depth: 0 }.deserialize(&mut de).and_then(|v| de.end().map(|()| v));
    match result {
        Ok(v) => Ok(v),
        Err(e) => Err(match state.fail.get() {
            Some(Fail::Size) => DecodeError::TooLarge,
            Some(Fail::Depth) => DecodeError::TooDeep,
            None if e.to_string().starts_with("recursion limit exceeded") => DecodeError::TooDeep,
            None => DecodeError::InvalidPayload,
        }),
    }
}

/// A binary packet whose attachments are still arriving.
#[derive(Debug)]
struct Pending {
    nsp: String,
    id: Option<u64>,
    /// `Some` for a BINARY_EVENT.
    name: Option<String>,
    args: Vec<IoValue>,
    expected: usize,
    buffers: Vec<Bytes>,
    cost: usize,
}

/// The streaming decoder of one Engine.IO connection: text messages and binary messages in the order they arrive.
#[derive(Debug)]
pub(crate) struct Decoder {
    limits: Limits,
    pending: Option<Pending>,
}

fn digits(b: &[u8], from: usize) -> usize {
    b[from..].iter().take_while(|c| c.is_ascii_digit()).count()
}

impl Decoder {
    pub(crate) fn new(limits: Limits) -> Self {
        Decoder { limits, pending: None }
    }

    /// `true` while the attachments of a binary packet are awaited.
    pub(crate) fn is_reassembling(&self) -> bool {
        self.pending.is_some()
    }

    /// What the header of the packet being reassembled costs (zero when none).
    pub(crate) fn pending_cost(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.cost)
    }

    /// Attachments still awaited for the packet being reassembled.
    #[cfg(test)]
    pub(crate) fn awaited(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.expected - p.buffers.len())
    }

    /// Adds a text message. `Ok(Some(packet))` when it is a complete packet, `Ok(None)` when it is the header of a
    /// binary packet.
    pub(crate) fn add_text(&mut self, text: &str) -> Result<Option<Packet>, DecodeError> {
        if self.pending.is_some() {
            return Err(DecodeError::UnexpectedText);
        }
        let b = text.as_bytes();
        let kind = match b.first() {
            Some(c @ b'0'..=b'6') => c - b'0',
            _ => return Err(DecodeError::UnknownType),
        };
        let binary = kind == 5 || kind == 6;
        let mut pos = 1;
        let mut expected = 0usize;
        if binary {
            let n = digits(b, pos);
            if n == 0 || b.get(pos + n) != Some(&b'-') {
                return Err(DecodeError::IllegalAttachments);
            }
            for c in &b[pos..pos + n] {
                expected = expected.saturating_mul(10).saturating_add(usize::from(c - b'0'));
            }
            if expected < 1 {
                return Err(DecodeError::IllegalAttachments);
            }
            if expected > self.limits.max_attachments {
                return Err(DecodeError::TooManyAttachments);
            }
            pos += n + 1;
        }
        let nsp = if b.get(pos) == Some(&b'/') {
            let end = memchr::memchr(b',', &b[pos..]).map_or(b.len(), |i| pos + i);
            if end - pos > MAX_NAMESPACE_LEN {
                return Err(DecodeError::InvalidNamespace);
            }
            // `pos` is the ASCII `/` and `end` an ASCII `,` or the end: both are character boundaries.
            let nsp = text[pos..end].to_owned();
            pos = if end < b.len() { end + 1 } else { end };
            nsp
        } else {
            "/".to_owned()
        };
        let id_len = digits(b, pos);
        let id = if id_len == 0 {
            None
        } else {
            if id_len > 16 {
                return Err(DecodeError::InvalidAckId);
            }
            let mut v = 0u64;
            for c in &b[pos..pos + id_len] {
                v = v * 10 + u64::from(c - b'0');
            }
            if v > MAX_ACK_ID {
                return Err(DecodeError::InvalidAckId);
            }
            pos += id_len;
            Some(v)
        };
        // `pos` follows ASCII only.
        let payload = &text[pos..];
        let budget = self.limits.max_decoded_bytes;
        match kind {
            0 => {
                if id.is_some() {
                    return Err(DecodeError::InvalidAckId);
                }
                let auth = if payload.is_empty() {
                    None
                } else {
                    let v = parse_json(payload, budget.min(self.limits.max_auth_bytes))?;
                    if !matches!(v, IoValue::Object(_)) {
                        return Err(DecodeError::InvalidPayload);
                    }
                    Some(v)
                };
                Ok(Some(Packet { nsp, body: Body::Connect(auth) }))
            }
            1 => {
                if id.is_some() {
                    return Err(DecodeError::InvalidAckId);
                }
                if !payload.is_empty() {
                    return Err(DecodeError::InvalidPayload);
                }
                Ok(Some(Packet { nsp, body: Body::Disconnect }))
            }
            4 => {
                if id.is_some() {
                    return Err(DecodeError::InvalidAckId);
                }
                let data = if payload.is_empty() {
                    IoValue::Null
                } else {
                    let v = parse_json(payload, budget)?;
                    if !matches!(v, IoValue::String(_) | IoValue::Object(_)) {
                        return Err(DecodeError::InvalidPayload);
                    }
                    v
                };
                Ok(Some(Packet { nsp, body: Body::ConnectError(data) }))
            }
            2 | 5 => {
                if payload.is_empty() {
                    return Err(DecodeError::InvalidPayload);
                }
                let IoValue::Array(mut items) = parse_json(payload, budget)? else { return Err(DecodeError::InvalidPayload) };
                if items.is_empty() || items.len() - 1 > self.limits.max_args {
                    return Err(if items.is_empty() { DecodeError::InvalidPayload } else { DecodeError::TooManyArguments });
                }
                let name = match items.remove(0) {
                    IoValue::String(s) => {
                        if is_reserved(&s) {
                            return Err(DecodeError::InvalidPayload);
                        }
                        if s.len() > self.limits.max_event_name_len {
                            return Err(DecodeError::EventNameTooLong);
                        }
                        s
                    }
                    IoValue::Number(n) => n.to_string(),
                    _ => return Err(DecodeError::InvalidPayload),
                };
                if binary {
                    let cost = items.iter().map(IoValue::cost).fold(0usize, usize::saturating_add).saturating_add(PACKET_COST);
                    self.pending = Some(Pending { nsp, id, name: Some(name), args: items, expected, buffers: Vec::new(), cost });
                    Ok(None)
                } else {
                    Ok(Some(Packet { nsp, body: Body::Event { id, name, args: items } }))
                }
            }
            3 | 6 => {
                let Some(id) = id else { return Err(DecodeError::InvalidAckId) };
                if payload.is_empty() {
                    return Err(DecodeError::InvalidPayload);
                }
                let IoValue::Array(items) = parse_json(payload, budget)? else { return Err(DecodeError::InvalidPayload) };
                if items.len() > self.limits.max_args {
                    return Err(DecodeError::TooManyArguments);
                }
                if binary {
                    let cost = items.iter().map(IoValue::cost).fold(0usize, usize::saturating_add).saturating_add(PACKET_COST);
                    self.pending = Some(Pending { nsp, id: Some(id), name: None, args: items, expected, buffers: Vec::new(), cost });
                    Ok(None)
                } else {
                    Ok(Some(Packet { nsp, body: Body::Ack { id, args: items } }))
                }
            }
            _ => Err(DecodeError::UnknownType),
        }
    }

    /// Adds a binary message: the next attachment of the packet being reassembled.
    pub(crate) fn add_binary(&mut self, data: Bytes) -> Result<Option<Packet>, DecodeError> {
        let Some(pending) = self.pending.as_mut() else { return Err(DecodeError::UnexpectedBinary) };
        pending.buffers.push(data);
        if pending.buffers.len() < pending.expected {
            return Ok(None);
        }
        let Pending { nsp, id, name, mut args, buffers, .. } = self.pending.take().expect("pending was just used");
        for arg in &mut args {
            restore(arg, &buffers, 1)?;
        }
        let body = match name {
            Some(name) => Body::Event { id, name, args },
            None => Body::Ack { id: id.expect("an acknowledgement has an id"), args },
        };
        Ok(Some(Packet { nsp, body }))
    }

    /// Forgets a packet whose attachments will not arrive.
    #[cfg(test)]
    pub(crate) fn reset(&mut self) {
        self.pending = None;
    }
}

/// Puts the attachments where their placeholders are, as `reconstructPacket` does. The same attachment may be
/// referenced by several placeholders (the bytes are shared, not copied); one that is not referenced is ignored; an
/// index that is not an integer below the number of attachments is an error. The depth is bounded by the parser.
fn restore(value: &mut IoValue, buffers: &[Bytes], depth: usize) -> Result<(), DecodeError> {
    if depth > MAX_DEPTH + 1 {
        return Err(DecodeError::TooDeep);
    }
    match value {
        IoValue::Array(items) => {
            for item in items {
                restore(item, buffers, depth + 1)?;
            }
            Ok(())
        }
        IoValue::Object(entries) => {
            if matches!(entries.get("_placeholder"), Some(IoValue::Bool(true))) {
                let index = match entries.get("num") {
                    Some(IoValue::Number(n)) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
                    _ => None,
                };
                let Some(buffer) = index.and_then(|i| buffers.get(i)) else { return Err(DecodeError::IllegalPlaceholder) };
                *value = IoValue::Binary(Binary::new(buffer.clone()));
                return Ok(());
            }
            for item in entries.values_mut() {
                restore(item, buffers, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn limits() -> Limits {
        Limits::from_options(&IoOptions::new())
    }

    fn decode(text: &str) -> Result<Option<Packet>, DecodeError> {
        Decoder::new(limits()).add_text(text)
    }

    fn event(nsp: &str, id: Option<u64>, name: &str, args: Vec<IoValue>) -> Packet {
        Packet { nsp: nsp.into(), body: Body::Event { id, name: name.into(), args } }
    }

    fn enc(packet: &Packet) -> Encoded {
        encode(packet, 10).unwrap()
    }

    fn v(value: serde_json::Value) -> IoValue {
        IoValue::try_from(value).unwrap()
    }

    #[test]
    fn packets_encode_as_the_npm_parser_does() {
        assert_eq!(enc(&event("/", None, "hi", vec![])).text, r#"2["hi"]"#);
        assert_eq!(
            enc(&event("/", None, "hi", vec![v(json!(1)), v(json!("two")), v(json!({ "three": [3, null, true] }))])).text,
            r#"2["hi",1,"two",{"three":[3,null,true]}]"#
        );
        assert_eq!(enc(&event("/admin", Some(12), "x", vec![])).text, r#"2/admin,12["x"]"#);
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Ack { id: 0, args: vec![v(json!("a"))] } }).text, r#"30["a"]"#);
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Disconnect }).text, "1");
        assert_eq!(enc(&Packet { nsp: "/a".into(), body: Body::Disconnect }).text, "1/a,");
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Connect(Some(v(json!({ "sid": "abc" })))) }).text, r#"0{"sid":"abc"}"#);
        assert_eq!(enc(&Packet { nsp: "/a".into(), body: Body::Connect(None) }).text, "0/a,");
        assert_eq!(
            enc(&Packet { nsp: "/a".into(), body: Body::ConnectError(v(json!({ "message": "no", "data": { "x": 1 } }))) }).text,
            r#"4/a,{"message":"no","data":{"x":1}}"#
        );
    }

    #[test]
    fn strings_escape_like_json_stringify_and_never_hold_the_record_separator() {
        let text = enc(&event("/", None, "e", vec![v(json!("a\u{1e}b\u{0}\"\\\n\u{7f}\u{2028}é"))])).text;
        assert_eq!(text, "2[\"e\",\"a\\u001eb\\u0000\\\"\\\\\\n\u{7f}\u{2028}é\"]");
        assert!(!text.contains('\u{1e}'));
    }

    #[test]
    fn binary_values_become_numbered_placeholders_depth_first() {
        let a = Binary::from(vec![1u8]);
        let b = Binary::from(vec![2u8, 2]);
        let c = Binary::from(vec![3u8, 3, 3]);
        let args = vec![
            IoValue::object([
                ("first", IoValue::Binary(a.clone())),
                ("list", IoValue::Array(vec![IoValue::Binary(b.clone()), v(json!(5))])),
            ]),
            IoValue::Binary(c.clone()),
        ];
        let encoded = enc(&event("/x", Some(4), "up", args));
        assert_eq!(
            encoded.text,
            r#"53-/x,4["up",{"first":{"_placeholder":true,"num":0},"list":[{"_placeholder":true,"num":1},5]},{"_placeholder":true,"num":2}]"#
        );
        assert_eq!(encoded.attachments, vec![a, b, c]);
        let ack = enc(&Packet { nsp: "/".into(), body: Body::Ack { id: 1, args: vec![IoValue::from(Bytes::from_static(b"z"))] } });
        assert_eq!(ack.text, r#"61-1[{"_placeholder":true,"num":0}]"#);
    }

    #[test]
    fn encoding_refuses_more_attachments_than_allowed_and_nesting_beyond_the_bound() {
        let many: Vec<IoValue> = (0..11).map(|_| IoValue::from(Bytes::from_static(b"x"))).collect();
        assert_eq!(encode(&event("/", None, "e", many.clone()), 10), Err(EncodeError::TooManyAttachments));
        assert!(encode(&event("/", None, "e", many[..10].to_vec()), 10).is_ok());
        let mut deep = IoValue::Null;
        for _ in 0..126 {
            deep = IoValue::Array(vec![deep]);
        }
        // The arguments array is level 1, so 126 more fit and 127 do not.
        assert!(encode(&event("/", None, "e", vec![deep.clone()]), 10).is_ok());
        assert_eq!(encode(&event("/", None, "e", vec![IoValue::Array(vec![deep])]), 10), Err(EncodeError::TooDeep));
    }

    #[test]
    fn packets_decode_as_the_npm_parser_does() {
        let p = decode(r#"2["hi",1,"two"]"#).unwrap().unwrap();
        assert_eq!(p, event("/", None, "hi", vec![v(json!(1)), v(json!("two"))]));
        let p = decode(r#"2/admin,12["x"]"#).unwrap().unwrap();
        assert_eq!(p, event("/admin", Some(12), "x", vec![]));
        assert_eq!(
            decode(r#"30["a",1]"#).unwrap().unwrap(),
            Packet { nsp: "/".into(), body: Body::Ack { id: 0, args: vec![v(json!("a")), v(json!(1))] } }
        );
        assert_eq!(decode("0").unwrap().unwrap(), Packet { nsp: "/".into(), body: Body::Connect(None) });
        assert_eq!(decode("0/admin,").unwrap().unwrap(), Packet { nsp: "/admin".into(), body: Body::Connect(None) });
        assert_eq!(decode("0/admin").unwrap().unwrap(), Packet { nsp: "/admin".into(), body: Body::Connect(None) });
        assert_eq!(
            decode(r#"0/admin,{"token":"abc"}"#).unwrap().unwrap(),
            Packet { nsp: "/admin".into(), body: Body::Connect(Some(v(json!({ "token": "abc" })))) }
        );
        assert_eq!(decode("1").unwrap().unwrap(), Packet { nsp: "/".into(), body: Body::Disconnect });
        assert_eq!(
            decode(r#"4{"message":"no"}"#).unwrap().unwrap(),
            Packet { nsp: "/".into(), body: Body::ConnectError(v(json!({ "message": "no" }))) }
        );
        // A number is an event name, written as text.
        assert_eq!(decode(r#"2[7,"x"]"#).unwrap().unwrap(), event("/", None, "7", vec![v(json!("x"))]));
        // The id is read as a number: leading zeros are dropped.
        assert_eq!(decode(r#"2007["x"]"#).unwrap().unwrap(), event("/", Some(7), "x", vec![]));
    }

    #[test]
    fn every_decode_error_has_its_case() {
        use DecodeError::*;
        let cases: Vec<(String, DecodeError)> = vec![
            ("".into(), UnknownType),
            ("7".into(), UnknownType),
            ("9".into(), UnknownType),
            ("a[\"x\"]".into(), UnknownType),
            (" 2[\"x\"]".into(), UnknownType),
            ("5[\"x\"]".into(), IllegalAttachments),
            ("5-[\"x\"]".into(), IllegalAttachments),
            ("5x-[\"x\"]".into(), IllegalAttachments),
            ("50-[\"x\"]".into(), IllegalAttachments),
            ("51[\"x\"]".into(), IllegalAttachments),
            ("5999999999999999999999999-[\"x\"]".into(), TooManyAttachments),
            ("511-[\"x\"]".into(), TooManyAttachments),
            (format!("2/{},[\"x\"]", "a".repeat(300)), InvalidNamespace),
            ("2123456789012345678[\"x\"]".into(), InvalidAckId),
            ("29007199254740992[\"x\"]".into(), InvalidAckId),
            (format!("2{}[\"x\"]", "1".repeat(17)), InvalidAckId),
            ("3[\"x\"]".into(), InvalidAckId),
            ("11".into(), InvalidAckId),
            ("00{}".into(), InvalidAckId),
            ("2".into(), InvalidPayload),
            ("2{\"a\":1}".into(), InvalidPayload),
            ("2[]".into(), InvalidPayload),
            ("2[null]".into(), InvalidPayload),
            ("2[true]".into(), InvalidPayload),
            ("2[{\"a\":1}]".into(), InvalidPayload),
            ("2[\"disconnect\"]".into(), InvalidPayload),
            ("2[\"connect\"]".into(), InvalidPayload),
            ("2[\"connect_error\"]".into(), InvalidPayload),
            ("2[\"disconnecting\"]".into(), InvalidPayload),
            ("2[\"newListener\"]".into(), InvalidPayload),
            ("2[\"removeListener\"]".into(), InvalidPayload),
            ("42abc[\"x\"]".into(), InvalidAckId),
            ("2[\"x\"] trailing".into(), InvalidPayload),
            ("2[\"x\"".into(), InvalidPayload),
            ("0[]".into(), InvalidPayload),
            ("0\"text\"".into(), InvalidPayload),
            ("0null".into(), InvalidPayload),
            ("1{}".into(), InvalidPayload),
            ("10".into(), InvalidAckId),
            ("4[]".into(), InvalidPayload),
            ("4 5".into(), InvalidPayload),
            ("30".into(), InvalidPayload),
            ("30{\"a\":1}".into(), InvalidPayload),
            ("2[\"x\",1e999]".into(), InvalidPayload),
            ("5[\"x\"]".into(), IllegalAttachments),
        ];
        for (input, expected) in cases {
            assert_eq!(decode(&input), Err(expected), "{input:?}");
        }
        assert_eq!(decode(&format!("2[\"{}\"]", "n".repeat(257))), Err(EventNameTooLong));
        assert!(decode(&format!("2[\"{}\"]", "n".repeat(256))).is_ok());
        let args = vec!["1"; 129].join(",");
        assert_eq!(decode(&format!("2[\"x\",{args}]")), Err(TooManyArguments));
        let args = vec!["1"; 128].join(",");
        assert!(decode(&format!("2[\"x\",{args}]")).is_ok());
        assert_eq!(decode(&format!("31[{}]", vec!["1"; 129].join(","))), Err(TooManyArguments));
    }

    #[test]
    fn the_largest_ack_id_is_accepted_and_echoed_exactly() {
        let p = decode(r#"29007199254740991["x"]"#).unwrap().unwrap();
        assert_eq!(p, event("/", Some(9_007_199_254_740_991), "x", vec![]));
        assert_eq!(enc(&p).text, r#"29007199254740991["x"]"#);
    }

    #[test]
    fn a_binary_packet_is_reassembled_in_place() {
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(r#"52-/x,9["up",{"a":{"_placeholder":true,"num":1}},[{"_placeholder":true,"num":0}]]"#), Ok(None));
        assert!(d.is_reassembling());
        assert_eq!(d.awaited(), 2);
        assert!(d.pending_cost() > 0);
        assert_eq!(d.add_binary(Bytes::from_static(b"zero")), Ok(None));
        let p = d.add_binary(Bytes::from_static(b"one")).unwrap().unwrap();
        assert!(!d.is_reassembling());
        let Body::Event { id, name, args } = p.body else { panic!() };
        assert_eq!((p.nsp.as_str(), id, name.as_str()), ("/x", Some(9), "up"));
        assert_eq!(args[0].get("a").and_then(IoValue::as_binary).unwrap().as_bytes(), b"one");
        assert_eq!(args[1].as_array().unwrap()[0].as_binary().unwrap().as_bytes(), b"zero");
        // A binary acknowledgement.
        assert_eq!(d.add_text(r#"61-4[{"_placeholder":true,"num":0}]"#), Ok(None));
        let p = d.add_binary(Bytes::from_static(b"x")).unwrap().unwrap();
        assert!(matches!(p.body, Body::Ack { id: 4, .. }));
    }

    #[test]
    fn a_repeated_placeholder_shares_one_buffer() {
        let mut d = Decoder::new(limits());
        let refs = vec![r#"{"_placeholder":true,"num":0}"#; 10_000].join(",");
        assert_eq!(d.add_text(&format!(r#"51-["x",[{refs}]]"#)), Ok(None));
        let buffer = Bytes::from(vec![7u8; 500_000]);
        let p = d.add_binary(buffer.clone()).unwrap().unwrap();
        let Body::Event { args, .. } = p.body else { panic!() };
        assert_eq!(args.len(), 1);
        let values = args[0].as_array().unwrap();
        assert_eq!(values.len(), 10_000);
        for arg in values {
            // Every argument points at the same allocation: no copy of the 500 kB.
            assert_eq!(arg.as_binary().unwrap().bytes().as_ptr(), buffer.as_ptr());
        }
    }

    #[test]
    fn placeholders_nested_in_objects_and_arrays_are_restored_and_unused_attachments_are_fine() {
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(r#"52-["x",{"a":[{"b":{"_placeholder":true,"num":1}}]}]"#), Ok(None));
        d.add_binary(Bytes::from_static(b"unused")).unwrap();
        let p = d.add_binary(Bytes::from_static(b"used")).unwrap().unwrap();
        let Body::Event { args, .. } = p.body else { panic!() };
        let inner = &args[0].get("a").unwrap().as_array().unwrap()[0];
        assert_eq!(inner.get("b").and_then(IoValue::as_binary).unwrap().as_bytes(), b"used");
    }

    #[test]
    fn illegal_placeholders_fail_when_the_last_attachment_arrives() {
        for bad in [
            r#"{"_placeholder":true,"num":1}"#,
            r#"{"_placeholder":true,"num":-1}"#,
            r#"{"_placeholder":true,"num":0.5}"#,
            r#"{"_placeholder":true,"num":"0"}"#,
            r#"{"_placeholder":true}"#,
            r#"{"_placeholder":true,"num":18446744073709551616}"#,
            r#"{"_placeholder":true,"num":1e300}"#,
        ] {
            let mut d = Decoder::new(limits());
            assert_eq!(d.add_text(&format!(r#"51-["x",{bad}]"#)), Ok(None), "{bad}");
            assert_eq!(d.add_binary(Bytes::from_static(b"b")), Err(DecodeError::IllegalPlaceholder), "{bad}");
        }
        // A placeholder that is not `true` is plain data.
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(r#"51-["x",{"_placeholder":1,"num":9},{"_placeholder":true,"num":0}]"#), Ok(None));
        let Body::Event { args, .. } = d.add_binary(Bytes::from_static(b"b")).unwrap().unwrap().body else { panic!() };
        assert!(args[0].as_object().is_some() && args[1].as_binary().is_some());
        // In a non-binary event the same shape is ordinary data, as in Node.
        let Body::Event { args, .. } = decode(r#"2["x",{"_placeholder":true,"num":7}]"#).unwrap().unwrap().body else { panic!() };
        assert!(args[0].as_object().is_some());
    }

    #[test]
    fn text_while_reassembling_and_binary_without_a_header_are_errors() {
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_binary(Bytes::new()), Err(DecodeError::UnexpectedBinary));
        assert_eq!(d.add_text(r#"51-["x",{"_placeholder":true,"num":0}]"#), Ok(None));
        assert_eq!(d.add_text(r#"2["y"]"#), Err(DecodeError::UnexpectedText));
        d.reset();
        assert!(!d.is_reassembling());
        assert_eq!(d.add_binary(Bytes::new()), Err(DecodeError::UnexpectedBinary));
    }

    #[test]
    fn a_declared_count_allocates_nothing() {
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(r#"510-["x"]"#), Ok(None));
        let Some(p) = &d.pending else { panic!() };
        assert_eq!(p.expected, 10);
        assert_eq!(p.buffers.capacity(), 0, "the list grows as attachments arrive");
    }

    #[test]
    fn the_attachment_limit_is_checked_before_anything_is_accepted() {
        let mut d = Decoder::new(Limits { max_attachments: 2, ..limits() });
        assert_eq!(d.add_text(r#"53-["x"]"#), Err(DecodeError::TooManyAttachments));
        assert!(!d.is_reassembling());
        assert_eq!(d.add_text(r#"52-["x"]"#), Ok(None));
    }

    #[test]
    fn nesting_is_bounded_without_recursing_beyond_the_limit() {
        let nested = |n: usize| format!("2[\"x\",{}{}]", "[".repeat(n), "]".repeat(n));
        // The arguments array is level 1: 126 nested arrays reach 127 levels, 127 reach 128 (one too many).
        assert!(decode(&nested(126)).is_ok());
        assert_eq!(decode(&nested(127)), Err(DecodeError::TooDeep));
        assert_eq!(decode(&nested(200)), Err(DecodeError::TooDeep));
        // 100,000 unclosed levels: refused at depth 129, no stack growth.
        assert_eq!(decode(&format!("2[\"x\",{}", "[".repeat(100_000))), Err(DecodeError::TooDeep));
        let objects = format!("2[\"x\",{}1{}]", "{\"a\":".repeat(200), "}".repeat(200));
        assert_eq!(decode(&objects), Err(DecodeError::TooDeep));
    }

    #[test]
    fn empty_structures_are_charged_like_any_other_value() {
        // A packet of 1 MB of `[]` would be 500,000 arrays: with the model of 48 bytes it costs 24 MB, above the 16 MiB budget.
        let small_budget = Limits { max_decoded_bytes: 1_000_000, ..limits() };
        let arrays = vec!["[]"; 30_000].join(",");
        let text = format!("2[\"x\",{arrays}]");
        assert!(text.len() < 100_000);
        assert_eq!(Decoder::new(small_budget).add_text(&text), Err(DecodeError::TooLarge));
        let objects = vec!["{}"; 30_000].join(",");
        assert_eq!(Decoder::new(small_budget).add_text(&format!("2[\"x\",{objects}]")), Err(DecodeError::TooLarge));
        // Within the budget the same shape decodes, and the packet reports its cost.
        let p = decode(&format!("2[\"x\",{}]", vec!["[]"; 100].join(","))).unwrap().unwrap();
        assert!(p.cost() >= 100 * NODE_COST);
    }

    #[test]
    fn the_auth_payload_has_its_own_smaller_budget() {
        let limits = Limits { max_auth_bytes: 1000, ..limits() };
        let mut d = Decoder::new(limits);
        assert!(d.add_text(r#"0{"token":"abc"}"#).is_ok());
        let big = format!(r#"0{{"token":"{}"}}"#, "x".repeat(2000));
        assert_eq!(d.add_text(&big), Err(DecodeError::TooLarge));
        // An event with the same payload is not subject to it.
        assert!(d.add_text(&format!(r#"2["x","{}"]"#, "x".repeat(2000))).is_ok());
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        // The delimiters are ASCII: bytes inside multibyte characters are never indexed.
        assert_eq!(decode("2/é,[\"x\"]").unwrap().unwrap(), event("/é", None, "x", vec![]));
        assert_eq!(decode("é"), Err(DecodeError::UnknownType));
        assert_eq!(decode("2é"), Err(DecodeError::InvalidPayload));
        assert_eq!(decode("5é-"), Err(DecodeError::IllegalAttachments));
        assert_eq!(decode("2/ééé"), Err(DecodeError::InvalidPayload));
        assert_eq!(decode("21é"), Err(DecodeError::InvalidPayload));
    }

    #[test]
    fn the_parser_does_not_depend_on_serde_json_features() {
        // `raw_value` is on in the dev build: the magic key of `RawValue` is plain data to the decoder.
        let p = decode(r#"2["x",{"$serde_json::private::RawValue":"[[[[[[["}]"#).unwrap().unwrap();
        let Body::Event { args, .. } = p.body else { panic!() };
        assert_eq!(args[0].get("$serde_json::private::RawValue").and_then(IoValue::as_str), Some("[[[[[[["));
        // And a duplicate key keeps the last value in the first position, like JSON.parse.
        let p = decode(r#"2["x",{"a":1,"b":2,"a":3}]"#).unwrap().unwrap();
        let Body::Event { args, .. } = p.body else { panic!() };
        assert_eq!(args[0].to_json().unwrap().to_string(), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn encoded_packets_decode_back() {
        let packets = vec![
            event("/", None, "hi", vec![v(json!({ "a": [1, 2.5, "x", null, true] }))]),
            event("/ns", Some(3), "hi", vec![]),
            Packet { nsp: "/".into(), body: Body::Ack { id: 9, args: vec![v(json!("é\u{1e}"))] } },
            Packet { nsp: "/z".into(), body: Body::Connect(Some(v(json!({ "sid": "s" })))) },
            Packet { nsp: "/z".into(), body: Body::Disconnect },
        ];
        for p in packets {
            let encoded = enc(&p);
            assert_eq!(Decoder::new(limits()).add_text(&encoded.text).unwrap().unwrap(), p, "{}", encoded.text);
        }
        let bin = event("/b", Some(1), "up", vec![IoValue::object([("data", IoValue::from(Bytes::from_static(b"\x00\x01")))])]);
        let encoded = enc(&bin);
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(&encoded.text), Ok(None));
        assert_eq!(d.add_binary(encoded.attachments[0].bytes()).unwrap().unwrap(), bin);
    }
}

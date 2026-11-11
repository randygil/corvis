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
use std::sync::Arc;

use bytes::Bytes;
use indexmap::IndexMap;
use serde::de::{self, DeserializeSeed, Visitor};

use super::engine::session::UNREAD_MESSAGE_OVERHEAD;
use super::options::IoOptions;
use super::value::{ALLOC_OVERHEAD, Binary, ENTRY, IoValue, MAP_BASE, MAP_TABLE, MAX_DEPTH, NUMBER_HEAP, SLOT, text_cost};

/// The event names a client may not send and the server may not emit (`RESERVED_EVENTS` of `socket.io-parser`).
pub(crate) const RESERVED_EVENTS: [&str; 6] = ["connect", "connect_error", "disconnect", "disconnecting", "newListener", "removeListener"];

/// The largest acknowledgement id (`Number.MAX_SAFE_INTEGER`): beyond it a JavaScript client's id is not exact.
pub(crate) const MAX_ACK_ID: u64 = (1 << 53) - 1;

/// The longest namespace a packet may carry; registered namespaces are at most this long.
pub(crate) const MAX_NAMESPACE_LEN: usize = 256;

/// What a decoded packet costs besides its namespace, name and values: the packet itself, moved into the task of
/// its handler.
const PACKET_COST: usize = std::mem::size_of::<Packet>() + ALLOC_OVERHEAD;

pub(crate) fn is_reserved(name: &str) -> bool {
    RESERVED_EVENTS.contains(&name)
}

/// One Socket.IO packet, binary attachments already restored in place of their placeholders. Opaque: an adapter
/// receives it before it is encoded and passes it on.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
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

/// What the arguments of a packet cost: their buffer and the values in it.
fn args_cost(args: &[IoValue], capacity: usize) -> usize {
    let buffer = if capacity > 0 { ALLOC_OVERHEAD + (capacity - args.len()) * SLOT } else { 0 };
    args.iter().map(IoValue::cost).fold(buffer, usize::saturating_add)
}

impl Packet {
    /// What the decoded packet costs in memory (the model of `value.rs`): the most its decoded structure can occupy.
    pub(crate) fn cost(&self) -> usize {
        let values = match &self.body {
            Body::Connect(v) => v.as_ref().map_or(0, IoValue::cost),
            Body::Disconnect => 0,
            Body::Event { name, args, .. } => text_cost(name.capacity()).saturating_add(args_cost(args, args.capacity())),
            Body::Ack { args, .. } => args_cost(args, args.capacity()),
            Body::ConnectError(v) => v.cost(),
        };
        PACKET_COST.saturating_add(text_cost(self.nsp.capacity())).saturating_add(values)
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
    /// What a session may hold unread, and the largest message the transport accepts: a binary packet whose header and
    /// attachments could not fit together would pause the reads that complete it.
    pub(crate) max_unread_bytes: usize,
    pub(crate) max_message_bytes: usize,
}

impl Limits {
    pub(crate) fn from_options(options: &IoOptions) -> Limits {
        Limits {
            max_attachments: options.max_attachments,
            max_decoded_bytes: options.max_decoded_bytes,
            max_auth_bytes: options.max_auth_bytes,
            max_event_name_len: options.max_event_name_len,
            max_args: options.max_args,
            max_unread_bytes: options.engine.max_unread_bytes,
            max_message_bytes: options.engine.max_http_buffer_size,
        }
    }

    /// `true` when a binary packet whose header costs `header` and that declares `attachments` can be reassembled
    /// inside the session's unread limit: the header's cost and every attachment (at its largest) are held together.
    fn fits_reassembly(&self, header: usize, attachments: usize) -> bool {
        let each = self.max_message_bytes.saturating_add(UNREAD_MESSAGE_OVERHEAD);
        header.saturating_add(attachments.saturating_mul(each)) <= self.max_unread_bytes
    }
}

// --------------------------------------------------------------------------------------------------- encoding

/// An encoded packet: one text message, then its attachments (each a binary message).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Encoded {
    /// The Engine.IO message: its type `4`, then the Socket.IO packet. The engine queues it as it is
    /// (`EngineMessage::Framed`), so neither the queue nor the transport copies it to put the type in front.
    pub(crate) text: String,
    pub(crate) attachments: Vec<Binary>,
}

impl Encoded {
    /// The Socket.IO packet, without the Engine.IO type in front.
    #[cfg(test)]
    pub(crate) fn packet(&self) -> &str {
        &self.text[1..]
    }

    /// Moves the text into a buffer that many queues can share: one copy of the packet for a whole broadcast.
    pub(crate) fn share(self) -> SharedEncoded {
        SharedEncoded { text: Arc::from(self.text), attachments: self.attachments.into_iter().map(Binary::into_bytes).collect() }
    }
}

/// An encoded packet whose text (the Engine.IO message, as in [`Encoded`]) and attachments are reference counted, so
/// every recipient of a broadcast queues the same bytes.
#[derive(Debug, Clone)]
pub(crate) struct SharedEncoded {
    pub(crate) text: Arc<str>,
    pub(crate) attachments: Vec<Bytes>,
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

/// The binary values of `value` at `depth`, not looking below `MAX_DEPTH`: the writer refuses a value nested deeper
/// (`TooDeep`) before it gets there. Allocates nothing.
fn count_binary(value: &IoValue, depth: usize) -> usize {
    match value {
        IoValue::Binary(_) => 1,
        IoValue::Array(items) if depth < MAX_DEPTH => items.iter().map(|item| count_binary(item, depth + 1)).sum(),
        IoValue::Object(entries) if depth < MAX_DEPTH => entries.values().map(|item| count_binary(item, depth + 1)).sum(),
        _ => 0,
    }
}

/// Encodes a packet into the Engine.IO message that carries it (see [`Encoded::text`]): the header, then the JSON, in
/// one buffer. A packet whose arguments hold binary data at any depth becomes BINARY_EVENT / BINARY_ACK, with the
/// placeholders numbered depth first, as `deconstructPacket` does; more than `max_attachments` of them fail.
pub(crate) fn encode(packet: &Packet, max_attachments: usize) -> Result<Encoded, EncodeError> {
    // The attachments are counted first, so the count can be written in the header before the JSON.
    let attachments = match &packet.body {
        Body::Event { args, .. } | Body::Ack { args, .. } => args.iter().map(|arg| count_binary(arg, 1)).sum(),
        _ => 0,
    };
    let binary = attachments > 0;
    let (kind, id) = match &packet.body {
        Body::Connect(_) => (0u8, None),
        Body::Disconnect => (1, None),
        Body::Event { id, .. } => (if binary { 5 } else { 2 }, *id),
        Body::Ack { id, .. } => (if binary { 6 } else { 3 }, Some(*id)),
        Body::ConnectError(_) => (4, None),
    };
    let mut out = Vec::with_capacity(64 + packet.nsp.len());
    out.push(b'4');
    out.push(b'0' + kind);
    if binary {
        write!(out, "{attachments}-").expect("writing to a Vec cannot fail");
    }
    if packet.nsp != "/" && !packet.nsp.is_empty() {
        out.extend_from_slice(packet.nsp.as_bytes());
        out.push(b',');
    }
    if let Some(id) = id {
        write!(out, "{id}").expect("writing to a Vec cannot fail");
    }
    let mut w = Writer { out, attachments: Vec::with_capacity(attachments), max_attachments, binary };
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
    let Writer { mut out, attachments: found, .. } = w;
    debug_assert_eq!(found.len(), attachments, "the count and the writer see the same binary values");
    // The buffer grew by doubling and stays queued until it is written: slack beyond an eighth of its length is given
    // back (a shrinking reallocation, in place with the system allocator).
    if out.capacity() - out.len() > out.len() / 8 {
        out.shrink_to_fit();
    }
    // UTF-8 by construction: serde_json writes valid UTF-8, a namespace is a `String`, and the rest is ASCII.
    let text = String::from_utf8(out).expect("an encoded packet is valid UTF-8");
    Ok(Encoded { text, attachments: found })
}

// --------------------------------------------------------------------------------------------------- decoding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    Size,
    Depth,
}

/// What a first pass over the JSON of a packet learns, without building anything: an upper bound of what the decoded
/// value costs, and how many elements each container has, so every buffer is allocated once at its exact size.
#[derive(Debug)]
struct Measured {
    /// An upper bound of the cost of the decoded value (escapes and repeated keys only make the real value smaller).
    bound: usize,
    /// The number of elements (array) or entries (object) of every container, in the order they open.
    sizes: Vec<u32>,
}

/// Measures the JSON text of a packet: string-aware, bounded by `MAX_DEPTH`, with memory proportional to the number
/// of containers (at most four bytes per `[` or `{` of the text). It does not validate; the parser that follows
/// does, and fails on anything this pass counted wrongly.
fn measure(json: &str) -> Result<Measured, DecodeError> {
    struct Frame {
        id: usize,
        is_object: bool,
        commas: usize,
        nonempty: bool,
    }
    let b = json.as_bytes();
    let openers = memchr::memchr2_iter(b'[', b'{', b).count();
    // A container takes two bytes of text at least (`[]`), whatever the brackets inside strings add to the count.
    let mut sizes: Vec<u32> = Vec::with_capacity(openers.min(b.len() / 2 + 1));
    let mut stack: Vec<Frame> = Vec::new();
    let (mut values, mut objects, mut full_objects, mut entries, mut full_arrays) = (1usize, 0usize, 0usize, 0usize, 0usize);
    let (mut string_bytes, mut full_strings, mut tokens) = (0usize, 0usize, 0usize);
    let mut in_token = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => {
                in_token = false;
                if let Some(top) = stack.last_mut() {
                    top.nonempty = true;
                }
                let start = i + 1;
                i = start;
                loop {
                    let Some(k) = memchr::memchr2(b'"', b'\\', &b[i..]) else { return Err(DecodeError::InvalidPayload) };
                    i += k;
                    if b[i] == b'"' {
                        break;
                    }
                    // A backslash: the next byte belongs to the escape.
                    i += 2;
                    if i > b.len() {
                        return Err(DecodeError::InvalidPayload);
                    }
                }
                let len = i - start;
                string_bytes = string_bytes.saturating_add(len);
                if len > 0 {
                    full_strings += 1;
                }
                i += 1;
            }
            b'[' | b'{' => {
                in_token = false;
                if stack.len() + 1 > MAX_DEPTH {
                    return Err(DecodeError::TooDeep);
                }
                if let Some(top) = stack.last_mut() {
                    top.nonempty = true;
                }
                sizes.push(0);
                stack.push(Frame { id: sizes.len() - 1, is_object: c == b'{', commas: 0, nonempty: false });
                i += 1;
            }
            b']' | b'}' => {
                in_token = false;
                let Some(frame) = stack.pop() else { return Err(DecodeError::InvalidPayload) };
                let count = if frame.nonempty { frame.commas + 1 } else { 0 };
                sizes[frame.id] = u32::try_from(count).unwrap_or(u32::MAX);
                values = values.saturating_add(count);
                if frame.is_object {
                    objects += 1;
                    entries = entries.saturating_add(count);
                    if count > 0 {
                        full_objects += 1;
                    }
                } else if count > 0 {
                    full_arrays += 1;
                }
                i += 1;
            }
            b',' => {
                in_token = false;
                if let Some(top) = stack.last_mut() {
                    top.commas += 1;
                }
                i += 1;
            }
            b' ' | b'\t' | b'\r' | b'\n' | b':' => {
                in_token = false;
                i += 1;
            }
            _ => {
                if !in_token {
                    in_token = true;
                    tokens += 1;
                }
                if let Some(top) = stack.last_mut() {
                    top.nonempty = true;
                }
                i += 1;
            }
        }
    }
    if !stack.is_empty() {
        return Err(DecodeError::InvalidPayload);
    }
    let bound = SLOT
        .saturating_mul(values)
        .saturating_add(MAP_BASE.saturating_mul(objects))
        .saturating_add(MAP_TABLE.saturating_mul(full_objects))
        .saturating_add(ENTRY.saturating_mul(entries))
        .saturating_add(ALLOC_OVERHEAD.saturating_mul(full_arrays))
        .saturating_add(string_bytes)
        .saturating_add(ALLOC_OVERHEAD.saturating_mul(full_strings))
        .saturating_add(NUMBER_HEAP.saturating_mul(tokens));
    sizes.shrink_to_fit();
    Ok(Measured { bound, sizes })
}

struct Budget {
    remaining: Cell<usize>,
    fail: Cell<Option<Fail>>,
    sizes: Vec<u32>,
    next: Cell<usize>,
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

    /// The element count the first pass found for the container that opens now, never more than the remaining
    /// budget could pay for at `per` bytes each.
    fn next_size(&self, per: usize) -> usize {
        let i = self.next.get();
        self.next.set(i + 1);
        let hint = self.sizes.get(i).copied().unwrap_or(0) as usize;
        hint.min(self.remaining.get() / per.max(1))
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
                self.0.charge::<E>(ENTRY.saturating_add(text_cost(v.len())))?;
                Ok(v.to_owned())
            }
            fn visit_string<E: de::Error>(self, mut v: String) -> Result<String, E> {
                self.0.charge::<E>(ENTRY.saturating_add(text_cost(v.len())))?;
                v.shrink_to_fit();
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
    fn container<E: de::Error>(&self, extra: usize) -> Result<(), E> {
        if self.depth + 1 > MAX_DEPTH {
            self.budget.fail.set(Some(Fail::Depth));
            return Err(E::custom("JSON nested too deeply"));
        }
        self.budget.charge::<E>(SLOT.saturating_add(extra))
    }
}

impl<'de> Visitor<'de> for Visit<'_> {
    type Value = IoValue;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT)?;
        Ok(IoValue::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT + NUMBER_HEAP)?;
        Ok(IoValue::from(v))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT + NUMBER_HEAP)?;
        Ok(IoValue::from(v))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT + NUMBER_HEAP)?;
        Ok(IoValue::from(v))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT.saturating_add(text_cost(v.len())))?;
        Ok(IoValue::String(v.to_owned()))
    }
    fn visit_string<E: de::Error>(self, mut v: String) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT.saturating_add(text_cost(v.len())))?;
        v.shrink_to_fit();
        Ok(IoValue::String(v))
    }
    fn visit_unit<E: de::Error>(self) -> Result<IoValue, E> {
        self.budget.charge::<E>(SLOT)?;
        Ok(IoValue::Null)
    }
    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<IoValue, A::Error> {
        self.container::<A::Error>(0)?;
        let hint = self.budget.next_size(SLOT);
        if hint > 0 {
            self.budget.charge::<A::Error>(ALLOC_OVERHEAD)?;
        }
        // The capacity comes from the first pass over this very text and is limited by what the budget can pay for.
        let mut items = Vec::with_capacity(hint);
        while let Some(item) = seq.next_element_seed(Seed { budget: self.budget, depth: self.depth + 1 })? {
            items.push(item);
        }
        if items.len() < items.capacity() {
            items.shrink_to_fit();
        }
        Ok(IoValue::Array(items))
    }
    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<IoValue, A::Error> {
        self.container::<A::Error>(MAP_BASE)?;
        let hint = self.budget.next_size(ENTRY);
        if hint > 0 {
            self.budget.charge::<A::Error>(MAP_TABLE)?;
        }
        let mut entries = IndexMap::with_capacity(hint);
        while let Some(key) = map.next_key_seed(KeySeed { budget: self.budget })? {
            let value = map.next_value_seed(Seed { budget: self.budget, depth: self.depth + 1 })?;
            entries.insert(key, value);
        }
        // A repeated key replaced an entry: give back the room that was reserved for it.
        if entries.len() < hint {
            entries.shrink_to_fit();
        }
        Ok(IoValue::Object(Box::new(entries)))
    }
}

/// Parses the JSON document of a packet, whole, spending at most what the first pass found.
fn parse_json(text: &str, measured: Measured) -> Result<IoValue, DecodeError> {
    let state = Budget { remaining: Cell::new(measured.bound), fail: Cell::new(None), sizes: measured.sizes, next: Cell::new(0) };
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

/// A text message whose header was read and whose JSON was measured, not yet decoded: what the connection reserves
/// room for before any value is built.
#[derive(Debug)]
pub(crate) struct Prepared {
    kind: u8,
    expected: usize,
    nsp: String,
    id: Option<u64>,
    /// The packet text, owned so the message can wait for room without being scanned again.
    text: String,
    payload_at: usize,
    measured: Measured,
    reservation: usize,
}

impl Prepared {
    /// The most the decode can occupy at any moment: the structure it builds, the packet around it and the
    /// element counts of the first pass. The reservation is shrunk to the packet's real cost afterwards.
    pub(crate) fn reservation(&self) -> usize {
        self.reservation
    }
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

    /// Adds a text message in one step (what the connection does in three: [`prepare`](Self::prepare), reserve,
    /// [`decode`](Self::decode)). `Ok(Some(packet))` when it is a complete packet, `Ok(None)` when it is the header
    /// of a binary packet.
    #[cfg(test)]
    pub(crate) fn add_text(&mut self, text: &str) -> Result<Option<Packet>, DecodeError> {
        let prepared = self.prepare(text.to_owned())?;
        self.decode(prepared)
    }

    /// Reads the header of a text message and measures its JSON, building nothing: the packet is refused here when
    /// its header is wrong or its decoded structure could cost more than `max_decoded_bytes`.
    pub(crate) fn prepare(&self, text: String) -> Result<Prepared, DecodeError> {
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
        // The checks that need no payload, in the order the packet types are read.
        let empty = pos == b.len();
        match kind {
            0 | 1 | 4 if id.is_some() => return Err(DecodeError::InvalidAckId),
            1 if !empty => return Err(DecodeError::InvalidPayload),
            2 | 5 if empty => return Err(DecodeError::InvalidPayload),
            3 | 6 if id.is_none() => return Err(DecodeError::InvalidAckId),
            3 | 6 if empty => return Err(DecodeError::InvalidPayload),
            _ => {}
        }
        // `pos` follows ASCII only.
        let measured = if empty { Measured { bound: 0, sizes: Vec::new() } } else { measure(&text[pos..])? };
        let json_limit =
            if kind == 0 { self.limits.max_decoded_bytes.min(self.limits.max_auth_bytes) } else { self.limits.max_decoded_bytes };
        let header = PACKET_COST.saturating_add(text_cost(nsp.len()));
        let reservation = measured.bound.saturating_add(header);
        if measured.bound > json_limit || reservation > self.limits.max_decoded_bytes {
            return Err(DecodeError::TooLarge);
        }
        // The first pass's own memory (one `u32` per container) lives until the decode ends.
        let scratch = measured.sizes.capacity().saturating_mul(std::mem::size_of::<u32>()).saturating_add(ALLOC_OVERHEAD);
        let reservation = reservation.saturating_add(scratch);
        Ok(Prepared { kind, expected, nsp, id, text, payload_at: pos, measured, reservation })
    }

    /// Decodes a prepared message. `Ok(Some(packet))` when it is a complete packet, `Ok(None)` when it is the header
    /// of a binary packet.
    pub(crate) fn decode(&mut self, prepared: Prepared) -> Result<Option<Packet>, DecodeError> {
        let Prepared { kind, expected, nsp, id, text, payload_at, measured, .. } = prepared;
        let binary = kind == 5 || kind == 6;
        let payload = &text[payload_at..];
        match kind {
            0 => {
                let auth = if payload.is_empty() {
                    None
                } else {
                    let v = parse_json(payload, measured)?;
                    if !matches!(v, IoValue::Object(_)) {
                        return Err(DecodeError::InvalidPayload);
                    }
                    Some(v)
                };
                Ok(Some(Packet { nsp, body: Body::Connect(auth) }))
            }
            1 => Ok(Some(Packet { nsp, body: Body::Disconnect })),
            4 => {
                let data = if payload.is_empty() {
                    IoValue::Null
                } else {
                    let v = parse_json(payload, measured)?;
                    if !matches!(v, IoValue::String(_) | IoValue::Object(_)) {
                        return Err(DecodeError::InvalidPayload);
                    }
                    v
                };
                Ok(Some(Packet { nsp, body: Body::ConnectError(data) }))
            }
            2 | 5 => {
                let IoValue::Array(mut items) = parse_json(payload, measured)? else { return Err(DecodeError::InvalidPayload) };
                if items.is_empty() || items.len() - 1 > self.limits.max_args {
                    return Err(if items.is_empty() { DecodeError::InvalidPayload } else { DecodeError::TooManyArguments });
                }
                let mut name = match items.remove(0) {
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
                name.shrink_to_fit();
                // Removing the name left a buffer one slot too large.
                items.shrink_to_fit();
                if binary {
                    let cost = pending_cost(&nsp, Some(&name), &items);
                    if !self.limits.fits_reassembly(cost, expected) {
                        return Err(DecodeError::TooLarge);
                    }
                    self.pending = Some(Pending { nsp, id, name: Some(name), args: items, expected, buffers: Vec::new(), cost });
                    Ok(None)
                } else {
                    Ok(Some(Packet { nsp, body: Body::Event { id, name, args: items } }))
                }
            }
            3 | 6 => {
                let Some(id) = id else { return Err(DecodeError::InvalidAckId) };
                let IoValue::Array(items) = parse_json(payload, measured)? else { return Err(DecodeError::InvalidPayload) };
                if items.len() > self.limits.max_args {
                    return Err(DecodeError::TooManyArguments);
                }
                if binary {
                    let cost = pending_cost(&nsp, None, &items);
                    if !self.limits.fits_reassembly(cost, expected) {
                        return Err(DecodeError::TooLarge);
                    }
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

/// What the packet a binary header announces costs once its attachments are in place.
fn pending_cost(nsp: &str, name: Option<&str>, args: &[IoValue]) -> usize {
    PACKET_COST
        .saturating_add(text_cost(nsp.len()))
        .saturating_add(name.map_or(0, |n| text_cost(n.len())))
        .saturating_add(args_cost(args, args.len()))
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

/// The codec against the bytes `socket.io-parser` 4.2.7 produced (`fixtures/parser.json`, recorded by
/// `parity/socket-io/scripts/record-parser.mjs`), in both directions.
#[cfg(test)]
mod fixture_tests {
    use super::*;
    use serde_json::{Value, json};

    const FIXTURE: &str = include_str!("fixtures/parser.json");

    /// Inputs the npm decoder accepts and this one refuses (the codec fails closed): see the module docs of `parser`.
    const STRICTER: [&str; 7] = [
        "ack without id",
        "event without payload",
        "empty string",
        "id on connect",
        "ack id above 2^53",
        "ack id of 22 digits",
        "lone surrogate",
    ];

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn from_neutral(v: &Value) -> IoValue {
        match v {
            Value::Object(o) if o.len() == 1 && o.contains_key("$binary") => {
                IoValue::Binary(Binary::from(unhex(o["$binary"].as_str().unwrap())))
            }
            Value::Array(a) => IoValue::Array(a.iter().map(from_neutral).collect()),
            Value::Object(o) => IoValue::object(o.iter().map(|(k, v)| (k.clone(), from_neutral(v)))),
            other => IoValue::try_from(other.clone()).unwrap(),
        }
    }

    fn to_neutral(v: &IoValue) -> Value {
        match v {
            IoValue::Binary(b) => json!({ "$binary": hex(b.as_bytes()) }),
            IoValue::Array(a) => Value::Array(a.iter().map(to_neutral).collect()),
            IoValue::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), to_neutral(v))).collect()),
            other => other.to_json().unwrap(),
        }
    }

    fn packet_from_case(p: &Value) -> Packet {
        let nsp = p["nsp"].as_str().unwrap().to_string();
        let id = p.get("id").and_then(Value::as_u64);
        let data = p.get("data");
        let list = || data.and_then(Value::as_array).unwrap().iter().map(from_neutral).collect::<Vec<_>>();
        let body = match p["type"].as_str().unwrap() {
            "CONNECT" => Body::Connect(data.map(from_neutral)),
            "DISCONNECT" => Body::Disconnect,
            "EVENT" => {
                let mut items = list();
                let IoValue::String(name) = items.remove(0) else { panic!("an event name") };
                Body::Event { id, name, args: items }
            }
            "ACK" => Body::Ack { id: id.unwrap(), args: list() },
            "CONNECT_ERROR" => Body::ConnectError(from_neutral(data.unwrap())),
            other => panic!("unknown type {other}"),
        };
        Packet { nsp, body }
    }

    fn packet_to_neutral(p: &Packet) -> Value {
        let mut out = serde_json::Map::new();
        out.insert("nsp".into(), json!(p.nsp));
        match &p.body {
            Body::Connect(data) => {
                out.insert("type".into(), json!("CONNECT"));
                if let Some(d) = data {
                    out.insert("data".into(), to_neutral(d));
                }
            }
            Body::Disconnect => {
                out.insert("type".into(), json!("DISCONNECT"));
            }
            Body::Event { id, name, args } => {
                out.insert("type".into(), json!("EVENT"));
                if let Some(id) = id {
                    out.insert("id".into(), json!(id));
                }
                let mut data = vec![json!(name)];
                data.extend(args.iter().map(to_neutral));
                out.insert("data".into(), Value::Array(data));
            }
            Body::Ack { id, args } => {
                out.insert("type".into(), json!("ACK"));
                out.insert("id".into(), json!(id));
                out.insert("data".into(), Value::Array(args.iter().map(to_neutral).collect()));
            }
            Body::ConnectError(data) => {
                out.insert("type".into(), json!("CONNECT_ERROR"));
                out.insert("data".into(), to_neutral(data));
            }
        }
        Value::Object(out)
    }

    /// Node keeps a numeric event name a number; this decoder writes it as text. Compare them as text.
    fn normalize(mut node: Value) -> Value {
        if node["type"] == "EVENT"
            && let Some(first) = node["data"].get_mut(0)
            && let Some(n) = first.as_i64()
        {
            *first = json!(n.to_string());
        }
        node
    }

    #[test]
    fn the_fixture_is_from_the_pinned_parser_version() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        assert_eq!(fixture["parser"], "socket.io-parser");
        assert_eq!(fixture["version"], "4.2.7");
        assert!(fixture["encode"].as_array().unwrap().len() >= 20 && fixture["decode"].as_array().unwrap().len() >= 50);
    }

    #[test]
    fn encoding_is_byte_equal_to_the_npm_encoder() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        for case in fixture["encode"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let packet = packet_from_case(&case["packet"]);
            let encoded = encode(&packet, 10).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            let expected = case["encoded"].as_array().unwrap();
            assert_eq!(encoded.packet(), expected[0].as_str().unwrap(), "{name}");
            let attachments: Vec<String> = encoded.attachments.iter().map(|a| hex(a.as_bytes())).collect();
            let wanted: Vec<String> = expected[1..].iter().map(|e| e["$binary"].as_str().unwrap().to_string()).collect();
            assert_eq!(attachments, wanted, "{name}");
        }
    }

    #[test]
    fn decoding_equals_the_npm_decoder_or_is_stricter_where_listed() {
        let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
        let mut stricter_seen = 0;
        for case in fixture["decode"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let mut decoder = Decoder::new(Limits::from_options(&IoOptions::new()));
            let mut packets = Vec::new();
            let mut failed = false;
            for frame in case["frames"].as_array().unwrap() {
                let result = match frame {
                    Value::String(text) => decoder.add_text(text),
                    other => decoder.add_binary(Bytes::from(unhex(other["$binary"].as_str().unwrap()))),
                };
                match result {
                    Ok(Some(packet)) => packets.push(packet_to_neutral(&packet)),
                    Ok(None) => {}
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            let node_failed = case.get("error").is_some();
            if node_failed {
                assert!(failed, "{name}: the npm decoder refuses this ({}) and so must this one", case["error"]);
            } else if STRICTER.contains(&name) {
                assert!(failed, "{name}: listed as stricter, but it decoded");
                stricter_seen += 1;
            } else {
                assert!(!failed, "{name}: the npm decoder accepts this");
                let expected: Vec<Value> = case["packets"].as_array().unwrap().iter().cloned().map(normalize).collect();
                assert_eq!(packets, expected, "{name}");
            }
        }
        assert_eq!(stricter_seen, STRICTER.len());
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
        assert_eq!(enc(&event("/", None, "hi", vec![])).packet(), r#"2["hi"]"#);
        assert_eq!(
            enc(&event("/", None, "hi", vec![v(json!(1)), v(json!("two")), v(json!({ "three": [3, null, true] }))])).packet(),
            r#"2["hi",1,"two",{"three":[3,null,true]}]"#
        );
        assert_eq!(enc(&event("/admin", Some(12), "x", vec![])).packet(), r#"2/admin,12["x"]"#);
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Ack { id: 0, args: vec![v(json!("a"))] } }).packet(), r#"30["a"]"#);
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Disconnect }).packet(), "1");
        assert_eq!(enc(&Packet { nsp: "/a".into(), body: Body::Disconnect }).packet(), "1/a,");
        assert_eq!(enc(&Packet { nsp: "/".into(), body: Body::Connect(Some(v(json!({ "sid": "abc" })))) }).packet(), r#"0{"sid":"abc"}"#);
        assert_eq!(enc(&Packet { nsp: "/a".into(), body: Body::Connect(None) }).packet(), "0/a,");
        assert_eq!(
            enc(&Packet { nsp: "/a".into(), body: Body::ConnectError(v(json!({ "message": "no", "data": { "x": 1 } }))) }).packet(),
            r#"4/a,{"message":"no","data":{"x":1}}"#
        );
    }

    #[test]
    fn strings_escape_like_json_stringify_and_never_hold_the_record_separator() {
        let encoded = enc(&event("/", None, "e", vec![v(json!("a\u{1e}b\u{0}\"\\\n\u{7f}\u{2028}é"))]));
        let text = encoded.packet();
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
            encoded.packet(),
            r#"53-/x,4["up",{"first":{"_placeholder":true,"num":0},"list":[{"_placeholder":true,"num":1},5]},{"_placeholder":true,"num":2}]"#
        );
        assert_eq!(encoded.attachments, vec![a, b, c]);
        let ack = enc(&Packet { nsp: "/".into(), body: Body::Ack { id: 1, args: vec![IoValue::from(Bytes::from_static(b"z"))] } });
        assert_eq!(ack.packet(), r#"61-1[{"_placeholder":true,"num":0}]"#);
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
        assert_eq!(enc(&p).packet(), r#"29007199254740991["x"]"#);
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
        // 30,000 empty arrays are 90 kB of text and cost 1.2 MB (a 40-byte slot each), above this 1 MB budget.
        let small_budget = Limits { max_decoded_bytes: 1_000_000, ..limits() };
        let arrays = vec!["[]"; 30_000].join(",");
        let text = format!("2[\"x\",{arrays}]");
        assert!(text.len() < 100_000);
        assert_eq!(Decoder::new(small_budget).add_text(&text), Err(DecodeError::TooLarge));
        let objects = vec!["{}"; 30_000].join(",");
        assert_eq!(Decoder::new(small_budget).add_text(&format!("2[\"x\",{objects}]")), Err(DecodeError::TooLarge));
        // Within the budget the same shape decodes, and the packet reports its cost.
        let p = decode(&format!("2[\"x\",{}]", vec!["[]"; 100].join(","))).unwrap().unwrap();
        assert!(p.cost() >= 100 * SLOT);
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
    fn a_binary_header_that_could_not_be_reassembled_inside_the_unread_limit_is_refused_at_once() {
        // Unread limit 100 kB, attachments of up to 10 kB: a header of `n` empty objects (136 bytes each) leaves room
        // for its attachments only while it costs at most 100,000 - 2 x 10,064.
        let limits = Limits { max_unread_bytes: 100_000, max_message_bytes: 10_000, ..limits() };
        let header =
            |n: usize| format!(r#"52-["x",[{}],{{"_placeholder":true,"num":0}},{{"_placeholder":true,"num":1}}]"#, vec!["{}"; n].join(","));
        assert!(Decoder::new(limits).add_text(&header(500)).is_ok(), "64 kB of header and two attachments fit");
        assert_eq!(Decoder::new(limits).add_text(&header(700)), Err(DecodeError::TooLarge), "90 kB of header leaves no room for them");
    }

    #[test]
    fn a_lone_surrogate_is_refused_and_a_pair_of_them_is_a_character() {
        // `JSON.parse` keeps `"\ud800"` (JavaScript strings are UTF-16); a Rust string cannot hold it, so the packet
        // is undecodable and the connection closes: stricter than Node, fail closed (a listed expected difference).
        assert_eq!(decode(r#"2["x","\ud800"]"#), Err(DecodeError::InvalidPayload));
        assert_eq!(decode(r#"2["x","\udc00"]"#), Err(DecodeError::InvalidPayload));
        assert_eq!(decode(r#"2["x",{"\ud800":1}]"#), Err(DecodeError::InvalidPayload));
        assert_eq!(decode(r#"2["x","\ud800A"]"#), Err(DecodeError::InvalidPayload));
        assert_eq!(decode(r#"0{"token":"\ud800"}"#), Err(DecodeError::InvalidPayload));
        let pair = decode(r#"2["x","😀"]"#).unwrap().unwrap();
        assert_eq!(pair, event("/", None, "x", vec![IoValue::from("\u{1F600}")]));
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
            assert_eq!(Decoder::new(limits()).add_text(encoded.packet()).unwrap().unwrap(), p, "{}", encoded.packet());
        }
        let bin = event("/b", Some(1), "up", vec![IoValue::object([("data", IoValue::from(Bytes::from_static(b"\x00\x01")))])]);
        let encoded = enc(&bin);
        let mut d = Decoder::new(limits());
        assert_eq!(d.add_text(encoded.packet()), Ok(None));
        assert_eq!(d.add_binary(encoded.attachments[0].bytes()).unwrap().unwrap(), bin);
    }
}

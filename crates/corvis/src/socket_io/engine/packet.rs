//! The Engine.IO v4 packet and payload codec (`engine.io-parser` 5.x).
//!
//! A packet is a type digit `0`-`6` followed by its data. On long-polling, packets are joined with the record
//! separator `\x1e` and a binary message is `b` followed by its base64; on a WebSocket every packet is its own
//! frame and a binary message is a binary frame without type byte.

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig, general_purpose::STANDARD};
use std::sync::Arc;

use bytes::Bytes;

/// The record separator of a polling payload.
pub const SEPARATOR: char = '\x1e';

/// Decodes base64 with or without padding, as `engine.io-parser` does.
const LENIENT: GeneralPurpose =
    GeneralPurpose::new(&base64::alphabet::STANDARD, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent));

/// The payload of a `message` packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineMessage {
    Text(String),
    Binary(Bytes),
    /// A text message whose bytes are shared with other queues: what a broadcast queues on every recipient, so a
    /// broadcast holds one copy of its text however many sockets it reaches. Only ever sent, never received.
    Shared(Arc<str>),
}

impl EngineMessage {
    /// Bytes of the payload.
    pub fn len(&self) -> usize {
        match self {
            EngineMessage::Text(t) => t.len(),
            EngineMessage::Binary(b) => b.len(),
            EngineMessage::Shared(t) => t.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<String> for EngineMessage {
    fn from(v: String) -> Self {
        EngineMessage::Text(v)
    }
}

impl From<&str> for EngineMessage {
    fn from(v: &str) -> Self {
        EngineMessage::Text(v.to_string())
    }
}

impl From<Bytes> for EngineMessage {
    fn from(v: Bytes) -> Self {
        EngineMessage::Binary(v)
    }
}

impl From<Vec<u8>> for EngineMessage {
    fn from(v: Vec<u8>) -> Self {
        EngineMessage::Binary(Bytes::from(v))
    }
}

/// An Engine.IO packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    /// `0`, with the JSON handshake data.
    Open(String),
    /// `1`.
    Close,
    /// `2`, with the data (`probe` during an upgrade, empty otherwise).
    Ping(String),
    /// `3`, with the data (`probe` during an upgrade, empty otherwise).
    Pong(String),
    /// `4`.
    Message(EngineMessage),
    /// `5`.
    Upgrade,
    /// `6`.
    Noop,
}

/// What a received frame or payload record is: a packet, or something that is not one (Node's `error` packet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    Packet(Packet),
    Invalid,
}

/// A WebSocket frame body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsFrame {
    Text(String),
    Binary(Bytes),
}

impl Packet {
    fn type_digit(&self) -> char {
        match self {
            Packet::Open(_) => '0',
            Packet::Close => '1',
            Packet::Ping(_) => '2',
            Packet::Pong(_) => '3',
            Packet::Message(_) => '4',
            Packet::Upgrade => '5',
            Packet::Noop => '6',
        }
    }

    /// Length in bytes of the polling encoding of the packet (without separator).
    pub fn polling_len(&self) -> usize {
        match self {
            Packet::Open(d) | Packet::Ping(d) | Packet::Pong(d) => 1 + d.len(),
            Packet::Message(EngineMessage::Text(t)) => 1 + t.len(),
            Packet::Message(EngineMessage::Shared(t)) => 1 + t.len(),
            Packet::Message(EngineMessage::Binary(b)) => 1 + b.len().div_ceil(3) * 4,
            Packet::Close | Packet::Upgrade | Packet::Noop => 1,
        }
    }

    /// Appends the polling encoding of the packet.
    pub fn encode_polling(&self, out: &mut String) {
        match self {
            Packet::Message(EngineMessage::Binary(b)) => {
                out.push('b');
                STANDARD.encode_string(b, out);
            }
            Packet::Message(EngineMessage::Text(t)) => {
                out.push('4');
                out.push_str(t);
            }
            Packet::Message(EngineMessage::Shared(t)) => {
                out.push('4');
                out.push_str(t);
            }
            Packet::Open(d) | Packet::Ping(d) | Packet::Pong(d) => {
                out.push(self.type_digit());
                out.push_str(d);
            }
            Packet::Close | Packet::Upgrade | Packet::Noop => out.push(self.type_digit()),
        }
    }

    /// The WebSocket frame of the packet.
    pub fn encode_ws(&self) -> WsFrame {
        match self {
            Packet::Message(EngineMessage::Binary(b)) => WsFrame::Binary(b.clone()),
            other => {
                let mut s = String::with_capacity(other.polling_len());
                other.encode_polling(&mut s);
                WsFrame::Text(s)
            }
        }
    }
}

/// Decodes one packet from text: a type digit and data, or `b` and base64.
pub fn decode_text(text: &str) -> Incoming {
    let mut chars = text.chars();
    let Some(first) = chars.next() else { return Incoming::Invalid };
    let rest = chars.as_str();
    let packet = match first {
        'b' => match LENIENT.decode(rest) {
            Ok(bytes) => Packet::Message(EngineMessage::Binary(Bytes::from(bytes))),
            Err(_) => return Incoming::Invalid,
        },
        '0' => Packet::Open(rest.to_string()),
        '1' => Packet::Close,
        '2' => Packet::Ping(rest.to_string()),
        '3' => Packet::Pong(rest.to_string()),
        '4' => Packet::Message(EngineMessage::Text(rest.to_string())),
        '5' => Packet::Upgrade,
        '6' => Packet::Noop,
        _ => return Incoming::Invalid,
    };
    Incoming::Packet(packet)
}

/// Decodes one WebSocket frame.
pub fn decode_frame(frame: WsFrame) -> Incoming {
    match frame {
        WsFrame::Text(t) => decode_text(&t),
        WsFrame::Binary(b) => Incoming::Packet(Packet::Message(EngineMessage::Binary(b))),
    }
}

/// The number of records of a payload: its separators plus one. Does not allocate.
pub fn count_records(payload: &str) -> usize {
    memchr::memchr_iter(SEPARATOR as u8, payload.as_bytes()).count() + 1
}

/// A payload has more records than allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooManyPackets;

/// Decodes a polling payload, stopping after the first record that is not a packet (which is included, as
/// `engine.io-parser` does). Fails before decoding anything when it holds more than `max_packets` records.
pub fn decode_payload(payload: &str, max_packets: usize) -> Result<Vec<Incoming>, TooManyPackets> {
    if count_records(payload) > max_packets {
        return Err(TooManyPackets);
    }
    let mut out = Vec::new();
    for record in payload.split(SEPARATOR) {
        let decoded = decode_text(record);
        let invalid = decoded == Incoming::Invalid;
        out.push(decoded);
        if invalid {
            break;
        }
    }
    Ok(out)
}

/// Encodes packets as a polling payload, joined with the record separator.
pub fn encode_payload<'a>(packets: impl IntoIterator<Item = &'a Packet>) -> String {
    let mut out = String::new();
    for (i, packet) in packets.into_iter().enumerate() {
        if i > 0 {
            out.push(SEPARATOR);
        }
        packet.encode_polling(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Packet {
        Packet::Message(EngineMessage::Text(s.to_string()))
    }

    #[test]
    fn encodes_every_packet_type() {
        let cases: Vec<(Packet, &str)> = vec![
            (Packet::Open("{\"sid\":\"x\"}".into()), "0{\"sid\":\"x\"}"),
            (Packet::Close, "1"),
            (Packet::Ping(String::new()), "2"),
            (Packet::Ping("probe".into()), "2probe"),
            (Packet::Pong(String::new()), "3"),
            (Packet::Pong("probe".into()), "3probe"),
            (text("hello"), "4hello"),
            (text(""), "4"),
            (Packet::Upgrade, "5"),
            (Packet::Noop, "6"),
        ];
        for (packet, wire) in cases {
            let mut out = String::new();
            packet.encode_polling(&mut out);
            assert_eq!(out, wire);
            assert_eq!(packet.polling_len(), wire.len());
        }
    }

    #[test]
    fn binary_is_b_plus_base64_on_polling_and_a_raw_frame_on_websocket() {
        let p = Packet::Message(EngineMessage::Binary(Bytes::from_static(&[1, 2, 3, 4])));
        let mut out = String::new();
        p.encode_polling(&mut out);
        assert_eq!(out, "bAQIDBA==");
        assert_eq!(p.polling_len(), out.len());
        assert_eq!(p.encode_ws(), WsFrame::Binary(Bytes::from_static(&[1, 2, 3, 4])));
        assert_eq!(text("a").encode_ws(), WsFrame::Text("4a".into()));
    }

    #[test]
    fn decodes_text_packets() {
        assert_eq!(decode_text("2probe"), Incoming::Packet(Packet::Ping("probe".into())));
        assert_eq!(decode_text("3"), Incoming::Packet(Packet::Pong(String::new())));
        assert_eq!(decode_text("4héllo"), Incoming::Packet(text("héllo")));
        assert_eq!(decode_text("5"), Incoming::Packet(Packet::Upgrade));
        assert_eq!(decode_text("bAQIDBA=="), Incoming::Packet(Packet::Message(EngineMessage::Binary(Bytes::from_static(&[1, 2, 3, 4])))));
        // Unpadded base64 is accepted too.
        assert_eq!(decode_text("bAQIDBA"), decode_text("bAQIDBA=="));
        assert_eq!(decode_text(""), Incoming::Invalid);
        assert_eq!(decode_text("7"), Incoming::Invalid);
        assert_eq!(decode_text("x"), Incoming::Invalid);
        assert_eq!(decode_text("b***"), Incoming::Invalid);
    }

    #[test]
    fn frames_decode_like_text_or_binary_messages() {
        assert_eq!(decode_frame(WsFrame::Text("4a".into())), Incoming::Packet(text("a")));
        assert_eq!(
            decode_frame(WsFrame::Binary(Bytes::from_static(b"\x00\x01"))),
            Incoming::Packet(Packet::Message(EngineMessage::Binary(Bytes::from_static(b"\x00\x01"))))
        );
    }

    #[test]
    fn payloads_split_on_the_record_separator() {
        let decoded = decode_payload("4a\x1e4b\x1ebAQID\x1e2", 10).unwrap();
        assert_eq!(
            decoded,
            vec![
                Incoming::Packet(text("a")),
                Incoming::Packet(text("b")),
                Incoming::Packet(Packet::Message(EngineMessage::Binary(Bytes::from_static(&[1, 2, 3])))),
                Incoming::Packet(Packet::Ping(String::new())),
            ]
        );
    }

    #[test]
    fn a_payload_stops_at_its_first_invalid_record() {
        let decoded = decode_payload("4a\x1e9\x1e4never", 10).unwrap();
        assert_eq!(decoded, vec![Incoming::Packet(text("a")), Incoming::Invalid]);
        assert_eq!(decode_payload("", 10).unwrap(), vec![Incoming::Invalid]);
    }

    #[test]
    fn the_record_count_is_checked_before_decoding() {
        let body = ["4"; 5].join("\x1e");
        assert_eq!(count_records(&body), 5);
        assert!(decode_payload(&body, 5).is_ok());
        assert_eq!(decode_payload(&body, 4), Err(TooManyPackets));
        // One byte past the limit is refused even when the first record is invalid.
        assert_eq!(decode_payload("9\x1e4\x1e4", 2), Err(TooManyPackets));
    }

    /// Output of `engine.io-parser` 5.x for these packets (`parity/socket-io/scripts/record-parser-vectors.mjs`).
    const NODE_PAYLOAD: &str = "4hello\x1e4héllo €\x1ebAQIDBA==\x1eb/wAH\x1e2probe\x1e3probe\x1e5\x1e6\x1e1";

    #[test]
    fn matches_the_bytes_recorded_from_the_node_parser() {
        let packets = vec![
            text("hello"),
            text("héllo €"),
            Packet::Message(EngineMessage::Binary(Bytes::from_static(&[1, 2, 3, 4]))),
            Packet::Message(EngineMessage::Binary(Bytes::from_static(&[255, 0, 7]))),
            Packet::Ping("probe".into()),
            Packet::Pong("probe".into()),
            Packet::Upgrade,
            Packet::Noop,
            Packet::Close,
        ];
        assert_eq!(encode_payload(&packets), NODE_PAYLOAD);
        let decoded: Vec<Incoming> = packets.into_iter().map(Incoming::Packet).collect();
        assert_eq!(decode_payload(NODE_PAYLOAD, 20).unwrap(), decoded);
    }

    #[test]
    fn payloads_round_trip() {
        let packets =
            vec![text("a"), Packet::Noop, Packet::Message(EngineMessage::Binary(Bytes::from_static(&[255, 0, 7]))), Packet::Close];
        let wire = encode_payload(&packets);
        let back: Vec<Packet> = decode_payload(&wire, 10)
            .unwrap()
            .into_iter()
            .map(|i| match i {
                Incoming::Packet(p) => p,
                Incoming::Invalid => panic!("invalid"),
            })
            .collect();
        assert_eq!(back, packets);
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    /// What is kept of a received message must not be larger than it is charged (its length plus the accounting
    /// overhead): a text message is copied to a buffer of its own length, never a slice of the payload it came in.
    #[test]
    fn text_messages_taken_from_a_payload_own_exactly_their_bytes() {
        let big = "x".repeat(100_000);
        let payload = format!("4{big}\x1e4a\x1e4{big}\x1e4bc");
        let decoded = decode_payload(&payload, 10).unwrap();
        assert_eq!(decoded.len(), 4);
        for item in decoded {
            let Incoming::Packet(Packet::Message(EngineMessage::Text(text))) = item else { panic!("a text message") };
            assert_eq!(text.capacity(), text.len(), "a {}-byte message holds a buffer of {}", text.len(), text.capacity());
        }
        let Incoming::Packet(Packet::Message(EngineMessage::Text(text))) = decode_text(&format!("4{big}")) else { panic!() };
        assert_eq!(text.capacity(), text.len());
    }

    #[test]
    fn a_base64_message_keeps_at_most_two_bytes_more_than_it_decodes_to() {
        // `Bytes` does not expose its capacity: the decoder's own bound is checked on the vector it builds.
        for n in [0usize, 1, 2, 3, 4, 5, 100, 1_001] {
            let data = vec![9u8; n];
            for padded in [true, false] {
                let mut wire = String::new();
                STANDARD.encode_string(&data, &mut wire);
                if !padded {
                    wire = wire.trim_end_matches('=').to_string();
                }
                let v = LENIENT.decode(&wire).unwrap();
                assert_eq!(v.len(), n);
                assert!(v.capacity() - v.len() <= 2, "{} bytes in a buffer of {}", v.len(), v.capacity());
            }
        }
    }
}

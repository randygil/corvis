//! Property-based fuzzing of the Engine.IO codec and session ids. The module name keeps these properties inside
//! `cargo test -p corvis --features socket-io --lib fuzz`. More cases: `PROPTEST_CASES=20000`.
//!
//! The properties are the ones a hostile client attacks: no panic on any input, output proportional to the input,
//! and the packet count bound never exceeded.

use bytes::Bytes;
use proptest::prelude::*;

use super::packet::{EngineMessage, Incoming, Packet, WsFrame, count_records, decode_frame, decode_payload, decode_text, encode_payload};
use super::sid;
use crate::fuzz_tests::config;

fn packet() -> impl Strategy<Value = Packet> {
    prop_oneof![
        any::<String>().prop_map(Packet::Open),
        Just(Packet::Close),
        prop::sample::select(vec!["", "probe"]).prop_map(|s| Packet::Ping(s.to_string())),
        prop::sample::select(vec!["", "probe"]).prop_map(|s| Packet::Pong(s.to_string())),
        // The record separator cannot appear inside a polling text packet.
        "[^\x1e]*".prop_map(|s| Packet::Message(EngineMessage::Text(s))),
        prop::collection::vec(any::<u8>(), 0..64).prop_map(|b| Packet::Message(EngineMessage::Binary(Bytes::from(b)))),
        Just(Packet::Upgrade),
        Just(Packet::Noop),
    ]
}

/// Fragments that make payloads that are often valid, or almost.
fn soup() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "0", "1", "2", "3", "4", "5", "6", "7", "b", "bAQID", "probe", "\x1e", "{}", "é", " ", "=", "***", "4hello",
        ]),
        0..40,
    )
    .prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(config(512))]

    #[test]
    fn decoding_never_panics_on_any_text(input in any::<String>(), max in 1usize..64) {
        let _ = decode_text(&input);
        let _ = decode_payload(&input, max);
        let _ = decode_frame(WsFrame::Text(input));
    }

    #[test]
    fn decoding_never_panics_on_a_soup_of_fragments(input in soup(), max in 1usize..64) {
        let _ = decode_text(&input);
        let _ = decode_payload(&input, max);
    }

    #[test]
    fn decoding_never_panics_on_any_frame_bytes(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode_frame(WsFrame::Binary(Bytes::from(bytes.clone())));
        let _ = decode_payload(&String::from_utf8_lossy(&bytes), 100);
    }

    #[test]
    fn a_payload_round_trips(packets in prop::collection::vec(packet(), 1..20)) {
        // An open packet whose JSON holds the separator cannot appear in a payload either.
        prop_assume!(packets.iter().all(|p| match p {
            Packet::Open(d) => !d.contains('\x1e'),
            _ => true,
        }));
        let wire = encode_payload(&packets);
        let decoded = decode_payload(&wire, 100).unwrap();
        let back: Vec<Packet> = decoded
            .into_iter()
            .map(|i| match i {
                Incoming::Packet(p) => p,
                Incoming::Invalid => panic!("invalid record in {wire:?}"),
            })
            .collect();
        prop_assert_eq!(back, packets);
    }

    #[test]
    fn decoded_size_is_proportional_to_the_input(input in soup(), max in 1usize..64) {
        if let Ok(decoded) = decode_payload(&input, max) {
            let total: usize = decoded
                .iter()
                .map(|i| match i {
                    Incoming::Packet(Packet::Message(m)) => m.len(),
                    Incoming::Packet(Packet::Open(d) | Packet::Ping(d) | Packet::Pong(d)) => d.len(),
                    _ => 0,
                })
                .sum();
            prop_assert!(total <= input.len());
            prop_assert!(decoded.len() <= count_records(&input));
        }
    }

    #[test]
    fn the_packet_count_bound_is_never_exceeded(input in soup(), max in 1usize..64) {
        match decode_payload(&input, max) {
            Ok(decoded) => {
                prop_assert!(decoded.len() <= max);
                prop_assert!(count_records(&input) <= max);
            }
            Err(_) => prop_assert!(count_records(&input) > max),
        }
    }

    #[test]
    fn the_shape_check_accepts_exactly_twenty_url_safe_characters(input in any::<String>()) {
        let expected = input.chars().count() == 20 && input.len() == 20 && input.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        prop_assert_eq!(sid::has_valid_shape(&input), expected);
    }
}

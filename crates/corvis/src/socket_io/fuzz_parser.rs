//! Property tests of the Socket.IO codec. `cargo test -p corvis --features socket-io --lib fuzz` runs them (the name
//! of the module selects them); `PROPTEST_CASES=20000` runs them harder.

use bytes::Bytes;
use proptest::prelude::*;

use super::options::IoOptions;
use super::parser::{Body, Decoder, Limits, Packet, encode};
use super::value::{Binary, IoValue, MAX_DEPTH};

fn limits() -> Limits {
    Limits::from_options(&IoOptions::new())
}

/// One input of the decoder: a text message or a binary one.
#[derive(Debug, Clone)]
enum Input {
    Text(String),
    Binary(Vec<u8>),
}

fn arb_input() -> impl Strategy<Value = Input> {
    prop_oneof![
        3 => any::<String>().prop_map(Input::Text),
        // Text shaped like packets, so the generator reaches the deeper checks.
        6 => (0u8..8, prop_oneof![Just(""), Just("1-"), Just("2-"), Just("11-"), Just("0-"), Just("x-")], prop_oneof![Just(""), Just("/a,"), Just("/a"), Just("/é,")], prop_oneof![Just(""), Just("0"), Just("7"), Just("99999999999999999999")], "[\\[\\]{}0-9a-z\",: _.-]{0,40}")
            .prop_map(|(t, att, nsp, id, json)| Input::Text(format!("{t}{att}{nsp}{id}{json}"))),
        2 => proptest::collection::vec(any::<u8>(), 0..64).prop_map(Input::Binary),
    ]
}

fn arb_leaf() -> impl Strategy<Value = IoValue> {
    prop_oneof![
        Just(IoValue::Null),
        any::<bool>().prop_map(IoValue::Bool),
        any::<i64>().prop_map(IoValue::from),
        any::<u32>().prop_map(IoValue::from),
        any::<String>().prop_map(IoValue::String),
        proptest::collection::vec(any::<u8>(), 0..16).prop_map(|b| IoValue::Binary(Binary::from(b))),
    ]
}

fn arb_value() -> impl Strategy<Value = IoValue> {
    arb_leaf().prop_recursive(5, 40, 5, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..5).prop_map(IoValue::Array),
            proptest::collection::vec(("[a-z_]{0,6}", inner), 0..5).prop_map(IoValue::object),
        ]
    })
}

fn arb_packet() -> impl Strategy<Value = Packet> {
    let nsp = prop_oneof![Just("/".to_string()), "/[a-z]{1,8}"];
    let args = proptest::collection::vec(arb_value(), 0..4);
    let name = "[a-zA-Z0-9_-]{1,12}".prop_filter("not a reserved name", |n| !super::parser::is_reserved(n));
    (nsp, prop_oneof![Just(None), (0u64..1_000_000).prop_map(Some)], name, args, any::<bool>(), 0u8..3).prop_map(
        |(nsp, id, name, args, is_ack, kind)| match kind {
            0 if is_ack => Packet { nsp, body: Body::Ack { id: id.unwrap_or(0), args } },
            0 | 1 => Packet { nsp, body: Body::Event { id, name, args } },
            _ => Packet { nsp, body: Body::Disconnect },
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Whatever arrives, the decoder returns a packet or an error: it never panics, and a packet it returns is
    /// within the limits.
    #[test]
    fn fuzz_arbitrary_interleavings_never_panic(inputs in proptest::collection::vec(arb_input(), 0..24)) {
        let limits = limits();
        let mut decoder = Decoder::new(limits);
        for input in inputs {
            let result = match input {
                Input::Text(t) => decoder.add_text(&t),
                Input::Binary(b) => decoder.add_binary(Bytes::from(b)),
            };
            match result {
                Ok(Some(packet)) => {
                    prop_assert!(packet.cost() <= limits.max_decoded_bytes + 256);
                    if let Body::Event { name, args, .. } = &packet.body {
                        prop_assert!(name.len() <= limits.max_event_name_len);
                        prop_assert!(args.len() <= limits.max_args);
                    }
                }
                Ok(None) => {}
                // A failure ends the connection: the next input would not exist. Start over, as a new connection.
                Err(_) => decoder = Decoder::new(limits),
            }
        }
    }

    /// Arbitrary text alone: same.
    #[test]
    fn fuzz_arbitrary_text_never_panics(text in any::<String>()) {
        let mut decoder = Decoder::new(limits());
        let _ = decoder.add_text(&text);
    }

    /// What is decoded costs a bounded multiple of the input (the model charges each value a constant).
    #[test]
    fn fuzz_decoded_size_is_proportional_to_the_input(
        body in "[\\[\\]{},0-9a-z\":. ]{0,400}",
        prefix in prop_oneof![Just("2"), Just("0"), Just("3"), Just("4"), Just("21"), Just("2/a,")],
    ) {
        let mut decoder = Decoder::new(limits());
        let text = format!("{prefix}{body}");
        if let Ok(Some(packet)) = decoder.add_text(&text) {
            // Every value takes at least one byte of input and costs a constant: 48 per value, 64 per entry.
            prop_assert!(packet.cost() <= 256 + 128 * (text.len() + 1));
        }
    }

    /// The declared attachment count never exceeds the limit, and a rejected header leaves nothing pending.
    #[test]
    fn fuzz_attachment_count_never_above_the_limit(declared in 0u64..100_000_000_000, max in 1usize..20) {
        let mut decoder = Decoder::new(Limits { max_attachments: max, ..limits() });
        let result = decoder.add_text(&format!("5{declared}-[\"x\"]"));
        match result {
            Ok(None) => {
                prop_assert!(declared >= 1 && declared as usize <= max);
                prop_assert!(decoder.is_reassembling());
                prop_assert_eq!(decoder.awaited(), declared as usize);
            }
            Ok(Some(_)) => prop_assert!(false, "a binary header is never a complete packet"),
            Err(_) => {
                prop_assert!(declared < 1 || declared as usize > max);
                prop_assert!(!decoder.is_reassembling());
            }
        }
    }

    /// Encode, then decode: the same packet, binary values restored in place; no encoded text holds the record
    /// separator and the attachment count is at most the limit.
    #[test]
    fn fuzz_round_trip(packet in arb_packet()) {
        let encoded = match encode(&packet, 10) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };
        let separator = '\u{1e}';
        prop_assert!(!encoded.text.contains(separator));
        prop_assert!(encoded.attachments.len() <= 10);
        let mut decoder = Decoder::new(limits());
        let mut result = decoder.add_text(&encoded.text).unwrap();
        for attachment in &encoded.attachments {
            prop_assert!(result.is_none());
            result = decoder.add_binary(attachment.bytes()).unwrap();
        }
        let decoded = result.expect("a complete packet");
        // Numbers may be written differently but parse to the same value.
        prop_assert_eq!(decoded, packet);
    }

    /// An encoded value never nests deeper than the bound, whatever tree it is given.
    #[test]
    fn fuzz_encoding_respects_the_depth_bound(depth in 100usize..160) {
        let mut value = IoValue::Null;
        for _ in 0..depth {
            value = IoValue::Array(vec![value]);
        }
        let packet = Packet { nsp: "/".into(), body: Body::Event { id: None, name: "e".into(), args: vec![value] } };
        match encode(&packet, 10) {
            Ok(encoded) => {
                prop_assert!(depth < MAX_DEPTH);
                prop_assert!(Decoder::new(limits()).add_text(&encoded.text).is_ok());
            }
            Err(_) => prop_assert!(depth >= MAX_DEPTH),
        }
    }
}

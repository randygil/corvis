//! Property-based fuzzing of the multipart parser (busboy) and of the field-name nesting
//! (`append_field`). Inputs are built from fragments of real multipart bodies mixed with random
//! bytes, so most cases reach deep into the parser instead of failing on the first byte.
//!
//! More cases: `PROPTEST_CASES=20000 cargo test -p corvis multer::fuzz`.

use bytes::Bytes;
use proptest::prelude::*;
use serde_json::{Map, Value};

use super::Limits;
use super::append_field::{ArrayBudget, DEFAULT_MAX_FIELD_DEPTH, MAX_ARRAY_HOLES, append_field};
use super::busboy::{self, Config, Event};

const BOUNDARY: &str = "XyZ";

fn multipart_piece() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        4 => prop::sample::select(vec![
            "--XyZ", "--XyZ--", "--XyZ\r\n", "\r\n", "\r\n\r\n", "\r", "\n", "--", "-", ":", ";", "\"", "=",
            "Content-Disposition: form-data; name=\"f\"",
            "Content-Disposition: form-data; name=\"a[b][0]\"",
            "Content-Disposition: form-data; name=\"up\"; filename=\"../../etc/passwd\"",
            "Content-Disposition: form-data; name=\"up\"; filename=\"C:\\\\x\\\\a.txt\"",
            "Content-Disposition: form-data; name*=utf-8''%e2%82%ac; filename*=UTF-8''%F0%9F%98%80.png",
            "content-disposition: FORM-DATA; NAME=x",
            "Content-Disposition: attachment",
            "Content-Type: text/plain",
            "Content-Type: application/octet-stream; charset=utf-8",
            "Content-Transfer-Encoding: 7bit",
            "X-Junk: ",
            "hello", "value",
        ])
        .prop_map(|s| s.as_bytes().to_vec()),
        1 => prop::collection::vec(any::<u8>(), 0..24),
    ]
}

fn multipart_body() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(multipart_piece(), 0..48).prop_map(|parts| parts.concat())
}

fn limits() -> impl Strategy<Value = Limits> {
    (
        prop::option::of(0u64..64),
        prop::option::of(0u64..8),
        prop::option::of(0u64..8),
        prop::option::of(0u64..8),
        prop::option::of(0u64..64),
    )
        .prop_map(|(field_size, fields, files, parts, file_size)| {
            let mut l = Limits::new();
            l.field_size = field_size;
            l.fields = fields;
            l.files = files;
            l.parts = parts;
            l.file_size = file_size;
            l
        })
}

fn field_key() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", "b", "x", "[", "]", "[]", "[0]", "[1]", "[19]", "[20]", "[999]", "[4294967294]", "[-1]", "[b]",
            "[__proto__]", "[constructor]", ".", "%5B", "]]", "[[",
        ]),
        0..40,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(crate::fuzz_tests::config(2000))]

    /// The parser never panics, every event is backed by a part delimiter, and the content it
    /// keeps never exceeds the body it was given.
    #[test]
    fn busboy_parse_is_total_and_bounded(
        body in multipart_body(),
        limits in limits(),
        preserve_path in any::<bool>(),
        // busboy accepts any string boundary, the empty one included (`boundary=""`).
        boundary in prop::sample::select(vec![BOUNDARY, "", "-", "\r\n", "X"]),
    ) {
        let cfg = Config { limits: &limits, preserve_path, def_param_charset: "latin1" };
        let events = busboy::parse(&Bytes::from(body.clone()), boundary.as_bytes(), &cfg);
        let delim = format!("--{boundary}").into_bytes();
        let delimiters = body.windows(delim.len()).filter(|w| *w == &delim[..]).count();
        let parts = events.iter().filter(|e| matches!(e, Event::Field { .. } | Event::FileStart { .. })).count();
        prop_assert!(parts <= delimiters, "{parts} parts from {delimiters} delimiters");
        let kept: usize = events
            .iter()
            .map(|e| match e {
                Event::Field { value, .. } => value.len(),
                Event::FileEnd { data, .. } => data.len(),
                _ => 0,
            })
            .sum();
        // A latin1 value can double its size when it becomes UTF-8.
        prop_assert!(kept <= 2 * body.len(), "kept {kept} bytes from a {}-byte body", body.len());
        for e in &events {
            if let Event::FileStart { filename: Some(name), .. } = e {
                if !preserve_path {
                    prop_assert!(!name.contains('/') && !name.contains('\\'), "filename kept a path: {name:?}");
                }
            }
        }
    }

    /// `Content-Type` parsing and boundary extraction accept any bytes.
    #[test]
    fn content_type_parsing_is_total(raw in prop::collection::vec(any::<u8>(), 0..96), piece in multipart_piece()) {
        let _ = busboy::parse_content_type(&raw);
        let _ = busboy::boundary_of(Some(&raw));
        let mut ct = b"multipart/form-data; boundary=".to_vec();
        ct.extend_from_slice(&piece);
        // Like busboy, an empty boundary (`boundary=""`) is accepted; the parse test covers it.
        let _ = busboy::boundary_of(Some(&ct));
    }

    /// Field names nest at most `max_depth` levels and arrays never grow past the hole budget,
    /// whatever the keys: the stored object stays proportional to the input.
    #[test]
    fn append_field_is_bounded(fields in prop::collection::vec((field_key(), "[a-z]{0,8}"), 0..40), max_depth in 0usize..64) {
        let mut store = Map::new();
        let mut budget = ArrayBudget::default();
        let mut input = 0;
        for (k, v) in &fields {
            input += k.len() + v.len();
            append_field(&mut store, &mut budget, k, Value::String(v.clone()), max_depth.min(DEFAULT_MAX_FIELD_DEPTH));
        }
        let size = serde_json::to_string(&Value::Object(store)).unwrap().len();
        let bound = 8 * MAX_ARRAY_HOLES + 16 * input + 64;
        prop_assert!(size <= bound, "{size} bytes stored from {input} bytes of input");
    }
}

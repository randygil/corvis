//! Property-based fuzzing of the request parsers: `JSON.parse` (`json_parse`), `qs`, the body
//! parser, JWT verification, route paths and trust proxy (`proxy_addr`). The multipart parser has
//! its own module (`multer::fuzz`), static files are fuzzed end to end in `tests/fuzz_static.rs`.
//!
//! The properties are the ones a hostile client attacks: no panic or stack overflow on any input,
//! output proportional to the input, and no token accepted unless it was signed with the key.
//! More cases: `PROPTEST_CASES=20000 cargo test -p corvis fuzz_tests`.

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, header};
use proptest::prelude::*;
use serde_json::{Value, json};

use crate::http::body_parser::{BodyParserOptions, parse_body, qs};
use crate::http::json_parse;
use crate::http::proxy_addr::{TrustProxy, all_addrs};
use crate::jwt::{JwtModuleOptions, JwtService, parse_duration_secs};
use crate::router::{join_paths, to_matchit};

/// `cases` by default; `PROPTEST_CASES` overrides it for longer fuzzing runs.
pub(crate) fn config(cases: u32) -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(cases);
    ProptestConfig { cases, ..ProptestConfig::default() }
}

/// Fragments of JSON documents: random soups of them are often valid, or almost.
fn json_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "{",
            "}",
            "[",
            "]",
            ",",
            ":",
            " ",
            "\n",
            "\"a\"",
            "\"b\"",
            "\"__proto__\"",
            "\"0\"",
            "\"10\"",
            "1",
            "-0",
            "0.5",
            "1e400",
            "-1e-400",
            "12345678901234567890",
            "true",
            "false",
            "null",
            "\"\\u00e9\"",
            "\"\\ud800\"",
            "\"\\\"\"",
            "\"x",
            "\\",
            "/",
            "\u{feff}",
            "NaN",
            "01",
        ]),
        0..64,
    )
    .prop_map(|parts| parts.concat())
}

/// JSON values without numeric keys (JS enumerates those first) and with integers only, so
/// `JSON.parse` and serde_json must agree exactly.
fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(|n| json!(n)),
        "\\PC{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(6, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::btree_map("[a-z_]{1,6}", inner, 0..8).prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// Query strings / urlencoded bodies built from the characters `qs` gives a meaning to.
fn query_string() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a",
            "b",
            "=",
            "&",
            ";",
            "[",
            "]",
            "[]",
            "[0]",
            "[1]",
            "[20]",
            "[21]",
            "[999999999]",
            "[__proto__]",
            "[constructor][prototype]",
            ".",
            "%5B",
            "%5D",
            "%26",
            "%3D",
            "%",
            "%zz",
            "%e9",
            "%C3%A9",
            "+",
            " ",
            "utf8=%E2%9C%93",
            "utf8=%26%2310003%3B",
            "&#10003;",
        ]),
        0..80,
    )
    .prop_map(|parts| parts.concat())
}

/// `(array_limit, depth, parameter_limit, latin1, strict_depth)` (`qs::Options` is not `Debug`).
fn qs_options() -> impl Strategy<Value = (usize, usize, usize, bool, bool)> {
    (0usize..40, 0usize..12, 1usize..2000, any::<bool>(), any::<bool>())
}

fn json_size(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
}

proptest! {
    #![proptest_config(config(2000))]

    #[test]
    fn json_parse_is_total(text in "\\PC{0,64}", soup in json_soup()) {
        let _ = json_parse::parse(&text);
        let _ = json_parse::parse(&soup);
    }

    /// Whatever serde_json accepts, `JSON.parse` accepts too.
    #[test]
    fn json_parse_accepts_what_serde_accepts(soup in json_soup()) {
        if serde_json::from_str::<Value>(&soup).is_ok() {
            prop_assert!(json_parse::parse(&soup).is_ok(), "rejected {soup:?}");
        }
    }

    #[test]
    fn json_parse_round_trips(v in json_value()) {
        let text = serde_json::to_string(&v).unwrap();
        prop_assert_eq!(json_parse::parse(&text).unwrap(), v);
    }

    /// Deep or unbalanced nesting ends in an error, never in a stack overflow.
    #[test]
    fn json_depth_is_bounded(open in prop::collection::vec(prop::sample::select(vec!['[', '{', '"', ':', ',', 'a']), 0..20_000), max_depth in 0usize..300) {
        let units: Vec<u16> = open.iter().map(|c| *c as u16).collect();
        let _ = json_parse::parse_utf16_with_depth(&units, max_depth);
        let deep: Vec<u16> = std::iter::repeat_n(b'[' as u16, open.len()).chain(std::iter::repeat_n(b']' as u16, open.len())).collect();
        let r = json_parse::parse_utf16_with_depth(&deep, max_depth);
        if open.len() > max_depth {
            prop_assert!(r.is_err(), "{} levels accepted with max_depth {max_depth}", open.len());
        }
    }

    /// `qs` never panics, only `strictDepth` can fail, and the result stays proportional to the
    /// query (sparse indexes become object keys or are compacted, never huge arrays).
    #[test]
    fn qs_is_total_and_bounded(s in query_string(), (array_limit, depth, parameter_limit, latin1, strict) in qs_options()) {
        let o = qs::Options { array_limit, depth, parameter_limit, latin1, strict_depth: strict };
        match qs::parse(&s, &o) {
            Ok(v) => {
                let size = json_size(&v);
                prop_assert!(size <= 16 * s.len() + 64, "{size} bytes from a {}-byte query", s.len());
            }
            Err(_) => prop_assert!(strict, "non-strict parse failed for {s:?}"),
        }
    }

    /// The body parser maps any body, content type and encoding to a value or an HTTP error.
    #[test]
    fn parse_body_is_total(
        ct in prop::sample::select(vec![
            "application/json", "application/json; charset=utf-16le", "application/json; charset=latin1",
            "application/json; charset=koi8-r", "application/x-www-form-urlencoded",
            "application/x-www-form-urlencoded; charset=iso-8859-1", "application/x-www-form-urlencoded; charset=utf-16",
            "text/plain", "application/json;;charset=\"utf-8\"", "application/merge-patch+json",
        ]),
        encoding in prop::option::of(prop::sample::select(vec!["identity", "gzip", "deflate", "br", "x-unknown"])),
        body in prop_oneof![json_soup().prop_map(String::into_bytes), query_string().prop_map(String::into_bytes), prop::collection::vec(any::<u8>(), 0..256)],
    ) {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(body.len()));
        if let Some(e) = encoding {
            h.insert(header::CONTENT_ENCODING, HeaderValue::from_static(e));
        }
        let _ = parse_body(&h, &Bytes::from(body), &BodyParserOptions::default());
    }

    /// Random tokens and `alg` tricks are rejected without panicking.
    #[test]
    fn jwt_rejects_forged_tokens(
        alg in prop::sample::select(vec!["none", "None", "NONE", "HS256", "HS384", "HS512", "RS256", "ES256", "PS256", "EdDSA", ""]),
        payload in json_value(),
        sig in "[A-Za-z0-9_-]{0,64}",
        junk in "[A-Za-z0-9_.=+/-]{0,128}",
    ) {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let svc = JwtService::new(JwtModuleOptions::new("fuzz-secret"));
        let header = b64.encode(json!({ "alg": alg, "typ": "JWT" }).to_string());
        let body = b64.encode(payload.to_string());
        let token = format!("{header}.{body}.{sig}");
        prop_assert!(svc.verify::<Value>(&token).is_err(), "accepted {token}");
        prop_assert!(svc.verify::<Value>(&junk).is_err(), "accepted {junk}");
        let _ = svc.decode(&junk);
        let _ = parse_duration_secs(&junk);
    }

    /// Correctly signed tokens are still rejected when they use another HMAC algorithm with the
    /// configured secret (algorithm confusion) or HS256 with another key.
    #[test]
    fn jwt_rejects_valid_signatures_of_other_algorithms_and_keys(
        alg in prop::sample::select(vec![jsonwebtoken::Algorithm::HS384, jsonwebtoken::Algorithm::HS512]),
        sub in "[a-z0-9]{1,16}",
        other_key in "[ -~]{1,32}",
    ) {
        use jsonwebtoken::{EncodingKey, Header, encode};
        let svc = JwtService::new(JwtModuleOptions::new("fuzz-secret"));
        let claims = json!({ "sub": sub, "admin": true });
        let confused = encode(&Header::new(alg), &claims, &EncodingKey::from_secret(b"fuzz-secret")).unwrap();
        prop_assert!(svc.verify::<Value>(&confused).is_err(), "accepted {alg:?} token {confused}");
        prop_assume!(other_key != "fuzz-secret");
        let wrong_key = encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(other_key.as_bytes())).unwrap();
        prop_assert!(svc.verify::<Value>(&wrong_key).is_err(), "accepted token signed with {other_key:?}");
        // Control: the same claims signed with the configured key and algorithm verify.
        let good = encode(&Header::new(jsonwebtoken::Algorithm::HS256), &claims, &EncodingKey::from_secret(b"fuzz-secret")).unwrap();
        prop_assert!(svc.verify::<Value>(&good).is_ok());
    }

    /// Changing any character of the signed header or payload invalidates the token.
    #[test]
    fn jwt_detects_tampering(sub in "[a-z0-9]{1,16}", pick in any::<prop::sample::Index>(), with in prop::sample::select(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".to_vec())) {
        let svc = JwtService::new(JwtModuleOptions::new("fuzz-secret"));
        let token = svc.sign(&json!({ "sub": sub, "admin": false })).unwrap();
        prop_assert!(svc.verify::<Value>(&token).is_ok());
        let signed_len = token.rfind('.').unwrap();
        let i = pick.index(signed_len);
        let mut bytes = token.clone().into_bytes();
        if bytes[i] == b'.' || bytes[i] == with {
            return Ok(());
        }
        bytes[i] = with;
        let forged = String::from_utf8(bytes).unwrap();
        prop_assert!(svc.verify::<Value>(&forged).is_err(), "tampered token accepted: {forged}");
    }

    #[test]
    fn route_paths_are_total(parts in prop::collection::vec("[a-z:*/{}()?+.~\\\\-]{0,12}", 0..6)) {
        let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
        let joined = join_paths(&refs);
        let _ = to_matchit(&joined);
        for p in &parts {
            let _ = to_matchit(p);
        }
    }

    /// `X-Forwarded-For` can only add hops: the result starts at the socket and has at most one
    /// entry per forwarded address.
    #[test]
    fn trust_proxy_is_total_and_bounded(
        spec in "[0-9a-f.:/, a-z]{0,40}",
        trust in prop::sample::select(vec!["none", "loopback", "uniquelocal", "10.0.0.0/8, 127.0.0.1", "::1/128", "1", "2", "0", "all"]),
        socket in prop::sample::select(vec!["127.0.0.1", "8.8.8.8", "10.1.2.3", "::1", "192.168.0.7", "2001:db8::1"]),
        xff in prop::collection::vec("[0-9a-f.:, \\[\\]]{0,24}", 0..6),
    ) {
        let _ = TrustProxy::parse(&spec);
        let trust = match trust {
            "none" => TrustProxy::None,
            "all" => TrustProxy::All,
            n if n.parse::<usize>().is_ok() => TrustProxy::Hops(n.parse().unwrap()),
            list => TrustProxy::parse(list).unwrap(),
        };
        let mut h = HeaderMap::new();
        let mut entries = 0;
        for v in &xff {
            if let Ok(hv) = HeaderValue::from_str(v) {
                entries += v.split(',').count();
                h.append("x-forwarded-for", hv);
            }
        }
        let addrs = all_addrs(socket.into(), &h, &trust);
        prop_assert_eq!(addrs.first().map(String::as_str), Some(socket));
        prop_assert!(addrs.len() <= entries + 1, "{} addresses from {entries} entries", addrs.len());
        // Spoofing: a forwarded address only counts when every hop before it is trusted.
        if trust == TrustProxy::None {
            prop_assert_eq!(addrs.len(), 1, "X-Forwarded-For used without trusting any proxy");
        }
        for (i, a) in addrs.iter().enumerate().take(addrs.len() - 1) {
            prop_assert!(trust.trusts(a, i), "hop {i} ({a}) is not trusted but the chain goes past it: {addrs:?}");
        }
    }
}

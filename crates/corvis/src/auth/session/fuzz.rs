//! Property tests of the signed cookies and of the cookie header the session middleware reads: no panic, no value
//! without a valid signature, no session loaded for anything but the one correctly signed cookie. More cases:
//! `PROPTEST_CASES=20000 cargo test -p corvis --features auth-session --lib fuzz`.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Uri};
use proptest::prelude::*;
use serde_json::json;

use super::{MemorySessionStore, SessionManager, SessionOptions, SessionStore};
use crate::fuzz_tests::config;
use crate::http::{CookieKeys, Request};

const SECRET_A: &str = "0123456789abcdef0123456789abcdef";
const SECRET_B: &str = "fedcba9876543210fedcba9876543210";
const ID: &str = "validIdentifierValidIdentifier01";

fn encode(s: &str) -> String {
    crate::http::res::encode_uri_component(s)
}

/// Flips, drops, inserts or duplicates something in `s` at a position drawn from `seed`.
fn mutate(s: &str, seed: usize, kind: u8) -> String {
    let mut bytes = s.as_bytes().to_vec();
    if bytes.is_empty() {
        return String::new();
    }
    let at = seed % bytes.len();
    match kind % 5 {
        0 => bytes[at] = bytes[at].wrapping_add(1),
        1 => {
            bytes.remove(at);
        }
        2 => bytes.insert(at, b'.'),
        3 => bytes.truncate(at),
        _ => {
            let copy = bytes[at..].to_vec();
            bytes.extend(copy);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

proptest! {
    #![proptest_config(config(512))]

    #[test]
    fn fuzz_unsign_never_panics_and_accepts_only_what_it_signed(input in ".{0,200}", value in ".{0,100}", seed in 0usize..400, kind in 0u8..5) {
        let keys = CookieKeys::new([SECRET_A, SECRET_B]).unwrap();
        let other = CookieKeys::new(["a-completely-different-32-byte-secret!!"]).unwrap();
        // Random input is accepted only when it is exactly a signature of its own value.
        if let Some(v) = keys.unsign(&input) {
            let resigned = [CookieKeys::new([SECRET_A]).unwrap().sign(&v), CookieKeys::new([SECRET_B]).unwrap().sign(&v)];
            prop_assert!(resigned.contains(&input), "{:?} was accepted", input);
        }
        let signed = keys.sign(&value);
        prop_assert_eq!(keys.unsign(&signed), Some(value.clone()));
        prop_assert_eq!(other.unsign(&signed), None);
        // A mutated valid value is refused, or it is another value with its own valid signature.
        let mutated = mutate(&signed, seed, kind);
        if mutated != signed
            && let Some(v) = keys.unsign(&mutated)
        {
            let resigned = [CookieKeys::new([SECRET_A]).unwrap().sign(&v), CookieKeys::new([SECRET_B]).unwrap().sign(&v)];
            prop_assert!(resigned.contains(&mutated), "{:?} was accepted for {:?}", mutated, v);
        }
        // The bound holds for any length.
        prop_assert_eq!(keys.unsign(&format!("s:{}", "a".repeat(5000))), None);
    }
}

fn request_with(cookie_headers: &[String]) -> Option<Request> {
    let mut headers = HeaderMap::new();
    for h in cookie_headers {
        // A header value the HTTP layer would refuse never reaches the middleware.
        headers.append(http::header::COOKIE, HeaderValue::from_str(h).ok()?);
    }
    Request::new(Method::GET, Uri::from_static("/"), headers, Bytes::new()).ok()
}

fn fragment() -> impl Strategy<Value = String> {
    let keys = CookieKeys::new([SECRET_A]).unwrap();
    let valid = format!("connect.sid={}", encode(&keys.sign(ID)));
    let flipped = format!("connect.sid={}", encode(&keys.sign(ID)).replace('A', "B"));
    let unsigned = format!("connect.sid={ID}");
    prop_oneof![
        Just(valid),
        Just(flipped),
        Just(unsigned),
        "connect\\.sid=[ -~]{0,60}",
        "[a-z]{1,6}=[ -~]{0,20}",
        "[ -~]{0,30}",
        Just(String::new()),
    ]
}

proptest! {
    #![proptest_config(config(256))]

    #[test]
    fn fuzz_cookie_headers_never_load_a_session_without_a_valid_unique_signature(
        parts in prop::collection::vec(fragment(), 0..5),
        extra_header in prop::option::of(fragment()),
        mutate_at in prop::option::of((0usize..200, 0u8..5)),
    ) {
        let keys = CookieKeys::new([SECRET_A]).unwrap();
        let mut header = parts.join("; ");
        if let Some((seed, kind)) = mutate_at {
            header = mutate(&header, seed, kind);
        }
        let mut headers = vec![header];
        headers.extend(extra_header);
        let Some(req) = request_with(&headers) else { return Ok(()) };

        let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let loaded = runtime.block_on(async {
            let store = Arc::new(MemorySessionStore::new());
            let record = json!({ "cookie": { "originalMaxAge": null, "expires": null }, "who": "owner" });
            store.create(ID, &record, Duration::from_secs(3600)).await.unwrap();
            let options = SessionOptions::new(keys.clone()).store_arc(store);
            let manager = Arc::new(SessionManager::new(options).unwrap());
            let session = manager.load(&req).await;
            // Never an error because of the cookie.
            let session = session.map_err(|_| "the load failed".to_string()).unwrap();
            (!session.is_new(), session.get::<String>("who"))
        });

        if loaded.0 {
            prop_assert_eq!(loaded.1.as_deref(), Some("owner"));
            // Exactly one session cookie, correctly signed, for that identifier.
            let values = req.cookie_values("connect.sid", 3, 1 << 20);
            prop_assert_eq!(values.len(), 1, "{:?}", headers);
            let unsigned = keys.unsign(&values[0]);
            prop_assert_eq!(unsigned.as_deref(), Some(ID), "{:?}", headers);
        }
    }
}

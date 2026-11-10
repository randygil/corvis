//! Signed cookies against the original: every signature, verification result and `Set-Cookie` header recorded from
//! `cookie-signature` 1.0.7 and `cookie-parser` 1.4.7 (`tests/fixtures/auth/signed_cookies.mjs`) is replayed. No Node
//! is needed at test time. A case with a `divergence` is asserted against the specification.
#![cfg(feature = "auth-session")]

use bytes::Bytes;
use corvis::http::{CookieKeys, CookieOptions, Method, Request, res::serialize_cookie};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/auth/signed_cookies.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn secret(fixture: &Value, name: &str) -> String {
    fixture["secrets"][name].as_str().unwrap().to_string()
}

fn request_with_cookie(header: &str) -> Request {
    let mut headers = http::HeaderMap::new();
    headers.insert(http::header::COOKIE, http::HeaderValue::from_str(header).unwrap());
    Request::new(Method::GET, "/".parse().unwrap(), headers, Bytes::new()).unwrap()
}

#[test]
fn signatures_equal_the_original() {
    let f = fixture();
    let cases = f["sign"].as_array().unwrap();
    assert!(cases.len() >= 20);
    for case in cases {
        let keys = CookieKeys::new([secret(&f, case["secret"].as_str().unwrap())]).unwrap();
        let value = case["value"].as_str().unwrap();
        let signed = case["signed"].as_str().unwrap();
        assert_eq!(keys.sign(value), signed, "sign({value:?})");
        assert_eq!(keys.unsign(signed).as_deref(), Some(value), "unsign({signed:?})");
    }
}

#[test]
fn the_other_secret_does_not_verify_a_recorded_signature() {
    let f = fixture();
    let a = CookieKeys::new([secret(&f, "a")]).unwrap();
    let b = CookieKeys::new([secret(&f, "b")]).unwrap();
    for case in f["sign"].as_array().unwrap() {
        let keys = if case["secret"] == "a" { &b } else { &a };
        assert_eq!(keys.unsign(case["signed"].as_str().unwrap()), None);
    }
}

#[test]
fn verification_results_equal_cookie_parser() {
    let f = fixture();
    let cases = f["read"].as_array().unwrap();
    assert!(cases.len() >= 25);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let secrets: Vec<String> = case["secrets"].as_array().unwrap().iter().map(|s| secret(&f, s.as_str().unwrap())).collect();
        let keys = CookieKeys::new(secrets).unwrap();
        let req = request_with_cookie(case["header"].as_str().unwrap());
        for (cookie, key) in [("prefs", "signed"), ("other", "other")] {
            let got = req.signed_cookie(cookie, &keys);
            if let Some(divergence) = case.get("divergence") {
                // Asserted from the specification: nothing is returned for an input over 4096 bytes.
                assert!(divergence.is_string());
                assert_eq!(got, None, "{name}: {divergence}");
                continue;
            }
            // `false` (wrong signature) and `null` (no such signed cookie) are both "nothing" here.
            let expected = case[key].as_str();
            assert_eq!(got.as_deref(), expected, "{name} ({cookie})");
        }
    }
}

#[test]
fn an_unsigned_cookie_is_still_a_plain_cookie() {
    let f = fixture();
    let keys = CookieKeys::new([secret(&f, "a")]).unwrap();
    let req = request_with_cookie("prefs=dark");
    assert_eq!(req.signed_cookie("prefs", &keys), None);
    assert_eq!(req.cookie("prefs").as_deref(), Some("dark"));
}

#[test]
fn set_cookie_headers_equal_res_cookie_signed() {
    let f = fixture();
    let keys = CookieKeys::new([secret(&f, "a")]).unwrap();
    for case in f["set"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let q = &case["query"];
        let value = q["value"].as_str().unwrap_or("dark");
        let mut options = CookieOptions::new();
        if q["httpOnly"] == "1" {
            options = options.http_only(true);
        }
        if q["secure"] == "1" {
            options = options.secure(true);
        }
        if let Some(s) = q["sameSite"].as_str() {
            options = options.same_site(s);
        }
        if let Some(p) = q["path"].as_str() {
            options = options.path(p);
        }
        let header = serialize_cookie("prefs", &keys.sign(value), &options);
        assert_eq!(case["setCookie"][0].as_str().unwrap(), header, "{name}");
    }
}

#[test]
fn a_cookie_written_by_corvis_reads_back_through_the_request() {
    let f = fixture();
    let keys = CookieKeys::new([secret(&f, "a")]).unwrap();
    for value in ["dark", "a b:c", "caf\u{e9}", "100%"] {
        let header = serialize_cookie("prefs", &keys.sign(value), &CookieOptions::new());
        let pair = header.split(';').next().unwrap();
        let req = request_with_cookie(pair);
        assert_eq!(req.signed_cookie("prefs", &keys).as_deref(), Some(value), "{pair}");
    }
}

#[test]
fn the_versions_are_the_ones_the_design_names() {
    let f = fixture();
    assert_eq!(f["versions"]["cookie-signature"], "1.0.7");
    assert_eq!(f["versions"]["cookie-parser"], "1.4.7");
}

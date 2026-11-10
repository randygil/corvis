//! `PasswordHasher` against `bcryptjs` 3.0.3: every case recorded by `tests/fixtures/auth/bcrypt.mjs` is replayed, in
//! both directions, without Node. Hashes made by `bcryptjs` must verify like `compareSync` says (including non-ASCII
//! passwords, NUL characters and passwords over 72 bytes), and hashes made by Corvis (`corvis_bcrypt_hashes.json`) must
//! be accepted by `bcryptjs` as recorded.
#![cfg(feature = "auth-local")]

use corvis::auth::local::{PasswordError, PasswordHasher, PasswordHasherOptions};
use serde::Deserialize;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/auth/bcrypt.json");
const CORVIS_HASHES: &str = include_str!("fixtures/auth/corvis_bcrypt_hashes.json");

#[derive(Deserialize)]
struct Fixture {
    bcryptjs: String,
    #[serde(rename = "fromOriginal")]
    from_original: Vec<Original>,
    #[serde(rename = "fromCorvis")]
    from_corvis: Vec<FromCorvis>,
}

#[derive(Deserialize)]
struct Original {
    name: String,
    password: String,
    hash: String,
    checks: Vec<Check>,
}

#[derive(Deserialize)]
struct Check {
    what: String,
    password: String,
    expected: bool,
}

#[derive(Deserialize)]
struct FromCorvis {
    name: String,
    password: String,
    hash: String,
    expected: bool,
    #[serde(rename = "wrongPassword")]
    wrong_password: String,
    #[serde(rename = "wrongExpected")]
    wrong_expected: bool,
}

#[derive(Deserialize)]
struct CorvisHash {
    name: String,
    password: String,
    hash: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).unwrap()
}

/// The hasher of the tests: argon2id by default configuration is irrelevant here, only verification and bcrypt
/// hashing at the costs of the fixture matter.
fn bcrypt_hasher(cost: u32) -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::bcrypt(cost).allow_weak_parameters()).unwrap()
}

#[test]
fn the_fixture_is_from_bcryptjs_3() {
    let f = fixture();
    assert_eq!(f.bcryptjs, "3.0.3");
    assert!(f.from_original.len() >= 15 && !f.from_corvis.is_empty());
}

#[tokio::test]
async fn hashes_from_the_original_verify_like_compare_sync() {
    let f = fixture();
    // The default hasher (argon2id) verifies them too: the algorithm comes from the hash.
    let hashers = [bcrypt_hasher(4), PasswordHasher::new(PasswordHasherOptions::default()).unwrap()];
    for hasher in &hashers {
        for case in &f.from_original {
            for check in &case.checks {
                let got = hasher.verify(&check.password, &case.hash).await.unwrap();
                assert_eq!(got, check.expected, "{} / {} ({:?})", case.name, check.what, check.password);
            }
        }
    }
}

#[tokio::test]
async fn long_passwords_against_original_hashes_follow_the_original() {
    let f = fixture();
    let h = bcrypt_hasher(4);
    let case = f.from_original.iter().find(|c| c.name == "bytes-73").unwrap();
    assert_eq!(case.password.len(), 73);
    let first_72 = &case.password[..72];
    assert!(h.verify(&case.password, &case.hash).await.unwrap());
    assert!(h.verify(first_72, &case.hash).await.unwrap(), "bcrypt covers the first 72 bytes only, like bcryptjs");
}

#[tokio::test]
async fn nul_and_non_ascii_passwords_follow_the_original() {
    let f = fixture();
    let h = bcrypt_hasher(4);
    for name in ["latin", "emoji", "nul-middle", "nul-end", "nul-start", "multibyte-boundary"] {
        let case = f.from_original.iter().find(|c| c.name == name).unwrap();
        assert!(h.verify(&case.password, &case.hash).await.unwrap(), "{name}");
        let wrong = case.checks.iter().find(|c| c.what == "changed").unwrap();
        assert!(!wrong.expected && !h.verify(&wrong.password, &case.hash).await.unwrap(), "{name}");
    }
}

#[tokio::test]
async fn hashes_made_by_corvis_are_accepted_by_the_original() {
    let f = fixture();
    let h = bcrypt_hasher(4);
    for case in &f.from_corvis {
        assert!(case.expected, "bcryptjs must accept the Corvis hash of {}", case.name);
        assert!(!case.wrong_expected, "bcryptjs must reject a wrong password for {}", case.name);
        assert!(case.hash.starts_with("$2b$"), "{}", case.name);
        assert!(h.verify(&case.password, &case.hash).await.unwrap(), "{}", case.name);
        assert!(!h.verify(&case.wrong_password, &case.hash).await.unwrap(), "{}", case.name);
    }
}

#[test]
fn the_recorded_corvis_hashes_are_the_committed_ones() {
    let committed: Vec<CorvisHash> = serde_json::from_str(CORVIS_HASHES).unwrap();
    let f = fixture();
    assert_eq!(committed.len(), f.from_corvis.len());
    for (c, r) in committed.iter().zip(&f.from_corvis) {
        assert_eq!((&c.name, &c.password, &c.hash), (&r.name, &r.password, &r.hash), "re-run bcrypt.mjs after write_corvis_hashes");
    }
}

#[tokio::test]
async fn corvis_refuses_what_bcryptjs_truncates() {
    let h = bcrypt_hasher(4);
    assert!(h.hash("a".repeat(72)).await.is_ok());
    assert_eq!(h.hash("a".repeat(73)).await, Err(PasswordError::TooLong { max: 72 }));
    let f = fixture();
    for case in f.from_original.iter().filter(|c| c.password.len() > 72) {
        assert!(h.hash(&case.password).await.is_err(), "{}", case.name);
    }
}

/// Rewrites `tests/fixtures/auth/corvis_bcrypt_hashes.json` from the passwords of `bcrypt.json`. Run it, then run
/// `node bcrypt.mjs` in `tests/fixtures/auth` to record what `bcryptjs` says about the new hashes:
/// `cargo test -p corvis --features auth-local --test auth_password -- --ignored write_corvis_hashes`.
#[tokio::test]
#[ignore = "regenerates a fixture"]
async fn write_corvis_hashes() {
    let f = fixture();
    let mut out: Vec<Value> = Vec::new();
    for case in f.from_original.iter().filter(|c| c.name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')) {
        if case.password.len() > 72 || case.name.ends_with("-cost-10") || case.name.ends_with("-2a") || case.name.ends_with("-2y") {
            continue;
        }
        let hash = bcrypt_hasher(4).hash(&case.password).await.unwrap();
        out.push(serde_json::json!({ "name": case.name, "password": case.password, "hash": hash }));
    }
    for (name, password) in [("ascii-cost-10", "secret"), ("latin-cost-10", "pässword")] {
        let hash = bcrypt_hasher(10).hash(password).await.unwrap();
        out.push(serde_json::json!({ "name": name, "password": password, "hash": hash }));
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/auth/corvis_bcrypt_hashes.json");
    std::fs::write(path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
}

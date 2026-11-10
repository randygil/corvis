//! Unit tests of the password hasher. Most use small parameters (`allow_weak_parameters`); the few with the real
//! defaults run in milliseconds because the dev profile optimizes the hashing crates.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use proptest::prelude::*;

use super::hasher::probe::Gate;
use super::*;

const SMALL: Argon2Params = Argon2Params { memory_kib: 8, iterations: 1, parallelism: 1 };

fn small_argon() -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters()).unwrap()
}

fn small_bcrypt() -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::bcrypt(4).allow_weak_parameters()).unwrap()
}

/// A bcrypt hash of "secret" at cost 4 made by `bcryptjs` (fixed so the tests do not depend on this crate).
const BCRYPTJS_SECRET_4: &str = "$2b$04$iQJJhVONs/5UD7Hw1ADyBusLeIrz/tMsKqJxw7KFBLjqJm.Of4IyK";

fn derivations(h: &PasswordHasher) -> usize {
    h.probe().count()
}

#[tokio::test]
async fn default_hash_format() {
    let h = PasswordHasher::new(PasswordHasherOptions::default()).unwrap();
    let hash = h.hash("secret").await.unwrap();
    assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{hash}");
    assert!(h.verify("secret", &hash).await.unwrap());
    assert!(!h.verify("Secret", &hash).await.unwrap());
    assert!(!h.needs_rehash(&hash));
}

#[tokio::test]
async fn two_hashes_of_the_same_password_differ() {
    let h = small_argon();
    let (a, b) = (h.hash("secret").await.unwrap(), h.hash("secret").await.unwrap());
    assert_ne!(a, b);
    assert!(h.verify("secret", &a).await.unwrap() && h.verify("secret", &b).await.unwrap());
}

#[tokio::test]
async fn bcrypt_hashes_are_2b_and_verify() {
    let h = small_bcrypt();
    let hash = h.hash("secret").await.unwrap();
    assert!(hash.starts_with("$2b$04$"), "{hash}");
    assert!(h.verify("secret", &hash).await.unwrap());
    assert!(!h.verify("other", &hash).await.unwrap());
}

#[test]
fn parameters_below_the_floor_stop_construction() {
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(SMALL)).is_err());
    assert!(
        PasswordHasher::new(PasswordHasherOptions::argon2id(Argon2Params { memory_kib: 19456, iterations: 1, parallelism: 1 })).is_err()
    );
    assert!(PasswordHasher::new(PasswordHasherOptions::bcrypt(9)).is_err());
    // The other OWASP equivalent is accepted: 46 MiB with one iteration.
    assert!(
        PasswordHasher::new(PasswordHasherOptions::argon2id(Argon2Params { memory_kib: 47104, iterations: 1, parallelism: 1 })).is_ok()
    );
    assert!(PasswordHasher::new(PasswordHasherOptions::bcrypt(10)).is_ok());
    // The opt-out is explicit.
    assert!(PasswordHasher::new(PasswordHasherOptions::bcrypt(4).allow_weak_parameters()).is_ok());
}

#[test]
fn parameters_above_the_verification_limits_stop_construction() {
    let big = Argon2Params { memory_kib: 300_000, iterations: 2, parallelism: 1 };
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(big)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::bcrypt(15)).is_err());
    // Raising the limit makes it valid.
    let limits = VerifyLimits::default().max_argon2_memory_kib(400_000);
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(big).verify_limits(limits)).is_ok());
    // A dummy above the limits is refused too.
    assert!(PasswordHasher::new(PasswordHasherOptions::default().dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 20 })).is_err());
}

#[test]
fn invalid_options_stop_construction() {
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_password_bytes(0)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_password_bytes(MAX_PASSWORD_BYTES_CAP + 1)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_concurrent(0)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().verify_limits(VerifyLimits::default().max_hash_bytes(10))).is_err());
    // Parallelism with too little memory is rejected by the crate, not a panic.
    let bad = Argon2Params { memory_kib: 8, iterations: 1, parallelism: 8 };
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(bad).allow_weak_parameters()).is_err());
}

#[tokio::test]
async fn planted_high_cost_bcrypt_hash_is_refused_without_derivation() {
    let h = small_bcrypt();
    let before = derivations(&h);
    let planted = "$2b$31$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie";
    assert_eq!(h.verify("x", planted).await, Err(PasswordError::OverLimit { what: "bcrypt cost" }));
    assert_eq!(derivations(&h), before);
    assert!(h.needs_rehash(planted));
}

#[tokio::test]
async fn planted_high_memory_argon2_hash_is_refused_without_derivation() {
    let h = small_argon();
    let before = derivations(&h);
    for planted in [
        "$argon2id$v=19$m=4194304,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG",
        "$argon2id$v=19$m=8,t=4000000000,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG",
        "$argon2id$v=19$m=65536,t=1,p=1000$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG",
    ] {
        assert!(matches!(h.verify("x", planted).await, Err(PasswordError::OverLimit { .. }) | Err(PasswordError::Malformed)), "{planted}");
    }
    assert!(matches!(
        h.verify("x", "$argon2id$v=19$m=4194304,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG").await,
        Err(PasswordError::OverLimit { what: "argon2 memory" })
    ));
    assert_eq!(derivations(&h), before);
}

#[tokio::test]
async fn a_hash_over_512_bytes_is_refused_without_derivation() {
    let h = small_argon();
    let before = derivations(&h);
    let long = format!("$argon2id$v=19$m=8,t=1,p=1${}", "A".repeat(600));
    assert_eq!(h.verify("x", &long).await, Err(PasswordError::OverLimit { what: "hash length" }));
    assert_eq!(derivations(&h), before);
}

#[tokio::test]
async fn malformed_hashes_are_errors_without_derivation_or_panic() {
    let h = small_bcrypt();
    let before = derivations(&h);
    for bad in [
        "",
        "not-a-hash",
        "$",
        "$2",
        "$2b$",
        "$2b$10$short",
        "$2b$ab$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie",
        "$2b$10$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxci!",
        "$2b$03$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie",
        "$argon2id$",
        "$argon2id$v=19$m=8,t=1,p=1$$",
        "$argon2id$v=19$m=8,t=1,p=1$c29tZXNhbHRzb21lc2FsdA",
        "$argon2id$v=99$m=8,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG",
        "$scrypt$ln=16,r=8,p=1$aM15713r3Xsvxbi31lqr1Q$nFNh2CVHVjNldFVKDHDlm4CbdRSCdEBsjjJxD+iCs5E",
        "$2x$04$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie",
        "\u{0}\u{1}\u{2}",
        "$2b$04$é\u{1F600}",
    ] {
        assert!(h.verify("x", bad).await.is_err(), "{bad:?}");
        assert!(h.needs_rehash(bad), "{bad:?}");
    }
    assert_eq!(derivations(&h), before);
    assert_eq!(h.verify("x", "$scrypt$ln=16").await, Err(PasswordError::Unsupported));
}

#[tokio::test]
async fn hashing_an_oversized_password_fails() {
    let h = small_argon();
    let ok = "a".repeat(1024);
    let too_long = "a".repeat(1025);
    assert!(h.hash(&ok).await.is_ok());
    assert_eq!(h.hash(&too_long).await, Err(PasswordError::TooLong { max: 1024 }));
    assert_eq!(h.hash_blocking(&too_long), Err(PasswordError::TooLong { max: 1024 }));
}

#[tokio::test]
async fn verifying_an_oversized_password_is_false_without_derivation() {
    let h = small_argon();
    let hash = h.hash("secret").await.unwrap();
    let before = derivations(&h);
    assert_eq!(h.verify("a".repeat(1025), &hash).await, Ok(false));
    assert_eq!(h.verify_blocking("a".repeat(1025), &hash), Ok(false));
    assert_eq!(derivations(&h), before);
}

#[tokio::test]
async fn bcrypt_never_truncates_when_hashing() {
    let h = small_bcrypt();
    assert!(h.hash("a".repeat(72)).await.is_ok());
    assert_eq!(h.hash("a".repeat(73)).await, Err(PasswordError::TooLong { max: 72 }));
    // Argon2 has no such limit below max_password_bytes.
    assert!(small_argon().hash("a".repeat(73)).await.is_ok());
}

#[tokio::test]
async fn bcryptjs_hashes_verify_whatever_the_configured_algorithm() {
    let h = small_argon();
    assert!(h.verify("secret", BCRYPTJS_SECRET_4).await.unwrap());
    assert!(!h.verify("secreT", BCRYPTJS_SECRET_4).await.unwrap());
    assert!(h.needs_rehash(BCRYPTJS_SECRET_4), "an argon2 hasher wants to replace a bcrypt hash");
}

#[tokio::test]
async fn needs_rehash_follows_the_configuration() {
    let two =
        PasswordHasher::new(PasswordHasherOptions::argon2id(Argon2Params { memory_kib: 19456, iterations: 2, parallelism: 1 })).unwrap();
    let old = small_argon().hash("x").await.unwrap(); // m=8, t=1
    assert!(two.needs_rehash(&old));
    let fresh = two.hash("x").await.unwrap();
    assert!(!two.needs_rehash(&fresh));
    // Stronger stored hashes are kept.
    assert!(!small_argon().needs_rehash(&fresh));
    // Another variant is replaced.
    let argon2i = "$argon2i$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG";
    assert!(two.needs_rehash(argon2i));
    // bcrypt: a lower cost is replaced, an equal one is kept; a bcrypt hasher replaces argon2.
    let b10 = PasswordHasher::new(PasswordHasherOptions::bcrypt(10)).unwrap();
    assert!(b10.needs_rehash(BCRYPTJS_SECRET_4));
    let b10_hash = b10.hash("x").await.unwrap();
    assert!(!b10.needs_rehash(&b10_hash));
    assert!(b10.needs_rehash(&fresh));
    assert!(small_bcrypt().needs_rehash(&fresh));
}

#[tokio::test]
async fn an_unknown_user_costs_one_derivation_like_a_known_one() {
    for h in [small_argon(), small_bcrypt()] {
        let stored = h.hash("secret").await.unwrap();
        let before = derivations(&h);
        assert_eq!(h.verify_or_dummy("secret", Some(&stored)).await, Ok(true));
        let known = derivations(&h) - before;
        let before = derivations(&h);
        assert_eq!(h.verify_or_dummy("secret", None).await, Ok(false));
        let unknown = derivations(&h) - before;
        assert_eq!((known, unknown), (1, 1));
        assert_eq!(h.verify_or_dummy_blocking("secret", None), Ok(false));
        assert_eq!(derivations(&h) - before, 2);
    }
}

#[tokio::test]
async fn the_dummy_algorithm_can_differ() {
    let h = PasswordHasher::new(
        PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 4 }),
    )
    .unwrap();
    assert!(!h.verify_dummy("x").await);
    assert_eq!(derivations(&h), 1);
}

#[tokio::test]
async fn the_dummy_never_matches() {
    let h = small_argon();
    assert!(!h.verify_dummy("").await);
    assert!(!h.verify_dummy("secret").await);
    assert!(!h.verify_dummy("a".repeat(5000)).await);
}

#[tokio::test]
async fn a_malformed_stored_hash_costs_one_derivation() {
    let h = small_argon();
    let before = derivations(&h);
    assert!(h.verify_or_dummy("pw", Some("not-a-hash")).await.is_err());
    assert_eq!(derivations(&h) - before, 1);
    let planted = "$2b$31$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie";
    let before = derivations(&h);
    assert!(h.verify_or_dummy("pw", Some(planted)).await.is_err());
    assert_eq!(derivations(&h) - before, 1);
    assert!(h.verify_or_dummy_blocking("pw", Some("x")).is_err());
}

#[tokio::test]
async fn at_most_max_concurrent_derivations_run_at_once() {
    let h = PasswordHasher::new(PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().max_concurrent(2)).unwrap();
    let stored = h.hash("secret").await.unwrap();
    let base = derivations(&h);
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    let tasks: Vec<_> = (0..5)
        .map(|_| {
            let (h, stored) = (h.clone(), stored.clone());
            tokio::spawn(async move { h.verify("secret", &stored).await })
        })
        .collect();
    while h.probe().running.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    assert_eq!(h.probe().running.load(Ordering::SeqCst), 2);
    assert_eq!(derivations(&h) - base, 2, "the other three wait for a slot");
    gate.open();
    for t in tasks {
        assert_eq!(t.await.unwrap(), Ok(true));
    }
    assert_eq!(derivations(&h) - base, 5);
    assert_eq!(h.probe().peak.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn blocking_variants_work_outside_the_async_workers() {
    let h = small_argon();
    let hash = std::thread::scope(|s| s.spawn(|| h.hash_blocking("secret")).join().unwrap()).unwrap();
    assert!(std::thread::scope(|s| s.spawn(|| h.verify_blocking("secret", &hash)).join().unwrap()).unwrap());
}

#[test]
fn debug_output_names_no_secret() {
    let h = small_argon();
    let printed = format!("{h:?}");
    assert!(printed.contains("PasswordHasher") && !printed.contains("$argon2"), "{printed}");
    let e = PasswordError::OverLimit { what: "bcrypt cost" };
    assert!(!e.to_string().contains('$'));
}

#[tokio::test]
async fn the_hasher_is_injectable_with_default_options() {
    use crate::prelude::*;
    #[module(providers = [Provider::class::<PasswordHasher>()])]
    struct M;
    let app = CorvisFactory::create::<M>().await.unwrap();
    let h = app.get::<PasswordHasher>().unwrap();
    assert!(h.hash("x").await.unwrap().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn random_input_never_panics(password in any::<Vec<u8>>(), stored in any::<String>()) {
        let a = small_argon_shared();
        let _ = a.verify_blocking(&password, &stored);
        let _ = a.needs_rehash(&stored);
        let _ = a.verify_or_dummy_blocking(&password, Some(&stored));
    }

    #[test]
    fn mutated_valid_hashes_never_panic(password in "\\PC{0,40}", cut in 0usize..120, at in 0usize..120, byte in any::<char>(), which in 0usize..2) {
        let (a, hashes) = shared();
        let valid = &hashes[which];
        let mut mutated: Vec<char> = valid.chars().collect();
        let at = at % mutated.len();
        mutated[at] = byte;
        let mutated: String = mutated.into_iter().collect();
        let truncated: String = valid.chars().take(cut).collect();
        for stored in [&mutated, &truncated] {
            let _ = a.verify_blocking(&password, stored);
            let _ = a.needs_rehash(stored);
        }
        prop_assert_eq!(a.verify_blocking("secret", valid), Ok(true));
    }
}

fn shared() -> &'static (PasswordHasher, [String; 2]) {
    static SHARED: std::sync::OnceLock<(PasswordHasher, [String; 2])> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| {
        let a = small_argon();
        let b = small_bcrypt();
        let hashes = [a.hash_blocking("secret").unwrap(), b.hash_blocking("secret").unwrap()];
        (a, hashes)
    })
}

fn small_argon_shared() -> &'static PasswordHasher {
    &shared().0
}

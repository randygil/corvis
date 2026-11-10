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
fn the_default_verification_limits_are_low() {
    let l = VerifyLimits::default();
    assert_eq!((l.max_argon2_memory_kib, l.max_argon2_iterations, l.max_argon2_parallelism, l.max_bcrypt_cost), (65_536, 8, 16, 14));
}

#[tokio::test]
async fn the_hasher_always_verifies_its_own_hashes() {
    // Above the default limits (65536 KiB, 8 iterations): the limits are raised to the hasher's own parameters.
    let big = Argon2Params { memory_kib: 100_000, iterations: 9, parallelism: 1 };
    let h = PasswordHasher::new(PasswordHasherOptions::argon2id(big)).unwrap();
    let hash = h.hash("secret").await.unwrap();
    assert!(h.verify("secret", &hash).await.unwrap());
    // Even with explicit lower limits.
    let lower = VerifyLimits::default().max_argon2_memory_kib(8192);
    let h = PasswordHasher::new(PasswordHasherOptions::argon2id(big).verify_limits(lower)).unwrap();
    assert!(h.verify("secret", &hash).await.unwrap());
    // A default hasher refuses the same hash without deriving.
    let default = small_bcrypt();
    let before = derivations(&default);
    assert!(matches!(default.verify("secret", &hash).await, Err(PasswordError::OverLimit { .. })));
    assert_eq!(derivations(&default), before);
}

#[test]
fn verification_limits_have_upper_bounds() {
    let ok = |l: VerifyLimits| PasswordHasher::new(PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().verify_limits(l)).is_ok();
    assert!(ok(VerifyLimits::default().max_argon2_memory_kib(1_048_576).max_argon2_iterations(64).max_bcrypt_cost(14)));
    assert!(!ok(VerifyLimits::default().max_argon2_memory_kib(1_048_577)));
    assert!(!ok(VerifyLimits::default().max_argon2_iterations(65)));
    assert!(!ok(VerifyLimits::default().max_bcrypt_cost(21)));
    // The hasher's own parameters cannot go above the bounds either.
    let big = Argon2Params { memory_kib: 2_000_000, iterations: 2, parallelism: 1 };
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(big)).is_err());
    let many = Argon2Params { memory_kib: 19456, iterations: 65, parallelism: 1 };
    assert!(PasswordHasher::new(PasswordHasherOptions::argon2id(many)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::bcrypt(21)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 21 })).is_err());
}

#[test]
fn invalid_options_stop_construction() {
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_password_bytes(0)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_password_bytes(MAX_PASSWORD_BYTES_CAP + 1)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_concurrent(0)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().max_waiting(0)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().acquire_timeout(std::time::Duration::ZERO)).is_err());
    assert!(PasswordHasher::new(PasswordHasherOptions::default().acquire_timeout(std::time::Duration::from_secs(301))).is_err());
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
}

#[tokio::test]
async fn verifying_an_oversized_password_is_false_without_derivation() {
    let h = small_argon();
    let hash = h.hash("secret").await.unwrap();
    let before = derivations(&h);
    assert_eq!(h.verify("a".repeat(1025), &hash).await, Ok(false));
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

#[tokio::test(flavor = "multi_thread")]
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
    }
}

#[tokio::test]
async fn the_dummy_algorithm_can_differ() {
    let h = PasswordHasher::new(
        PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 4 }),
    )
    .unwrap();
    assert_eq!(h.verify_dummy("x").await, Ok(false));
    assert_eq!(derivations(&h), 1);
}

#[tokio::test]
async fn the_dummy_never_matches() {
    let h = small_argon();
    assert_eq!(h.verify_dummy("").await, Ok(false));
    assert_eq!(h.verify_dummy("secret").await, Ok(false));
    assert_eq!(h.verify_dummy("a".repeat(5000)).await, Ok(false));
}

const REFUSED: [&str; 8] = [
    "",
    "not-a-hash",
    "$2b$15$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie",
    "$2b$",
    "$2x$04$EGdrhbKUv8Oc9vGiXX0HQOxSg445d458Muh7DAHskb6QbtCvdxcie",
    "$scrypt$ln=16,r=8,p=1$aM15713r3Xsvxbi31lqr1Q$nFNh2CVHVjNldFVKDHDlm4CbdRSCdEBsjjJxD+iCs5E",
    "$argon2id$v=19$m=4194304,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG",
    "$argon2id$",
];

#[tokio::test]
async fn a_refused_stored_hash_answers_false_after_one_derivation() {
    let h = small_argon();
    for stored in REFUSED {
        let before = derivations(&h);
        assert_eq!(h.verify_or_dummy("pw", Some(stored)).await, Ok(false), "{stored:?}");
        assert_eq!(derivations(&h) - before, 1, "{stored:?}: the dummy verification ran");
        // verify keeps reporting the refusal.
        assert!(h.verify("pw", stored).await.unwrap_err().is_refused_hash(), "{stored:?}");
    }
}

#[test]
fn a_refused_stored_hash_answers_false_in_the_blocking_variant_too() {
    let h = small_argon();
    for stored in REFUSED {
        let before = derivations(&h);
        assert_eq!(h.verify_or_dummy_blocking("pw", Some(stored)), Ok(false), "{stored:?}");
        assert_eq!(derivations(&h) - before, 1, "{stored:?}");
        assert!(h.verify_blocking("pw", stored).unwrap_err().is_refused_hash());
    }
}

#[tokio::test]
async fn bcrypt_hashes_with_non_canonical_trailing_bits_are_refused_before_any_derivation() {
    let h = small_bcrypt();
    assert!(h.verify("secret", BCRYPTJS_SECRET_4).await.unwrap());
    let before = derivations(&h);
    // The last salt character `u` (value 48) and the last digest character `K` (value 12) have zero low bits; the
    // next character in each position sets one.
    let salt_bits = BCRYPTJS_SECRET_4.replacen("BusLeI", "BvsLeI", 1);
    let digest_bits = format!("{}L", &BCRYPTJS_SECRET_4[..59]);
    for planted in [salt_bits, digest_bits] {
        assert_ne!(planted, BCRYPTJS_SECRET_4);
        assert_eq!(h.verify("secret", &planted).await, Err(PasswordError::Malformed), "{planted}");
        assert!(h.needs_rehash(&planted));
    }
    assert_eq!(derivations(&h), before, "no derivation for a refused hash");
}

#[tokio::test]
async fn a_stored_hash_that_differs_from_the_dummy_warns_once() {
    let h = small_argon();
    let argon = h.hash("secret").await.unwrap();
    assert_eq!(h.verify_or_dummy("secret", Some(&argon)).await, Ok(true));
    assert!(!h.mismatch_warned(), "a hash with the dummy's parameters does not warn");
    assert_eq!(h.verify_or_dummy("secret", Some(BCRYPTJS_SECRET_4)).await, Ok(true));
    assert!(h.mismatch_warned(), "a bcrypt hash against an argon2 dummy warns");
    // A matching dummy does not.
    let options = PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 4 });
    let migrating = PasswordHasher::new(options).unwrap();
    assert_eq!(migrating.verify_or_dummy("secret", Some(BCRYPTJS_SECRET_4)).await, Ok(true));
    assert!(!migrating.mismatch_warned());
    // Refused hashes are not a mismatch: they never parse.
    assert_eq!(migrating.verify_or_dummy("secret", Some("")).await, Ok(false));
    assert!(!migrating.mismatch_warned());
}

fn busy_hasher(waiting: usize) -> PasswordHasher {
    PasswordHasher::new(PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().max_concurrent(1).max_waiting(waiting)).unwrap()
}

async fn until(mut condition: impl FnMut() -> bool) {
    while !condition() {
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn a_full_queue_fails_fast_with_busy_and_a_cancelled_waiter_frees_its_place() {
    let h = busy_hasher(2);
    let stored = h.hash("secret").await.unwrap();
    let base = derivations(&h);
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    let spawn = |h: &PasswordHasher| {
        let (h, stored) = (h.clone(), stored.clone());
        tokio::spawn(async move { h.verify("secret", &stored).await })
    };
    let running = spawn(&h);
    until(|| h.probe().running.load(Ordering::SeqCst) == 1).await;
    let (second, third) = (spawn(&h), spawn(&h));
    until(|| h.waiting() == 2).await;
    // The next call fails at once, with no derivation, whichever method it is.
    assert_eq!(h.verify("secret", &stored).await, Err(PasswordError::Busy));
    assert_eq!(h.verify_or_dummy("secret", None).await, Err(PasswordError::Busy));
    assert_eq!(h.verify_or_dummy("secret", Some("not-a-hash")).await, Err(PasswordError::Busy));
    assert_eq!(h.hash("secret").await, Err(PasswordError::Busy));
    assert_eq!(derivations(&h) - base, 1, "Busy ran no derivation");
    // A waiter that is cancelled frees its place for the next call.
    third.abort();
    let _ = third.await;
    until(|| h.waiting() == 1).await;
    let fourth = spawn(&h);
    until(|| h.waiting() == 2).await;
    assert_eq!(h.verify("secret", &stored).await, Err(PasswordError::Busy));
    gate.open();
    for t in [running, second, fourth] {
        assert_eq!(t.await.unwrap(), Ok(true));
    }
    assert_eq!(h.waiting(), 0);
    // And once the queue drains, calls are served again.
    *h.probe().gate.lock().unwrap() = None;
    assert_eq!(h.verify("secret", &stored).await, Ok(true));
}

#[tokio::test]
async fn a_wait_that_times_out_fails_with_busy() {
    let options = PasswordHasherOptions::argon2id(SMALL)
        .allow_weak_parameters()
        .max_concurrent(1)
        .acquire_timeout(std::time::Duration::from_millis(50));
    let h = PasswordHasher::new(options).unwrap();
    let stored = h.hash("secret").await.unwrap();
    let base = derivations(&h);
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    let (hh, ss) = (h.clone(), stored.clone());
    let running = tokio::spawn(async move { hh.verify("secret", &ss).await });
    until(|| h.probe().running.load(Ordering::SeqCst) == 1).await;
    assert_eq!(h.verify("secret", &stored).await, Err(PasswordError::Busy));
    assert_eq!(h.waiting(), 0, "the timed out waiter left the queue");
    assert_eq!(derivations(&h) - base, 1);
    gate.open();
    assert_eq!(running.await.unwrap(), Ok(true));
}

#[test]
fn the_blocking_variants_fail_fast_with_busy_too() {
    let h = busy_hasher(1);
    let stored = h.hash_blocking("secret").unwrap();
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    std::thread::scope(|s| {
        let holder = s.spawn(|| h.verify_blocking("secret", &stored));
        while h.probe().running.load(Ordering::SeqCst) < 1 {
            std::thread::yield_now();
        }
        let waiter = s.spawn(|| h.verify_blocking("secret", &stored));
        while h.waiting() < 1 {
            std::thread::yield_now();
        }
        assert_eq!(h.verify_blocking("secret", &stored), Err(PasswordError::Busy));
        gate.open();
        assert_eq!(holder.join().unwrap(), Ok(true));
        assert_eq!(waiter.join().unwrap(), Ok(true));
    });
}

#[test]
fn busy_is_a_503_and_everything_else_a_500() {
    assert_eq!(PasswordError::Busy.into_http_exception().get_status(), 503);
    assert_eq!(PasswordError::Internal.into_http_exception().get_status(), 500);
    assert_eq!(PasswordError::InsideRuntime.into_http_exception().get_status(), 500);
}

/// Every blocking method, called from `where`; each must answer `InsideRuntime` and derive nothing.
fn assert_blocking_refused(h: &PasswordHasher, stored: &str) {
    let before = derivations(h);
    assert_eq!(h.hash_blocking("secret"), Err(PasswordError::InsideRuntime));
    assert_eq!(h.verify_blocking("secret", stored), Err(PasswordError::InsideRuntime));
    assert_eq!(h.verify_dummy_blocking("secret"), Err(PasswordError::InsideRuntime));
    assert_eq!(h.verify_or_dummy_blocking("secret", None), Err(PasswordError::InsideRuntime));
    assert_eq!(h.verify_or_dummy_blocking("secret", Some(stored)), Err(PasswordError::InsideRuntime));
    // Not even the cheap checks run first: an oversized password is an error too, not `Ok(false)` or `TooLong`.
    assert_eq!(h.hash_blocking("a".repeat(5000)), Err(PasswordError::InsideRuntime));
    assert_eq!(h.verify_blocking("a".repeat(5000), stored), Err(PasswordError::InsideRuntime));
    assert_eq!(derivations(h), before, "no derivation ran");
    assert_eq!(h.waiting(), 0, "nothing waited");
}

const ARGON_SMALL_HASH: &str = "$argon2id$v=19$m=8,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$RdescudvJCsgt3ub+b+dWRWJTmaaJObG";

#[test]
fn blocking_variants_refuse_to_run_inside_any_runtime() {
    let h = small_argon();
    let base = derivations(&h);
    // Multi-thread runtime: a worker, a block_on thread, a spawn_blocking thread and a LocalSet.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let hh = h.clone();
    rt.block_on(async {
        assert_blocking_refused(&hh, ARGON_SMALL_HASH);
        let on_worker = hh.clone();
        tokio::spawn(async move { assert_blocking_refused(&on_worker, ARGON_SMALL_HASH) }).await.unwrap();
        let on_blocking = hh.clone();
        tokio::task::spawn_blocking(move || assert_blocking_refused(&on_blocking, ARGON_SMALL_HASH)).await.unwrap();
        let local = tokio::task::LocalSet::new();
        let on_local = hh.clone();
        local
            .run_until(async move {
                tokio::task::spawn_local(async move { assert_blocking_refused(&on_local, ARGON_SMALL_HASH) }).await.unwrap()
            })
            .await;
    });
    // Current-thread runtime, with and without a LocalSet.
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let hh = h.clone();
    rt.block_on(async {
        assert_blocking_refused(&hh, ARGON_SMALL_HASH);
        let local = tokio::task::LocalSet::new();
        let on_local = hh.clone();
        local
            .run_until(async move {
                tokio::task::spawn_local(async move { assert_blocking_refused(&on_local, ARGON_SMALL_HASH) }).await.unwrap()
            })
            .await;
        let on_blocking = hh.clone();
        tokio::task::spawn_blocking(move || assert_blocking_refused(&on_blocking, ARGON_SMALL_HASH)).await.unwrap();
    });
    assert_eq!(derivations(&h), base);
    // The same hasher works from a thread without a runtime.
    let hash = std::thread::scope(|s| s.spawn(|| h.hash_blocking("secret")).join().unwrap()).unwrap();
    assert!(std::thread::scope(|s| s.spawn(|| h.verify_blocking("secret", &hash)).join().unwrap()).unwrap());
}

#[test]
fn blocking_variants_give_busy_after_the_acquire_timeout_outside_a_runtime() {
    let timeout = std::time::Duration::from_millis(100);
    let options = PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().max_concurrent(1).acquire_timeout(timeout);
    let h = PasswordHasher::new(options).unwrap();
    let stored = h.hash_blocking("secret").unwrap();
    let base = derivations(&h);
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    std::thread::scope(|s| {
        let holder = s.spawn(|| h.verify_blocking("secret", &stored));
        while h.probe().running.load(Ordering::SeqCst) < 1 {
            std::thread::yield_now();
        }
        let started = std::time::Instant::now();
        assert_eq!(h.verify_blocking("secret", &stored), Err(PasswordError::Busy));
        assert!(started.elapsed() >= timeout, "it waited for the whole timeout");
        assert_eq!(h.hash_blocking("secret"), Err(PasswordError::Busy));
        assert_eq!(h.verify_or_dummy_blocking("secret", Some("")), Err(PasswordError::Busy));
        assert_eq!(h.waiting(), 0, "the timed out waiters left the queue");
        assert_eq!(derivations(&h) - base, 1, "Busy ran no derivation");
        gate.open();
        assert_eq!(holder.join().unwrap(), Ok(true));
    });
    // Once the slot is free the same call succeeds.
    *h.probe().gate.lock().unwrap() = None;
    assert_eq!(h.verify_blocking("secret", &stored), Ok(true));
}

#[test]
fn a_blocking_waiter_gets_the_slot_when_it_frees_before_the_timeout() {
    let options = PasswordHasherOptions::argon2id(SMALL).allow_weak_parameters().max_concurrent(1);
    let h = PasswordHasher::new(options).unwrap();
    let stored = h.hash_blocking("secret").unwrap();
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    std::thread::scope(|s| {
        let holder = s.spawn(|| h.verify_blocking("secret", &stored));
        while h.probe().running.load(Ordering::SeqCst) < 1 {
            std::thread::yield_now();
        }
        let waiter = s.spawn(|| h.verify_blocking("secret", &stored));
        while h.waiting() < 1 {
            std::thread::yield_now();
        }
        gate.open();
        assert_eq!(holder.join().unwrap(), Ok(true));
        assert_eq!(waiter.join().unwrap(), Ok(true));
    });
}

/// The review's mixed load: many async verifications and hashes at once on a small multi-thread runtime, with a queue
/// that overflows. Every call ends with `Ok` or `Busy`, and a timer keeps firing while it runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mixed_async_load_finishes_with_ok_or_busy_and_the_timer_keeps_firing() {
    let options = PasswordHasherOptions::argon2id(SMALL)
        .allow_weak_parameters()
        .max_concurrent(2)
        .max_waiting(16)
        .acquire_timeout(std::time::Duration::from_secs(30));
    let h = PasswordHasher::new(options).unwrap();
    let stored = h.hash("secret").await.unwrap();
    let probe = tokio::spawn(async {
        for _ in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    });
    let tasks: Vec<_> = (0..400)
        .map(|i| {
            let (h, stored) = (h.clone(), stored.clone());
            tokio::spawn(
                async move { if i % 3 == 0 { h.hash("new password").await.map(|_| true) } else { h.verify("secret", &stored).await } },
            )
        })
        .collect();
    let (mut ok, mut busy) = (0, 0);
    for t in tasks {
        match t.await.unwrap() {
            Ok(_) => ok += 1,
            Err(PasswordError::Busy) => busy += 1,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(ok + busy, 400);
    assert!(ok >= 1);
    tokio::time::timeout(std::time::Duration::from_secs(60), probe).await.expect("the timer kept firing").unwrap();
    assert_eq!(h.waiting(), 0);
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

#[test]
fn the_blocking_variants_check_lengths_outside_a_runtime() {
    let h = small_argon();
    assert_eq!(h.hash_blocking("a".repeat(1025)), Err(PasswordError::TooLong { max: 1024 }));
    let hash = h.hash_blocking("secret").unwrap();
    let before = derivations(&h);
    assert_eq!(h.verify_blocking("a".repeat(1025), &hash), Ok(false));
    assert_eq!(derivations(&h), before);
}

#[tokio::test]
async fn a_refused_hash_warns_only_after_the_dummy_ran() {
    let options = PasswordHasherOptions::argon2id(SMALL)
        .allow_weak_parameters()
        .max_concurrent(1)
        .acquire_timeout(std::time::Duration::from_millis(50));
    let h = PasswordHasher::new(options).unwrap();
    let stored = h.hash("secret").await.unwrap();
    let gate = Arc::new(Gate::default());
    *h.probe().gate.lock().unwrap() = Some(gate.clone());
    let (hh, ss) = (h.clone(), stored.clone());
    let running = tokio::spawn(async move { hh.verify("secret", &ss).await });
    until(|| h.probe().running.load(Ordering::SeqCst) == 1).await;
    // The only slot is taken: the dummy cannot run, so nothing is reported for the refused hash.
    assert_eq!(h.verify_or_dummy("pw", Some("")).await, Err(PasswordError::Busy));
    assert_eq!(h.verify_or_dummy("pw", Some("$scrypt$x")).await, Err(PasswordError::Busy));
    assert_eq!(h.refused_reports(), 0);
    gate.open();
    assert_eq!(running.await.unwrap(), Ok(true));
    *h.probe().gate.lock().unwrap() = None;
    assert_eq!(h.verify_or_dummy("pw", Some("")).await, Ok(false));
    assert_eq!(h.refused_reports(), 1, "reported once the dummy verification ran");
}

#[test]
fn refusal_warnings_are_limited_to_one_per_reason_per_minute() {
    let h = small_argon();
    let t0 = std::time::Instant::now();
    let at = |secs: u64| t0 + std::time::Duration::from_secs(secs);
    let first = h.refused_message_at(&PasswordError::Malformed, at(0)).expect("the first one is written");
    assert!(!first.contains("not logged"), "{first}");
    for second in 1..=3 {
        assert_eq!(h.refused_message_at(&PasswordError::Malformed, at(second)), None);
    }
    // Another reason has its own allowance.
    assert!(h.refused_message_at(&PasswordError::Unsupported, at(4)).is_some());
    assert!(h.refused_message_at(&PasswordError::OverLimit { what: "bcrypt cost" }, at(4)).is_some());
    assert_eq!(h.refused_message_at(&PasswordError::Malformed, at(59)), None);
    let next = h.refused_message_at(&PasswordError::Malformed, at(61)).expect("a minute later it is written again");
    assert!(next.contains("4 more of this kind"), "{next}");
    assert_eq!(h.refused_message_at(&PasswordError::Malformed, at(62)), None);
    assert!(!next.contains('$'));
}

#[tokio::test]
async fn an_argon2_hash_with_a_short_salt_is_refused_before_any_slot_is_taken() {
    let h = small_argon();
    let short = "$argon2id$v=19$m=8,t=1,p=1$YWJjZGVmZw$RdescudvJCsgt3ub+b+dWRWJTmaaJObG"; // 7-byte salt
    let enough = "$argon2id$v=19$m=8,t=1,p=1$YWJjZGVmZ2g$RdescudvJCsgt3ub+b+dWRWJTmaaJObG"; // 8-byte salt
    let base = derivations(&h);
    assert_eq!(h.verify("secret", short).await, Err(PasswordError::Malformed));
    assert!(h.needs_rehash(short));
    assert_eq!(derivations(&h), base, "no derivation and no slot for the short salt");
    assert_eq!(h.verify("secret", enough).await, Ok(false));
    assert_eq!(derivations(&h) - base, 1);
    // A login with it costs exactly the dummy verification.
    let base = derivations(&h);
    assert_eq!(h.verify_or_dummy("secret", Some(short)).await, Ok(false));
    assert_eq!(derivations(&h) - base, 1);
}

#[test]
fn the_default_max_concurrent_is_capped_so_the_defaults_never_warn() {
    let h = PasswordHasher::new(PasswordHasherOptions::default()).unwrap();
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    assert_eq!(h.slot_count(), cores.min(16));
    assert!(h.slot_count() as u64 * u64::from(VerifyLimits::default().max_argon2_memory_kib) <= 1_048_576);
    // An explicit setting is still honoured.
    let more = PasswordHasher::new(PasswordHasherOptions::default().max_concurrent(32)).unwrap();
    assert_eq!(more.slot_count(), 32);
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

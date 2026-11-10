//! `PasswordHasher`: argon2id hashes for new passwords, verification of argon2 and bcrypt hashes, and the bounds that
//! keep a login flood or a planted hash from burning the server.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use futures::FutureExt;
use rand::RngCore;
use rand::rngs::OsRng;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

use crate::di::{DiError, Injectable, ResolveCtx};
use crate::http::{HttpException, InternalServerErrorException, ServiceUnavailableException};
use crate::logger::Logger;

/// The lowest argon2 memory (KiB) accepted without [`allow_weak_parameters`](PasswordHasherOptions::allow_weak_parameters):
/// the OWASP Password Storage Cheat Sheet minimum for argon2id with two iterations.
pub const MIN_ARGON2_MEMORY_KIB: u32 = 19456;
/// The lowest argon2 iterations accepted with [`MIN_ARGON2_MEMORY_KIB`]. With at least
/// [`ARGON2_MEMORY_FOR_ONE_ITERATION_KIB`] a single iteration is accepted (the other OWASP equivalent).
pub const MIN_ARGON2_ITERATIONS: u32 = 2;
/// The memory (KiB) from which one iteration is enough (46 MiB).
pub const ARGON2_MEMORY_FOR_ONE_ITERATION_KIB: u32 = 47104;
/// The lowest bcrypt cost accepted without [`allow_weak_parameters`](PasswordHasherOptions::allow_weak_parameters).
pub const MIN_BCRYPT_COST: u32 = 10;
/// The longest password a bcrypt hash covers: longer ones are refused when hashing instead of being truncated.
pub const BCRYPT_MAX_PASSWORD_BYTES: usize = 72;
/// The default and the highest value of [`max_password_bytes`](PasswordHasherOptions::max_password_bytes) is bounded
/// by this cap so a misconfiguration cannot turn the hasher into a memory sink.
pub const MAX_PASSWORD_BYTES_CAP: usize = 65536;

/// The highest `max_argon2_memory_kib` a [`VerifyLimits`] may have (1 GiB), also the highest argon2 memory a hasher may
/// be configured with.
pub const MAX_VERIFY_ARGON2_MEMORY_KIB: u32 = 1_048_576;
/// The highest `max_argon2_iterations` a [`VerifyLimits`] may have.
pub const MAX_VERIFY_ARGON2_ITERATIONS: u32 = 64;
/// The highest `max_bcrypt_cost` a [`VerifyLimits`] may have.
pub const MAX_VERIFY_BCRYPT_COST: u32 = 20;
/// The default of `max_waiting` is this many waiting calls per `max_concurrent` slot.
pub const DEFAULT_WAITING_PER_SLOT: usize = 32;
/// The default of `acquire_timeout`.
pub const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);

const DEFAULT_MAX_PASSWORD_BYTES: usize = 1024;
const MAX_CONCURRENT_CAP: usize = 1024;
const MAX_WAITING_CAP: usize = 1_000_000;
const MIN_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(300);
/// Above this many KiB (1 GiB) of memory for the verifications that may run at once, a warning is logged at startup.
const MEMORY_WARNING_KIB: u64 = 1_048_576;
const BCRYPT_HASH_LEN: usize = 60;
const BCRYPT_ALPHABET_OK: fn(u8) -> bool = |b| b == b'.' || b == b'/' || b.is_ascii_alphanumeric();

/// Why hashing or verifying failed. The text never contains the password or the stored hash.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordError {
    /// The password is longer than the limit (`max_password_bytes`, or 72 bytes for a bcrypt hash).
    #[error("the password is longer than {max} bytes")]
    TooLong { max: usize },
    /// The stored hash is not a well-formed hash.
    #[error("malformed password hash")]
    Malformed,
    /// The stored hash uses an algorithm or a format this hasher does not verify.
    #[error("unsupported password hash")]
    Unsupported,
    /// The stored hash asks for more than the verification limits allow (`what` names the limit).
    #[error("the password hash is over the verification limit of {what}")]
    OverLimit { what: &'static str },
    /// The options are invalid (the hasher could not be created).
    #[error("invalid password hasher options: {0}")]
    Config(&'static str),
    /// The operating system, the hashing crate or a blocking task failed.
    #[error("password hashing failed")]
    Internal,
    /// Every hashing slot is taken and the waiting queue is full or the wait timed out. No key derivation ran: answer
    /// `503 Service Unavailable` (see [`into_http_exception`](PasswordError::into_http_exception)).
    #[error("the password hasher is busy")]
    Busy,
    /// A blocking method was called on a current-thread Tokio runtime, where waiting for a slot could block the only
    /// thread that releases it. Use the async method.
    #[error("blocking password methods cannot run on a current-thread runtime; use the async methods")]
    CurrentThreadRuntime,
}

impl PasswordError {
    /// `true` for the errors that say the stored hash itself is refused: malformed, unsupported or over a limit.
    pub fn is_refused_hash(&self) -> bool {
        matches!(self, PasswordError::Malformed | PasswordError::Unsupported | PasswordError::OverLimit { .. })
    }

    /// The response a login validator gives for this error: `503 Service Unavailable` for [`Busy`](Self::Busy), the
    /// same for every user, and an empty `500` for anything else. The text never says which account was involved.
    pub fn into_http_exception(self) -> HttpException {
        match self {
            PasswordError::Busy => ServiceUnavailableException::empty(),
            _ => InternalServerErrorException::empty(),
        }
    }
}

/// Argon2 cost parameters. The default is the OWASP minimum, which is also the `argon2` crate's default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2Params {
    /// Memory in KiB.
    pub memory_kib: u32,
    /// Number of passes.
    pub iterations: u32,
    /// Degree of parallelism (lanes).
    pub parallelism: u32,
}

impl Default for Argon2Params {
    fn default() -> Self {
        Argon2Params { memory_kib: MIN_ARGON2_MEMORY_KIB, iterations: MIN_ARGON2_ITERATIONS, parallelism: 1 }
    }
}

/// The algorithm of new hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Algorithm {
    /// argon2id, version 19, 16-byte salt, 32-byte output, PHC string format.
    Argon2id(Argon2Params),
    /// bcrypt (`$2b$`), for databases the Node side keeps reading.
    Bcrypt { cost: u32 },
}

/// What `verify_dummy` hashes and verifies when there is no stored hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DummyAlgorithm {
    /// The algorithm and parameters of new hashes.
    #[default]
    Configured,
    /// bcrypt with this cost, for a migration where most stored hashes are bcrypt.
    Bcrypt { cost: u32 },
    /// argon2id with these parameters.
    Argon2id(Argon2Params),
}

/// What a stored hash may ask for before it is refused without any key derivation.
///
/// The defaults are low on purpose: a hash planted in the database costs what it asks for on every login attempt. The
/// hasher raises each limit to its own hashing parameters when those are higher, so its own hashes always verify. The
/// limits have upper bounds ([`MAX_VERIFY_ARGON2_MEMORY_KIB`], [`MAX_VERIFY_ARGON2_ITERATIONS`],
/// [`MAX_VERIFY_BCRYPT_COST`]); a value above one makes [`PasswordHasher::new`] fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyLimits {
    /// Longest stored hash in bytes (512).
    pub max_hash_bytes: usize,
    /// Highest bcrypt cost (14).
    pub max_bcrypt_cost: u32,
    /// Highest argon2 memory in KiB (65536).
    pub max_argon2_memory_kib: u32,
    /// Highest argon2 iterations (8).
    pub max_argon2_iterations: u32,
    /// Highest argon2 parallelism (16).
    pub max_argon2_parallelism: u32,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        VerifyLimits {
            max_hash_bytes: 512,
            max_bcrypt_cost: 14,
            max_argon2_memory_kib: 65_536,
            max_argon2_iterations: 8,
            max_argon2_parallelism: 16,
        }
    }
}

impl VerifyLimits {
    pub fn max_hash_bytes(mut self, v: usize) -> Self {
        self.max_hash_bytes = v;
        self
    }
    pub fn max_bcrypt_cost(mut self, v: u32) -> Self {
        self.max_bcrypt_cost = v;
        self
    }
    pub fn max_argon2_memory_kib(mut self, v: u32) -> Self {
        self.max_argon2_memory_kib = v;
        self
    }
    pub fn max_argon2_iterations(mut self, v: u32) -> Self {
        self.max_argon2_iterations = v;
        self
    }
    pub fn max_argon2_parallelism(mut self, v: u32) -> Self {
        self.max_argon2_parallelism = v;
        self
    }
}

/// Options of [`PasswordHasher::new`]. The default is argon2id with 19456 KiB, 2 iterations and parallelism 1.
///
/// Parameters below the documented floor ([`MIN_ARGON2_MEMORY_KIB`] with [`MIN_ARGON2_ITERATIONS`], or
/// [`ARGON2_MEMORY_FOR_ONE_ITERATION_KIB`] with one iteration; bcrypt [`MIN_BCRYPT_COST`]) make `new` fail unless
/// [`allow_weak_parameters`](Self::allow_weak_parameters) is set (tests do), which logs a warning.
///
/// At most [`max_concurrent`](Self::max_concurrent) derivations run at once. A call that finds every slot taken waits,
/// up to [`max_waiting`](Self::max_waiting) calls for at most [`acquire_timeout`](Self::acquire_timeout); beyond that
/// it fails with [`PasswordError::Busy`] without deriving anything.
#[derive(Debug, Clone)]
pub struct PasswordHasherOptions {
    algorithm: Algorithm,
    max_password_bytes: usize,
    max_concurrent: Option<usize>,
    max_waiting: Option<usize>,
    acquire_timeout: Duration,
    verify_limits: VerifyLimits,
    dummy_algorithm: DummyAlgorithm,
    allow_weak: bool,
}

impl Default for PasswordHasherOptions {
    fn default() -> Self {
        Self::argon2id(Argon2Params::default())
    }
}

impl PasswordHasherOptions {
    /// New hashes with argon2id and these parameters.
    pub fn argon2id(params: Argon2Params) -> Self {
        PasswordHasherOptions {
            algorithm: Algorithm::Argon2id(params),
            max_password_bytes: DEFAULT_MAX_PASSWORD_BYTES,
            max_concurrent: None,
            max_waiting: None,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            verify_limits: VerifyLimits::default(),
            dummy_algorithm: DummyAlgorithm::Configured,
            allow_weak: false,
        }
    }

    /// New hashes with bcrypt (`$2b$`) at `cost` (10 is the `bcryptjs` default).
    pub fn bcrypt(cost: u32) -> Self {
        PasswordHasherOptions { algorithm: Algorithm::Bcrypt { cost }, ..Self::default() }
    }

    /// The longest password hashed or verified, 1024 bytes by default (at most [`MAX_PASSWORD_BYTES_CAP`]).
    pub fn max_password_bytes(mut self, max: usize) -> Self {
        self.max_password_bytes = max;
        self
    }

    /// The most key derivations running at once, `available_parallelism()` by default.
    pub fn max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = Some(max);
        self
    }

    /// The most calls waiting for a slot, `32 * max_concurrent` by default (at least 1). The next call fails with
    /// [`PasswordError::Busy`].
    pub fn max_waiting(mut self, max: usize) -> Self {
        self.max_waiting = Some(max);
        self
    }

    /// The longest a call waits for a slot, 10 seconds by default (between 1 ms and 300 s). Then it fails with
    /// [`PasswordError::Busy`].
    pub fn acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = timeout;
        self
    }

    /// The verification limits for stored hashes.
    pub fn verify_limits(mut self, limits: VerifyLimits) -> Self {
        self.verify_limits = limits;
        self
    }

    /// The algorithm of the dummy hash used for unknown users.
    pub fn dummy_algorithm(mut self, dummy: DummyAlgorithm) -> Self {
        self.dummy_algorithm = dummy;
        self
    }

    /// Opts out of the parameter floor. Meant for tests; a warning is logged when the hasher is created.
    pub fn allow_weak_parameters(mut self) -> Self {
        self.allow_weak = true;
        self
    }
}

/// How a stored hash was understood, after every limit was checked and before any key derivation.
enum Parsed<'a> {
    Bcrypt { cost: u32 },
    Argon2 { hash: Box<PasswordHash<'a>>, algorithm: argon2::Algorithm, version: argon2::Version, params: argon2::Params, salt_len: usize },
}

struct Inner {
    algorithm: Algorithm,
    max_password_bytes: usize,
    limits: VerifyLimits,
    slots: Arc<Semaphore>,
    waiting: AtomicUsize,
    max_waiting: usize,
    acquire_timeout: Duration,
    dummy_hash: String,
    dummy: Algorithm,
    mismatch_warned: AtomicBool,
    #[cfg(test)]
    probe: probe::Probe,
}

/// Hashes and verifies passwords. Cloning is cheap (shared state). Key derivations run on blocking threads, at most
/// `max_concurrent` at a time; construction hashes one dummy password, so create it once (it is a singleton when
/// registered with `Provider::class::<PasswordHasher>()` or `Provider::use_value`).
///
/// ```ignore
/// let hasher = PasswordHasher::new(PasswordHasherOptions::default())?;
/// let hash = hasher.hash("secret").await?;                       // $argon2id$v=19$m=19456,t=2,p=1$...
/// assert!(hasher.verify("secret", &hash).await?);
/// // in a login validator: one real verification whether the user exists or not
/// let ok = hasher.verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str())).await?;
/// ```
#[derive(Clone)]
pub struct PasswordHasher {
    inner: Arc<Inner>,
}

/// Prints the algorithm only.
impl std::fmt::Debug for PasswordHasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordHasher").field("algorithm", &self.inner.algorithm).finish_non_exhaustive()
    }
}

impl Injectable for PasswordHasher {
    /// A hasher with the default options. For other options, `Provider::use_value(PasswordHasher::new(..)?)`.
    fn construct(_ctx: ResolveCtx) -> futures::future::BoxFuture<'static, Result<Self, DiError>> {
        async {
            let built = tokio::task::spawn_blocking(|| PasswordHasher::new(PasswordHasherOptions::default())).await;
            match built {
                Ok(Ok(h)) => Ok(h),
                _ => Err(DiError::Factory {
                    token: "PasswordHasher".to_string(),
                    cause: Arc::new(anyhow::anyhow!("the password hasher could not be created")),
                }),
            }
        }
        .boxed()
    }
}

/// The value of a character of the bcrypt alphabet `./A-Za-z0-9` (already checked to belong to it).
fn bcrypt_value(c: u8) -> u8 {
    match c {
        b'.' => 0,
        b'/' => 1,
        b'A'..=b'Z' => c - b'A' + 2,
        b'a'..=b'z' => c - b'a' + 28,
        _ => c - b'0' + 54,
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], PasswordError> {
    let mut bytes = [0u8; N];
    OsRng.try_fill_bytes(&mut bytes).map_err(|_| PasswordError::Internal)?;
    Ok(bytes)
}

fn argon2_params(p: &Argon2Params) -> Result<argon2::Params, PasswordError> {
    argon2::Params::new(p.memory_kib, p.iterations, p.parallelism, Some(32))
        .map_err(|_| PasswordError::Config("argon2 parameters out of range"))
}

fn check_floor(algorithm: &Algorithm) -> Result<(), PasswordError> {
    match algorithm {
        Algorithm::Argon2id(p) => {
            let ok = (p.memory_kib >= MIN_ARGON2_MEMORY_KIB && p.iterations >= MIN_ARGON2_ITERATIONS)
                || (p.memory_kib >= ARGON2_MEMORY_FOR_ONE_ITERATION_KIB && p.iterations >= 1);
            if ok {
                Ok(())
            } else {
                Err(PasswordError::Config(
                    "argon2 parameters are below the minimum (19456 KiB with 2 iterations, or 47104 KiB with 1); set allow_weak_parameters() to opt out",
                ))
            }
        }
        Algorithm::Bcrypt { cost } => {
            if *cost >= MIN_BCRYPT_COST {
                Ok(())
            } else {
                Err(PasswordError::Config("bcrypt cost is below the minimum of 10; set allow_weak_parameters() to opt out"))
            }
        }
    }
}

/// The parameters must be accepted by the hashing crates and stay within the upper bounds of the verification limits.
fn check_algorithm_range(algorithm: &Algorithm) -> Result<(), PasswordError> {
    match algorithm {
        Algorithm::Argon2id(p) => {
            argon2_params(p)?;
            if p.memory_kib > MAX_VERIFY_ARGON2_MEMORY_KIB || p.iterations > MAX_VERIFY_ARGON2_ITERATIONS {
                return Err(PasswordError::Config("argon2 parameters are above the supported maximum (1048576 KiB, 64 iterations)"));
            }
        }
        Algorithm::Bcrypt { cost } => {
            if !(4..=MAX_VERIFY_BCRYPT_COST).contains(cost) {
                return Err(PasswordError::Config("bcrypt cost must be between 4 and 20"));
            }
        }
    }
    Ok(())
}

fn check_limit_bounds(limits: &VerifyLimits) -> Result<(), PasswordError> {
    if limits.max_hash_bytes < 64 || limits.max_hash_bytes > 4096 {
        return Err(PasswordError::Config("max_hash_bytes must be between 64 and 4096"));
    }
    if limits.max_argon2_memory_kib > MAX_VERIFY_ARGON2_MEMORY_KIB {
        return Err(PasswordError::Config("max_argon2_memory_kib is above the upper bound of 1048576"));
    }
    if limits.max_argon2_iterations > MAX_VERIFY_ARGON2_ITERATIONS {
        return Err(PasswordError::Config("max_argon2_iterations is above the upper bound of 64"));
    }
    if limits.max_bcrypt_cost > MAX_VERIFY_BCRYPT_COST {
        return Err(PasswordError::Config("max_bcrypt_cost is above the upper bound of 20"));
    }
    Ok(())
}

/// Raises each limit to the parameters of a hash this hasher makes itself, so its own hashes always verify.
fn raise_to(limits: &mut VerifyLimits, algorithm: &Algorithm) {
    match algorithm {
        Algorithm::Argon2id(p) => {
            limits.max_argon2_memory_kib = limits.max_argon2_memory_kib.max(p.memory_kib);
            limits.max_argon2_iterations = limits.max_argon2_iterations.max(p.iterations);
            limits.max_argon2_parallelism = limits.max_argon2_parallelism.max(p.parallelism);
        }
        Algorithm::Bcrypt { cost } => limits.max_bcrypt_cost = limits.max_bcrypt_cost.max(*cost),
    }
}

fn describe(algorithm: &Algorithm) -> String {
    match algorithm {
        Algorithm::Argon2id(p) => format!("argon2id m={} t={} p={}", p.memory_kib, p.iterations, p.parallelism),
        Algorithm::Bcrypt { cost } => format!("bcrypt cost {cost}"),
    }
}

impl PasswordHasher {
    /// Creates a hasher and its dummy hash (one key derivation, so call it from a blocking context or at startup).
    /// Fails when the parameters are below the floor (unless weak ones are allowed), above the verification limits,
    /// or out of range.
    pub fn new(options: PasswordHasherOptions) -> Result<Self, PasswordError> {
        let PasswordHasherOptions {
            algorithm,
            max_password_bytes,
            max_concurrent,
            max_waiting,
            acquire_timeout,
            verify_limits,
            dummy_algorithm,
            allow_weak,
        } = options;
        if max_password_bytes == 0 || max_password_bytes > MAX_PASSWORD_BYTES_CAP {
            return Err(PasswordError::Config("max_password_bytes must be between 1 and 65536"));
        }
        let concurrent = match max_concurrent {
            Some(0) => return Err(PasswordError::Config("max_concurrent must be at least 1")),
            Some(n) => n.min(MAX_CONCURRENT_CAP),
            None => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(MAX_CONCURRENT_CAP),
        };
        let waiting = match max_waiting {
            Some(0) => return Err(PasswordError::Config("max_waiting must be at least 1")),
            Some(n) => n.min(MAX_WAITING_CAP),
            None => concurrent.saturating_mul(DEFAULT_WAITING_PER_SLOT).min(MAX_WAITING_CAP),
        };
        if !(MIN_ACQUIRE_TIMEOUT..=MAX_ACQUIRE_TIMEOUT).contains(&acquire_timeout) {
            return Err(PasswordError::Config("acquire_timeout must be between 1 ms and 300 s"));
        }
        check_limit_bounds(&verify_limits)?;
        check_algorithm_range(&algorithm)?;
        match check_floor(&algorithm) {
            Ok(()) => {}
            Err(PasswordError::Config(why)) if allow_weak => {
                Logger::new("PasswordHasher").warn(format!("weak password hashing parameters are in use (tests only): {why}"));
            }
            Err(e) => return Err(e),
        }
        let dummy = match dummy_algorithm {
            DummyAlgorithm::Configured => algorithm,
            DummyAlgorithm::Bcrypt { cost } => Algorithm::Bcrypt { cost },
            DummyAlgorithm::Argon2id(p) => Algorithm::Argon2id(p),
        };
        check_algorithm_range(&dummy)?;
        let mut limits = verify_limits;
        raise_to(&mut limits, &algorithm);
        raise_to(&mut limits, &dummy);
        let worst_kib = concurrent as u64 * u64::from(limits.max_argon2_memory_kib);
        if worst_kib > MEMORY_WARNING_KIB {
            Logger::new("PasswordHasher").warn(format!(
                "max_concurrent ({concurrent}) times the largest argon2 memory a stored hash may ask for ({} KiB) is {} MiB; lower max_concurrent or the verification limits",
                limits.max_argon2_memory_kib,
                worst_kib / 1024
            ));
        }
        let dummy_hash = hash_with(&dummy, random_bytes::<32>()?.as_slice(), None)?;
        Ok(PasswordHasher {
            inner: Arc::new(Inner {
                algorithm,
                max_password_bytes,
                limits,
                slots: Arc::new(Semaphore::new(concurrent)),
                waiting: AtomicUsize::new(0),
                max_waiting: waiting,
                acquire_timeout,
                dummy_hash,
                dummy,
                mismatch_warned: AtomicBool::new(false),
                #[cfg(test)]
                probe: probe::Probe::default(),
            }),
        })
    }

    /// Hashes `password` with the configured algorithm: a PHC string for argon2id, a `$2b$` hash for bcrypt.
    /// Fails with [`PasswordError::TooLong`] over `max_password_bytes` (and over 72 bytes for bcrypt).
    pub async fn hash(&self, password: impl AsRef<[u8]>) -> Result<String, PasswordError> {
        let password = password.as_ref();
        self.inner.check_hash_length(password)?;
        let password = password.to_vec();
        self.run(move |inner| inner.hash_now(&password)).await
    }

    /// `true` when `password` matches `stored`. `Ok(false)` for a password over `max_password_bytes` (no key
    /// derivation runs); an error for a stored hash that is malformed, unsupported or over the verification limits
    /// (no key derivation runs either). The algorithm comes from the hash, not from the configuration.
    pub async fn verify(&self, password: impl AsRef<[u8]>, stored: &str) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        if password.len() > self.inner.max_password_bytes {
            return Ok(false);
        }
        self.inner.parse(stored)?;
        let (password, stored) = (password.to_vec(), stored.to_string());
        self.run(move |inner| inner.verify_now(&password, &stored)).await
    }

    /// One verification against the hash made when the hasher was built, with the configured (or dummy) algorithm.
    /// Always `Ok(false)`; an error only when the hasher is busy. Use it when the user does not exist so the answer
    /// costs the same as for a known user.
    pub async fn verify_dummy(&self, password: impl AsRef<[u8]>) -> Result<bool, PasswordError> {
        let dummy = self.inner.dummy_hash.clone();
        self.verify(password, &dummy).await?;
        Ok(false)
    }

    /// Verifies against `stored` when there is one and runs [`verify_dummy`](Self::verify_dummy) otherwise.
    ///
    /// A stored hash that is refused (malformed, unsupported, over the verification limits: an empty string for an
    /// account without a password, a hash of another system) answers `Ok(false)` after the dummy verification, so it
    /// costs and answers like an unknown user; one warning naming the reason (never the hash or the password) is
    /// logged each time. `Err` is for a busy hasher ([`PasswordError::Busy`]) and for internal failures.
    /// [`verify`](Self::verify) keeps returning the refusal as an error.
    pub async fn verify_or_dummy(&self, password: impl AsRef<[u8]>, stored: Option<&str>) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        let Some(hash) = stored else { return self.verify_dummy(password).await };
        self.inner.note_dummy_mismatch(hash);
        match self.verify(password, hash).await {
            Err(e) if e.is_refused_hash() => {
                self.inner.warn_refused(&e);
                self.verify_dummy(password).await?;
                Ok(false)
            }
            other => other,
        }
    }

    /// `true` when `stored` should be replaced by a fresh hash of the same password: another algorithm or argon2
    /// variant, weaker memory, iterations, output or salt, a lower bcrypt cost, or a hash that cannot be parsed. No
    /// key derivation runs.
    pub fn needs_rehash(&self, stored: &str) -> bool {
        let Ok(parsed) = self.inner.parse(stored) else { return true };
        match (&self.inner.algorithm, parsed) {
            (Algorithm::Bcrypt { cost: want }, Parsed::Bcrypt { cost }) => cost < *want,
            (Algorithm::Argon2id(want), Parsed::Argon2 { algorithm, version, params, salt_len, .. }) => {
                algorithm != argon2::Algorithm::Argon2id
                    || version != argon2::Version::V0x13
                    || params.m_cost() < want.memory_kib
                    || params.t_cost() < want.iterations
                    || params.output_len().unwrap_or(0) < 32
                    || salt_len < 16
            }
            _ => true,
        }
    }

    /// [`hash`](Self::hash) for code outside an async context. It blocks the calling thread.
    ///
    /// Inside a multi-thread Tokio runtime it runs through `tokio::task::block_in_place`, so a synchronous handler may
    /// call it; inside a current-thread runtime it fails with [`PasswordError::CurrentThreadRuntime`]; outside any
    /// runtime it just blocks.
    pub fn hash_blocking(&self, password: impl AsRef<[u8]>) -> Result<String, PasswordError> {
        let password = password.as_ref();
        self.inner.check_hash_length(password)?;
        self.run_blocking(|inner| inner.hash_now(password))
    }

    /// [`verify`](Self::verify) for code outside an async context; see [`hash_blocking`](Self::hash_blocking) for
    /// where it may be called.
    pub fn verify_blocking(&self, password: impl AsRef<[u8]>, stored: &str) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        if password.len() > self.inner.max_password_bytes {
            return Ok(false);
        }
        self.inner.parse(stored)?;
        self.run_blocking(|inner| inner.verify_now(password, stored))
    }

    /// [`verify_dummy`](Self::verify_dummy) for code outside an async context.
    pub fn verify_dummy_blocking(&self, password: impl AsRef<[u8]>) -> Result<bool, PasswordError> {
        self.verify_blocking(password, &self.inner.dummy_hash)?;
        Ok(false)
    }

    /// [`verify_or_dummy`](Self::verify_or_dummy) for code outside an async context; see
    /// [`hash_blocking`](Self::hash_blocking) for where it may be called.
    pub fn verify_or_dummy_blocking(&self, password: impl AsRef<[u8]>, stored: Option<&str>) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        let Some(hash) = stored else { return self.verify_dummy_blocking(password) };
        self.inner.note_dummy_mismatch(hash);
        match self.verify_blocking(password, hash) {
            Err(e) if e.is_refused_hash() => {
                self.inner.warn_refused(&e);
                self.verify_dummy_blocking(password)?;
                Ok(false)
            }
            other => other,
        }
    }

    /// Waits for a slot, then runs `work` on a blocking thread holding it, so the number of running derivations
    /// stays bounded even if the caller stops waiting.
    async fn run<T, F>(&self, work: F) -> Result<T, PasswordError>
    where
        T: Send + 'static,
        F: FnOnce(&Inner) -> Result<T, PasswordError> + Send + 'static,
    {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| PasswordError::Internal)?;
        let permit = self.inner.acquire().await?;
        let inner = self.inner.clone();
        handle
            .spawn_blocking(move || {
                let _permit = permit;
                catch_unwind(AssertUnwindSafe(|| work(&inner))).unwrap_or(Err(PasswordError::Internal))
            })
            .await
            .map_err(|_| PasswordError::Internal)?
    }

    fn run_blocking<T>(&self, work: impl FnOnce(&Inner) -> Result<T, PasswordError>) -> Result<T, PasswordError> {
        let go = || {
            let _permit = self.inner.acquire_blocking()?;
            catch_unwind(AssertUnwindSafe(|| work(&self.inner))).unwrap_or(Err(PasswordError::Internal))
        };
        match tokio::runtime::Handle::try_current() {
            Err(_) => go(),
            Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(go),
            Ok(_) => Err(PasswordError::CurrentThreadRuntime),
        }
    }
}

/// Counts a call that waits for a slot; the place is freed when the call is dropped, even when its future is cancelled.
struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn enter(counter: &'a AtomicUsize, max: usize) -> Option<Self> {
        if counter.fetch_add(1, Ordering::SeqCst) >= max {
            counter.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Waiting(counter))
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Inner {
    /// A free slot at once; otherwise a place in the waiting queue (`Busy` when it is full) and a bounded wait.
    async fn acquire(&self) -> Result<OwnedSemaphorePermit, PasswordError> {
        match self.slots.clone().try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(TryAcquireError::Closed) => return Err(PasswordError::Internal),
            Err(TryAcquireError::NoPermits) => {}
        }
        let _place = Waiting::enter(&self.waiting, self.max_waiting).ok_or(PasswordError::Busy)?;
        match tokio::time::timeout(self.acquire_timeout, self.slots.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(PasswordError::Internal),
            Err(_) => Err(PasswordError::Busy),
        }
    }

    /// [`acquire`](Self::acquire) for a thread that may block: inside a runtime (only after `block_in_place`) it uses
    /// the runtime's timer, outside one it uses a small runtime of its own.
    fn acquire_blocking(&self) -> Result<OwnedSemaphorePermit, PasswordError> {
        match self.slots.clone().try_acquire_owned() {
            Ok(permit) => return Ok(permit),
            Err(TryAcquireError::Closed) => return Err(PasswordError::Internal),
            Err(TryAcquireError::NoPermits) => {}
        }
        let _place = Waiting::enter(&self.waiting, self.max_waiting).ok_or(PasswordError::Busy)?;
        // The timer is created inside the future, where a runtime is running.
        let (slots, timeout) = (self.slots.clone(), self.acquire_timeout);
        let wait = async move { tokio::time::timeout(timeout, slots.acquire_owned()).await };
        let waited = match tokio::runtime::Handle::try_current() {
            Ok(handle) => handle.block_on(wait),
            Err(_) => {
                tokio::runtime::Builder::new_current_thread().enable_time().build().map_err(|_| PasswordError::Internal)?.block_on(wait)
            }
        };
        match waited {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(PasswordError::Internal),
            Err(_) => Err(PasswordError::Busy),
        }
    }

    fn warn_refused(&self, reason: &PasswordError) {
        Logger::new("PasswordHasher")
            .warn(format!("a stored password hash was refused ({reason}); the login is answered as a wrong password"));
    }

    /// Warns once when a stored hash that parses costs something else than the dummy verification, which makes the
    /// response time tell a user with such a hash from an unknown one.
    fn note_dummy_mismatch(&self, stored: &str) {
        let Ok(parsed) = self.parse(stored) else { return };
        let (same, found) = match (&parsed, &self.dummy) {
            (Parsed::Bcrypt { cost }, Algorithm::Bcrypt { cost: want }) => (cost == want, format!("bcrypt cost {cost}")),
            (Parsed::Argon2 { algorithm, params, .. }, Algorithm::Argon2id(want)) => (
                *algorithm == argon2::Algorithm::Argon2id
                    && params.m_cost() == want.memory_kib
                    && params.t_cost() == want.iterations
                    && params.p_cost() == want.parallelism,
                format!("argon2 m={} t={} p={}", params.m_cost(), params.t_cost(), params.p_cost()),
            ),
            (Parsed::Bcrypt { cost }, _) => (false, format!("bcrypt cost {cost}")),
            (Parsed::Argon2 { params, .. }, _) => {
                (false, format!("argon2 m={} t={} p={}", params.m_cost(), params.t_cost(), params.p_cost()))
            }
        };
        if !same && !self.mismatch_warned.swap(true, Ordering::SeqCst) {
            Logger::new("PasswordHasher").warn(format!(
                "a stored password hash ({found}) differs from the dummy hash ({}): logins of unknown users take a different time than logins of known ones. Set dummy_algorithm to the algorithm and cost of the stored hashes and rehash on login (needs_rehash)",
                describe(&self.dummy)
            ));
        }
    }

    fn check_hash_length(&self, password: &[u8]) -> Result<(), PasswordError> {
        if password.len() > self.max_password_bytes {
            return Err(PasswordError::TooLong { max: self.max_password_bytes });
        }
        if matches!(self.algorithm, Algorithm::Bcrypt { .. }) && password.len() > BCRYPT_MAX_PASSWORD_BYTES {
            return Err(PasswordError::TooLong { max: BCRYPT_MAX_PASSWORD_BYTES });
        }
        Ok(())
    }

    fn hash_now(&self, password: &[u8]) -> Result<String, PasswordError> {
        #[cfg(test)]
        let _entered = self.probe.enter();
        hash_with(&self.algorithm, password, Some(self.max_password_bytes))
    }

    /// Reads a stored hash and checks every limit; nothing here derives a key.
    fn parse<'a>(&self, stored: &'a str) -> Result<Parsed<'a>, PasswordError> {
        if stored.len() > self.limits.max_hash_bytes {
            return Err(PasswordError::OverLimit { what: "hash length" });
        }
        if stored.starts_with("$argon2") {
            self.parse_argon2(stored)
        } else if stored.starts_with("$2") {
            self.parse_bcrypt(stored)
        } else if stored.starts_with('$') {
            Err(PasswordError::Unsupported)
        } else {
            Err(PasswordError::Malformed)
        }
    }

    fn parse_bcrypt<'a>(&self, stored: &str) -> Result<Parsed<'a>, PasswordError> {
        let b = stored.as_bytes();
        if b.len() < 4 || b[3] != b'$' || !matches!(b[2], b'a' | b'b' | b'y' | b'x') {
            return Err(PasswordError::Malformed);
        }
        if b[2] == b'x' {
            return Err(PasswordError::Unsupported);
        }
        if b.len() != BCRYPT_HASH_LEN || b[6] != b'$' || !b[4].is_ascii_digit() || !b[5].is_ascii_digit() {
            return Err(PasswordError::Malformed);
        }
        if !b[7..].iter().copied().all(BCRYPT_ALPHABET_OK) {
            return Err(PasswordError::Malformed);
        }
        // The 22-character salt carries 128 bits and the 31-character digest 184: the unused low bits of the last
        // character of each must be zero, as the hashing functions write them. Anything else is not a hash they made.
        if bcrypt_value(b[28]) & 0x0f != 0 || bcrypt_value(b[59]) & 0x03 != 0 {
            return Err(PasswordError::Malformed);
        }
        let cost = u32::from(b[4] - b'0') * 10 + u32::from(b[5] - b'0');
        if !(4..=31).contains(&cost) {
            return Err(PasswordError::Malformed);
        }
        if cost > self.limits.max_bcrypt_cost {
            return Err(PasswordError::OverLimit { what: "bcrypt cost" });
        }
        Ok(Parsed::Bcrypt { cost })
    }

    fn parse_argon2<'a>(&self, stored: &'a str) -> Result<Parsed<'a>, PasswordError> {
        let hash = PasswordHash::new(stored).map_err(|_| PasswordError::Malformed)?;
        let algorithm = argon2::Algorithm::try_from(hash.algorithm).map_err(|_| PasswordError::Unsupported)?;
        let version = match hash.version {
            None => argon2::Version::default(),
            Some(v) => argon2::Version::try_from(v).map_err(|_| PasswordError::Unsupported)?,
        };
        let params = argon2::Params::try_from(&hash).map_err(|_| PasswordError::Malformed)?;
        if params.m_cost() > self.limits.max_argon2_memory_kib {
            return Err(PasswordError::OverLimit { what: "argon2 memory" });
        }
        if params.t_cost() > self.limits.max_argon2_iterations {
            return Err(PasswordError::OverLimit { what: "argon2 iterations" });
        }
        if params.p_cost() > self.limits.max_argon2_parallelism {
            return Err(PasswordError::OverLimit { what: "argon2 parallelism" });
        }
        let (Some(salt), Some(_)) = (hash.salt, hash.hash) else { return Err(PasswordError::Malformed) };
        let mut buf = [0u8; 64];
        let salt_len = salt.decode_b64(&mut buf).map_err(|_| PasswordError::Malformed)?.len();
        Ok(Parsed::Argon2 { hash: Box::new(hash), algorithm, version, params, salt_len })
    }

    fn verify_now(&self, password: &[u8], stored: &str) -> Result<bool, PasswordError> {
        let parsed = self.parse(stored)?;
        #[cfg(test)]
        let _entered = self.probe.enter();
        match parsed {
            Parsed::Bcrypt { .. } => match bcrypt::verify(password, stored) {
                Ok(matched) => Ok(matched),
                Err(_) => Err(PasswordError::Malformed),
            },
            // The comparison of the derived output is constant-time: `password-hash` compares `Output` values with
            // `subtle::ConstantTimeEq`.
            Parsed::Argon2 { hash, algorithm, version, params, .. } => {
                match argon2::Argon2::new(algorithm, version, params).verify_password(password, hash.as_ref()) {
                    Ok(()) => Ok(true),
                    Err(argon2::password_hash::Error::Password) => Ok(false),
                    Err(_) => Err(PasswordError::Malformed),
                }
            }
        }
    }
}

/// One hash with `algorithm`. `max_password` is checked again here so no caller can skip it.
fn hash_with(algorithm: &Algorithm, password: &[u8], max_password: Option<usize>) -> Result<String, PasswordError> {
    if let Some(max) = max_password
        && password.len() > max
    {
        return Err(PasswordError::TooLong { max });
    }
    match algorithm {
        Algorithm::Argon2id(p) => {
            let params = argon2_params(p)?;
            let salt = SaltString::encode_b64(&random_bytes::<16>()?).map_err(|_| PasswordError::Internal)?;
            let hasher = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
            hasher.hash_password(password, &salt).map(|h| h.to_string()).map_err(|_| PasswordError::Internal)
        }
        Algorithm::Bcrypt { cost } => {
            if password.len() > BCRYPT_MAX_PASSWORD_BYTES {
                return Err(PasswordError::TooLong { max: BCRYPT_MAX_PASSWORD_BYTES });
            }
            let parts = bcrypt::hash_with_salt(password, *cost, random_bytes::<16>()?).map_err(|_| PasswordError::Internal)?;
            Ok(parts.format_for_version(bcrypt::Version::TwoB))
        }
    }
}

#[cfg(test)]
impl PasswordHasher {
    pub(crate) fn probe(&self) -> &probe::Probe {
        &self.inner.probe
    }

    /// Calls waiting for a slot right now.
    pub(crate) fn waiting(&self) -> usize {
        self.inner.waiting.load(Ordering::SeqCst)
    }

    /// Whether the one-time warning about a stored hash that differs from the dummy was logged.
    pub(crate) fn mismatch_warned(&self) -> bool {
        self.inner.mismatch_warned.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
pub(crate) mod probe {
    //! Counts the key derivations of a hasher, the peak of concurrent ones, and lets a test hold them.
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};

    #[derive(Default)]
    pub struct Probe {
        pub derivations: AtomicUsize,
        pub running: AtomicUsize,
        pub peak: AtomicUsize,
        pub gate: Mutex<Option<Arc<Gate>>>,
    }

    /// Holds derivations until `open` is called.
    #[derive(Default)]
    pub struct Gate {
        open: Mutex<bool>,
        changed: Condvar,
    }

    impl Gate {
        pub fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.changed.notify_all();
        }
        fn wait(&self) {
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.changed.wait(open).unwrap();
            }
        }
    }

    pub struct Entered<'a>(&'a Probe);

    impl Probe {
        pub fn enter(&self) -> Entered<'_> {
            self.derivations.fetch_add(1, Ordering::SeqCst);
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            let gate = self.gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.wait();
            }
            Entered(self)
        }

        pub fn count(&self) -> usize {
            self.derivations.load(Ordering::SeqCst)
        }
    }

    impl Drop for Entered<'_> {
        fn drop(&mut self) {
            self.0.running.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

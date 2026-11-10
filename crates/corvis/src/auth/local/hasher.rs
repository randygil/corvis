//! `PasswordHasher`: argon2id hashes for new passwords, verification of argon2 and bcrypt hashes, and the bounds that
//! keep a login flood or a planted hash from burning the server.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use futures::FutureExt;
use rand::RngCore;
use rand::rngs::OsRng;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::di::{DiError, Injectable, ResolveCtx};
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

const DEFAULT_MAX_PASSWORD_BYTES: usize = 1024;
const MAX_CONCURRENT_CAP: usize = 1024;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyLimits {
    /// Longest stored hash in bytes (512).
    pub max_hash_bytes: usize,
    /// Highest bcrypt cost (14).
    pub max_bcrypt_cost: u32,
    /// Highest argon2 memory in KiB (262144).
    pub max_argon2_memory_kib: u32,
    /// Highest argon2 iterations (16).
    pub max_argon2_iterations: u32,
    /// Highest argon2 parallelism (16).
    pub max_argon2_parallelism: u32,
}

impl Default for VerifyLimits {
    fn default() -> Self {
        VerifyLimits {
            max_hash_bytes: 512,
            max_bcrypt_cost: 14,
            max_argon2_memory_kib: 262_144,
            max_argon2_iterations: 16,
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
#[derive(Debug, Clone)]
pub struct PasswordHasherOptions {
    algorithm: Algorithm,
    max_password_bytes: usize,
    max_concurrent: Option<usize>,
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
    dummy_hash: String,
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

fn check_algorithm_range(algorithm: &Algorithm, limits: &VerifyLimits) -> Result<(), PasswordError> {
    match algorithm {
        Algorithm::Argon2id(p) => {
            argon2_params(p)?;
            if p.memory_kib > limits.max_argon2_memory_kib
                || p.iterations > limits.max_argon2_iterations
                || p.parallelism > limits.max_argon2_parallelism
            {
                return Err(PasswordError::Config(
                    "argon2 parameters are above the verification limits, the hasher could not verify its own hashes",
                ));
            }
        }
        Algorithm::Bcrypt { cost } => {
            if !(4..=31).contains(cost) {
                return Err(PasswordError::Config("bcrypt cost must be between 4 and 31"));
            }
            if *cost > limits.max_bcrypt_cost {
                return Err(PasswordError::Config(
                    "bcrypt cost is above the verification limit, the hasher could not verify its own hashes",
                ));
            }
        }
    }
    Ok(())
}

impl PasswordHasher {
    /// Creates a hasher and its dummy hash (one key derivation, so call it from a blocking context or at startup).
    /// Fails when the parameters are below the floor (unless weak ones are allowed), above the verification limits,
    /// or out of range.
    pub fn new(options: PasswordHasherOptions) -> Result<Self, PasswordError> {
        let PasswordHasherOptions { algorithm, max_password_bytes, max_concurrent, verify_limits, dummy_algorithm, allow_weak } = options;
        if max_password_bytes == 0 || max_password_bytes > MAX_PASSWORD_BYTES_CAP {
            return Err(PasswordError::Config("max_password_bytes must be between 1 and 65536"));
        }
        let concurrent = match max_concurrent {
            Some(0) => return Err(PasswordError::Config("max_concurrent must be at least 1")),
            Some(n) => n.min(MAX_CONCURRENT_CAP),
            None => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(MAX_CONCURRENT_CAP),
        };
        if verify_limits.max_hash_bytes < 64 || verify_limits.max_hash_bytes > 4096 {
            return Err(PasswordError::Config("max_hash_bytes must be between 64 and 4096"));
        }
        check_algorithm_range(&algorithm, &verify_limits)?;
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
        check_algorithm_range(&dummy, &verify_limits)?;
        let dummy_hash = hash_with(&dummy, random_bytes::<32>()?.as_slice(), None)?;
        Ok(PasswordHasher {
            inner: Arc::new(Inner {
                algorithm,
                max_password_bytes,
                limits: verify_limits,
                slots: Arc::new(Semaphore::new(concurrent)),
                dummy_hash,
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
    /// Always `false`. Use it when the user does not exist so the answer costs the same as for a known user.
    pub async fn verify_dummy(&self, password: impl AsRef<[u8]>) -> bool {
        let dummy = self.inner.dummy_hash.clone();
        let _ = self.verify(password, &dummy).await;
        false
    }

    /// Verifies against `stored` when there is one and runs [`verify_dummy`](Self::verify_dummy) otherwise. When the
    /// stored hash is refused, the dummy verification runs before the error is returned, so a broken stored hash
    /// does not answer faster than an unknown user.
    pub async fn verify_or_dummy(&self, password: impl AsRef<[u8]>, stored: Option<&str>) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        match stored {
            None => Ok(self.verify_dummy(password).await),
            Some(hash) => match self.verify(password, hash).await {
                Err(e) => {
                    self.verify_dummy(password).await;
                    Err(e)
                }
                ok => ok,
            },
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
    pub fn hash_blocking(&self, password: impl AsRef<[u8]>) -> Result<String, PasswordError> {
        let password = password.as_ref();
        self.inner.check_hash_length(password)?;
        self.run_blocking(|inner| inner.hash_now(password))
    }

    /// [`verify`](Self::verify) for code outside an async context. It blocks the calling thread.
    pub fn verify_blocking(&self, password: impl AsRef<[u8]>, stored: &str) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        if password.len() > self.inner.max_password_bytes {
            return Ok(false);
        }
        self.inner.parse(stored)?;
        self.run_blocking(|inner| inner.verify_now(password, stored))
    }

    /// [`verify_or_dummy`](Self::verify_or_dummy) for code outside an async context.
    pub fn verify_or_dummy_blocking(&self, password: impl AsRef<[u8]>, stored: Option<&str>) -> Result<bool, PasswordError> {
        let password = password.as_ref();
        let dummy = || {
            let _ = self.verify_blocking(password, &self.inner.dummy_hash);
        };
        match stored {
            None => {
                dummy();
                Ok(false)
            }
            Some(hash) => match self.verify_blocking(password, hash) {
                Err(e) => {
                    dummy();
                    Err(e)
                }
                ok => ok,
            },
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
        let permit: OwnedSemaphorePermit = self.inner.slots.clone().acquire_owned().await.map_err(|_| PasswordError::Internal)?;
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
        let _permit = futures::executor::block_on(self.inner.slots.clone().acquire_owned()).map_err(|_| PasswordError::Internal)?;
        catch_unwind(AssertUnwindSafe(|| work(&self.inner))).unwrap_or(Err(PasswordError::Internal))
    }
}

impl Inner {
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

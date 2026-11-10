//! Username and password authentication: [`LocalStrategy`] (the port of `passport-local` 1.0.0) and
//! [`PasswordHasher`] (argon2id for new hashes, argon2 and bcrypt verification, bounded resources). Needs the
//! `auth-local` cargo feature.
//!
//! ```ignore
//! #[module(
//!     providers = [Provider::class::<PasswordHasher>(), Provider::class::<LocalAuth>(), Provider::strategy::<LocalStrategy<LocalAuth>>()],
//!     controllers = [AuthController],
//! )]
//! pub struct AuthModule;
//!
//! #[post("login")]
//! #[use_guards(AuthGuard::new("local"))]
//! fn login(&self, user: User<PublicUser>) -> Value { ... }
//! ```

mod hasher;
mod strategy;

pub use hasher::{
    ARGON2_MEMORY_FOR_ONE_ITERATION_KIB, Algorithm, Argon2Params, BCRYPT_MAX_PASSWORD_BYTES, DummyAlgorithm, MAX_PASSWORD_BYTES_CAP,
    MIN_ARGON2_ITERATIONS, MIN_ARGON2_MEMORY_KIB, MIN_BCRYPT_COST, PasswordError, PasswordHasher, PasswordHasherOptions, VerifyLimits,
};
pub use strategy::{DEFAULT_MAX_CREDENTIAL_BYTES, LocalStrategy, LocalStrategyOptions, LocalValidator};

#[cfg(test)]
mod tests;

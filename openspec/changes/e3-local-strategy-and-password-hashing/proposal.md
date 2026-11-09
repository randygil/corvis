# Proposal

Implements [randygil/corvis#41](https://github.com/randygil/corvis/issues/41) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Design from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`: decisions 2 and 9, fact 5, the `auth/local` delta,
task group 3, Review notes 2 and 8: argon2id for new hashes, bcrypt hashes still verified, non-string credentials
treated as missing).

**Depends on #136** (`openspec/changes/e3-strategy-core-and-guard-responses/`): this change is written against the API
#136 proposes (`corvis::auth::Strategy`, `StrategyOutcome`, `AuthenticatedUser`, `Provider::strategy`, `AuthGuard`,
the fixture package `crates/corvis/tests/fixtures/auth/`) and is implemented after #136 lands (and, by the epic's
order, after #42). If that API changes during review, this change is updated before work starts.

## Why

Every app with a login form has `class LocalStrategy extends PassportStrategy(Strategy)` and a password hash in its
users table, usually made by `bcryptjs`. Corvis offers neither:

- `docs/security/encryption-and-hashing.md` is `status: unavailable` and sends users to the `argon2` crate by hand.
- `docs/security/authentication.md:96-100` compares passwords in plain text with a warning, and so does
  `examples/cats-app` (`src/auth/auth_service.rs:17`, plain passwords in `src/users/users_service.rs:17-18`).
- The port apps hash with `bcryptjs` 3.x in Node (`corvis-tests/nest/consi-backend/package.json:40`) and with the
  `bcrypt` 0.17 crate in Rust, each app on its own.

Password handling is where small mistakes become account takeovers: a fast or truncating hash, a non-constant-time
comparison, a login that answers faster for unknown users (user enumeration), or a hash string that makes the server
spend minutes of CPU. The framework should make the safe path the default one.

## What Changes

- New cargo feature `auth-local` (off by default) with `argon2` and `bcrypt` (and `subtle` only if a comparison is not
  already constant-time in those crates, design decision 4).
- `corvis::auth::local::PasswordHasher`:
  - new hashes with **argon2id, memory 19456 KiB, 2 iterations, parallelism 1, 16-byte salt, 32-byte output** by
    default (the OWASP Password Storage minimum, which is also the `argon2` crate's default), in PHC string format;
    bcrypt (cost 10, the `bcryptjs` default) can be chosen instead for apps whose Node side must keep reading new
    hashes during a migration;
  - verification chosen by the hash prefix: argon2 PHC strings and bcrypt `$2a$`/`$2b$`/`$2y$` hashes from `bcryptjs`,
    with the results of `bcryptjs.compareSync` for every password, including non-ASCII and over 72 bytes;
  - `needs_rehash(hash)` when the algorithm or the parameters are weaker than the configured ones;
  - `verify_dummy(password)` and `verify_or_dummy(password, Option<hash>)`, so an unknown user costs one real
    verification with the configured parameters;
  - limits, each with a default and a test: passwords at most 1024 bytes before any hashing; bcrypt hashing refuses
    passwords over 72 bytes instead of truncating them; stored hashes at most 512 bytes, bcrypt cost at most 14 and
    argon2 memory at most 262144 KiB, iterations at most 16 and parallelism at most 16 when verifying, so a planted
    hash cannot burn CPU or memory; at most `available_parallelism()` hash computations at a time, run off the async
    workers (`spawn_blocking`);
  - never panics on any input; constant-time comparison of derived hashes.
- `corvis::auth::local::LocalStrategy<V: LocalValidator>`: credentials read like `passport-local` 1.0.0 (fields
  `username`/`password`, configurable, `a[b]` paths, body then query), missing, empty or non-string values as missing
  credentials, credentials over 1024 bytes refused before the validator runs; name `"local"` overridable.
- Differential tests: `bcrypt.mjs` (`bcryptjs` 3.0.3, both directions) and `local.mjs` (`passport-local` 1.0.0) in the
  fixture package of #136, replayed by `crates/corvis/tests/auth_password.rs` and `crates/corvis/tests/auth_local.rs`.
- Docs (EN + ES): `docs/security/encryption-and-hashing.md` (hashing documented), `docs/security/authentication.md`
  (hashing instead of the plain-text comparison and its warning), local strategy section of `docs/recipes/passport.md`.
- Example: `examples/cats-app` stores argon2id hashes and logs in through `AuthGuard::new("local")`.

## Capabilities

### New Capabilities

- `auth/local`: the local strategy (credential reading, missing credentials, limits) and the password hasher
  (algorithms, parameters, bcryptjs compatibility, migration, enumeration resistance, resource limits, robustness).

### Modified Capabilities

None.

## Impact

- New `crates/corvis/src/auth/local/` (`hasher.rs`, `strategy.rs`), `#[cfg(feature = "auth-local")]`.
- `crates/corvis/Cargo.toml`: feature `auth-local`, optional `argon2` and `bcrypt` (versions in design decision 1).
- Workspace `Cargo.toml`: `[profile.dev.package]` `opt-level = 3` for `argon2`, `blake2`, `bcrypt` and `blowfish`,
  so tests with real parameters do not take seconds in debug builds (design decision 8).
- `.github/workflows/ci.yml:71`: `auth-local` added to the per-feature clippy loop, and a test step with
  `--features auth-local`.
- `deny.toml`: licenses of the new crates checked (`argon2`, `blake2`, `password-hash`, `bcrypt`, `blowfish` are
  MIT OR Apache-2.0 as far as their crate metadata says; the implementer confirms with `cargo deny check`).
- Fixture package `crates/corvis/tests/fixtures/auth/`: `passport-local` 1.0.0 and `bcryptjs` 3.0.3 added; `bcrypt.mjs`,
  `bcrypt.json`, `corvis_bcrypt_hashes.json`, `local.mjs`, `local.json`.
- Tests: `crates/corvis/tests/auth_password.rs`, `crates/corvis/tests/auth_local.rs`, unit and property tests in the
  module.
- Docs: `docs/security/encryption-and-hashing.md`, `docs/security/authentication.md`, `docs/recipes/passport.md` and
  their `docs/es/` copies. Example: `examples/cats-app`.
- Port apps: they do not enable `auth-local` and no code outside the feature changes, so apidiff cannot move; the
  corvis-parity check is run only if something outside the feature changed (Definition of done).

## Out of scope

- Login rate limiting and lockout: #46 (with the existing `ThrottlerGuard`).
- Password policy (minimum length, breached-password checks) and Unicode normalization of passwords: application
  decisions; possible new issue for a helper.
- scrypt and PBKDF2 hashes (`$scrypt$`, `$pbkdf2-sha256$`) and `$2x$` bcrypt hashes: verification returns an
  unsupported-hash error; possible new issue if a migrated database has them.
- Peppering (a server-side secret mixed into the hash): possible new issue.
- Zeroizing password buffers in memory.
- `AuthGuard::log_in()` for the session login pattern of `LocalAuthGuard`: #44.
- The `badRequestMessage` option of `passport-local`: its message only reaches the 400 that `AuthGuard` replaces with
  the 401 anyway.

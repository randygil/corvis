# Design

## Context

This change builds on #136 (`openspec/changes/e3-strategy-core-and-guard-responses/`): `corvis::auth` with `Strategy`
(`authenticate(&ExecutionContext, &AuthenticateOptions) -> StrategyOutcome`), `AuthenticatedUser`,
`Provider::strategy::<S>()`, `AuthGuard` (which turns every failure into the empty 401 and ignores the strategy's
status) and the fixture package `crates/corvis/tests/fixtures/auth/`. It is implemented after #136 (and after #42, the
epic's order: #136 → #42 → #41). Line numbers were checked on `origin/dev` at `6f5ff4d`.

### Corvis today

- No hashing API. `docs/security/encryption-and-hashing.md` is `status: unavailable` and shows the `argon2` crate used
  by hand (lines 16-27). `docs/security/authentication.md:96-100` compares passwords in plain text under a warning.
- `examples/cats-app` keeps plain passwords (`src/users/users_service.rs:17-18`) and compares them with `==`
  (`src/auth/auth_service.rs:17`), which is both plain-text storage and a timing-dependent comparison.
- Request data the strategy reads: `Request::body()` (parsed JSON or urlencoded form, `crates/corvis/src/http/request.rs:142-145`)
  and `Request::query()` (`request.rs:160-163`), both `serde_json` values.
- Dependencies: `rand = "0.8"` (`crates/corvis/Cargo.toml:43`, `rand_core` 0.6) and `sha2 = "0.10"` (line 45, `digest`
  0.10) are in the tree; no `argon2`, `bcrypt` or `subtle` direct dependency.
- Dev profile: dependencies are built without optimization (`Cargo.toml` of the workspace, `[profile.dev]` and
  `[profile.dev.package."*"]` set only `debug`), so a memory-hard hash with real parameters is slow in debug tests.

### Port apps and the original

- consi and facten hash with `bcryptjs` (`corvis-tests/nest/consi-backend/package.json:40`, `^3.0.2`; facten's
  `node_modules/bcryptjs` is 3.0.3) and their Rust ports with `bcrypt = "0.17"`
  (`corvis-tests/corvis/consi-backend/Cargo.toml:38`, `corvis-tests/corvis/facten-api/Cargo.toml:34`).
- `bcryptjs` 3.0.3 (`node_modules/bcryptjs/index.js` of facten): new salts are `$2b$` (line 96), rounds clamped to
  4..31 when generating (93-94), a constant-time-style `safeStringCompare` (211), and a `truncates(password)` helper
  that reports passwords over 72 UTF-8 bytes (318-325): hashing truncates them silently.
- `bcrypt` 0.17.1 (Rust, local registry copy, `src/lib.rs`): cost 4..=31 (29-30); the password gets a NUL terminator
  and is cut to 72 bytes unless the `non_truncating_*` functions are used, which return `BcryptError::Truncation`
  (107-129, 195, 217, 244); `verify` compares with `subtle::ConstantTimeEq` (257-272).
- `passport-local` 1.0.0 (spike fact 5): `lookup(req.body, field) || lookup(req.query, field)` for both fields; a
  falsy value is missing; missing → `fail({ message: 'Missing credentials' }, 400)`; `utils.lookup` splits the field
  with `field.split(']').join('').split('[')`, returns a non-object value as soon as it meets one, and returns `null`
  for an object at the end of the path or an absent segment.

## Goals / Non-Goals

**Goals**

- A safe default for new hashes with every parameter written down, and verification of the hashes migrated apps
  already have.
- No user enumeration through login timing when the app follows the documented validator.
- No resource exhaustion through long passwords, planted hashes or concurrent logins.
- `passport-local` behavior for credential reading, verified by a fixture.

**Non-Goals**

- Login throttling (#46), password policy, peppering, scrypt/PBKDF2 (proposal Out of scope).

## Decisions

### 1. Feature and crates

`auth-local = ["dep:argon2", "dep:bcrypt"]`, off by default (spike decision 9). Versions: the `argon2` release whose
`password-hash`, `blake2` and `rand_core` share the generation already in the tree (`rand_core` 0.6 through `rand`
0.8, `digest` 0.10 through `sha2` 0.10), today the 0.5 line; if the 0.6 line is stable when the work starts and
`cargo tree -d` shows no new duplicate generations, use it. `bcrypt` 0.17.x (same as the ports), with
`default-features` kept for `getrandom`. `cargo deny check` must pass.

Alternative: `password-auth` 2.0.0 (spike table): a convenience wrapper that hides the parameters this design wants
explicit and bounded. Rejected.

### 2. API

```rust
let hasher = PasswordHasher::new(PasswordHasherOptions::default())?;           // argon2id 19456 KiB, t=2, p=1
let hasher = PasswordHasher::new(PasswordHasherOptions::bcrypt(10))?;          // for a shared Node database
let hasher = PasswordHasher::new(
    PasswordHasherOptions::argon2id(Argon2Params { memory_kib: 65536, iterations: 3, parallelism: 4 })
        .max_password_bytes(1024)
        .max_concurrent(4)
        .verify_limits(VerifyLimits::default())        // bcrypt cost 14; argon2 262144 KiB, t 16, p 16; 512-byte hashes
        .dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 10 }),
)?;

hasher.hash(password).await?                    // PHC string or $2b$ hash
hasher.verify(password, &stored).await?         // Ok(true) / Ok(false) / Err(PasswordError)
hasher.verify_or_dummy(password, stored.as_deref()).await?
hasher.verify_dummy(password).await             // always false
hasher.needs_rehash(&stored)                    // bool, no key derivation
hasher.hash_blocking(..) / verify_blocking(..)  // outside async code
```

`PasswordHasher` is `Clone` (an `Arc` inside) and `Injectable` with default options (`Provider::class::<PasswordHasher>()`);
custom options go through `Provider::use_value`. `PasswordError`: `TooLong`, `Malformed`, `Unsupported`,
`OverLimit { what }`, `Internal`. `verify` returns `Ok(false)` for a password over `max_password_bytes` (it cannot match
a hash this hasher made) without running the key derivation.

### 3. Defaults and limits

| Setting | Default | Why |
|---|---|---|
| Algorithm | argon2id, v=19 | Spike Review note 2; RFC 9106 recommends argon2id |
| Memory / iterations / parallelism | 19456 KiB / 2 / 1 | OWASP Password Storage Cheat Sheet minimum; the `argon2` crate's own default (implementer confirms in the pinned version) |
| Salt / output | 16 / 32 bytes | RFC 9106 |
| bcrypt cost (when chosen) | 10 | `bcryptjs` default, spike decision 9 |
| `max_password_bytes` | 1024 | Long-password DoS; far above any human password |
| bcrypt hashing over 72 bytes | error | No silent truncation (spike decision 9) |
| `max_bcrypt_cost` (verify) | 14 | Cost 31 is 2^31 rounds; 14 leaves room above the usual 10-12 |
| argon2 verify limits | 262144 KiB, t ≤ 16, p ≤ 16 | A planted hash cannot ask for gigabytes or minutes |
| Hash string length | 512 bytes | A PHC string with these parameters is about 100 bytes |
| `max_concurrent` | `available_parallelism()` | Bounds CPU and memory (19 MiB each by default) |

Configured parameters weaker than the defaults log a warning when the hasher is created (tests use small parameters
on purpose); configured parameters above the verification limits are an error (the hasher could not verify its own
hashes). The limits are parsed from the hash string before any key derivation.

### 4. Constant-time comparison and verification paths

- bcrypt: parse the hash (`$2a$`/`$2b$`/`$2y$`, two-digit cost, 53 characters of salt and digest), check the limits,
  then `bcrypt::verify` (truncating, as `bcryptjs` does), which compares with `subtle` (`bcrypt` 0.17.1
  `src/lib.rs:257-272`). If the fixture shows a password for which the crate's result differs from `bcryptjs`
  (candidates: NUL characters, 72/73-byte boundaries), Corvis prepares the bytes so the result matches, and the case
  stays in the fixture.
- argon2: parse with `password-hash`'s `PasswordHash`, check variant and limits, then `PasswordVerifier::verify_password`.
  The implementer confirms in the pinned `password-hash` source that the output comparison is constant-time; if not,
  compute the raw output and compare with `subtle::ConstantTimeEq` (adding `subtle` to `auth-local`).
- Every path runs inside the semaphore slot and `spawn_blocking`; a panic in a blocking task is turned into
  `PasswordError::Internal` (it cannot be produced by input, but the boundary is closed anyway).

### 5. Enumeration resistance

At construction the hasher hashes 32 random bytes with the dummy algorithm (the configured one by default) and keeps
the hash; `verify_dummy` verifies the submitted password against it and returns `false` whatever happens.
`verify_or_dummy(password, None)` calls it; `verify_or_dummy(password, Some(bad))` runs it before returning the error
for a refused hash, so a broken stored hash does not answer faster than an unknown user. Building the dummy at
construction, not lazily, keeps the first unknown-user login from paying an extra hash.

Residual timing differences, documented: stored hashes with older parameters or another algorithm verify at their own
cost (`needs_rehash` plus rehash on login closes the gap over time; `dummy_algorithm` matches the majority during a
migration), and the app's own user lookup may differ in time.

### 6. Local strategy

```rust
#[async_trait]
pub trait LocalValidator: Injectable {
    const NAME: &'static str = "local";
    type User: Serialize + Clone + Send + Sync + 'static;
    fn options(&self) -> LocalStrategyOptions { LocalStrategyOptions::default() }
    async fn validate(&self, req: &Request, username: &str, password: &str) -> Result<Option<Self::User>, HttpException>;
    fn info(&self, _user: &Self::User) -> Option<Value> { None }
}

LocalStrategyOptions::default()
    .username_field("email")          // passport-local usernameField
    .password_field("password")       // passport-local passwordField
    .max_username_bytes(1024)
    .max_password_bytes(1024)
```

`LocalStrategy<V>` resolves `V` through DI (as `JwtStrategy<V>` in #42), validates the options when built (empty field
name → `DiError`, so the application does not start), and implements `Strategy`: compute both paths once at build time
with the `passport-local` split rule; for each credential, look in `req.body()` then `req.query()`, where a segment of
an object is a key and a segment of an array is an index (JS property access on arrays); a string value is returned
when found (also before the end of the path, like `lookup`); anything else, or an empty string, is missing in that
source. Missing → `Fail { challenge: {"message":"Missing credentials"}, status: 400 }`. Over a length limit →
`Fail { challenge: {"message":"Credentials too long"}, status: 400 }` without calling the validator. Then
`validate` → `Ok(Some)` success, `Ok(None)` fail with no challenge, `Err` error.

The documented validator:

```rust
async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<User>, HttpException> {
    let user = self.users.find_by_username(username).await?;
    let ok = self.hasher.verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str())).await
        .map_err(|_| InternalServerErrorException::empty())?;
    Ok(user.filter(|_| ok))
}
```

### 7. Fixtures

- `bcrypt.mjs` (`bcryptjs` 3.0.3): fixed salts at cost 4 (and two at cost 10) for `""`, `"secret"`, `"pässword"`, an
  emoji password, a password with a NUL, 71/72/73-byte and 100-byte passwords; for each hash, `compareSync` results for
  the password, its first 72 bytes, a one-character change and the empty string. Reverse direction: a Rust test marked
  `#[ignore]` writes `corvis_bcrypt_hashes.json` (Corvis hashes of the same passwords, committed); `bcrypt.mjs` reads it
  and records `compareSync` for each. `tests/auth_password.rs` replays both.
- `local.mjs` (`passport-local` 1.0.0): JSON body, form body, query only, body and query together, missing and empty
  fields, `0` and `false` values, nested `user[email]` (present, absent, string at `user`), array at the end of the
  path, non-string username (divergence), oversized password (divergence), validator returning `null` and throwing
  `ForbiddenException`; `tests/auth_local.rs` replays them.

### 8. Test speed and determinism

- Workspace `Cargo.toml`: `[profile.dev.package.argon2]`, `blake2`, `bcrypt` and `blowfish` with `opt-level = 3`, so
  the few tests with real parameters (default PHC prefix, dummy built with defaults) take milliseconds, not seconds.
- Most tests use small parameters (argon2 8 KiB, t=1; bcrypt cost 4).
- No timing assertions. The enumeration and concurrency scenarios use a `#[cfg(test)]` key-derivation counter and a
  test backend inside the crate: "exactly one derivation for a known and an unknown user"; for concurrency, a backend
  that blocks on a channel until the test releases it, records the peak number of running derivations, and the test
  waits for the running count to reach the limit by polling a counter with `tokio::task::yield_now`, not a sleep.

### 9. Example

`examples/cats-app`: enables `auth-local`; `UsersService` stores argon2id hashes (computed once and pasted as
constants, with a comment on how they were made), `LocalAuth` implements `LocalValidator` with `verify_or_dummy`,
`POST auth/login` uses `AuthGuard::new("local")` and signs the JWT for the user it receives.
`tests/login_throttle.rs` keeps passing.

## Risks / Trade-offs

- **Argon2 defaults cost about 19 MiB per login**; with `max_concurrent` at the core count the peak is bounded, but a
  login flood queues requests. #46 adds throttling; the docs say so.
- **bcryptjs edge cases** (NUL, long passwords) might differ from the `bcrypt` crate; the fixture decides, and a
  mismatch is fixed in Corvis, not hidden.
- **Verification limits could refuse legitimate legacy hashes** (bcrypt cost 15+). They are configurable and the error
  says which limit was hit.
- **`opt-level` for four dependencies** slightly lengthens the first debug build; accepted for test speed.
- **Two new crates in the supply chain**, both RustCrypto; `cargo deny` gates licenses and advisories.

## Open questions

1. Default `max_bcrypt_cost` of 14: confirm, or raise to 15 for apps with a higher legacy cost.
2. Should the argon2 defaults be stronger than the OWASP minimum (for example RFC 9106's 64 MiB, t=3, p=4)? The design
   keeps the minimum so a small server can absorb login bursts; apps can raise it.
3. `PasswordHasher` and `LocalStrategy` share one feature (`auth-local`). If an app wants the strategy with its own
   hashing, the hashing crates still compile; a separate `password-hashing` feature is possible but not proposed.

## Review notes (2026-11-09, maintainer)

1. `max_bcrypt_cost` 14 by default, configurable; a stored hash above the cap fails verification as a rejected hash
   (with the dummy verification), and the startup docs say how to raise it.
2. argon2id defaults stay at the OWASP minimum (m=19456 KiB, t=2, p=1), configurable; the docs show stronger settings.
3. One feature, `auth-local`, for the strategy and `PasswordHasher`.
4. Implemented after #136 lands: start by comparing the design with the API #136 shipped.

## Implementation notes

- **Core API as merged.** `corvis::auth` matches what the design assumed (`Strategy`, `StrategyOutcome`, `AuthenticatedUser`,
  `Provider::strategy`, `AuthGuard`, `AuthGuard::on_failure`). `LocalStrategy<V>` follows the shape of the sibling
  `JwtStrategy<V>` (resolve `V` with `get_or_construct`, validate the options in `construct`, a `DiError::Factory` that
  stops startup).
- **Versions (decision 1).** `argon2` 0.5.3 (`password-hash` 0.5, `rand_core` 0.6, `blake2` 0.10, the generation already in
  the tree; the 0.6 line was not used) and `bcrypt` 0.17.1. No `subtle` dependency: `password-hash` 0.5 compares `Output`
  values with `subtle::ConstantTimeEq` (`output.rs`, `impl PartialEq for Output`) and `PasswordVerifier::verify_password`
  uses that comparison; `bcrypt::verify` compares with `subtle` too. `cargo deny check` passes. `cargo tree -d` shows no
  new duplicate generation.
- **Parameter floor (deviation from decision 3).** The design only warned about parameters weaker than the defaults. The
  security review of the earlier changes asked for a floor that cannot be crossed by accident, so `PasswordHasher::new`
  now fails below the floor (argon2id 19456 KiB with 2 iterations, or 47104 KiB with 1, both from the OWASP table; bcrypt
  cost 10) unless `allow_weak_parameters()` is set, which logs the warning. The spec requirement and a scenario were
  updated. The dummy algorithm has no floor (it exists to match legacy costs) but obeys the verification limits.
- **Extra option bounds.** `max_password_bytes` is capped at 65536, `max_hash_bytes` must be 64..=4096, `max_concurrent`
  at least 1 (capped at 1024), and argon2 parameters the `argon2` crate rejects (for example memory under 8 blocks per
  lane) are a `PasswordError::Config`, not a panic. `PasswordError` gained the `Config(&'static str)` variant, and is
  `#[non_exhaustive]`.
- **Hash parsing.** Stored hashes are parsed by hand before any derivation. bcrypt: exactly 60 bytes, `$2a$`/`$2b$`/`$2y$`
  (`$2x$` is `Unsupported`), two-digit cost 4..=31, the bcrypt base64 alphabet only. argon2: `argon2id`, `argon2i` and
  `argon2d` hashes verify (`needs_rehash` is true for the last two), versions 0x10 and 0x13, `salt` of at least 8 bytes
  (the crate's own rule). Any other `$...$` prefix is `Unsupported`; text without a leading `$` is `Malformed`.
- **Construction does one key derivation** (the dummy hash), so `PasswordHasher::new` is synchronous and blocking; the
  `Injectable` implementation runs it with `spawn_blocking`.
- **Concurrency.** A `tokio::sync::Semaphore` with `max_concurrent` permits; the permit moves into the blocking task, so
  the number of running derivations stays bounded even when the caller stops waiting. The blocking variants wait for a
  permit with `futures::executor::block_on` and run on the calling thread (round 2 replaced this, see below). Waiters are not bounded: a login flood queues
  futures of a few hundred bytes each, which is why the docs point to the throttler.
- **bcryptjs comparison.** No byte preparation was needed: `bcrypt::verify` gave the result of `compareSync` for every
  case of the fixture (NUL characters, 71, 72 and 73 byte passwords, 100 bytes, multibyte at the boundary, `$2a$`,
  `$2b$`, `$2y$`). A 72-byte password is hashed (the terminating NUL falls off, as in `bcryptjs`); only more than 72 bytes
  is refused.
- **Fixtures.** `corvis_bcrypt_hashes.json` is written by the ignored test `write_corvis_hashes` from the passwords of
  `bcrypt.json`, then `node bcrypt.mjs` is run again to record `fromCorvis`; a second run gives the same file. Corvis
  hashes only the passwords of 72 bytes or fewer. The `local.mjs` cases run against the original behind `AuthGuard('local')`
  with four strategies (`local`, `nested`, `userobj`, `indexed`); four cases are divergences (non-string username and
  password, oversized username and password). Nothing else differed, including the form body with percent-encoding.
- **Test hooks.** The derivation counter, the running and peak counters and the gate that holds derivations are a
  `#[cfg(test)]` probe inside `PasswordHasher`; no sleeps, the concurrency test polls with `yield_now`.
- **Tests run in `cargo test --workspace`** because `examples/cats-app` enables `auth-local` and Cargo unifies the
  feature; the CI step `cargo test -p corvis --features auth-local` still runs them on their own.
- **Example.** `cats-app` keeps its global JWT guard; only `POST auth/login` gets `AuthGuard::new("local")`, imported as
  `PassportGuard` because the app has an `AuthGuard` of its own. The route keeps `#[body] SignInDto` (validation and
  OpenAPI) after the guard, so a bad body is a `401`, not a `400`. `tests/login.rs` is new.
- **Not done in this session:** the parity check, the security review, the CI run of the pull request, archiving.

## Security review (round 1)

The first security review found seven items. The maintainer decided the following; it overrides the design and the
implementation notes above where they differ.

1. **Refused stored hashes are a wrong password, not a 500.** `verify_or_dummy` (and its blocking variant) returns
   `Ok(false)` after the dummy verification when the stored hash is malformed, unsupported or over the verification
   limits (an empty string of a social-login account, `$2b$15$`, `$2x$`, `$scrypt$`), and logs one warning per
   occurrence that names the reason and never the hash or the user input. `verify` keeps returning the error. Errors
   that remain for `verify_or_dummy` are `Busy` and `Internal`. The rehash recipe in the hashing docs uses
   `verify_or_dummy` and then `needs_rehash`. `verify_dummy` now returns `Result<bool, PasswordError>` (always
   `Ok(false)`, `Err(Busy)` when saturated), and `verify_dummy_blocking` was added.
2. **Dummy hash versus migrated hashes.** No sleeping (the design stands). The hasher logs a warning, once, when a
   stored hash that parses differs from the dummy in algorithm or cost parameters, pointing to `dummy_algorithm`; the
   hashing docs and the bcrypt migration notes (EN and ES) say prominently to set `dummy_algorithm` to the algorithm
   and cost of the hashes actually stored and to rehash on login.
3. **The wait for a slot is bounded.** New options `max_waiting` (default 32 times `max_concurrent`, at least 1) and
   `acquire_timeout` (default 10 s, between 1 ms and 300 s). A free slot is taken at once; otherwise the call takes a
   place in a counted queue (a drop guard frees it, so a cancelled waiter gives it back) and waits at most
   `acquire_timeout`. A full queue or a timeout is the new `PasswordError::Busy`, without any key derivation. The
   strategy validator returns `Result<_, HttpException>`, so no core change was needed:
   `PasswordError::into_http_exception` maps `Busy` to `503 Service Unavailable` (an empty exception, the same body for
   every user) and anything else to an empty `500`; the documented validator, the cats example and the tests use
   `.map_err(PasswordError::into_http_exception)?`. A blanket `From<PasswordError> for HttpException` already exists
   (any `std::error::Error`) and cannot be specialised, hence the method.
4. **Lower verification caps.** Defaults: argon2 memory 65536 KiB, 8 iterations, parallelism 16; bcrypt cost 14
   unchanged. Each limit is raised to the hasher's own hashing parameters (and its dummy's) when those are higher, so
   its own hashes always verify; the former construction error for parameters above the limits is gone. Limits are
   overridable through `VerifyLimits` and bounded above (argon2 memory 1048576 KiB, iterations 64, bcrypt cost 20: a
   value above fails construction, as do own parameters above them). Startup warning when `max_concurrent` times the
   effective argon2 memory limit exceeds 1 GiB. The argon2 parallelism limit has no upper bound beyond the crate's own.
5. **Credentials in the query string are opt-in.** `LocalStrategyOptions::allow_query_credentials()` (off by default,
   a startup warning when on). Without it only the body is read. The `passport-local` fixture marks the four cases
   that need the query as deliberate divergences (default: 401, validator not called) and the replay runs them again
   against a strategy with the option, where they match the recorded original.
6. **Blocking variants are safe on async workers (overridden by round 2).** Inside a multi-thread runtime `hash_blocking`, `verify_blocking`,
   `verify_dummy_blocking` and `verify_or_dummy_blocking` run through `tokio::task::block_in_place` (the wait for the
   slot uses the runtime's timer through `Handle::block_on`); inside a current-thread runtime they return the new
   `PasswordError::CurrentThreadRuntime` before waiting; outside any runtime they block, with a small runtime of their
   own for the timed wait when the slot is contended. Documented in the hashing pages.
7. **Non-canonical bcrypt hashes are malformed.** `parse_bcrypt` refuses, before any derivation, a last salt character
   whose low 4 bits are not zero and a last digest character whose low 2 bits are not zero (the bcrypt crate accepted
   them at the cost of a derivation); that is item 1's path.

Not changed: the `LoggingInterceptor`, the dummy design (one real verification, no sleeping).

## Security review (round 2)

The second security review found five items. The maintainer decided the following; item 1 overrides round 1's item 6.

1. **The blocking variants are for code outside a runtime.** `block_in_place` let callers parked on workers consume the
   blocking pool, so async callers holding a permit found no blocking thread, no worker drove the timer and
   `acquire_timeout` never fired (1000 synchronous logins plus 500 async hashes with default options hung the server);
   it also panics inside a `LocalSet`. Now, inside any Tokio runtime context (`Handle::try_current()` succeeds: worker,
   `block_on`, `spawn_blocking` thread, `LocalSet`), `hash_blocking`, `verify_blocking`, `verify_dummy_blocking` and
   `verify_or_dummy_blocking` return `PasswordError::InsideRuntime` (renamed from `CurrentThreadRuntime`) before
   anything else: no wait, no derivation, no length check. There is no `block_in_place` and no `Handle::block_on`.
   Outside a runtime the slot is acquired by polling the semaphore's acquire future with a waker that unparks the
   thread and sleeping with `std::thread::park_timeout` until the `acquire_timeout` deadline, with the same
   `max_waiting` queue as the async path (`Busy` on overflow or timeout); the derivation runs on the calling thread.
   `Provider::class::<PasswordHasher>()` builds the hasher with `spawn_blocking(PasswordHasher::new)`, which never calls
   a blocking variant. Handlers, synchronous ones included, must use the async methods. The tests that hashed inside a
   runtime now make their hashes on a plain thread.
2. **The refusal warning follows the dummy and is rate limited.** `verify_or_dummy` logs after the dummy verification
   ran, never on `Busy`, at most one warning per reason (`malformed`, `unsupported`, or the name of the exceeded limit)
   per minute; the next one adds how many were not logged. A map of `reason -> (last written, suppressed)` behind a
   mutex does it; the keys are a fixed set. The module has no other per-request warning (the dummy mismatch warning is
   once per hasher, the rest run at startup).
3. **Early refusal of what the crate would refuse late.** `parse_argon2` refuses a salt under 8 bytes
   (`argon2::MIN_SALT_LEN`) and an output length under the crate's minimum before any slot is taken (the other
   parameter rules, such as memory against parallelism, zero iterations and zero lanes, are already enforced by
   `Params`, and the output of a PHC string is at least 10 bytes). `verify_or_dummy` runs the verification and, when a
   refusal still appears inside the task, the dummy inside the same permit.
4. **The default `max_concurrent` is capped at 16** (`available_parallelism().min(16)`), so with the default limits (64
   MiB each) the startup warning (above 1 GiB) never appears with default options. An explicit value is honoured up to
   1024.
5. **bcrypt cap stays at 14.** Lowering it would lock out migrated users with cost 13 or 14 hashes. The limits table
   (EN and ES) says a cost-14 hash costs about 60 times the default dummy verification and that an application whose
   stored hashes are all cost 12 or less can lower `VerifyLimits` accordingly.

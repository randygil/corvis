# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev` (and, by the epic's order, #42).
Before starting, compare this change with the API that landed (`corvis::auth`, `Strategy`, `Provider::strategy`,
fixture package) and update the artifacts if it differs.

## 1. Feature and dependencies

- [x] 1.1 Add the `auth-local` feature with optional `argon2` and `bcrypt` (design decision 1) to
  `crates/corvis/Cargo.toml`; check the dependency generations with `cargo tree -d -p corvis --features auth-local` and
  licenses with `cargo deny check`; record the chosen versions in `design.md`; verify `cargo clippy -p corvis
  --no-default-features --features auth-local -- -D warnings` passes on an empty module
- [ ] 1.2 Add `[profile.dev.package]` `opt-level = 3` for `argon2`, `blake2`, `bcrypt`, `blowfish` in the workspace
  `Cargo.toml` (design decision 8); add `auth-local` to the per-feature loop (`.github/workflows/ci.yml:71`) and a
  `cargo test -p corvis --features auth-local` step next to the `compression` one (line 113); verify the new steps run
  in the CI of the pull request

## 2. Password hasher

- [x] 2.1 `PasswordHasherOptions`, `Argon2Params`, `VerifyLimits`, `DummyAlgorithm`, `PasswordError`, construction-time
  checks (weak-parameter warning, parameters above verification limits as error); verify with unit tests for "Default
  hash format" (one hash with real parameters), the warning and the error
- [x] 2.2 Hash parsing and limit checks before any derivation (512-byte strings, bcrypt prefix and cost, PHC variant,
  version, memory, iterations, parallelism); verify with unit tests for "Planted high-cost bcrypt hash", "Planted
  high-memory argon2 hash" and "Malformed hash", using the `#[cfg(test)]` derivation counter to show zero derivations
- [x] 2.3 `hash` / `verify` / `*_blocking` with `spawn_blocking` and the `max_concurrent` semaphore; bcrypt and argon2
  verification paths (design decision 4), `max_password_bytes` on both; confirm in the pinned `password-hash` source
  that output comparison is constant-time, or compare with `subtle`, and note the finding in `design.md`; verify with
  unit tests for "Two hashes of the same password", "Hashing an oversized password", "Verifying an oversized
  password", "73-byte password" and "Concurrency limit" (blocking test backend, no sleeps)
- [x] 2.4 `needs_rehash`; verify with "Mixed hashes" and "Older parameters"
- [x] 2.5 Dummy hash at construction, `verify_dummy`, `verify_or_dummy` (dummy also before returning a refused-hash
  error); verify with "Unknown user" (derivation counter: one for an unknown user, one for a known user), "Dummy
  never matches" and "Malformed stored hash"
- [x] 2.6 Property test: random passwords against random strings and mutated valid hashes (bcrypt and argon2 with small
  parameters) never panic; verify it runs in `cargo test` with the default case count

## 3. bcryptjs fixture

- [x] 3.1 Add `bcryptjs` 3.0.3 to `crates/corvis/tests/fixtures/auth/package.json` and the lockfile; an `#[ignore]` Rust
  test that writes `corvis_bcrypt_hashes.json`; run it once and commit the file; verify `npm ci`
- [x] 3.2 `bcrypt.mjs` (design decision 7) writing `bcrypt.json` with both directions; verify a second run gives the
  same file
- [x] 3.3 `crates/corvis/tests/auth_password.rs` replaying `bcrypt.json`: "Hash from the original app", "Hash read back
  by the original", "Long password against an original hash", NUL and non-ASCII cases; adapt the byte preparation if a
  case differs (design decision 4); verify every case passes and that flipping one recorded result fails the test

## 4. Local strategy

- [x] 4.1 `LocalValidator`, `LocalStrategyOptions`, `LocalStrategy<V>`: path split rule, body then query, arrays by
  index, missing/empty/non-string as missing, length limits, outcome mapping (design decision 6); verify with unit tests
  for each lookup rule and HTTP tests for "Credentials in a JSON body", "Wrong password", "Validator throws", "Custom
  nested field", "String before the end of the path", "Object at the end of the path", "Credentials in the query
  string", "Form body", "Body value wins over the query", "Missing password",
  "Empty password", "Non-string username", "Oversized password" and "Raised limit"
- [x] 4.2 Add `passport-local` 1.0.0 to the fixture package; `local.mjs` writing `local.json` (design decision 7) with
  divergences marked; `crates/corvis/tests/auth_local.rs` replaying it; verify every case passes and that the empty
  field name stops the application at startup

## 5. Documentation and example

- [x] 5.1 `docs/security/encryption-and-hashing.md`: status `documented` for password hashing (enabling `auth-local`,
  defaults table with reasons, bcrypt verification and migration with `needs_rehash`, `verify_or_dummy`, limits,
  concurrency and blocking threads, the bcrypt 72-byte divergence); keep the note that general encryption is not
  provided; same in `docs/es/security/encryption-and-hashing.md`
- [x] 5.2 `docs/security/authentication.md`: replace the plain-text comparison and its warning (lines 96-100) with the
  hasher and the documented validator; `docs/recipes/passport.md`: local strategy section (options mapped from
  passport-local, the divergences, the validator pattern, login throttling pointer to #46); same in `docs/es/`; verify
  `npm run docs:check` in `docs/`
- [x] 5.3 `examples/cats-app` (design decision 9); verify the example builds, its tests pass including
  `login_throttle.rs`, and a login with the documented credentials works on 127.0.0.1

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with `auth-local` alone and with `--no-default-features`
- [x] 6.3 `cargo test --workspace` and `cargo test -p corvis --features auth-local` pass, including `auth_password.rs`
  and `auth_local.rs` without Node
- [x] 6.4 Regression tests for every scenario of `auth/local`
- [x] 6.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 6.6 corvis-parity check: run it if any framework runtime code outside the `auth-local` feature changed; otherwise
  record in the archive notes that the ports do not enable the feature and nothing else changed
- [x] 6.7 `cargo deny check` passes with the new crates
- [x] 6.8 Run the security-reviewer agent over `corvis::auth::local` (timing of unknown users and refused hashes,
  planted hashes, long passwords, concurrency bound, panics, the example's validator); verify every finding is fixed or
  recorded as a new issue

## 7. Security review (round 1)

- [x] 7.1 Refused stored hashes: `verify_or_dummy` and `verify_or_dummy_blocking` return `Ok(false)` after the dummy
  verification and log one warning naming the reason; `verify_dummy` returns a `Result`; fix the rehash recipe (EN, ES);
  verify through HTTP that unknown user, wrong password and each refused-hash form give the same status and body
- [x] 7.2 One-time warning when a parsed stored hash differs from the dummy in algorithm or cost; prominent
  `dummy_algorithm` section in the hashing docs and the migration notes (EN, ES); verify with a unit test of the
  warning flag
- [x] 7.3 `max_waiting` and `acquire_timeout`, `PasswordError::Busy`, `PasswordError::into_http_exception` (503) in the
  documented validator, the cats example and the tests; verify with `max_concurrent(1)`, `max_waiting(2)` and held
  slots (fails fast, cancelled waiter frees its place), the timeout, and a 503 over HTTP for a known and an unknown user
- [x] 7.4 Lower default verification limits raised to the hasher's own parameters, upper bounds, 1 GiB startup
  warning; verify with the own-parameters and upper-bound tests
- [x] 7.5 `allow_query_credentials()`, startup warning, fixture divergences replayed both ways, docs differences list
  (EN, ES); verify with the replay and `the_query_string_is_ignored_unless_allowed`
- [x] 7.6 Blocking variants through `block_in_place`, an error on a current-thread runtime; verify with the PoC (8
  async plus 2 blocking verifications on 2 workers), the current-thread error and a `spawn_blocking` caller
- [x] 7.7 Refuse non-canonical bcrypt trailing bits before any derivation; verify with altered salt and digest
  characters and the derivation counter
- [x] 7.8 Spec deltas updated; `openspec validate e3-local-strategy-and-password-hashing --strict` has no warnings

## 8. Security review (round 2)

- [x] 8.1 Blocking variants: `InsideRuntime` inside any runtime (no `block_in_place`, no `Handle::block_on`), slot wait
  with `park_timeout` outside; verify with tests on multi-thread, current-thread, `spawn_blocking` and `LocalSet`
  contexts, contention giving `Busy` after `acquire_timeout`, and a mixed async load with a timer probe
- [x] 8.2 Refusal warning after the dummy ran, not on `Busy`, one per reason per minute with the suppressed count;
  verify with the `Busy` test and the rate limit test
- [x] 8.3 Refuse argon2 salts under 8 bytes (and short outputs) in the parser; dummy in the same permit for late
  refusals; verify with a 7-byte and an 8-byte salt and the derivation counter
- [x] 8.4 Cap the default `max_concurrent` at 16; verify with the slot count
- [x] 8.5 Docs (EN, ES): blocking methods outside a runtime only, default cap, bcrypt cap note, warning rate limit
- [x] 8.6 Spec deltas updated; `openspec validate e3-local-strategy-and-password-hashing --strict` has no warnings

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #41, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).

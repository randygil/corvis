# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev` (and, by the epic's order, #42).
Before starting, compare this change with the API that landed (`corvis::auth`, `Strategy`, `Provider::strategy`,
fixture package) and update the artifacts if it differs.

## 1. Feature and dependencies

- [ ] 1.1 Add the `auth-local` feature with optional `argon2` and `bcrypt` (design decision 1) to
  `crates/corvis/Cargo.toml`; check the dependency generations with `cargo tree -d -p corvis --features auth-local` and
  licenses with `cargo deny check`; record the chosen versions in `design.md`; verify `cargo clippy -p corvis
  --no-default-features --features auth-local -- -D warnings` passes on an empty module
- [ ] 1.2 Add `[profile.dev.package]` `opt-level = 3` for `argon2`, `blake2`, `bcrypt`, `blowfish` in the workspace
  `Cargo.toml` (design decision 8); add `auth-local` to the per-feature loop (`.github/workflows/ci.yml:71`) and a
  `cargo test -p corvis --features auth-local` step next to the `compression` one (line 113); verify the new steps run
  in the CI of the pull request

## 2. Password hasher

- [ ] 2.1 `PasswordHasherOptions`, `Argon2Params`, `VerifyLimits`, `DummyAlgorithm`, `PasswordError`, construction-time
  checks (weak-parameter warning, parameters above verification limits as error); verify with unit tests for "Default
  hash format" (one hash with real parameters), the warning and the error
- [ ] 2.2 Hash parsing and limit checks before any derivation (512-byte strings, bcrypt prefix and cost, PHC variant,
  version, memory, iterations, parallelism); verify with unit tests for "Planted high-cost bcrypt hash", "Planted
  high-memory argon2 hash" and "Malformed hash", using the `#[cfg(test)]` derivation counter to show zero derivations
- [ ] 2.3 `hash` / `verify` / `*_blocking` with `spawn_blocking` and the `max_concurrent` semaphore; bcrypt and argon2
  verification paths (design decision 4), `max_password_bytes` on both; confirm in the pinned `password-hash` source
  that output comparison is constant-time, or compare with `subtle`, and note the finding in `design.md`; verify with
  unit tests for "Two hashes of the same password", "Hashing an oversized password", "Verifying an oversized
  password", "73-byte password" and "Concurrency limit" (blocking test backend, no sleeps)
- [ ] 2.4 `needs_rehash`; verify with "Mixed hashes" and "Older parameters"
- [ ] 2.5 Dummy hash at construction, `verify_dummy`, `verify_or_dummy` (dummy also before returning a refused-hash
  error); verify with "Unknown user" (derivation counter: one for an unknown user, one for a known user), "Dummy
  never matches" and "Malformed stored hash"
- [ ] 2.6 Property test: random passwords against random strings and mutated valid hashes (bcrypt and argon2 with small
  parameters) never panic; verify it runs in `cargo test` with the default case count

## 3. bcryptjs fixture

- [ ] 3.1 Add `bcryptjs` 3.0.3 to `crates/corvis/tests/fixtures/auth/package.json` and the lockfile; an `#[ignore]` Rust
  test that writes `corvis_bcrypt_hashes.json`; run it once and commit the file; verify `npm ci`
- [ ] 3.2 `bcrypt.mjs` (design decision 7) writing `bcrypt.json` with both directions; verify a second run gives the
  same file
- [ ] 3.3 `crates/corvis/tests/auth_password.rs` replaying `bcrypt.json`: "Hash from the original app", "Hash read back
  by the original", "Long password against an original hash", NUL and non-ASCII cases; adapt the byte preparation if a
  case differs (design decision 4); verify every case passes and that flipping one recorded result fails the test

## 4. Local strategy

- [ ] 4.1 `LocalValidator`, `LocalStrategyOptions`, `LocalStrategy<V>`: path split rule, body then query, arrays by
  index, missing/empty/non-string as missing, length limits, outcome mapping (design decision 6); verify with unit tests
  for each lookup rule and HTTP tests for "Credentials in a JSON body", "Wrong password", "Validator throws", "Custom
  nested field", "String before the end of the path", "Object at the end of the path", "Credentials in the query
  string", "Form body", "Body value wins over the query", "Missing password",
  "Empty password", "Non-string username", "Oversized password" and "Raised limit"
- [ ] 4.2 Add `passport-local` 1.0.0 to the fixture package; `local.mjs` writing `local.json` (design decision 7) with
  divergences marked; `crates/corvis/tests/auth_local.rs` replaying it; verify every case passes and that the empty
  field name stops the application at startup

## 5. Documentation and example

- [ ] 5.1 `docs/security/encryption-and-hashing.md`: status `documented` for password hashing (enabling `auth-local`,
  defaults table with reasons, bcrypt verification and migration with `needs_rehash`, `verify_or_dummy`, limits,
  concurrency and blocking threads, the bcrypt 72-byte divergence); keep the note that general encryption is not
  provided; same in `docs/es/security/encryption-and-hashing.md`
- [ ] 5.2 `docs/security/authentication.md`: replace the plain-text comparison and its warning (lines 96-100) with the
  hasher and the documented validator; `docs/recipes/passport.md`: local strategy section (options mapped from
  passport-local, the divergences, the validator pattern, login throttling pointer to #46); same in `docs/es/`; verify
  `npm run docs:check` in `docs/`
- [ ] 5.3 `examples/cats-app` (design decision 9); verify the example builds, its tests pass including
  `login_throttle.rs`, and a login with the documented credentials works on 127.0.0.1

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with `auth-local` alone and with `--no-default-features`
- [ ] 6.3 `cargo test --workspace` and `cargo test -p corvis --features auth-local` pass, including `auth_password.rs`
  and `auth_local.rs` without Node
- [ ] 6.4 Regression tests for every scenario of `auth/local`
- [ ] 6.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 6.6 corvis-parity check: run it if any framework runtime code outside the `auth-local` feature changed; otherwise
  record in the archive notes that the ports do not enable the feature and nothing else changed
- [ ] 6.7 `cargo deny check` passes with the new crates
- [ ] 6.8 Run the security-reviewer agent over `corvis::auth::local` (timing of unknown users and refused hashes,
  planted hashes, long passwords, concurrency bound, panics, the example's validator); verify every finding is fixed or
  recorded as a new issue

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #41, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).

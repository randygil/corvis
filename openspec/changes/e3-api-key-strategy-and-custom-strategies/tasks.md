# Tasks

Prerequisite: #136 (`e3-strategy-core-and-guard-responses`) is merged into `dev` (and, by the epic's order, #42 and
#41). Before starting, compare this change with the API that landed (`corvis::auth`, `Strategy`, `StrategyOutcome`,
`Provider::strategy` and its testing override, the fixture package, the header parser of #42) and update the artifacts
if it differs; record the adjustments under "Implementation notes" in `design.md`.

## 1. Feature and dependencies

- [x] 1.1 Add the optional `subtle = "2"` dependency and the `auth-api-key` feature (design decision 1) to
  `crates/corvis/Cargo.toml`; check with `cargo tree -d -p corvis --features auth-api-key` that no second generation
  of `subtle` appears and with `cargo deny check` that licenses pass; verify `cargo clippy -p corvis
  --no-default-features --features auth-api-key -- -D warnings` passes on an empty `auth::api_key` module
- [ ] 1.2 Add `auth-api-key` to the per-feature loop (`.github/workflows/ci.yml:72`) and a
  `cargo test -p corvis --features auth-api-key --no-fail-fast` step next to the `compression` one (line 114); verify
  the new steps run in the CI of the pull request

## 2. Comparison helpers and key types

- [x] 2.1 `constant_time_eq` (SHA-256 of both inputs, `subtle::ConstantTimeEq` on the digests) and `ApiKey`
  (`expose`, `ct_eq`, `sha256`, `sha256_hex`, redacted `Debug`, no `Display` / `Serialize` / `PartialEq`); verify with
  unit tests for "Comparison results", "Digest lookup" and "Debug output" (alone and inside a derived `Debug`)
- [x] 2.2 `ExpectedApiKey` (`new` with the 32-byte minimum, `from_sha256_hex`, `matches`, digest only, redacted
  `Debug`) and `ApiKeyConfigError` without the secret in its message; verify with unit tests for "Short configured
  key" (message checked for the absence of the secret) and "Key of 32 bytes"
- [x] 2.3 `ApiKeySet<T>` with a fold over every entry and a `#[cfg(test)]` comparison counter; verify with the "Every
  entry is compared" test and a test that two entries with the same key return the first

## 3. Request redaction

- [x] 3.1 `Request::redact_query_param` and `Request::redacted_url` in `crates/corvis/src/http/request.rs` (extension
  shared by clones, 16 names, raw query rewritten, percent-decoded name comparison, bracketed forms); verify with unit
  tests for "Encoded name and repeated parameter", "Nothing marked", the 17th name, a URL without a query and a
  parameter without a value
- [x] 3.2 `LoggingInterceptor` logs `redacted_url()` (`crates/corvis/src/common/interceptor.rs:88-91`); verify with a
  test that captures the logger for "Logged URL" and a regression test that an unmarked request logs the same line as
  before

## 4. Strategy

- [x] 4.1 `ApiKeySource` (`header`, `.prefix`, `bearer`, `query`), `ApiKeyStrategyOptions` and its validation when the
  strategy is built (no source, more than 8, empty or invalid names, empty prefix, `max_key_bytes` range) as a
  `DiError` naming the strategy; the query-source startup warning; verify with tests for "No source", "Empty header
  name", "Limit out of range" and "Startup warning" (warning text checked for the names and for the absence of keys)
- [x] 4.2 Source evaluation (design decision 3): latin1 header values joined with `", "`, literal prefix, the bearer
  rule shared with #42 (move the parser to a `pub(crate)` function of `corvis::auth` if it is private to `auth::jwt`),
  single-string query value; length and character limits per candidate; the conflict rule with the constant-time
  comparison; `redact_query_param` called first; verify with unit tests for each source, "Oversized key", "Control
  character in a query key", "Different keys in the header and the query" and "The same key twice"
- [x] 4.3 `ApiKeyValidator` and `ApiKeyStrategy<V>` (`Injectable`, `Strategy`, outcome mapping, fixed failure
  strings); verify with HTTP tests for "Valid key", "Wrong key", "Validator throws", "Key in the body", "Query key
  without the option" and "Failure reasons"

## 5. Differential fixtures

- [x] 5.1 Add `passport-headerapikey` 1.2.2 and `passport-custom` 1.2.1 (exact versions) to
  `crates/corvis/tests/fixtures/auth/package.json` and refresh the lockfile; re-read both package sources and correct
  the "The original" section of `design.md` if a fact differs; verify `npm ci` in that folder
- [x] 5.2 `api_key.mjs` (header comment: versions, how to run, 127.0.0.1 only, keys are fake test strings of at least
  32 bytes) recording the cases of design decision 8 into `api_key.json`, divergences marked with their reason; verify
  by running it twice and diffing the output (deterministic)
- [x] 5.3 `crates/corvis/tests/auth_api_key.rs`: the same routes, every case replayed (status, listed headers, body),
  divergences asserted from the spec, and the "Captured logs" check over the whole run; verify `cargo test -p corvis
  --features auth-api-key --test auth_api_key` passes and that editing one recorded status makes it fail
- [x] 5.4 `custom.mjs` → `custom.json` (a `passport-custom` strategy with an injected service: success with info,
  fail, `ForbiddenException`, plain `Error`, alone and second in a chain) and `crates/corvis/tests/auth_custom.rs`
  with `DeviceStrategy`, including "Built without auth features" (the test file has no feature gate) and "Override in
  a testing module"; verify `cargo test -p corvis --no-default-features --test auth_custom` passes
- [x] 5.5 Property test in `crates/corvis/src/fuzz_tests.rs` (under `#[cfg(feature = "auth-api-key")]`): random bytes
  in the key header and in `Authorization`, random query strings for `redacted_url`; verify no panic, the validator
  only sees keys within the limits, and `redacted_url` never contains the value of a marked parameter; make sure the
  CI fuzz job (`.github/workflows/ci.yml:165`) runs it with the feature enabled

## 6. Documentation and example

- [x] 6.1 `docs/recipes/passport.md`: "API keys" section (enabling `auth-api-key`, sources table with the original
  each one matches, the conflict rule, the query warning, `ExpectedApiKey` for one key from configuration,
  `ApiKeySet` for a few, the digest-in-database pattern for many, generating keys of at least 32 random bytes, limits
  table, divergences, the `404` body note) and "Writing a strategy" section (pattern, outcome mapping from
  `passport-custom` and `passport-strategy`, testing override, security checklist); same in
  `docs/es/recipes/passport.md`
- [x] 6.2 `docs/openapi/security.md`: a pointer from `add_api_key()` to the strategy that enforces it;
  `docs/interceptors.md`: `LoggingInterceptor` prints the redacted URL and how to mark a parameter; same in
  `docs/es/`; verify `npm run docs:check` in `docs/`
- [x] 6.3 `examples/cats-app` (design decision 9); verify the example builds, its existing tests pass (`posture.rs`,
  `startup.rs`, `openapi.rs`), and the new test covers a valid key, a wrong key and a startup failure with a short
  `SERVICE_API_KEY`

## 7. Definition of done

- [x] 7.1 `cargo build --workspace --all-targets` with no warnings
- [x] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with `auth-api-key` alone and with `--no-default-features`
- [x] 7.3 `cargo test --workspace` and `cargo test -p corvis --features auth-api-key` pass, including
  `auth_api_key.rs` and `auth_custom.rs` without Node
- [x] 7.4 Regression tests exist for every scenario of `auth/api-key` and `auth/custom-strategies`
- [x] 7.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 7.6 Framework runtime code changed (`Request`, `LoggingInterceptor`): run the corvis-parity check; verify the
  three port apps (consi, facten, houseplan) keep 0 apidiff differences
- [x] 7.7 `cargo deny check` passes with `subtle` as a direct dependency
- [ ] 7.8 Security review: run the security-reviewer agent over `corvis::auth::api_key`, the redaction code and the
  example's validator. Scope: timing of every comparison path (length, prefix, conflict rule, set lookup); key
  material in logs, `Debug`, errors, auth info and panics; source confusion (duplicate headers, header and query,
  bearer shared with JWT in a chain, parameter name encodings that escape redaction); the limits of design decision 6
  and their startup checks; latin1 and percent-decoding edge cases. Verify every finding is fixed or recorded as a new
  issue, and list the findings in the archive notes

## 8. Security review (round 1)

- [x] 8.1 Empty user (design note 18): `AuthenticatedUser::is_empty`, `Authenticator::authenticate` turns the `Success`
  into a failure with the fixed reason and a limited warning, `install`/`store_user`/`AuthGuard` refuse it; verify with
  the unit tests of `authenticator.rs` and `tests/auth_empty_user.rs` (API key with `bool` and `Option<Service>`, JWT,
  local, custom; optional guards; chains)
- [x] 8.2 `custom.mjs` records `done(null, false | 0 | '' | null | undefined | NaN)` and the chain case, `custom.json`
  has 24 cases, and `tests/auth_custom.rs` replays them; verify two runs of the generator give the same file
- [x] 8.3 Application-wide redaction (note 19): `Strategy::credential_query_params`, `AppRedaction` in
  `BodyParserOptions`, collected in `init()`; API key, JWT and local register their names; verify with
  `query_key_logged_when_the_query_strategy_does_not_run` and the no-query-source application test
- [x] 8.4 Cheap `redacted_url` (note 20): allocation-free scan, name decoding on the fly, parser only for bracketed names
  under the extended parser, hide-all above 1000 pairs, no line built when the level is off; verify with the corpus
  test against the previous implementation, `tests/redact_allocations.rs` and the logger-off test
- [x] 8.5 Prefix checked before the limits (note 21); verify with the unit tests for an unrelated long or odd
  `Authorization` next to a valid `X-Api-Key`
- [x] 8.6 Docs (note 22) and the danger block about the user, EN and ES, `docs/interceptors.md` and its copy; verify
  `npm run docs:check` in `docs/`
- [x] 8.7 Panic line through the limiter, no per-request `ExceptionsHandler` line (note 23); verify with the log test
  that counts the lines for forty panics
- [ ] 8.8 Maintainer: rerun the security review and the corvis-parity check over this round

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #45, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).

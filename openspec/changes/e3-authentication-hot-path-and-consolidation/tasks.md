# Tasks

Start from `dev`. Timing rule for every test: no `sleep`, no assertion on elapsed time, no retries; races are held with
gates, timers run on Tokio's paused clock, costs are counted (counters, counting allocator), never timed. New
integration test files that need a feature start with its `#![cfg(..)]`. Build in the shared target directory (no
`CARGO_TARGET_DIR` of your own).

## 1. Benchmark and baseline

- [x] 1.1 Re-check every line of design.md "Findings as checked" and of the invariants table against `dev`; note moved
  lines or changed facts at the top of design.md under "Implementation notes"; verify `cargo test -p corvis --features
  "auth-local auth-api-key auth-session auth-oauth2"` passes before any edit
- [x] 1.2 `bench/auth/Cargo.toml` and `bench/auth/src/main.rs`: workspace member `corvis-auth-bench` (`publish = false`,
  `corvis` with `jwt` and `auth-session`), added to `members` in the root `Cargo.toml`; the routes of design.md Decision 1
  (JWT strategy with a static HMAC key, a key provider returning clones of one cached RS256 `JwtKey`, a guard calling
  `JwtService::verify_or_unauthorized` with RS256 keys, `JwtService::sign` with HS256 and RS256, the session routes under
  the cookie path `/session`), setup routes that issue the tokens and seed a 16 KiB session, a counting global allocator,
  a request counter and `GET /__bench/stats`; listens on `127.0.0.1` only; verify `cargo build --release -p
  corvis-auth-bench` with no warnings
- [x] 1.3 `bench/auth/run.sh` and `bench/auth/README.md`, following `bench/run.sh` (Git Bash, `oha`, `CONNS`, `DURATION`,
  `WARMUP`, `RATE`, `SCENARIOS`, `SKIP_BUILD`, `--label`): check each scenario's status and body, measure closed loop
  and at a fixed rate, read the stats before and after, write `bench/auth/results/<label>.json` (add `bench/auth/results/`
  to `.gitignore`) and print the Markdown table; verify spec `bench/auth-hot-path` ("Full run", "Unexpected answer",
  "Allocations per request", "Missing binary", "Before and after") by running it, and that no `bench/auth/target`
  appears
- [x] 1.4 Baseline: `bash bench/auth/run.sh --label before` on `dev` before any framework edit; copy the table into
  design.md "Measurements" with the machine and parameters

## 2. `JwtService::sign` from the original's recording

- [x] 2.1 `crates/corvis/tests/fixtures/auth/package.json` and `package-lock.json`: pin `@nestjs/jwt` 11.0.2 and
  `jsonwebtoken` 9.0.3 as direct dependencies; verify `npm ci && node jwt.mjs` writes a `jwt.json` identical to the
  committed one
- [x] 2.2 `crates/corvis/tests/fixtures/auth/jwt_sign.mjs`: the recorder of design.md Decision 2 (header comment like
  `jwt.mjs`: what it records, how to run it, the format, the clock), the 34 cases of the table, the `divergence` of case 31;
  run it twice and verify the two `jwt_sign.json` are identical; commit `jwt_sign.json`
- [x] 2.3 Compare the recording with "Rules expected" of Decision 2 and write every difference under "Implementation
  notes" (the recording wins); add a case for any branch of the original found while reading it that the table misses
- [x] 2.4 `crates/corvis/src/jwt.rs`: the private `sign_at(payload, opts, now)` seam called by `sign_with`, and the unit test
  `sign_matches_the_recorded_original` replaying `jwt_sign.json`; run it on the unchanged signing code and list the cases
  that fail under "Implementation notes"
- [x] 2.5 Implement the recorded rules in `sign_at` (checks and their order, `iat` from the payload, `nbf` and `exp` from
  it, conflicts with the original's texts, the claim order, the header without `typ` for a non-object payload); add
  `not_before` and `jwt_id` to `JwtSignOptions` (rustdoc with the original's option names; `Debug` lists them); verify the
  replay passes, with any remaining case marked as a divergence with its reason, and `cargo test -p corvis --lib jwt`
- [ ] 2.6 Check every caller of `sign` / `sign_with` in the repository (`examples/`, `crates/corvis/tests/`,
  `crates/corvis/src/fuzz_tests.rs`) still signs: none passes a payload claim together with its option; verify
  `cargo test --workspace`

## 3. JWT keys and validations prepared once

- [x] 3.1 `JwtService::new` prepares the module encoding key, decoding key and `Validation` (design.md Decision 3) behind
  an `Arc`; cached failures returned as clones of `jsonwebtoken::errors::Error`; a `#[cfg(test)]` thread-local parse
  counter like `HMAC_RUNS` in `signed_cookie.rs`; unit tests for spec `auth/jwt` "Many calls, no parse" and "A key that
  does not parse", and `verify_errors_keep_their_type_with_cached_keys`
- [x] 3.2 Split `verify_claims` into `check_header` (callers) and `decode_claims`; `JwtService::verify` checks the header
  first; a `#[cfg(test)]` header-check counter; tests `verify_still_refuses_crit_after_the_single_header_check` and the
  strategy's "Header checked once"; verify `tests/auth_jwt_header.rs` and
  `a_token_with_a_crit_header_or_the_wrong_shape_never_reaches_the_key_provider` pass unchanged
- [x] 3.3 `auth/jwt/options.rs` and `auth/jwt/mod.rs`: `Resolved` holds the static key's `Validation` and one
  `Option<Validation>` per family for a provider; no `allowed.clone()` or `validation_for` per request; test
  `prepared_validations_follow_the_key_family`; verify `cargo test -p corvis --test auth_jwt` (the recorded fixture)
- [x] 3.4 `auth/jwt/key.rs`: `JwtKey` shares its parsed key across clones (`Arc<OnceLock<..>>`), `Debug` unchanged, the
  `JwtKeyProvider` rustdoc says to cache the `JwtKey`; a parse counter; tests "Cached provider key" and
  `a_cached_short_secret_is_refused_on_every_request` (spec "Short secret from a cache"); extend
  `debug_of_every_public_type` (`tests/auth_invariants.rs`) with a `JwtKey` whose parse is cached

## 4. Sessions

- [x] 4.1 `session/manager.rs`: the counting writer `Measure` with its early stop for the load check and
  `check_writable`; test `the_size_limit_holds_at_the_boundary` (spec "Size limit at the boundary"); verify the
  oversized-record tests of `auth_session_review.rs` pass
- [x] 4.2 `session/session.rs`: `baseline: Option<Map>`, `data` private with `data()` / `data_mut()`; every writer in
  `session.rs`, `login.rs` and `middleware.rs` through `data_mut()`; test "A value set to what it was"; verify the
  express-session saving fixture (`tests/auth_session.rs`) and both race suites pass unchanged
- [x] 4.3 `State::touch_record` and the unchanged branch of `commit`; `SessionStore::touch` rustdoc (`store.rs:75-78`)
  says the record holds only `cookie` and `__corvis`; test `touch_receives_only_the_timestamps` with the recording store of
  `tests/session_common/` (spec "Touch receives the timestamps")
- [x] 4.4 `State::id: Option<String>`, drawn by `Session::id()`, by `commit` for a new record and by its retry; a
  `#[cfg(test)]` draw counter; tests for spec "Anonymous request", "First write" and "Failed draw"
  (`a_failed_draw_at_commit_writes_nothing`); verify the identifier tests ("Identifier shape", "Signed but unknown
  identifier") and the OAuth2 binding tests pass
- [x] 4.5 `session/memory.rs`: `purge(now, PURGE_BATCH)` in the room loop; tests
  `one_write_purges_a_bounded_number_of_expired_records` (spec "Store full of expired records") and "Expired before
  live" in `memory_tests.rs`; verify `tests/auth_session_memory_heap.rs` passes
- [x] 4.6 `crates/corvis/tests/auth_hot_path_heap.rs` (own binary, counting allocator like `redact_allocations.rs`,
  `#![cfg(feature = "auth-session")]`): bytes allocated by the middleware for an unchanged session of 16 KiB and 64 KiB
  beyond the store's parse (spec "Read-only request with a large session"), and buffers of the record's size per
  `update`; set the bounds from the numbers before and after, and verify by checking out the files of group 4 from `dev`
  that each assertion fails on the old code

## 5. Consolidation

- [x] 5.1 `auth/warn_limit.rs`: `WarnLimiter::line`; the hasher uses `WarnLimiter`; the eight copies and the `more`
  helper of `authenticator.rs` use `line`; update the tests that pin the old suffixes; verify
  `cargo test -p corvis --features "auth-local auth-api-key auth-session auth-oauth2" --lib`
- [x] 5.2 `auth/outcome.rs` (private): `service_unavailable`, `with_deadline`, `user_outcome`; the JWT, local and API key
  strategies use them; `PasswordError::into_http_exception` and the OAuth2 busy answers use `service_unavailable`; test
  `every_framework_503_has_the_same_bytes` (spec "Busy hasher and slow validator on one route", "OAuth2 at its limit"),
  comparing the body text, not the parsed JSON; verify `tests/auth_callback_timeouts.rs` and
  `tests/auth_api_key_options.rs`
- [ ] 5.3 `crates/corvis/src/untrusted_json.rs` (cfg `any(feature = "jwt", feature = "auth-oauth2")`); `jwt.rs` and
  `oauth2/json.rs` use it; verify `tests/auth_jwt_header.rs` and the OAuth2 JSON tests, and `cargo check -p corvis
  --no-default-features --features auth-oauth2` and `--features jwt`
- [ ] 5.4 `crates/corvis/src/auth/auth_header.rs`: the tokenizer of design.md Decision 11; the JWT extractor and the API key
  `bearer()` source use it; `extract_bearer_token` rustdoc; verify the `jwt.json` and `api_key.json` replays and the
  extractor unit tests (no-break space, non-UTF-8 bytes, the 640-byte window)
- [ ] 5.5 `is_empty_json` in `auth/strategy.rs`, used by `AuthenticatedUser::is_empty` and the session; `has_cookie_prefix`
  in `http/res.rs`, used by the four prefix checks; the OAuth2 `valid_cookie_name` renamed `is_token_cookie_name` with
  its comment; test that a JWT cookie named `__HOST-jwt` logs no startup warning (`tests/auth_jwt_log.rs`)
- [ ] 5.6 Before editing `oauth2/state.rs`, record the outputs of `derive_verifier`, `session_binding` and `user_binding`
  for fixed inputs as constants; then `CookieKeys::mac` and the three derivations through it; test `derivations_are_unchanged`
- [ ] 5.7 `CookieKeys` keeps a keyed `Hmac<Sha256>` per secret; verify the `HMAC_RUNS` tests, `tests/signed_cookies.rs`
  (the recorded fixture) and `debug_of_every_public_type`

## 6. Fixes

- [ ] 6.1 `oauth2/state.rs`: `StateCookie::read` through `cookie_values`; test `a_state_cookie_sent_twice_is_no_state`
  (spec "Planted second cookie", "One cookie") with the mock provider's request counter at 0
- [ ] 6.2 `Request::typed_user::<T>()` in `http/request.rs` (with the `auth` items it needs; rustdoc); the rustdoc of `User<T>`,
  `AuthUserSlot` and `Request::set_user` per design.md Decision 13e; tests for spec "A guard reads the typed user", "Route
  without an AuthGuard" and `typed_user_never_reads_a_foreign_extension`; verify `roles_guard_without_an_auth_guard`
- [ ] 6.3 `api_key/strategy.rs`: the mark only without an application redaction holding the names; unit test that no
  extension is written inside an application and `a_strategy_outside_an_application_still_marks_its_names`; verify the
  redaction tests of `tests/auth_api_key*.rs`
- [ ] 6.4 `http/request.rs`: `insert`, `get`, `remove` recover a poisoned lock; unit test that they work after a poisoning
  panic, and `a_handler_that_panics_with_a_poisoned_lock_is_a_500`
- [x] 6.5 `local/hasher.rs`: hidden `PasswordHasher::hold_slot` and `HeldSlot`; rewrite
  `a_busy_hasher_answers_503_for_every_user` (`tests/auth_local.rs`) to hold the slot, with no thread and no sleep; verify
  it passes 20 runs in a row

## 7. Nice to have

- [ ] 7.1 `GuardKind`, `NoteLevel`, `Phase` (design.md Decision 13f); constant-time `PartialEq` for `OAuth2Token` with a unit
  test of equal and unequal tokens
- [ ] 7.2 `Authenticator::default_names` and the borrowed names in `AuthGuard::names`; verify the default-strategy tests
- [ ] 7.3 The doc comment of `oauth2/options.rs:732-734` moved; `every_outcome_can_be_built` matches each outcome
- [ ] 7.4 `OAuth2Client` rustdoc (one connection per call); the `refresh` warning through a `WarnLimiter`, with a test that
  two failures within a minute log one line

## 8. Example

- [ ] 8.1 `examples/auth-app/src/auth/jwt.rs:45` comment; `RolesGuard` with `typed_user::<Account>()`; verify
  `cargo test -p auth-app`, then `cargo build -p auth-app` and `node scripts/smoke-examples.mjs auth-app`

## 9. Docs and naming

- [ ] 9.1 Rustdoc for every public item added or changed (`JwtSignOptions::{not_before, jwt_id}`, `JwtService::sign` with
  its rules, `Request::typed_user`, `User<T>`, `SessionStore::touch`, `OAuth2Client`, `extract_bearer_token`); verify
  `cargo doc -p corvis --all-features --no-deps` without warnings and `cargo test -p corvis --doc --all-features`
- [ ] 9.2 Docs, English then Spanish (`docs/` and `docs/es/`), with the `docs-writer` agent: `recipes/passport-migration.md`
  (the `jwt.sign` row: the claim rules, `not_before`, `jwt_id`, and any recorded divergence), `recipes/passport.md` (lines
  193 and 199: the session user is a typed user; `Request::typed_user` for guards; the `503` body where it is quoted),
  `http/session.md` (the `touch` row of the store table), `security/encryption-and-hashing.md` (the busy hasher's body if
  quoted); verify `npm run docs:check` in `docs/`
- [ ] 9.3 Rename `tests/auth_session_review.rs` → `auth_session_races.rs`, `auth_session_review2.rs` →
  `auth_session_guard_login.rs`, `auth_oauth2_review.rs` → `auth_oauth2_abuse.rs` (header comments keep their origin);
  update the links in `security/authentication-threat-model.md` in both languages and any reference in `.github/`; verify
  `cargo test -p corvis --test auth_source_checks` and `node scripts/check-naming.mjs`

## 10. Measurements and review

- [ ] 10.1 `bash bench/auth/run.sh --label after` on the finished branch, same machine and parameters as 1.4; fill
  design.md "Measurements"; explain any scenario more than 5% slower; check each number of the table of design.md Decision
  1 moved as named
- [ ] 10.2 Security review of the change with the `security-reviewer` agent over the invariants table of design.md
  (header checks and the key provider, family allow-lists with prepared validations, cached keys and `Debug`, the session
  size boundary and change detection, the timestamps-only touch, the lazy identifier and generator failure, the purge
  bound, the MAC derivations and prepared HMAC keys, the repeated state cookie, the API key redaction, the typed user slot,
  the new `sign` errors); repeat rounds until no open finding; record findings and fixes in design.md under "Security
  review"; verify every finding has a regression test or a recorded reason

## 11. Definition of done

- [ ] 11.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 11.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with each auth feature alone, all together and `--no-default-features`
- [ ] 11.3 `cargo test --workspace` passes, and `cargo test -p corvis --features "auth-local auth-api-key auth-session
  auth-oauth2"` passes including `auth_invariants`, `auth_hot_path_heap`, `auth_source_checks` and `fuzz_auth`
- [ ] 11.4 Regression tests: one per spec scenario of this change and one per pinning test of design.md's invariants table,
  as listed in groups 1 to 8
- [ ] 11.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 11.6 Framework runtime code changed (`crates/corvis/src/jwt.rs`, `auth/`, `http/`): run the `corvis-parity` check and
  verify the three port apps keep 0 apidiff differences; a difference caused by `JwtService::sign` is handled as design.md
  Risks says (behaviour kept, divergence recorded) before archiving
- [ ] 11.7 `cargo deny check` passes (no new dependency) and `openspec validate e3-authentication-hot-path-and-consolidation
  --strict` is valid
- [ ] 11.8 The security review of 10.2 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `auth/jwt`, `auth/jwt-strategy`, `auth/sessions`, `auth/oauth2`,
  `auth/strategies` deltas are applied and `bench/auth-hot-path` is created).
- Close issue #186, tick it in epic #39 and move its card to Done (see the `corvis-task` skill).
- #181 and #182 stay open; comment on each that #186 left the example's workaround in place for them.

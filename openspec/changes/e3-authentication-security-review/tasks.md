# Tasks

Prerequisite: #42, #41, #45, #44 and #43 are merged into `dev` (the core, #136, already is). This change tests and
reviews what they shipped, so nothing below starts before that.

## 1. Check the ground

- [x] 1.1 Compare this change with the merged code of the five strategy changes (module paths, option and type
  names, defaults, the session user restoring, `AuthGuard::log_in`, the OAuth2 state storage, `redact_query_param`,
  the local lookup function) and with their archive notes; update `design.md` (Context table, decisions 2, 5, 7),
  the spec scenarios and this file where they differ, and record the adjustments under "Implementation notes" in
  `design.md`; verify `openspec validate e3-authentication-security-review --strict` still passes
- [x] 1.2 Collect the findings of the six earlier `security-reviewer` rounds (archive notes of #136, #42, #41, #45,
  #44, #43) into a working list with their fixes and regression tests; verify each regression test still exists and
  is named in the list
- [x] 1.3 Ask the maintainer's session to create the two issues of design decision 9 ("Strategies at the WebSocket
  handshake", "Align or warn on JwtService defaults") if they do not exist; verify their numbers are written into
  `design.md`

## 2. Shared test application

- [x] 2.1 `crates/corvis/tests/auth_common/mod.rs`: the application of design decision 4 (every strategy, the grid
  of routes per guard shape, call counters for validators, key derivation, session store and mock provider, logger
  capture, test clock, the `PROPTEST_CASES` helper); move the mock OAuth2 provider of #43's tests there if it is
  private to `auth_oauth2.rs`; verify with a smoke test that each strategy authenticates its own route and that the
  existing `auth_oauth2.rs` still passes
- [x] 2.2 The reference model for the gate property: a pure function from (route, credentials) to (handler runs,
  user, status), written from the specs as a table; verify with unit tests of the model for one row of each guard
  shape

## 3. Invariant tests (`auth/security-invariants`)

- [x] 3.1 Identity: "Session of one user and token of another", "Optional token guard with a session cookie",
  "Session strategy in the chain", "Token-only route with only a session cookie", "Roles guard without an AuthGuard"
  in `crates/corvis/tests/auth_invariants.rs`; verify `cargo test -p corvis --features "auth-local auth-api-key
  auth-session auth-oauth2" --test auth_invariants identity`
- [x] 3.2 Chains and failures: "Failing dependency before a weaker strategy", "Reasons are indistinguishable",
  "Chain position is not revealed", "Failed login with an anonymous session", "Failed OAuth2 callback"; verify the
  tests pass and that each fails when its assertion is inverted once (no vacuous test)
- [x] 3.3 Public routes and places: "Invalid credentials on a public route", "Controller marked public, handler not
  marked", "Posture with only an optional guard", "Bearer value shared by two strategies", "Token in a place that is
  not configured"; verify with the strategy call counters
- [x] 3.4 OAuth2 with sessions: "Callback that logs in", "Callback replayed after logout", "Forged callback on an
  authenticated session", with and without `keep_session_info_on_login`; verify with the store contents and the
  provider counter
- [x] 3.5 Sockets: "Session cookie on an upgrade request" (a gateway test that reads `client.handshake().user()`)
  and "Global AuthGuard and a gateway" (extend `crates/corvis/tests/auth_ws.rs` or reference it); verify both pass
  with the `websockets` feature
- [x] 3.6 Secrets in output: "Debug of every public type" (a list of every public type of `corvis::auth` that can
  hold a secret, built with marker values); verify the test fails when a derived `Debug` is added to one of them in
  a scratch edit, then revert

## 4. Login rate limiting (`auth/login-rate-limiting`)

- [x] 4.1 Make the lookup function of the local strategy `pub(crate)` and add
  `corvis::auth::local::login_tracker(options)` (design decision 7: digest value, client-address fallback, length
  limit); verify with unit tests that replay the lookup cases of `crates/corvis/tests/fixtures/auth/local.json`
  through both the strategy and the tracker and get the same username
- [x] 4.2 `ThrottlerGuard::as_any` in `crates/corvis/src/modules/throttler.rs` and the guard-order warning in
  `auth::check_routes` (`crates/corvis/src/auth/mod.rs:94-112`, design decision 6); verify with tests for "Global
  AuthGuard, route-level throttler" and "Throttler first" in a test binary of its own (the logger is process-wide)
- [x] 4.3 `crates/corvis/tests/auth_login_throttle.rs`: "Password guessing over the limit" (derivation counter),
  "OAuth2 login start over the limit", "Callback over the limit", "Same account from many addresses", "Username
  moved to the query string", "Nested field name", "Missing username", "Existing and unknown accounts", "Storage
  keys"; verify every scenario of the spec has a test with its name
- [ ] 4.4 Run the corvis-parity check after 4.2 (code compiled in every build changed); verify the parity-verifier
  reports 0 apidiff differences for consi, facten and houseplan

## 5. Fuzz targets

- [x] 5.1 `crates/corvis/tests/fuzz_auth.rs`: `gate_matches_model` and `canary_secrets` (design decision 8); verify
  they pass with the default case count and with `PROPTEST_CASES=20000`, and that breaking the model for one guard
  shape makes `gate_matches_model` fail with a minimized case
- [x] 5.2 `session_state_machine`, `oauth2_callback` and `redirect_targets` in the same file; verify the same two
  runs, and commit any `proptest-regressions` seed a failure produced together with its fix
- [ ] 5.3 CI: add the invariant, login-throttle and defaults binaries to the test jobs and `fuzz_auth` to the `fuzz`
  job of `.github/workflows/ci.yml` (lines 143-177) and of `.github/workflows/nightly.yml` (lines 33-69), each with
  the four auth features; verify in the pull request's CI log that the test names appear (not zero tests run)

## 6. Dependencies (`infra/auth-dependencies`)

- [x] 6.1 Build the inventory of design decision 11 from `cargo tree -e features -p corvis --features "auth-local
  auth-api-key auth-session auth-oauth2"` and `cargo metadata`; check each crate's enabled features, last release
  and RustSec entries; verify the table has every crate that implements a primitive, a hash, a token format, TLS or
  randomness
- [x] 6.2 `deny.toml`: ban `openssl`, `openssl-sys`, `native-tls`; add the single-generation entries for the
  inventoried crates that have one version (confirm the syntax with `cargo-deny` 0.20.2); verify `cargo deny check`
  passes, and fails when a scratch dependency adds a second version of `subtle` (do not commit the scratch edit)
- [x] 6.3 Confirm `cargo deny check` and `cargo audit` evaluate the graph with the auth features and that neither
  ignore list (`deny.toml`, `.cargo/audit.toml`) names an inventoried crate; verify by listing the advisory-checked
  packages for one auth-only crate
- [x] 6.4 `crates/corvis/tests/auth_source_checks.rs`: the randomness scan and the inventory-versus-`Cargo.lock`
  check (design decisions 10 and 11); verify each fails on a scratch edit (a `thread_rng` call in the session module;
  a changed major in the page) and passes after reverting

## 7. The review

- [x] 7.1 Run the `security-reviewer` agent with the scope list of design decision 2 (items 1-6), the earlier
  findings of 1.2 and the test application; every suspicion is first written as a failing test in
  `auth_invariants.rs` or as a proof of concept in the scratchpad; verify each scope item has a written result
  (finding or "nothing found", with what was tried)
- [x] 7.2 Second round for scope items 7-12 (CSRF posture, sockets, defaults, secrets in output, resource use,
  feature combinations), including the Engine.IO and Socket.IO handshakes if merged; verify the same way
- [x] 7.3 Triage every finding with design decision 9; fix the in-change ones, each with its regression test, and
  add a `MODIFIED` delta to this change for any requirement of an archived auth change that a fix alters (then
  update that strategy's docs in English and Spanish); verify `openspec validate --strict` and the full auth test
  set after each fix
- [x] 7.4 (filed as #177 and #178, with #173 and #174 already open) Write the text of each issue to file (title, severity, labels `area:security`, description, interim
  guidance) in the findings register and hand the list to the maintainer's session; verify the issue numbers are
  written back, and that any deferred high or critical finding was reported before continuing
- [ ] 7.5 Re-review: run the `security-reviewer` agent once more over the fixes only; verify no fix opened a new
  path and record the result in the register

## 8. Threat model and documentation

- [x] 8.1 `docs/security/authentication-threat-model.md` (status `documented`): Scope and assumptions, Assets, Trust
  boundaries, Attackers, one section with a table per surface of the `auth/threat-model` spec (each row with its
  test file or spec scenario), Defaults (table plus the `key = value` block), Dependencies (inventory), Findings
  (register, linking the earlier reviews); add it to the Security group of `docs/.vitepress/sidebar.mts` after
  "Route posture"; verify `auth_source_checks.rs` resolves every referenced test path
- [x] 8.2 `crates/corvis/tests/auth_defaults.rs` (design decision 5) reading the block of 8.1: "Secret minimum",
  "Cookies set by the module", "Credentials in a URL", and one comparison per row; verify it fails when one value in
  the page is edited, then revert
- [x] 8.3 `docs/security/rate-limiting.md`: rewrite the login part (lines 105-121 and the "Client-chosen trackers"
  warning at 205-214) around the recipe of design decision 7 with `login_tracker`, the guard order and its startup
  warning, what a throttled request does not cost, lockout, OAuth2 routes; add a pointer from the local and OAuth2
  sections of `docs/recipes/passport.md`; verify the sample matches `auth_login_throttle.rs`
- [x] 8.4 Spanish pages for 8.1 and 8.3 (`docs/es/security/authentication-threat-model.md`,
  `docs/es/security/rate-limiting.md`, `docs/es/recipes/passport.md`) with the `docs-writer` agent, same headings,
  tables and code fences; verify `npm run docs:check` in `docs/`

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with each auth feature alone, all together and `--no-default-features`
- [x] 9.3 `cargo test --workspace` passes, and `cargo test -p corvis --features "auth-local auth-api-key
  auth-session auth-oauth2"` passes including `auth_invariants`, `auth_login_throttle`, `auth_defaults`,
  `auth_source_checks` and `fuzz_auth`
- [x] 9.4 Regression tests: one test per scenario of `auth/security-invariants`, `auth/login-rate-limiting`,
  `auth/threat-model` ("Renamed test file", "Default changed in code only") and `infra/auth-dependencies`, plus one
  per finding fixed in this change
- [x] 9.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 9.6 Framework runtime code changed (`ThrottlerGuard::as_any`, the startup warning, any fix): run the
  corvis-parity check again at the end; verify the three port apps (consi, facten, houseplan) keep 0 apidiff
  differences
- [x] 9.7 `cargo deny check` and `cargo audit` pass with the new `deny.toml` entries
- [x] 9.8 The findings register is complete: every finding is fixed with a named test or has an issue number, and
  the archive notes list them with severities

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #46, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
- The issues created from the findings are linked from the archive notes and added to epic #39 or to the epic of
  the area they belong to.

# Tasks

Start after `e3-authentication-hot-path-and-consolidation` and `q-http-server-robustness` are merged into `dev`
(design.md "Order with the other changes"). Shared Cargo target directory only. Fixture servers and test servers bind to
`127.0.0.1`. No test sleeps to synchronize or compares timings. Behavior must not change except where design.md
"Behavior visible to applications" says so; where the fixture decides a helper, the decision is recorded in
"Implementation notes" of design.md.

## 1. Fixture first (design.md Decision 1)

- [ ] 1.1 `crates/corvis/tests/fixtures/node_compat/`: `package.json` pinning the versions of Decision 1,
  `package-lock.json`, `generate.mjs` (header comment with usage and the loopback rule) recording the `cookie`,
  `encodeurl`, `fresh`, `vary`, `escape_html` and `number` arrays; run `npm ci && node generate.mjs` and commit
  `node_compat.json`; verify the file names the Node and package versions and every array is non-empty
- [ ] 1.2 Re-check every line of design.md "Findings as checked" against `dev` with e3 merged and record moved lines in a
  new "Implementation notes" section; replay skeleton `crates/corvis/tests/node_compat.rs` plus unit tests in a new
  `http/node_compat.rs` that run every case against today's code and list the failures per copy (each existing copy run
  on the cases of its helper); record in Implementation notes which copy matches the fixture for `encodeurl`, `fresh`,
  `vary`, `js_number` and `parse_http_date`, and which cases match neither

## 2. Errors as values (Decisions 3 and 4)

- [ ] 2.1 `http/res.rs`: `CookieError`, `SameSite` (moved from `auth/session/options.rs`, re-exported there and as
  `http::SameSite`), `CookiePriority`, both with `FromStr`; `CookieOptions` fields and builders take the enums;
  `serialize_cookie` returns `Result<String, CookieError>`; adapt `auth/session/manager.rs` and `auth/oauth2/state.rs`
  (their names are validated at startup; an error is a `500`, never a panic); verify `cargo test -p corvis --features
  auth-session,auth-oauth2 --test signed_cookies --test auth_session --test auth_oauth2`
- [ ] 2.2 `http/response.rs`: `Cookie::same_site` as `SameSite`, `try_to_string` through `serialize_cookie`, no
  `Display`; `Response::with_cookie` / `set_cookie` return `Result`; rustdoc of `set_header` / `with_header` (an invalid
  name or value is ignored); rewrite `outgoing_cookie_attributes_are_validated` from panics to `Err` values with the same
  cases, and `invalid_cookies_answer_500_without_the_header` with `?`; verify `--test static_response_security --test
  cookie_max_age --test guard_responses`
- [ ] 2.3 `ResError`; the validating `Res` methods of Decision 3 return `Result`; `Res::json(impl Serialize) -> Result<(),
  ResError>`; `Res::send` under one lock; adapt the framework's own callers (`router/mod.rs` route headers through
  `set_header_value`) and the crate's tests and examples found by `cargo build --workspace --all-targets`; verify
  `--test res --test express_parity` and the replay of `fixtures/res_express.json`
- [ ] 2.4 Unit tests: every `Res` method of Decision 3 with an invalid input returns its error with the original text and
  leaves the response unchanged; `Display` of each error variant equals the recorded `TypeError` / `RangeError` message
  where the fixture has one; verify `cargo test -p corvis --lib http::res`

## 3. One implementation per helper (Decision 2)

- [ ] 3.1 `http/node_compat.rs`: `encode_url` per the fixture; `Response::redirect`, `Res::location` and the static
  directory redirect use it; `http::res::encode_url` re-exports it; delete `serve_static.rs`'s copy; verify the
  `encodeurl` replay and `redirect_location_is_encoded`
- [ ] 3.2 `fresh(req, etag, last_modified)` per the fixture; `express::finalize` and `ServeStatic::is_fresh` use it;
  `parse_http_date` and `http_date` with one unit; delete both copies; verify the `fresh` replay (direct and
  `express.static` cases) and `cargo test -p corvis --test fuzz_static --test static_response_security`
- [ ] 3.3 `vary_append` per the fixture used by `Res::vary`, CORS and compression; `router::cors::vary_append`
  `#[deprecated]` over it; `escape_html` and `latin1` single copies; `js_number` per the fixture used by the body parser
  and the compression negotiation; verify the `vary`, `escape_html` and `number` replays, `--test compression --features
  compression` and the CORS tests of `tests/app.rs`
- [ ] 3.4 Cookies: the `cookie` replay through `serialize_cookie`, `Cookie::try_to_string` and `Res::cookie` /
  `clear_cookie` in a test application (both APIs give the recorded header; `Expires` within one second of the recorded
  offset); update `cookie_max_age.rs` and the static-response assertions that pinned the old `Cookie` form; verify the
  replay has no failing case without a divergence entry

## 4. No silent `null` (Decision 5)

- [ ] 4.1 `Response::json(impl Into<Value>)`, `Response::try_json`; `MessageEvent::new(impl Into<Value>)`,
  `MessageEvent::try_new`; `__private.rs` `SerializeTag::convert` moves its `Value`; adapt callers; verify
  `cargo test -p corvis --test typed_responses --test empty_results`
- [ ] 4.2 Tests for the scenarios of the spec requirement "A value that cannot be serialized is never answered as null"
  (typed handler result, `Response::try_json`, `Res::json`, `MessageEvent::try_new`) and the allocation test of Decision 5
  (`Response::json` keeps the `Value` it is given); verify they pass

## 5. Statuses (Decision 6)

- [ ] 5.1 `IntoStatus for i32` through `u16::try_from`; rustdoc of `IntoStatus`; unit test of Decision 6; verify
  `cargo test -p corvis --lib http::exception`

## 6. Docs

- [ ] 6.1 `docs/http/cookies.md`: one serialization for both APIs, `SameSite` / `CookiePriority`, errors as values;
  `docs/recipes/express-migration.md`: `res.cookie(..)?` and the other `Res` methods where the original throws, the
  `Response::json` / `try_json` split; `docs/controllers.md` examples with `?`; `docs/http/server-sent-events.md`
  (`MessageEvent::try_new`); `docs/security/csrf.md` (`SameSite` enum); translate in `docs/es/`; verify
  `npm run docs:check` in `docs/`

## 7. Review

- [ ] 7.1 Security review with the `security-reviewer` agent: cookie serialization (name, value, `Domain`, `Path`
  validation unchanged; no attribute injection through any API), `encode_url` in `Location` and in HTML, `escape_html`,
  the freshness rule of static files (no `304` that hides a changed file), the session and OAuth2 cookie callers; repeat
  rounds until no open finding; record findings and fixes under "Security review" in design.md

## 8. Definition of done

- [ ] 8.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 8.2 `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo clippy -p corvis --no-default-features
  -- -D warnings`
- [ ] 8.3 Port apps: add the `?` at `consi-backend/src/settlement/settlement.controller.rs:37-40` and
  `houseplan-api/src/http/auth.controller.rs:23` in the port repository (record the commit in Implementation notes); then
  run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences, investigating any difference
  per design.md Risks
- [ ] 8.4 `cargo test --workspace` passes; per-feature checks: `cargo check -p corvis --tests --features <f>` for
  websockets, compression, tls, auth-local, auth-api-key, auth-session, auth-oauth2 and socket-io, with no warnings
- [ ] 8.5 Regression tests: one per spec scenario, as listed in groups 1-5; every fixture case passes or has a divergence
  entry with its reason
- [ ] 8.6 Docs pages updated in English and Spanish, then `npm run docs:check` in `docs/`
- [ ] 8.7 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 8.8 The security review of 7.1 is closed; a "Verification" table is added to design.md

## Workflow follow-up

- Archive the change with `/opsx:archive` (the deltas of `http/cookies`, `http/responses`, `http/static-files` and
  `http/exceptions` applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-up for the silent `unwrap_or(Value::Null)` of `websockets.rs` and `modules/cache.rs`, and remove the
  deprecated `router::cors::vary_append` in the next release.

# Tasks

Prerequisites: #136 is merged; #44 (`e3-sessions-and-signed-cookies`) and #45
(`e3-api-key-strategy-and-custom-strategies`) are merged into `dev` (epic order: #136 → #42 → #41 → #45 → #44 → #43).
No compilation is needed to review this plan; every task below ends with how to verify it.

## 1. Check the ground and add the feature

- [x] 1.1 Compare this change with the merged code: `corvis::auth` (`Strategy`, `StrategyOutcome::Redirect`,
  `Authenticator`, `AuthGuard::with_options`, `GuardPhase::is_active`), #44's `CookieKeys` (`sign`, `unsign`, error
  type), `Request::session()`, `Session::set` / `get` / `remove`, the session cookie's `SameSite` accessor, and #45's
  `Request::redact_query_param`; re-check the line references of `design.md` and the maintainer's answers to its Open
  questions; record every adjustment under "Implementation notes" in `design.md`; verify by listing each API with the
  file and line where it lives
- [x] 1.2 Add `auth-oauth2 = ["auth-session", "dep:tokio-rustls", "dep:webpki-roots"]` to
  `crates/corvis/Cargo.toml` and the empty `corvis::auth::oauth2` module behind it; verify
  `cargo clippy -p corvis --no-default-features --features auth-oauth2 -- -D warnings` passes, `cargo tree -d -p
  corvis --features auth-oauth2` shows no new duplicate, `Cargo.lock` gains no package, and `cargo deny check` passes
- [ ] 1.3 Add `auth-oauth2` to the per-feature loop (`.github/workflows/ci.yml:68-73`) and a
  `cargo test -p corvis --features auth-oauth2 --no-fail-fast` step next to the `compression` one (line 114); verify
  the steps run in the pull request's CI

## 2. Secret wrappers and options

- [x] 2.1 `secret.rs`: `OAuth2Token` (`expose`, redacted `Debug`, no `Display`, no `Serialize`) and the crate-private
  `ClientSecret`; verify with unit tests that `{:?}` of each never contains the value and that the types do not
  implement `Serialize` (a compile-fail doc test or a static assertion)
- [x] 2.2 `options.rs`: `OAuth2Options` (`new`, `google`, `github`), `OAuth2Endpoints`, `ClientAuth`,
  `OAuth2HttpOptions`, the state storage choice, `return_to_param`, `failure_redirect`, `authorization_param`,
  `scope`, `pkce(false)`, `state_ttl`, the limits of design decision 11, hand-written `Debug`; verify with unit tests
  that the presets hold the documented URLs and that `Debug` shows the client id and hosts but no secret or key
- [x] 2.3 Options validation returning one error per problem that names the option: URL rules (https or loopback
  http, no fragment, no user info), absolute redirect URI, empty client id or secret, missing state storage, scope
  grammar, empty scope on the Google preset, reserved and oversized extra parameters, every limit's bounds, cookie
  name rules, failure redirect rule; verify with a table test covering each row of the limits table and the spec
  scenarios "Relative redirect URI", "Plain HTTP to a remote host", "Reserved parameter", "Limit out of bounds",
  "Unsafe failure page", and that no error text contains the client secret
- [x] 2.4 The relative-path rule as one function (`is_safe_relative_path`), used by `return_to` and
  `failure_redirect`; verify with unit tests for every value of the scenario "Unsafe values dropped" and a proptest
  that an accepted value always starts with one `/`, has no backslash, no control character and no `//` prefix

## 3. Provider HTTP client

- [x] 3.1 `client.rs`: resolve, connect with the connect timeout, loopback check for `http`, `tokio-rustls` handshake
  (installed provider or `ring`, safe default versions, `webpki-roots` plus extra DER roots, host name from the URL),
  `hyper` HTTP/1.1 client connection with `Connection: close`, `Accept-Encoding: identity`, a `User-Agent`, total
  timeout around the whole exchange; `OAuth2Error` without provider payloads; verify against a scripted listener on
  `127.0.0.1:0` with tests for a normal JSON answer, "Provider does not answer" (paused `tokio` time or a 1 second
  configured timeout) and a connect failure
- [x] 3.2 Response limits: `Content-Length` over the limit fails before reading, a streamed body fails when it
  passes the limit, the header block is capped at 16 KiB, 3xx is `Redirected` without reading `Location`; verify
  with tests for "Oversized response" (both forms, asserting how many bytes the mock managed to write), "Token
  endpoint redirects" (a second listener that must receive nothing) and an oversized header block
- [x] 3.3 TLS path: verify with a mock that serves TLS with an `rcgen` certificate (the helper of
  `crates/corvis/tests/tls_common`) for "Untrusted certificate" (refused by default, accepted as an extra root, and
  refused when the certificate's name does not match the host)
- [x] 3.4 Scheme and environment rules: `http` to a name that resolves to a non-loopback address is refused at
  request time, `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` are ignored; verify with a test that sets the variables to
  a listener that must receive nothing, and a unit test of the loopback check for `127.0.0.1`, `[::1]`, `localhost`
  and a non-loopback literal
- [x] 3.5 Per-strategy semaphore with immediate `Busy`; verify with "Too many requests in flight", holding the mock's
  answers with a channel (no sleeps) and asserting the extra callback gets 503 and the mock sees no extra request
- [x] 3.6 Token request and response: form body builder (authorization code and refresh), `ClientAuth::RequestBody`
  and `Basic` (form-encoded before base64), JSON and form-encoded responses, `access_token` and `token_type` checks,
  400/401 as `Rejected` with the sanitized code; verify with unit tests for "Basic authentication", "Form-encoded
  token response", "Token response without access token", and the code sanitizer (length, character set)

## 4. State and PKCE

- [x] 4.1 `state.rs`: generation of `state` and verifier from `OsRng` (43 base64url characters), S256 challenge, the
  record and its compact JSON, a crate-private clock; verify with unit tests for "Two logins", the RFC 7636 appendix
  B test vector for the challenge, and a record round trip
- [x] 4.2 Cookie storage: sign the record with `CookieKeys`, write it with `serialize_cookie` and the attributes of
  design decision 4, read and verify it (signature, base64, JSON, strategy name, age, future skew), the deleting
  cookie; verify with tests for "Cookie attributes", "Record of another strategy", "Expired state" (clock moved, no
  sleep), a tampered value, a value over 4096 bytes, and the `secure(false)` variant (no prefix, no `Secure`, startup
  warning captured)
- [x] 4.3 Session storage: `Session::set` / `remove` under `oauth2:<name>`, startup errors without `SessionModule` or
  with a `SameSite=Strict` session cookie, the 500 path when the request has no session; verify with tests for
  "Session storage without sessions" and "Replay with session storage"
- [x] 4.4 Constant-time comparison of the `state` bytes with `subtle`; verify with a unit test for equal, different
  and different-length inputs, and by reading that no early return precedes the comparison

## 5. Strategy

- [x] 5.1 `OAuth2Validator`, `OAuth2Grant`, `OAuth2TokenResponse`, `OAuth2Strategy<V>` as `Injectable` (resolves
  `V`, validates `options()` once, `DiError` naming the strategy) and `Strategy` (name `V::NAME`); verify with a test
  app whose invalid options stop `init()` ("No storage chosen") and one that starts
- [x] 5.2 Login start: guard-phase check first, record stored, authorization URL built with the parameter order of
  the recorded original, `Cache-Control: no-store` and `Referrer-Policy: no-referrer`, per-guard `scope` option;
  verify with tests for "Login starts", "Scope from the guard", "Redirect outside the guard phase" and "Host header
  ignored"
- [x] 5.3 Callback checks before any provider request: method, single-string parameters, length limits, `code` with
  `error`, `redact_query_param` for `code` and `state`, record removal on every path, state verification; verify with
  tests for "Forged callback", "No state record", "Repeated parameter", "State cookie deleted" and "Logged callback
  URL", each asserting the mock provider received nothing
- [x] 5.4 Exchange, profile and validator call; failure mapping of design decision 9 with fixed challenges; verify
  with tests for "Validator receives the grant", "Validator rejects the user", "Challenge matches the verifier",
  "User denies consent", "Other provider error", "Code rejected", "Token endpoint down", "Replay with cookie storage"
- [x] 5.5 `failure_redirect` and `return_to` (read at start, stored in the record, private request extension set on
  success, `oauth2::return_to`); verify with tests for "Denied consent with a failure page", "Relative path kept",
  "Unsafe values dropped" and "Callback query is not consulted"
- [x] 5.6 Logging: fixed lines with strategy name, provider host, status, error variant and sanitized code; verify
  with "Logs of a failed exchange" in its own test binary (the logger is process-wide, as
  `tests/guard_responses_log.rs`) and "Debug output" for every public type of the module

## 6. Profiles and refresh

- [x] 6.1 `profile.rs`: `OAuth2Profile`, `OAuth2Email`, JSON form with the original's keys, the OpenID userinfo
  mapping (Google and generic), `verified_email()`; verify with unit tests for "Unverified Google address" and
  documents with missing optional fields
- [x] 6.2 GitHub: `/user` and, by scope, `/user/emails`, bearer header, `User-Agent` and `Accept` of the recorded
  original, numeric `id` as a string, a failed e-mail request fails the login; verify with tests for "GitHub profile
  with e-mails", "GitHub e-mail request fails" and "GitHub primary verified address"
- [x] 6.3 `OAuth2Client::new` and `refresh` with `OAuth2TokenSet`; verify with tests for "Refresh" and "Refresh
  rejected"

## 7. Fixture recorded from the original

- [x] 7.1 Add `passport-oauth2` 1.8.0, `passport-google-oauth20` 2.0.0, `passport-github2` 0.1.12 and
  `passport-oauth2-refresh` 2.2.0 (exact versions) to `crates/corvis/tests/fixtures/auth/package.json` and update
  `package-lock.json`; re-read the packages' sources and correct "The original" section of `design.md` where it
  differs; verify `npm ci` installs exactly those versions
- [x] 7.2 `oauth2.mjs`: fake provider and the original app on `127.0.0.1` (header comment with how to run it, as
  `strategies.mjs`), writing `oauth2.json` with the cases of design decision 14, fake fixed credentials, and a
  `divergence` note on: `state` and PKCE always on, provider errors and rejected codes answered 401, bearer header
  for profile requests if the original uses the query, the e-mail flags, absolute redirect URI; verify the generator
  runs twice with identical output and the JSON holds no real credential
- [x] 7.3 `crates/corvis/tests/auth_oauth2.rs`: the scripted mock provider and the replay of every recorded case
  (redirect parameters, token request, profile requests, profiles field by field, outcomes, refresh); verify
  "Replay without Node" by running the test with Node absent from `PATH` and with the network disabled

## 8. Docs and example

- [x] 8.1 `docs/recipes/passport.md`: an OAuth2 section (validator, presets, the two routes, state storage choice and
  its trade-off, redirect URI rule, `return_to`, `failure_redirect`, identifying users by `(provider, id)` and
  linking only by `verified_email()`, the unverified `id_token`, refresh, the limits table with defaults, the
  divergences from the original, the note about proxy access logs and about rate limiting the login route); update
  line 9 of that page and `docs/security/authentication.md:257`; verify the examples compile as doc tests or match
  the example app
- [x] 8.2 Spanish pages `docs/es/recipes/passport.md` and `docs/es/security/authentication.md` (the `docs-writer`
  agent); verify `npm run docs:check` in `docs/` passes
- [x] 8.3 `examples/cats-app`: GitHub login per design decision 15 with endpoints from configuration and a test
  against a mock provider on `127.0.0.1`; verify the example builds, its existing tests (`posture.rs`, `startup.rs`,
  `login_throttle.rs`) still pass and the new test passes

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and the per-feature loop of
  `.github/workflows/ci.yml` passes with `auth-oauth2` alone and with `--no-default-features`
- [x] 9.3 `cargo test --workspace` and `cargo test -p corvis --features auth-oauth2` pass, including
  `auth_oauth2.rs`, without Node and without network access beyond `127.0.0.1`
- [x] 9.4 Regression tests exist for every scenario of `auth/oauth2`
- [x] 9.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 9.6 corvis-parity check: the change adds code only behind `auth-oauth2`, which the ports do not enable; if any
  runtime code compiled without the feature changed (for example `Request`, `AuthGuard`, the `Authenticator`), run
  the check and verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences; otherwise record in
  the archive notes that nothing outside the feature changed
- [x] 9.7 `cargo deny check` passes and `Cargo.lock` has no new package
- [x] 9.8 **Security review (required before the merge)**: run the `security-reviewer` agent over
  `corvis::auth::oauth2`, the fixture generator and the example. Scope: login CSRF and `state` handling (generation,
  binding, expiry, replay in both storage modes, cookie tossing and the `__Host-` prefix, a record reused across
  strategies); PKCE (verifier entropy, challenge, downgrade); redirect URI and host header injection; open redirect
  through `return_to`, `failure_redirect` and the authorization URL; the provider client (redirects, loopback rule
  and DNS answers, TLS verification and extra roots, timeouts, body and header limits, slow bodies, proxy variables,
  concurrency limit as a denial of service, request smuggling through configured values); client authentication
  encoding; token response parsing (duplicate keys, wrong types, form-encoded answers); profile mapping and
  `verified_email()` (account takeover through unverified or non-primary addresses); failure mapping and what an
  unauthenticated visitor can make the server log or answer; secrets, codes, tokens and personal data in `Debug`,
  logs, errors and responses; panics reachable from a query, a cookie or a provider response; the startup checks and
  every default of the limits table. Verify every finding is fixed with a regression test or recorded as a new issue,
  and list the findings in the archive notes

## 10. Security review (round 1) fixes

- [x] 10.1 M1a: a bounded per-instance set of redeemed states (`state_ttl` retention, `redeemed_states_max` option),
  claimed before the token request; verify with "Concurrent replays of one flow", "Forged states do not enter the
  set" and "The set is bounded"
- [x] 10.2 M1b: per-client-address share of the in-flight slots (`max_in_flight_per_client`, a quarter by default);
  verify with "Share reached" and "Slots are given back"
- [x] 10.3 M2: `provider` is the preset's name only for unchanged preset endpoints, otherwise the strategy name;
  `issuer` on the profile; verify with "Three namespaces for one subject" and "Issuer"
- [x] 10.4 L1: startup warnings for a state cookie and a session cookie without the `__Host-` prefix; verify with the
  scenarios of "Cookies without the host prefix warn at startup"
- [x] 10.5 L2: the cats example answers `redacted_url()` in error bodies; verify with its test
- [x] 10.6 I1: overflow-safe flow expiry; verify with the unit test and "Extreme timestamps"
- [x] 10.7 I2: the validator's refusal goes to `failure_redirect`; verify with "Validator refusal"
- [x] 10.8 Docs in English and Spanish (callback throttling, in-flight cap and per-address share, `(provider, id)`
  with the rule and the warning against e-mail alone, the string `id`, the cookie-name warnings); `npm run docs:check`
  passes
- [x] 10.9 Maintainer: re-run the security review over the fixes (task 9.8 stays open until then)

## 11. Security review (round 2) fixes

- [x] 11.1 The router's 404 message uses the redacted URL; verify with
  `a_callback_url_that_misses_its_route_does_not_answer_its_query_back`
- [x] 11.2 `provider_namespace`: `name@issuer` for a strategy named like a preset that is not the preset, with a
  startup warning; verify with the provider assertions and the log test
- [x] 11.3 Redeemed entries kept through the second they expire in; cookie-name prefixes compared without case
- [x] 11.4 Docs (EN, ES), the option's rustdoc, the example's comment and the spec state the provider rule

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #43, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).
- Align the change name in the issue body and `ROADMAP.md:101` (`e3-generic-oauth2-google-and-github`) with
  `e3-oauth2-strategies` (maintainer session).
- Create the follow-up issues accepted from the proposal's "Out of scope" (OpenID Connect strategy with ID token
  validation; redirect URI allow-list for multi-domain apps) under epic #39 and link them from the archive notes.

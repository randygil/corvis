# Tasks

This change is a spike: group 1 finishes it. Groups 2-8 are the plan for the follow-up issues of epic #39; each one
becomes its own OpenSpec change (named in the group title) when work starts, copying the relevant tasks and the spec
delta of this change. Order: #42 first (it carries the strategy core), then #41, #45, #44, #43, #46 and #47. Every
group ends with its own definition of done (group 9 lists the checks).

## 1. Spike deliverables (#40)

- [x] 1.1 Maintainer reviews `design.md` and answers the Open questions (module name, default hasher, core in #42,
  oracle `package.json`, API keys in URLs, the smaller divergences); verify each answer is written back into
  `design.md` and, when it changes behavior, into the affected spec delta
- [x] 1.2 Validate the change with `openspec validate e3-strategy-module-design --strict`; verify it reports no errors
- [x] 1.3 Update the description of the follow-up issues #41-#47 with a link to this change and to the group of
  `tasks.md` that plans them (maintainer session, `gh issue edit`); verify each issue links back

## 2. #42 JWT strategy and the strategy core (`e3-jwt-strategy`)

- [ ] 2.1 Guard short-circuit and response headers (`http/guard-responses`): add `Request::respond_with` and
  `Request::append_response_header` in `crates/corvis/src/http/request.rs`; check the stored response after each
  guard in `run_pipeline` (`crates/corvis/src/router/mod.rs:613-618`) and merge pending headers in `execute_route`
  (`router/mod.rs:572-610`) for success, exception and short-circuit; verify with new tests in
  `crates/corvis/tests/` for every scenario of `http/guard-responses`, plus the unchanged 403 test
- [ ] 2.2 Run the `corvis-parity` check (three port apps, 0 apidiff differences) after 2.1, before building on it;
  verify the report of the parity-verifier
- [ ] 2.3 Add the `auth` cargo feature and the module `crates/corvis/src/auth/` with `Strategy`, `StrategyOutcome`,
  `AuthUser`, `AuthInfo` (design decisions 1 and 5); verify with unit tests that build each outcome
- [ ] 2.4 `Provider::strategy::<S>()` (new tag next to `enhancer` in `crates/corvis/src/di/provider.rs:117`) and the
  `StrategyRegistry` built in `CorvisApplication::init()` (`crates/corvis/src/application.rs:361-397`), with the
  duplicate-name startup error; verify with tests for "found by default name", "two strategies of the same kind"
  and "duplicate name"
- [ ] 2.5 `Authenticator::authenticate` as a port of `authenticate.js` (order, first success, fail moves on, error
  stops, all failed, pass, redirect through `respond_with`); verify with unit tests over fake strategies, one per
  scenario of `auth/strategies`
- [ ] 2.6 `AuthGuard::new`, `chain`, `default`, `.optional()`, `.on_failure(..)`, and `StrategyModule::register` for
  the default strategy; the `User<T>` extractor and `Request::auth_info()`; verify with HTTP tests through
  `corvis::testing::request`
- [ ] 2.7 Startup check for unknown strategy names (`CanActivate::as_any` default method, override in `AuthGuard`,
  walk compiled route guards in `init()`); verify with tests for the "typo in a guard" and "no default strategy"
  scenarios, and that a user guard calling `Authenticator` with an unknown name answers the 500
- [ ] 2.8 Oracle tooling: `crates/corvis/tests/fixtures/auth/package.json` with exact versions and lockfile, and a
  README of how to refresh fixtures (design decision 11); verify `npm ci` there and one trivial oracle run
- [ ] 2.9 Core oracle `strategies.oracle.mjs` (custom `passport-strategy` subclasses driven by a header to each
  outcome, chained guards, optional guard, error thrown from `validate`) and `tests/auth_strategies.rs`; verify every
  recorded case passes
- [ ] 2.10 `auth-jwt` feature: `JwtStrategy<V: JwtValidator>`, `JwtStrategyOptions`, extractors with the
  `passport-jwt` semantics (`(\S+)\s+(\S+)` for the auth header, `from_cookie` extra), static key or key provider,
  `issuer`/`audience`/`algorithms`/`ignore_expiration`; reuse the verification core of `JwtService`
  (`crates/corvis/src/jwt.rs:250-292`) without changing `JwtService::verify` or `extract_bearer_token`
  (`jwt.rs:314-318`); verify with unit tests per extractor and option
- [ ] 2.11 JWT oracle `jwt.oracle.mjs` (`passport-jwt` 4.0.1) with fixed keys and tokens: no token, tab and extra
  words in the header, lowercase scheme, expired with and without `ignore_expiration`, audience list, issuer, key
  provider by `iss`, key provider error, RS256 key, `alg: none`; `tests/auth_jwt.rs` replays it; verify all cases pass
- [ ] 2.12 Docs: `docs/recipes/passport.md` (status, the strategy model, JWT), `docs/security/authentication.md`
  (replace the hand-made guard with `AuthGuard::new("jwt")`, keep the manual path as an alternative) and their
  Spanish pages; example: switch the JWT part of an existing example (`examples/cats-app` or `examples/tertulia`) to
  the strategy; verify `npm run docs:check` in `docs/` and that the example runs
- [ ] 2.13 Definition of done of group 9, including the corvis-parity check (core router changed)

## 3. #41 Local strategy and password hashing (`e3-local-strategy-and-password-hashing`)

- [ ] 3.1 `auth-local` feature with `bcrypt`, `argon2` and `subtle`; `PasswordHasher` (bcrypt cost 10 by default,
  argon2id PHC, verify by prefix, `needs_rehash`, `verify_dummy`, error for bcrypt passwords over 72 bytes); verify
  with unit tests, including malformed hashes that must not panic
- [ ] 3.2 Property test: random strings as hashes and passwords never panic in `verify`; verify it runs in
  `cargo test` with the default case count
- [ ] 3.3 bcryptjs oracle `bcrypt.oracle.mjs` (`bcryptjs` 3.0.3): hashes of ASCII, non-ASCII, empty, 72-byte and
  73-byte passwords at cost 4 and 10, and Corvis-made hashes checked with `compareSync`; verify both directions in
  `tests/auth_password.rs`
- [ ] 3.4 `LocalStrategy<V: LocalValidator>` with field names, `a[b]` paths, body then query, non-string values as
  missing; verify with unit tests
- [ ] 3.5 Local oracle `local.oracle.mjs` (`passport-local` 1.0.0): JSON body, form body, query, missing fields,
  nested field, non-string username (marked divergence), validator returning none and throwing; verify every case in
  `tests/auth_local.rs`
- [ ] 3.6 Docs: `docs/security/encryption-and-hashing.md` (from `unavailable` to documented hashing),
  `docs/security/authentication.md` (hash instead of plain-text comparison, remove the warning of lines 96-100),
  `docs/recipes/passport.md` (local strategy) and Spanish pages; verify `npm run docs:check`
- [ ] 3.7 Definition of done of group 9 (corvis-parity only if runtime code outside `auth` changed)

## 4. #45 API key strategy and custom strategies (`e3-api-key-strategy-and-custom-strategies`)

- [ ] 4.1 `ApiKeyStrategy<V: ApiKeyValidator>` in the `auth` feature: header (default `X-Api-Key`, case-insensitive),
  literal prefix, optional query parameter; constant-time comparison helper; verify with unit tests
- [ ] 4.2 Oracle `api_key.oracle.mjs` (`passport-headerapikey` 1.2.2 and `passport-custom` 1.2.1 for the query):
  valid key, wrong key, missing header, prefix, wrong prefix, header name case, query key; `tests/auth_api_key.rs`;
  verify every case
- [ ] 4.3 Custom strategy guide: extend `strategies.oracle.mjs` cases if needed so all five outcomes of a user
  `Strategy` are covered with DI (the strategy injects a service); verify in `tests/auth_strategies.rs`
- [ ] 4.4 Docs: "API keys" and "Writing a strategy" sections in `docs/recipes/passport.md` with the warning about keys
  in URLs, and Spanish pages; verify `npm run docs:check`
- [ ] 4.5 Definition of done of group 9

## 5. #44 Sessions and signed cookies (`e3-sessions-and-signed-cookies`)

- [ ] 5.1 Signed cookie primitives (`hmac` aligned with `sha2 = "0.10"`, design table): sign and unsign in the
  `cookie-signature` format with a list of secrets and constant-time comparison; verify with vectors produced by
  `cookie-signature` 1.0.7 in an oracle
- [ ] 5.2 `SessionModule` middleware (`CorvisMiddleware`, `crates/corvis/src/common/middleware.rs:29-31`): cookie
  name and attributes, `resave`, `save_uninitialized`, `rolling`, `Set-Cookie` through `append_response_header`;
  `Request::session()` API; verify with HTTP tests
- [ ] 5.3 `SessionStore` trait and the memory store with expiry; verify with unit tests
- [ ] 5.4 Redis store compatible with `connect-redis` 10.0.0 (prefix, JSON, TTL) behind `auth-session-redis`; verify
  against a local Redis bound to 127.0.0.1, skipped when unavailable like the Postgres tests
- [ ] 5.5 ORM store (`sid`, `sess`, `expire` table); verify with the ORM's SQLite test setup
- [ ] 5.6 `Authenticator::log_in` / `log_out` with session regeneration, `SessionSerializer`, session user restored
  before guards, the built-in `"session"` strategy and `AuthGuard::..log_in()`; verify fixation and logout scenarios
- [ ] 5.7 Oracle `session.oracle.mjs` (`express-session` 1.19.0, `passport` 0.7.0, `connect-redis` 10.0.0): cookies
  read both ways, secret rotation, tampered cookie, the five save cases with explicit options, login/logout, deleted
  user, shared Redis; `tests/auth_session.rs`; verify every case
- [ ] 5.8 Docs: a sessions page (or section in `docs/security/authentication.md`), `docs/security/csrf.md` updated to
  say sessions exist (CSRF tokens stay out of scope), Spanish pages; verify `npm run docs:check`
- [ ] 5.9 Definition of done of group 9

## 6. #43 Generic OAuth2, Google and GitHub (`e3-generic-oauth2-google-and-github`)

- [ ] 6.1 `auth-oauth2` feature with `oauth2` 5.x and `reqwest` (rustls, no redirects, timeout); `OAuth2Strategy<V>`
  with options (URLs, client, scopes, separator, extra params); verify with unit tests on the authorization URL
- [ ] 6.2 State and PKCE storage in the session or a signed short-lived cookie, single use; reject `plain`; verify
  forged, missing, expired and replayed state
- [ ] 6.3 Fake authorization server for tests (authorization, token, profile endpoints on 127.0.0.1, scriptable
  answers: error, redirect, missing token); verify it is shared by the oracle and the Rust test
- [ ] 6.4 Callback handling (`access_denied`, other errors, token exchange and profile errors, validator call);
  verify with the fake server
- [ ] 6.5 Google and GitHub strategies with the originals' default URLs and profile normalization; verify the
  profiles against recorded profiles of `passport-google-oauth20` 2.0.0 and `passport-github2` 0.1.12
- [ ] 6.6 Refresh helper with the form body of `passport-oauth2-refresh` 2.2.0; verify against the fake server
- [ ] 6.7 Oracle `oauth2.oracle.mjs` and `tests/auth_oauth2.rs`: redirect parameters (state/PKCE values ignored, marked
  divergence for their presence), each callback outcome, profiles, refresh; verify every case
- [ ] 6.8 Docs: OAuth2 section in `docs/recipes/passport.md` and Spanish page, with the Google and GitHub setup;
  verify `npm run docs:check`
- [ ] 6.9 Definition of done of group 9

## 7. #46 Authentication security review (`e3-authentication-security-review`)

- [ ] 7.1 Run the `security-reviewer` agent over the `auth` module and the router changes of 2.1: OAuth CSRF and
  state reuse, session fixation, timing of key and password checks, user enumeration (`verify_dummy` used in docs
  and examples), cookie flags, redirects; verify every finding is fixed or recorded as a new issue
- [ ] 7.2 Login rate limiting with the existing `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs:159-165`,
  per-route `THROTTLE_LIMIT`/`THROTTLE_TTL`): document and test a stricter limit on login routes, keyed by client
  and, if needed, by username; verify with an HTTP test that the limit answers 429
- [ ] 7.3 Fuzz targets for the new parsers (auth header, signed cookie, session JSON, OAuth2 callback query) in the
  style of the existing proptest fuzzing; verify they run in `cargo test` and in the CI fuzz job of #6
- [ ] 7.4 Docs: a "Security notes" section listing every divergence of design decision 10 and the specs, in English
  and Spanish; verify `npm run docs:check`
- [ ] 7.5 Definition of done of group 9

## 8. #47 Authentication docs and example (`e3-authentication-docs-and-example`)

- [ ] 8.1 Rewrite `docs/security/authentication.md` and `docs/security/authorization.md` around `AuthGuard`, with a
  migration table from the original's API (`PassportStrategy`, `AuthGuard('jwt')`, `req.user`, `handleRequest`) to
  Corvis; mark `docs/recipes/passport.md` as documented; verify the frontmatter `status` values and links
- [ ] 8.2 New example `examples/auth` with local login (hashed passwords), JWT, an API key route, a session login and
  an OAuth2 login against the fake server; verify it builds, runs on 127.0.0.1, and its tests pass
- [ ] 8.3 Spanish translation of every page touched in 8.1; verify `npm run docs:check`
- [ ] 8.4 Definition of done of group 9 (no runtime code changes expected; corvis-parity not needed if so)

## 9. Definition of done (applies to every follow-up change; for this spike, only 9.6 applies)

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, also with `--features` combinations of the
  new auth features (`auth`, `auth-jwt`, `auth-local`, `auth-oauth2`, `auth-session`) and with
  `--no-default-features`
- [ ] 9.3 `cargo test --workspace` passes, including the fixture replays (no Node needed)
- [ ] 9.4 Regression tests for every scenario of the change's spec delta, and the differential fixture against the
  original Node library, with divergences marked
- [ ] 9.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 9.6 This spike changes no code: confirm `git status` shows only files under
  `openspec/changes/e3-strategy-module-design/`
- [ ] 9.7 corvis-parity check when framework runtime code outside the new `auth` module changes (always for #42):
  the three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #40, tick it in epic #39, and move its card to Done (see the `corvis-task` skill).

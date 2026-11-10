# Design

## Context

Epic #39 delivers authentication in seven changes. The core is merged; the five strategy changes are approved and
are implemented, reviewed and merged one by one before this change starts. Line numbers below were checked on
`origin/dev` at `c837dcb`. Statements about #42, #41, #45, #44 and #43 come from their artifacts in
`openspec/changes/` (design plus Review notes, which override the design); the JWT implementation in progress was
read for its Implementation notes. They are designs until merged: task 1.1 re-checks every one.

### What is merged (strategy core, #136)

- `Authenticator::authenticate` (`crates/corvis/src/auth/authenticator.rs:129-173`) runs the named strategies in
  order: `Fail` records the failure and continues (line 148), `Error(e)` returns it (151), `Pass` ends the chain and
  lets the request through (152), `Redirect` stores a response through `respond_with`, which only works in the guard
  phase.
- `AuthGuard::can_activate` (`crates/corvis/src/auth/guard.rs:154-190`): any context that is not HTTP is denied
  without running a strategy (157-159); `skip_public` is checked before the authenticator (160-162) and only when
  the guard was built with `.skip_public()`; `Pass` and an optional failure call `req.clear_user()` (174-183);
  `controls_access()` is `false` for an optional guard (192-194).
- `Request::set_user`, `clear_user`, `user`, `auth_info`, `user_as` (`crates/corvis/src/http/request.rs:248-272`);
  the typed user is in a private slot read only by `User<T>` (core Implementation notes 11 and 12).
- The router runs `route.guards` in the order global, controller, route, each inside the guard phase
  (`crates/corvis/src/router/mod.rs:806-826`); `Ok(false)` is `403 Forbidden resource` (822).
- Global guards provided through DI (`Provider::app_guard`) are inserted before the instance guards of
  `use_global_guards` (`crates/corvis/src/application.rs:532-534`, `226-228`).
- Startup: `auth::build_registry` (`application.rs:553`), `auth::check_routes` (567,
  `crates/corvis/src/auth/mod.rs:94-112`: walks every compiled route's guards through `CanActivate::as_any`) and
  `auth::warn_gateways` (569, `auth/mod.rs:115-131`).
- Tests of the core: `crates/corvis/tests/auth_guard.rs`, `auth_strategies.rs`, `auth_hardening.rs`, `auth_ws.rs`,
  `guard_responses.rs`.

### What the strategy changes add (as designed)

| Change | Module / feature | Security decisions that matter across modules |
|---|---|---|
| #42 | `auth::jwt`, feature `jwt` | allow-list by key family, `exp` required, HMAC secret ≥ 32 bytes (startup error), `max_token_length` 8192, extractors only where configured, startup warning for the query extractor; `JwtService` unchanged |
| #41 | `auth::local`, `auth-local` | argon2id 19456 KiB / t=2 / p=1, bcrypt verification, `verify_or_dummy`, 1024-byte credentials, bounded concurrency; login throttling left to #46 |
| #45 | `auth::api_key`, `auth-api-key` | default name `"api-key"`, header / `bearer()` / opt-in query sources, conflicting keys fail, digest comparison with `subtle`, `Request::redact_query_param` and `redacted_url()` |
| #44 | `auth::session`, `http::signed_cookie`, `auth-session` | `CookieKeys` (≥ 32 bytes), `HttpOnly` + `Secure` + `SameSite=Lax`, idle 24 h / absolute 7 days, regeneration on login and logout, the session user restored **before the guards on every route**, strategy `"session"` |
| #43 | `auth::oauth2`, `auth-oauth2` | `state` + PKCE S256, state in a signed `__Host-` cookie or in the session (explicit choice), exact redirect URI, private TLS client, `return_to` rule; throttling of login and callback left to #46 |

Each of them ends with a `security-reviewer` round over its own module (their last task). The core's round found a
`respond_with` usable outside the guard phase, a typed user readable through a foreign extension and redirects with
non-3xx statuses; #42's found lax `iss`/`aud` handling in the older `JwtService` path and a wrapper guard that hid a
misconfiguration until the first request.

### Surfaces outside the HTTP guard pipeline

- Upgrade requests are dispatched before the router: gateways at `crates/corvis/src/server.rs:560-561`, raw
  `upgrade` listeners at 565-567. No middleware runs, so there is no session on a handshake (#44 Out of scope).
- A gateway can refuse a handshake with `verify_client` (`crates/corvis/src/websockets.rs:391-392`, `433-435`,
  called at 742-744); `examples/tertulia` authenticates in `handle_connection`
  (`examples/tertulia/src/chat/chat_gateway.rs:212`) with `JwtService`, the pattern
  `docs/security/authentication.md:235-250` documents. Browsers send cookies on the handshake;
  `GatewayOptions::allowed_origins` (`websockets.rs:387-390`) exists against cross-site WebSocket hijacking.
- Engine.IO and Socket.IO (`openspec/changes/e2-*`) are approved changes, not merged; their requests are handled
  before the router as well, and connection authentication is the connect middleware with the `auth` payload.

### Rate limiting, fuzzing and dependency checks today

- `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs:1181`) does not control access (1256-1258) and has no
  `as_any` override. Per-route overrides and named trackers exist (`throttle(name)`, `with_named_tracker`, line 783);
  `ThrottlerTrackerFn` receives the `ExecutionContext` (174-175). `docs/security/rate-limiting.md:105-121` shows a
  login override and its "Client-chosen trackers" warning (205-214) already describes per-account counting and
  lockout. `examples/cats-app/src/auth/auth_controller.rs:19-20` limits login to 5 per minute and
  `examples/cats-app/tests/login_throttle.rs` tests it.
- Fuzzing is property testing: `crates/corvis/src/fuzz_tests.rs` (`config(cases)` reads `PROPTEST_CASES`, lines
  21-24), `crates/corvis/tests/fuzz_static.rs`; the CI job `fuzz` (`.github/workflows/ci.yml:143-177`, 20,000 cases)
  and the nightly run (`.github/workflows/nightly.yml:33-69`, 100,000 cases). The strategy changes add their own
  parser properties to `fuzz_tests.rs` (header parser, hashes, key header, signed cookies, cookie headers).
- `deny.toml`: `all-features = true` (line 4), `multiple-versions = "warn"` (32), no banned crates;
  `.cargo/audit.toml` ignores two advisories that do not touch auth crates. `Cargo.lock` today: `jsonwebtoken`
  9.3.1, `ring` 0.17.14, `rustls` 0.23.46, `subtle` 2.6.1, `hmac` 0.12.1, `sha2` 0.10.9, `digest` 0.10.7, one
  generation each; `rand` (0.8.8 and 0.10.3), `rand_core` and `getrandom` already have two; no `openssl`,
  `native-tls` or `aws-lc-rs`.

## Goals / Non-Goals

**Goals**

- Find and close what only exists between modules: identity confusion between a session and a token, chain
  fallthrough, guard order, public routes, OAuth2 state with session login, sockets, inconsistent defaults.
- Leave the properties behind as tests, so a later change that breaks one fails `cargo test`.
- One document that says what is protected, how, and what is left to the application.
- Login rate limiting that cannot be dodged by moving the username and that costs nothing when it rejects.
- A written policy for the crypto crates.

**Non-Goals**

- Re-reviewing each module for its own threats; new strategies or features; handshake authentication through
  strategies; changing `JwtService`; a coverage-guided fuzzer (proposal Out of scope).

## Decisions

### 1. The review is scoped to interactions, with the earlier findings as input

The `security-reviewer` agent gets: the list of decision 2, the findings of the six earlier rounds (archive notes of
each change), and the test application of decision 4 so that every suspicion can be turned into a failing test
before it is called a finding. It reads the whole `crates/corvis/src/auth/` tree, `http/signed_cookie.rs`, the guard
loop of the router, the `Request` user slots, the handshake code and the throttler hooks.

Alternative: one more full review of every module. Rejected: six rounds already did it with the module's own threat
list in hand, and a seventh would spend its attention on the same ground.

### 2. Scope list of the review

1. **Identity**: session-restored user versus guard-established user on every guard shape; `set_user` by a later
   hand-written guard; the session user on routes without an `AuthGuard`; `User<T>` versus `Request::user()`.
2. **Chains**: fallthrough after `Fail` versus stop after `Error` for every built-in strategy (store outage, provider
   outage, key provider error, hasher error); any path to `Pass`; `on_failure` handlers that echo failure reasons to
   the client; auth info of a chain.
3. **Shared places**: `Authorization: Bearer` read by JWT and by the API key `bearer()` source; a JWT cookie
   (`from_cookie`) next to the session cookie; query parameters read by a JWT extractor, an API key source and the
   OAuth2 callback; duplicate headers.
4. **Guard order and public routes**: throttler after `AuthGuard`; a global `AuthGuard` with and without
   `.skip_public()`; controller-level `#[public]` with route-level guards; wrapper guards that hide an `AuthGuard`
   from `as_any` (and so from the startup checks and from decision 6's warning); route posture with optional guards.
5. **Session with OAuth2**: state in the session and `.log_in()` on the callback (order of removal, regeneration and
   destruction); `keep_session_info_on_login`; a login started in one session and finished in another; a callback on
   an already authenticated session; logout while a login is in flight; the state cookie next to the session cookie
   (names, prefixes, `SameSite`).
6. **Cookies on failure paths**: appended headers are sent on a later `401`/`403` (core Implementation notes 11);
   every place a strategy or the session middleware appends a cookie.
7. **CSRF posture**: every cookie-borne credential (session, JWT cookie) on state-changing routes; `SameSite`
   defaults; the session user on routes the author believes are token-only (item 1).
8. **Sockets**: handshake authentication with `JwtService` versus the JWT strategy's rules; tokens in the handshake
   query and request logs; `apply_global_enhancers_to_gateways(false)` removing more than the `AuthGuard`; Engine.IO
   and Socket.IO handshakes when merged.
9. **Defaults**: the table of decision 5, row by row, for rules that differ between strategies without a reason.
10. **Secrets in output** across the combination: logs of one module printing what another redacts (the redacted
    URL of #45 versus middleware and the `404` body; `Debug` of composite types that embed another module's types).
11. **Resource use in combination**: unauthenticated cost per request of each route shape (hash, store read, user
    lookup, provider request) with and without the throttler.
12. **Feature combinations**: each auth feature alone, all together, and none; code that changes behavior when
    another feature is enabled.

### 3. Invariants are a checklist, each one a test

The spec `auth/security-invariants` is the checklist. One test file, `crates/corvis/tests/auth_invariants.rs`, has a
test per scenario, named after it. They need every auth feature, so the file starts with
`#![cfg(all(feature = "jwt", feature = "auth-local", feature = "auth-api-key", feature = "auth-session", feature =
"auth-oauth2"))]` and CI runs it with those features (decision 8). Counters, not timing: the test application
exposes how many times each validator, the hasher's key derivation, the session store and the mock provider were
called.

The invariants pin today's designed behavior where it is defensible (for example: an optional token guard clears a
session-restored user). If the review concludes a behavior is wrong, the fix follows decision 9 and the scenario
changes with it.

### 4. One test application for invariants, fuzzing and the review

`crates/corvis/tests/auth_common/mod.rs` builds an application with every strategy registered (JWT with header and
cookie extractors, local with a small-parameter hasher, API key with header and `bearer()`, session with the memory
store and a test clock, a generic OAuth2 strategy against a mock provider on `127.0.0.1`), and a grid of routes: one
per guard shape (each strategy alone, chains in both orders, optional, default, `skip_public` on public and
non-public handlers, a roles-only route, `.log_in()` routes, a logout). It records which handler ran and with which
user, and captures the logger. The mock provider reuses the one #43 adds for `auth_oauth2.rs`; if that one is
private to its test file, task 1.2 moves it to `tests/auth_common/`.

Alternative: use `examples/cats-app`, which accumulates every strategy. Rejected: the example is user-facing and
must stay readable; the grid of guard shapes is test scaffolding.

### 5. Defaults table with a test

The threat model has one table: setting, default, where it is set, the original's default. A fenced block under it
repeats the rows as `key = value` lines; `crates/corvis/tests/auth_defaults.rs` reads that block from
`docs/security/authentication-threat-model.md` and compares each key with the value the code reports
(`JwtStrategyOptions::default()`, `PasswordHasherOptions::default()`, `ApiKeyStrategyOptions::default()`,
`SessionOptions` defaults, `OAuth2Options` defaults). A row without a code accessor is compared through behavior
(the 31/32-byte secret scenarios). Rows, at least: secret minimums (JWT HMAC, `CookieKeys`, `ExpectedApiKey`); token,
key, credential, cookie and session size limits; `clock_tolerance`; `exp` required; argon2id parameters and verify
limits; session cookie attributes, idle and absolute timeouts; OAuth2 state lifetime, timeouts, body limits,
concurrency, PKCE; URL-borne credentials off.

Known inconsistencies to settle in the review rather than here: the JWT cookie extractor has no startup note while
the query extractor warns; the OAuth2 client secret has no minimum (it is issued by the provider); `JwtService`
accepts what the JWT strategy refuses.

Alternative: generate the table from code. Rejected: the point is a human-reviewed statement that fails when code
drifts, not a mirror of the code.

### 6. Guard-order warning through `as_any`

`ThrottlerGuard` overrides `CanActivate::as_any` (default `None`, added by the core). `auth::check_routes` already
walks each compiled route's guards; while walking, it notes the position of the first `AuthGuard` and of any
`ThrottlerGuard` after it, and logs one warning per route:
`AuthGuard runs before ThrottlerGuard on POST /auth/login: the rate limit is checked after authentication. Register
ThrottlerGuard as a global guard (Provider::app_guard) or list it first`. It is a warning, not an error: an
application may throttle authenticated users per account on purpose after authenticating them.

A wrapper guard that does not forward `as_any` hides both guards; that is the pattern #42's review flagged. The
warning cannot see through it; the threat model says so, and `docs/recipes/passport.md:117` already tells a wrapper
to return the inner guard from `as_any`.

Alternatives: compare `guard_name()` strings (breaks for wrappers and renamed types); an error instead of a warning
(blocks the legitimate per-account-after-auth setup).

### 7. `login_tracker`: the account the strategy will read

`corvis::auth::local::login_tracker(options: LocalStrategyOptions) -> impl Fn(ExecutionContext) -> ..` (the shape
`with_named_tracker` takes). It runs the same lookup the strategy runs (#41 decision 6: the split rule, body then
query, strings only, the length limit), on the same request. Value: `"acct:" + hex(SHA-256(username bytes))`; no
readable username → the built-in tracker value (normalized client address, through `normalize_ip`), so the request is
still counted. The digest keeps usernames out of the storage keys and bounds the key size. No case folding: the
application's validator decides what an account is; the docs tell an application that lowercases e-mails to wrap the
tracker.

The lookup function of the strategy becomes `pub(crate)` and is shared, so tracker and strategy cannot drift; a unit
test feeds both the fixture cases of `local.json`.

The recipe (documented and tested in `auth_login_throttle.rs`): the `default` throttler counts by client with the
shorter `ttl`, a `login` throttler counts by account on the login route with `.tracker("login-account")`, and
`ThrottlerGuard` is global. For OAuth2: the per-client throttler on the login and callback routes.

Alternatives: document a hand-written tracker (each application re-implements the lookup and gets the body/query
order wrong); a lockout counter of failed attempts only (needs the outcome of the guard, which the throttler does
not see; out of scope).

### 8. Fuzz targets and where they run

`crates/corvis/tests/fuzz_auth.rs`, using `proptest` and the shared application, with the case count read from
`PROPTEST_CASES` like `fuzz_tests.rs:21-24` (the helper is copied into `auth_common`, since `fuzz_tests::config` is
crate-private):

| Target | Input | Property |
|---|---|---|
| `gate_matches_model` | a route of the grid and a set of credentials, each valid, expired, forged, truncated, moved to another place or random bytes | handler ran and user equal to a pure reference model of "which credential is valid for which strategy"; no `5xx`; no panic |
| `session_state_machine` | a sequence of login, logout, request, request with an older cookie, rotate secrets, advance the clock, destroy | a destroyed or replaced identifier never authenticates; after logout no user; the store never holds two live records for one login |
| `oauth2_callback` | random query strings and cookie headers, sometimes derived from a real login start | the provider counter increases only after a valid, unused state; no `5xx` from request data |
| `redirect_targets` | random `return_to` and paths | every `Location` is configured or passes the relative-path rule |
| `canary_secrets` | credentials that are unique marker strings | no marker in captured logs, bodies or headers, except the session cookie's own `Set-Cookie` |

Default case counts are small (the application is built once per target and reused; each case is a few in-process
requests), so `cargo test` stays fast; the CI `fuzz` job and the nightly job run them with their `PROPTEST_CASES`.
CI gets `cargo test -p corvis --features "auth-local auth-api-key auth-session auth-oauth2" --test fuzz_auth` in the
`fuzz` job and the same line in `nightly.yml`, and the invariant and login-throttle binaries in the test jobs.

Alternative: `cargo fuzz` targets. Rejected for this change (proposal Out of scope): it needs nightly Rust and does
not run on Windows runners; the repository's fuzzing is property-based.

### 9. Triage: fixed here or filed

- **Fixed in this change** when all hold: the fix is inside `crates/corvis/src/auth/` or
  `crates/corvis/src/http/signed_cookie.rs`; it changes no requirement of a capability outside the `auth/*` and
  `http/signed-cookies` specs; it fits one session. Each fix comes with the regression test that failed before it.
  If it changes a requirement of an archived auth change, a `MODIFIED` delta for that requirement is added to this
  change first, and the docs of that strategy are updated in English and Spanish.
- **Filed as an issue** otherwise: anything in the router, the WebSocket code, the throttler (beyond `as_any`), the
  ORM, a new feature, or a decision the maintainer must take. The implementer does not create issues; the register
  lists the text of each proposed issue (title, severity, description, interim guidance) and the maintainer's session
  creates them and writes the numbers back.
- **Severity**: critical (authentication bypass without preconditions), high (bypass or account takeover with a
  realistic precondition, secret disclosure), medium (weakened protection, denial of service by an unauthenticated
  client), low (hardening). A deferred high or critical finding is reported to the maintainer before archiving and
  blocks #47, whose guidance page would otherwise document an unsafe pattern.

Already known to be filed, whatever else the review finds: strategies at the WebSocket handshake (the gap between
`JwtService` and the JWT strategy), and aligning or warning on `JwtService` defaults.

### 10. Threat model on the documentation site

`docs/security/authentication-threat-model.md` and `docs/es/security/authentication-threat-model.md`, in the
Security group of `docs/.vitepress/sidebar.mts` (lines 79-89) after "Route posture". Sections: Scope and
assumptions; Assets; Trust boundaries; Attackers; one section per surface with its table (threat, mitigation, test,
residual risk); Defaults; Dependencies; Findings. It is written for a reader who must decide whether to rely on the
module, and it is the source #47's security guidance page summarizes for application authors.

`crates/corvis/tests/auth_source_checks.rs` (no feature gate, no network) reads the English page and checks that
every `crates/corvis/tests/*.rs` and `examples/*/tests/*.rs` path in it exists; it also holds the two source checks
of decision 11.

Alternatives: a `SECURITY.md` or a file under `openspec/` (not published, not translated, and users of an
authentication module should be able to read its threat model); a section of `docs/recipes/passport.md` (too long
already, and #47 consolidates that page).

### 11. Dependency policy

- **Inventory** in the threat model, per feature: `jwt` (`jsonwebtoken`, `ring`), `auth-local` (`argon2`, `blake2`,
  `password-hash`, `bcrypt`, `blowfish`), `auth-api-key` (`subtle`, `sha2`), `auth-session` (`hmac`, `sha2`,
  `subtle`, `rand`/`getrandom`), `auth-oauth2` (`tokio-rustls`, `rustls`, `ring`, `webpki-roots`, `sha2`), with
  versions from `Cargo.lock`, enabled features, role, and maintenance status (last release, RustSec entries).
  `auth_source_checks.rs` parses `Cargo.lock` and fails when the major version of an inventoried crate differs from
  the page (majors are stable enough not to fail on routine updates).
- **`deny.toml`**: `[bans] deny` entries for `openssl`, `openssl-sys`, `native-tls`; and for each inventoried crate
  with one version today, a `deny-multiple-versions` entry (the implementer confirms the syntax for
  `cargo-deny` 0.20.2, the version pinned in `ci.yml:194`). `rand`, `rand_core` and `getrandom` already have two
  generations in the lockfile, so they are listed in the threat model with the reason instead.
- **Advisories**: `deny.toml` already evaluates all features; the task confirms the new auth features are part of
  that graph and that neither ignore list names an inventoried crate.
- **Randomness**: the source check scans `crates/corvis/src/auth/` and `crates/corvis/src/http/signed_cookie.rs`
  for `thread_rng`, `rand::random`, `SmallRng`, `StdRng` and `seed_from_u64` outside `#[cfg(test)]` code and fails
  with file and line. The designs name `rand::rngs::OsRng` (#44 decision 4, #43 decision 4); salts of the hashing
  crates use their own `OsRng`.

Alternative: `cargo vet` or `cargo crev` attestations. Useful, larger than this issue; possible new issue.

## Risks / Trade-offs

- **Everything rests on five unmerged designs.** Names, defaults and even behaviors may differ when this starts.
  Task 1.1 reconciles first and updates specs, scenarios and the scope list before any test is written.
- **Invariant tests need all auth features**, so they do not run in the default `cargo test --workspace`. Mitigated
  by explicit CI steps; a missed step would silently skip them, so task 5.3 checks the CI log for the test names.
- **The gate model can be wrong** in the same way as the code. Mitigated by keeping the model a table lookup written
  from the specs, not from the implementation, and by the hand-written scenarios.
- **The guard-order warning adds noise** for applications that throttle after authenticating on purpose. One line
  per route at startup, with the reason; there is no option to silence it (Open question 3).
- **A per-account limit is a lockout tool** for an attacker. Documented; the block duration is the application's
  choice.
- **Source checks are text scans**: they can be fooled by an alias and can flag a comment. They are a tripwire, not
  a proof; the review reads the real call sites.
- **The review may find something large** (for example that restoring the session user on every route is unsafe by
  design). Then the fix is a design change in #44's territory, filed with high severity, and #47 waits.
- **Two documents close to each other** (this threat model and #47's guidance page) can drift. The guidance page
  links to the threat model for every claim instead of restating the tables.

## Open questions

1. **Where the threat model lives.** This design publishes it on the documentation site in both languages (decision
   10). The alternative is a repository-only file for maintainers. Recommended: the site.
2. **`login_tracker` as new public API in a review change** (decision 7). It is small and closes a real bypass of
   the documented per-account recipe. Recommended: keep it here; the alternative is a follow-up issue plus a
   hand-written tracker in the docs.
3. **Guard-order warning versus error** (decision 6), and whether it needs a way to be silenced. Recommended:
   warning, no switch.
4. **The session user on routes without an `AuthGuard`** (#44 decision 10) is pinned as documented behavior
   (requirement "Routes without an AuthGuard are documented as seeing the session user"). The stricter alternative
   is to restore the user only when the `"session"` strategy runs. Recommended: let the review decide with the test
   application in hand; if it recommends the stricter rule, it is a `MODIFIED` delta of `auth/sessions` in this
   change, since the code is inside the auth module.
5. **An optional token guard clears a session-restored user** (`guard.rs:180-183` with #44's restoring). Pinned as
   an invariant here. Recommended: keep (the route's guard decides identity); confirm.
6. **The spike's task 7.4** ("Security notes" listing every divergence) is split: security-motivated divergences
   appear as mitigations in the threat model (this change); the full list for people migrating is part of #47's
   migration page. Recommended: confirm the split.
7. **Socket.IO handshake**: reviewed here only if `e2-socket-io-*` is merged when this starts; otherwise a row in
   the threat model marked pending plus an issue. Recommended: do not wait for it.
8. **Two issues to create regardless of the review** (decision 9): "Strategies at the WebSocket handshake" and
   "Align or warn on JwtService defaults (secret length, `exp`, issuer and audience)". Recommended: create both
   under epic #39 when this change starts, so the threat model can link them.

## Review notes

These settle the open questions and override the design where they differ.

1. The threat model is a page of the docs site, in English and Spanish.
2. `login_tracker` is added as public API in this change: it closes a real bypass.
3. A throttler placed after `AuthGuard` produces a startup warning, with no switch to silence it.
4. Whether a session-restored user stays visible on routes without an `AuthGuard` is decided by the review itself.
   If the stricter rule is chosen (restore only when the `"session"` strategy runs), it lands here as a `MODIFIED`
   delta of `auth/sessions`; otherwise the behavior is pinned by a test and documented in the security page.
5. An optional token guard clears a session-restored user, as designed.
6. The divergence list of spike task 7.4 is split: security-relevant differences in the threat model, the full list
   in the migration page of #47.
7. The Socket.IO handshake is reviewed only if the `e2-socket-io-*` changes are merged when this change starts;
   otherwise the threat model carries a pending row and an issue is filed.
8. The two follow-up issues ("Strategies at the WebSocket handshake", "Align or warn on JwtService defaults") are
   created under epic #39 when this change starts. The review of #42 already made `JwtService::verify` strict about
   `iss`/`aud`; the second issue covers what remains after that.
9. Implemented after #41, #42, #43, #44 and #45 land; task 1.1 reconciles the design with the merged code.

## Security review

The cross-cutting review of decision 2 ran in two rounds by the `security-reviewer` agent over the implementation, every
suspicion first written as a proof of concept (kept in the maintainer's session notes) and then, for what was fixed, as a
regression test. **No high or critical finding is open, so #47 is not blocked.** One medium finding is outside this
change's reach and is filed (#177); everything else is low or informational.

### Round 1 (scope items 1 to 6)

| Item | Verdict | Notes |
|---|---|---|
| 1 Identity | Holds | the guard that ran last decides the user and the info; `set_user` drops the typed user so `User<T>` never disagrees; a global optional guard clears the restored user. Info: a required guard followed by an optional one that fails runs the handler anonymous while posture counts the route as protected (fails closed) |
| 2 Chains | Holds, three info notes | the JWT key provider being down is a `Fail` and the chain continues (parity with `passport-jwt`, no bypass); API key normalizes validator errors of 500 and above while JWT, local and OAuth2 return them as they are; `Pass` cleared the user but not the auth info of an earlier guard (fixed here) |
| 3 Shared places | Finding, low | a JWT cookie sent twice or planted in front of the victim's read the first value (fixed here); the rest holds (bearer and API key read the first `Authorization` value alike, repeated `X-Api-Key` headers never pick one) |
| 4 Guard order and public routes | Finding, low | a composite guard exposing `[AuthGuard, ThrottlerGuard]` through `inner_guards` got no startup warning and limited nothing (fixed here); a wrapper that hides its guards is a documented limit |
| 5 Session with OAuth2 | Finding, low | an OAuth2 flow whose state is in a cookie survived a logout and another user's login (fixed here); with `state_in_session()` the same sequence is refused. Info: a session cookie named like a state cookie is accepted at startup (configuration error) |
| 6 Cookies on failure paths | Holds, two info notes | cookies a strategy appends are sent on a later `4xx` (Express parity; no built-in strategy appends authority); the row about failed logins creating no session holds only with `save_uninitialized(false)` (now said in the threat model) |

### Round 2 (scope items 7 to 12)

| Item | Verdict | Notes |
|---|---|---|
| 7 CSRF posture | Holds as documented, three info notes | four forced OAuth2 starts cancel a victim's pending flow; login CSRF through an urlencoded form works without an origin check, and `SameOriginGuard` exists only in the example (a candidate for the framework); a cookie-borne JWT or a session cookie without `SameSite` authenticates cross-site `POST`s in browsers that do not default to `Lax` |
| 8 Sockets | Finding, two low (Socket.IO layer), the gateway side holds | a connect middleware on the main namespace does not cover the others, and `Debug` of `Handshake` prints the `auth` payload: filed as #178; a session never reaches an upgrade or a handshake; `apply_global_enhancers_to_gateways(false)` also drops a global throttler from gateways (info) |
| 9 Defaults | Finding, low | the API key validator had a deadline and the JWT key provider, JWT validator and local validator had none (fixed here); startup warnings are asymmetric across strategies (info); length limits are capped only for API keys (info) |
| 10 Secrets in output | Redaction holds; finding, medium | `CacheInterceptor` registered globally replays one user's `GET` response to others (filed as #177, cache module); a handler that formats a credential into a panic or error leaks it, as in the original |
| 11 Resource use | Holds, two info notes | a throttler in front stops every strategy cost; a request that carries a session cookie costs two store calls and a user lookup even on a `404` or a `429`; an anonymous OAuth2 start with the state in the session creates a record (rate limit the route) |
| 12 Feature combinations | Holds | every feature gate is an API-presence gate; the one place a feature decided a security property indirectly was the duplicate cookie scan, now available without `auth-session` |

### Findings fixed in this change

| Id | Severity | What | Where | Test |
|---|---|---|---|---|
| R-7 | Low | a JWT cookie sent twice authenticated with the first one; no warning for a cookie name without `__Host-` | `auth/jwt/extract.rs`, `auth/jwt/options.rs`, `http/request.rs` (`cookie_values` available with `jwt` or `auth-session`) | `a_jwt_cookie_sent_twice_is_no_token`, `auth_jwt_log.rs` |
| R-8 | Low | the guard-order warning missed composite guards | `auth/mod.rs` | `auth_guard_order_log.rs` |
| R-9 | Low | a cookie-state OAuth2 flow survived logout and another user's login | `auth/oauth2/state.rs`, `auth/oauth2/strategy.rs` | `oauth2_cookie_state_is_bound_to_the_session_it_started_under` |
| R-10 | Low | no deadline for the JWT key provider, JWT validator and local validator | `auth/jwt/`, `auth/local/strategy.rs` | `auth_callback_timeouts.rs` (paused clock) |
| R-11 | Info | a guard that passed kept the auth info of an earlier guard | `auth/guard.rs` | `a_later_pass_clears_the_auth_info_of_an_earlier_guard_too` |

### Issues filed

- #177 (medium): `CacheInterceptor` registered globally serves one user's response to others on authenticated `GET` routes.
  Interim guidance: do not register it for the whole application when it has authenticated `GET` routes (threat model
  warning box and `docs/data/caching.md`, English and Spanish).
- #178 (low): Socket.IO connect middleware covers the main namespace only, and `Debug` of `Handshake` prints the `auth`
  payload. Interim guidance: register the middleware on every namespace, never format a handshake with `{:?}` (threat model rows).
- #173 (strategies at the WebSocket handshake) and #174 (`JwtService` defaults) were already open and are referenced in the
  threat model; #172 (redirect URI allow-list) and #170, #171 (Redis and ORM session stores) too.

### Known and accepted

Each is on the threat model page or in the strategy docs; none is a way in.

- The JWT key provider being down is a `Fail` (the chain continues): parity with the original, and the next strategy still
  needs its own valid credential.
- API key normalizes validator errors of 500 and above to a fixed body; JWT, local and OAuth2 return them as the validator
  wrote them: documented per strategy ("passed on as it is"), and a validator controls its own message.
- Four forced OAuth2 starts cancel a victim's pending flow: a nuisance, the victim retries; the limit of four flows per
  browser is what keeps the cookie small.
- Login CSRF through an urlencoded form works without an origin check; `SameOriginGuard` exists only in the example and
  is a candidate to move into `corvis::auth` (a point for #47, not a change of this one).
- A cookie-borne JWT without `SameSite` authenticates cross-site `POST`s in browsers that do not default to `Lax`: the
  docs say to set `SameSite`.
- A session cookie costs two store calls and a user lookup even on a `404` or a `429`, because the session middleware runs
  before every guard: only a limiter ahead of `SessionModule` shields the store (same as `express-session`).
- An anonymous OAuth2 start with the state in the session creates a record: rate limit the route (documented).
- Startup warnings are asymmetric across strategies (for example `from_body_field` and `without_same_site()` are silent):
  each is a deliberate choice, and the threat model's "known differences" paragraph says so.
- A strategy that appends a cookie with `append_response_header` has it sent on a later `4xx`/`5xx`: no built-in strategy
  does; a custom one must log in through the session.

### Re-review

Task 7.5 (a third `security-reviewer` run over the fixes only) is left to the maintainer's session.

## Implementation notes

Task 1.1 compared this design with the merged code (`dev` at `62df145`). Adjustments, all recorded here:

1. **The strategies are merged and match the design's names**; the differences that mattered:
   - The mock OAuth2 provider and the recording session store already live in `tests/oauth2_common/` and
     `tests/session_common/`, so `tests/auth_common/mod.rs` reuses them (the binaries declare `mod oauth2_common;
     mod session_common;` next to `mod auth_common;`) instead of moving anything.
   - The session user is kept in a private `SessionUser` extension next to the request's user, so `clear_user()` by an
     optional guard does not affect the `"session"` strategy. `log_in()` is `AuthGuard::log_in` and the pipeline
     rejection mark decides if it is kept (earlier reviews).
   - The OAuth2 state in the session is an `oauth2`-named key of the record; a forged callback in cookie mode sends the
     browser's own state cookie back unchanged (it does not delete it). The scenario "Failed OAuth2 callback" now says so.
   - Named throttlers answer `Retry-After-<name>` and `X-RateLimit-*-<name>`; the password guessing scenario says so.
   - The key derivation counter is the call counter of the local validator: each call derives one key
     (`verify_or_dummy` verifies a dummy hash for an unknown user), and no hasher accessor was added.
   - The OAuth2 grant cannot be built from outside, so its `Debug` is formatted inside the validator of the test
     application (`Probe::grants`).
2. **The defaults test stays an integration test** (`tests/auth_defaults.rs`) and reads values through public constants,
   `Debug` of the default options and behavior (renewal, data kept at login, the state cookie, credentials in a URL),
   so no accessor was added to a public type. The dependency inventory is parsed from the page by
   `tests/auth_source_checks.rs`, which also checks `deny.toml` and runs the randomness scan.
3. **`cargo deny`**: `deny-multiple-versions = true` inside `[bans] deny` entries works with cargo-deny 0.20.2 and does not
   ban the crate itself (checked with `getrandom`, which has three versions). `webpki-roots` 0.26.11 is a shim of 1.0.11, so
   it is documented and not in the list; `rand` has one version in the graph (0.10 comes from `proptest`).
4. **Decision on the session user (Review note 4): keep, pin with a test, document.** Reasons are on the threat model
   page: it adds no authority (same cookie, same user as `AuthGuard::new("session")`, and every `AuthGuard` replaces or
   clears it), parity with `passport.session()` is the point of the module, and the stricter rule does not remove the real
   risk (a hand-written guard that trusts `Request::user()`), it only turns it into a deny that depends on guard order. No
   `MODIFIED` delta was needed. Test: `roles_guard_without_an_auth_guard`.
5. **Findings.** Fixed here: `Debug` of `JwtModuleOptions` and `JwtSignOptions` printed the HMAC secret and PEM private key
   (Medium; `crates/corvis/src/jwt.rs`, inside the JWT module, no requirement changed; test
   `debug_of_every_public_type`). Pinned and documented: an OAuth2 strategy ends a chain with a redirect; the API key bearer
   reader is stricter than the token reader on more than 640 bytes of leading whitespace. Nothing high or critical was
   found, so nothing blocks #47.
6. **The two follow-up issues of decision 9** exist: #173 (strategies at the WebSocket handshake) and #174 (`JwtService`
   defaults); #170, #171 and #172 are linked in the threat model too. Socket.IO (#33) was not merged when this change started and its row was pending; it is merged now and
   was reviewed in round 2 (its row says what was found, #178). Engine.IO is dispatched before the router (`server.rs`),
   so its handshake has no session.
7. **Sockets**: "Global AuthGuard and a gateway" is tested again in `auth_invariants.rs` next to `tests/auth_ws.rs`, with an
   optional global guard so the HTTP routes keep working.
8. **Always-compiled files touched**: `src/auth/mod.rs` (the guard-order warning), `src/modules/throttler.rs` (`as_any`,
   `ipv6_subnet_prefix`), `src/jwt.rs` (`Debug`); behind `auth-local`: `src/auth/local/strategy.rs` (`login_tracker`, the
   shared username lookup).
9. **Left to the maintainer's session**: the corvis-parity check (4.4, 9.6), an external `security-reviewer` run (7.1, 7.2,
   7.5: the implementer reviewed the scope list himself, turning each suspicion into a test; the two rounds of 7.1 and 7.2
   ran afterwards, see "Security review"), filing issues (7.4: #177 and #178 are filed) and the check of the CI log in the
   pull request (5.3).
10. **The review fixes** (R-7 to R-11, "Security review"). Decisions: (a) the duplicate cookie scan (`Request::cookie_values`)
    is no longer gated on `auth-session`: it is compiled with `jwt` or `auth-session`, so a `jwt`-only build refuses a
    duplicated cookie too and no other build changes; (b) the guard-order warning flattens `inner_guards` in execution order
    (an `AuthGuard` kind, a `ThrottlerGuard` kind, depth capped by `MAX_GUARD_DEPTH`) and warns when any `AuthGuard`
    precedes a `ThrottlerGuard`; (c) the OAuth2 binding is a SHA-256 of an HMAC (state cookie key) over the strategy, the
    state and the session identifier, stored per flow as `b` in the signed cookie record, with `-` for "no session"; a new
    session counts as "no session" unless `save_uninitialized` is on, because only then does the start request persist it;
    the binding is not used with `state_in_session()` (the flow lives in the session); (d) `key_provider_timeout` and
    `validator_timeout` of `JwtStrategyOptions` and `validator_timeout` of `LocalStrategyOptions` are 5 s by default, 1 ms to
    300 s like the API key one, answer `HttpException::new((), 503)` and share the rate-limited warning helper
    (`WarnLimiter`); (e) a guard that passes clears the auth info in `AuthGuard::can_activate`, not in the authenticator,
    whose unit test keeps the info untouched when the chain itself passes.
11. **Always-compiled files touched by the fixes**: `src/auth/mod.rs` (the warning), `src/auth/guard.rs` (`Pass`),
    `src/http/request.rs` (the `cfg` of `cookie_values`) and `src/auth/jwt/` and `src/auth/local/` behind their features.
    An application that enables none of the auth features compiles the same code as before.

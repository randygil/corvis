# Proposal

Implements [randygil/corvis#44](https://github.com/randygil/corvis/issues/44) (epic
[#39](https://github.com/randygil/corvis/issues/39)). Design from the #40 spike
(`openspec/changes/archive/2026-11-08-e3-strategy-module-design/`: fact 7, decisions 8 and 9, the `auth/sessions`
delta, task group 5, Review note 8: `resave` and `save_uninitialized` default to `false`).

**Depends on #136** (`openspec/changes/e3-strategy-core-and-guard-responses/`): this change is written against the API
#136 proposes (`corvis::auth::Strategy`, `StrategyOutcome`, `AuthenticatedUser`, `Authenticator`, `AuthGuard`,
`Provider::strategy`, the startup check of strategy names, the fixture package
`crates/corvis/tests/fixtures/auth/`). #136 is being implemented in another worktree while this is drafted, so that
API is described here as designed, not as shipped. By the epic's order this change is implemented after #136, #42,
#41 and #45; if the API changes, this change is updated before work starts. #136 left two things to this change:
`AuthGuard::log_in()` and the requirement that `AuthGuard` does not log in to a session by itself.

## Why

Server-rendered apps and many APIs behind a browser keep the login in a server-side session: `express-session` with
`passport.session()`, `req.logIn()` after a local login, a signed `connect.sid` cookie. Corvis has none of it:

- `docs/http/session.md` is `status: unavailable` ("Corvis has no server-side sessions") and tells users to build
  their own with a random identifier in a cookie and a map.
- `docs/http/cookies.md:32-33`: "There are no signed cookies (`signedCookies`)". `Request::cookies`
  (`crates/corvis/src/http/request.rs:223-230`) parses like cookie-parser and `Res::cookie`
  (`crates/corvis/src/http/res.rs:729-731`) writes, with no signing.
- `docs/security/csrf.md:6-7` rests on the same absence.

A hand-written session is where the classic mistakes live: a guessable or reused identifier, a session that survives
the login (fixation) or the logout, a cookie readable by scripts or sent cross-site, a signature compared with `==`,
a secret that cannot be rotated, sessions that never expire, and a memory store that grows without bound (the
original's `MemoryStore` is documented as leaking). The framework should ship the session with those closed by
default, and keep the cookie and record formats of the original so that a Node app and a Corvis app can read each
other's sessions during a migration. OAuth2 (#43) also needs it: its `state` lives in the session or in a signed
cookie.

## What Changes

- New cargo feature `auth-session` (off by default; optional dependencies `hmac` and `subtle`, both already in the
  lockfile).
- Signed cookies (`http/signed-cookies`): `CookieKeys` (1 to 8 secrets, each at least 32 bytes, else an error at
  construction), `sign` / `unsign` in the format of `cookie-signature` 1.0.7 with the `s:` prefix of cookie-parser and
  express-session (HMAC-SHA256, base64 without padding), the first secret signs and every secret verifies (rotation),
  constant-time verification, and `Request::signed_cookie(name, &keys)`.
- Sessions (`auth/sessions`), in `corvis::auth::session`:
  - `SessionModule::for_root(SessionOptions)` / `for_root_async`: a middleware that reads the session cookie
    (`connect.sid` by default, express-session 1.19.0 value format), loads the session before the guards, exposes it
    as `Request::session()`, and after the handler writes the store and the `Set-Cookie` header with the saving rules
    of express-session (`resave`, `save_uninitialized`, `rolling`).
  - `Session`: `id`, `get`, `set`, `remove`, `clear`, `regenerate`, `destroy`, `is_new`.
  - Session identifiers: 24 bytes from the operating system's random generator, base64url (the shape of
    express-session's `uid-safe` identifiers). An identifier the store does not know is never adopted.
  - `SessionStore` trait (`load`, `create`, `update`, `touch`, `destroy`) and `MemorySessionStore`, bounded by number
    of sessions and total bytes, with expiry and least-recently-used eviction. `update` never re-creates a destroyed
    session.
  - Cookie attributes with secure defaults: `HttpOnly`, `SameSite=Lax`, `Secure` (configurable; all three are
    documented divergences from the original's defaults), `Path=/`, no `Domain`; startup errors for `SameSite=None`
    without `Secure` and for `__Host-` / `__Secure-` names whose attributes do not match the prefix.
  - Idle timeout (default 24 hours) and absolute timeout (default 7 days), enforced by the middleware on load
    whatever the store does.
  - Session size limit (default 16 KiB serialized).
- Login through a session: `SessionSerializer` (`serialize_user` / `deserialize_user`) wrapped by
  `SessionStrategy<S>`, registered with `Provider::strategy` under the name `"session"`; `Authenticator::log_in` /
  `log_out`, which regenerate the session identifier and destroy the old session (fixation), storing the serialized
  user under `passport.user` as passport 0.7.0 does; the session user restored before the guards;
  `AuthGuard::new("local").log_in()` for the login route; a startup error when `.log_in()` is used without a
  registered session strategy.
- Differential tests against `express-session` 1.19.0, `cookie-signature` 1.0.7, `cookie-parser` and `passport`
  0.7.0: `session.mjs` and `signed_cookies.mjs` in the fixture package of #136 record cookies, responses and store
  contents; `crates/corvis/tests/auth_session.rs` and `signed_cookies.rs` replay them with no Node at test time.
  Divergences are marked.
- Docs (EN + ES): `docs/http/session.md` becomes `documented`; signed cookies in `docs/http/cookies.md`; the session
  section of `docs/recipes/passport.md`; `docs/security/csrf.md` updated (sessions exist, `SameSite=Lax` by default,
  tokens still not provided). Example: `examples/cats-app` gets a session login, a protected route and a logout.

## Capabilities

### New Capabilities

- `http/signed-cookies`: cookie signing and verification, key rotation, secret strength, limits, differential
  verification.
- `auth/sessions`: session cookie, identifiers, saving rules, cookie attributes, timeouts, store trait and memory
  store, login and logout with regeneration, session user, startup checks, differential verification.

### Modified Capabilities

None. `auth/strategies` (#136) is extended only through its own extension points (a strategy named `"session"`,
`AuthGuard::log_in()` specified here). `http/cookies` (`openspec/specs/http/cookies/spec.md`, the `Max-Age` unit) is
unchanged.

## Impact

- `crates/corvis/Cargo.toml`: optional `hmac = "0.12"` (0.12.1 in `Cargo.lock`, same `digest` 0.10 generation as
  `sha2 = "0.10"`, line 45) and `subtle = "2"` (2.6.1 in `Cargo.lock`; shared with #45 if it landed); feature
  `auth-session = ["dep:hmac", "dep:subtle"]`. `rand = "0.8"` (line 43) and `base64 = "0.22"` (line 39) are already
  dependencies.
- New `crates/corvis/src/http/signed_cookie.rs` and `crates/corvis/src/auth/session/` (`mod.rs`, `options.rs`,
  `middleware.rs`, `session.rs`, `store.rs`, `memory.rs`, `login.rs`), all `#[cfg(feature = "auth-session")]`.
- `crates/corvis/src/http/request.rs`: `signed_cookie` and `session` under the feature.
- `crates/corvis/src/auth/` (module of #136): `AuthGuard::log_in()`, `Authenticator::log_in` / `log_out` under the
  feature; the startup check of #136 also requires the `"session"` strategy for guards with `.log_in()`.
- `.github/workflows/ci.yml`: `auth-session` in the per-feature clippy loop (line 72), a
  `cargo test -p corvis --features auth-session` step next to the `compression` one (line 114), and the fuzz job
  (line 165) with the feature.
- `crates/corvis/tests/fixtures/auth/`: `express-session` 1.19.0, `cookie-parser` and `passport` (already there)
  pinned in `package.json` and the lockfile; `session.mjs`, `session.json`, `signed_cookies.mjs`,
  `signed_cookies.json`.
- Tests: `crates/corvis/tests/auth_session.rs`, `crates/corvis/tests/signed_cookies.rs`, unit tests in the new
  modules, property tests in `crates/corvis/src/fuzz_tests.rs`.
- Docs: `docs/http/session.md`, `docs/http/cookies.md`, `docs/security/csrf.md`, `docs/recipes/passport.md`,
  `docs/security/authentication.md` and their `docs/es/` copies.
- Example: `examples/cats-app`.
- Port apps: none enables the feature and no code outside it changes behavior; the corvis-parity check runs if the
  implementation touches anything that is compiled without the feature.

## Out of scope

- **Redis and ORM session stores.** The issue text names them. This change ships the `SessionStore` trait, the memory
  store and the record format of express-session (so a store can share sessions with a Node app), and leaves both
  stores to new issues: there is no Redis client in the workspace yet (it arrives with the queues epic), and an ORM
  store belongs in `corvis-orm`, which depends on `corvis` and not the other way round
  (`crates/corvis-orm/Cargo.toml:24`). See Open question 1.
- `Secure` decided per request ("auto": only over TLS or behind a trusted proxy that reports HTTPS): `Request` does
  not know its transport or `X-Forwarded-Proto` today. Possible new issue (`Request::protocol()`), after which an
  automatic mode can be added.
- CSRF tokens (`docs/security/csrf.md` stays without a token middleware): possible new issue now that sessions exist.
- Flash messages, `failureFlash`, `successFlash`, `failureMessage` (spike non-goals).
- Sessions in WebSocket gateways: upgrade requests do not run HTTP middleware
  (`crates/corvis/src/server.rs:560-567`); reading the session in `handle_connection` is a possible new issue.
- Stateless sessions that keep the data in an encrypted cookie, and encryption of signed cookie values.
- The express-session options `genid`, `unset`, `proxy`, `store.all` / `clear` / `length`, and the `req.session.cookie`
  object editable per request; listing a user's sessions and "log out everywhere"; "remember me"; limits on
  concurrent sessions per user. Possible new issues.
- Restricting the session middleware to some routes: it applies to every route, as `app.use(session())` does; a
  request without the cookie costs no store access.
- Storing OAuth2 `state` (#43 builds on this change), the authentication security review (#46), the overall
  authentication docs and example (#47).
- `AuthGuard` on WebSocket gateways (fails closed since #136).

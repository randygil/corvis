# Design

## Context

This change builds on #136 (`openspec/changes/e3-strategy-core-and-guard-responses/`), which adds `corvis::auth` with
`Strategy`, `StrategyOutcome`, `AuthenticatedUser`, `Authenticator`, `AuthGuard`, `Provider::strategy::<S>()`, the
startup check of strategy names (`CanActivate::as_any`, its decision 8) and the fixture package
`crates/corvis/tests/fixtures/auth/`. #136 is being implemented in another worktree while this is drafted: its API is
described as designed (its `design.md` and Review notes), not as shipped. The adapter shape (`XStrategy<V>` resolving
a user type through DI) follows #42 and #41. Line numbers were checked on `origin/dev` at `6f58ee1`.

### Corvis today

- **Reading cookies.** `Request::cookies` / `cookie` (`crates/corvis/src/http/request.rs:223-230`) join every `Cookie`
  header as latin1 and parse like `cookie.parse` of cookie 0.7 (`parse_cookies`, `request.rs:262-311`): first
  occurrence of a name wins, quotes removed, a value with `%` is URI-decoded. So `connect.sid=s%3Aabc.sig` is read as
  `s:abc.sig`, as express-session reads it.
- **Writing cookies.** `serialize_cookie` (`crates/corvis/src/http/res.rs:246-302`) is `cookie.serialize` with the
  defaults of `res.cookie`: the value goes through `encode_uri_component` (`res.rs:312`), `Path=/` by default,
  `max_age` writes both `Max-Age` and `Expires`, and an invalid name, value, domain, path, priority or `SameSite`
  **panics** (as the package throws). The validators are `valid_cookie_name` (209), `valid_cookie_domain` (227) and
  `valid_cookie_path` (239). `Res::cookie` (`res.rs:729-731`) and `Response::with_cookie` / `set_cookie`
  (`crates/corvis/src/http/response.rs:168-174`) append a `Set-Cookie` header.
- **No signing, no session.** `docs/http/cookies.md:32-33` and `docs/http/session.md` (status `unavailable`) say so;
  a search for `signed` in `crates/corvis/src` finds only JWT tests.
- **Middleware.** `CorvisMiddleware::use_(req, next) -> HandlerResult` (`crates/corvis/src/common/middleware.rs:29-31`)
  runs before the guards and receives the response back from `next.run(req)` (42-55). In `dispatch`
  (`crates/corvis/src/router/mod.rs:580-634`): a CORS preflight is answered before any middleware (583-587); the chain
  is the global middleware followed by the module middleware that matches the path (601-606); the endpoint is
  `route_request`, whose `execute_route` already turned handler exceptions and handler panics into a `Response`
  (`router/mod.rs:696-740`, `catch_unwind` at 701), so the middleware sees `Ok(response)` for them. It sees `Err` only
  when no route matched or a parameter could not be decoded (`router/mod.rs:652-673`) or when an inner middleware
  returned an error; those go to the global filters after the chain (625-630). `HttpException` carries no headers.
- **Installing middleware from a module.** `ModuleDefinition::configure` (`crates/corvis/src/di/module.rs:30` and
  85-88) takes a `fn(&Instance, &mut MiddlewareConsumer)`; `build_middleware` (`router/mod.rs:496`) runs it for every
  module and resolves `consumer.apply::<M>()` middleware through DI (`middleware.rs:164`). `DynamicModule`
  (`di/module.rs:111-118`) has no `configure` of its own but keeps its base module's definition, so a library module
  can install a middleware. `app.use_middleware(instance)` adds a global one (`crates/corvis/src/application.rs:244-247`).
- **Request extensions** shared by clones: `Request::insert` / `get` / `remove` (`request.rs:233-242`); `set_user`
  stores the JSON user (245-249).
- **Module with a bounded memory store as a precedent.** `ThrottlerModule::for_root` / `for_root_async`
  (`crates/corvis/src/modules/throttler.rs:948-963`) provide a global storage value; `ThrottlerStorage` is bounded by
  `DEFAULT_THROTTLER_MAX_ENTRIES = 100_000` (line 315) with amortized purging and eviction, no background task.
- **Transport.** The application knows whether it listens with TLS (`application.rs:687`, `local_secure`), but a
  `Request` has no `protocol()` / `secure()` and no reading of `X-Forwarded-Proto` (a search for `x-forwarded-proto`
  in `crates/corvis/src` finds nothing).
- **WebSocket upgrades** are answered before `dispatch` (`crates/corvis/src/server.rs:560-567`), so HTTP middleware
  does not run for them.
- **Dependencies.** `rand = "0.8"`, `sha2 = "0.10"`, `base64 = "0.22"` (`crates/corvis/Cargo.toml:43`, 45, 39).
  `hmac` 0.12.1 and `subtle` 2.6.1 are in `Cargo.lock` as transitive dependencies. Random values in the crate use
  `rand::random` (for example upload file names, `crates/corvis/src/multer/storage.rs:126`).
- **No Redis client** in any `Cargo.toml` of the workspace; `corvis-orm` depends on `corvis` optionally
  (`crates/corvis-orm/Cargo.toml:18` and 24), not the reverse.

### The original

Facts from the spike (its fact 7 and versions list) and from the packages' documented behavior; the implementer
re-reads the sources when writing the generators and corrects this section if they differ.

- `cookie-signature` 1.0.7: `sign(val, secret)` is `val + '.' + base64(HMAC-SHA256(val, secret))` with `=` removed;
  `unsign(input, secret)` signs the part before the last `.` again and compares the two strings with
  `crypto.timingSafeEqual` after a length check.
- `express-session` 1.19.0 (`index.js`): cookie name `connect.sid`; identifier from `uid-safe` with 24 bytes
  (32 base64url characters); cookie value `s:` + signed identifier, written with `cookie.serialize` (so `s%3A...`);
  `secret` may be an array: the first signs, all verify; a cookie without the `s:` prefix or with a bad signature is
  ignored; a signed identifier the store does not have starts a new session with a new identifier; cookie defaults
  `{ path: '/', httpOnly: true, secure: false, maxAge: null }`; with `maxAge` the cookie carries `Expires`; a `secure`
  cookie is not sent over a connection the module does not consider secure; `resave` and `saveUninitialized` default
  to `true` with a deprecation warning; the session is "modified" when its JSON without the `cookie` key changed;
  `Set-Cookie` is sent for a new identifier when `saveUninitialized` or modified, and for an existing one when
  `rolling`, or when the cookie has an expiry and the session was modified; an existing unmodified session is
  `touch`ed; the store record is the session JSON with a `cookie` object (`originalMaxAge`, `expires`, `httpOnly`,
  `path`, and `secure`, `domain`, `sameSite` when set); `store.set` creates or overwrites; `MemoryStore` has no size
  bound and expires a record only through `cookie.expires`.
- `passport` 0.7.0 (`lib/sessionmanager.js`, `lib/strategies/session.js`): `req.logIn` regenerates the session, then
  stores the serialized user under `req.session.passport.user` and saves; session data from before the login is
  dropped unless `keepSessionInfo`; `req.logOut` removes the user, saves, and regenerates; the `session` strategy
  deserializes `passport.user` on each request, removes it when the user no longer exists, and always passes.
- `@nestjs/passport` 12.0.1: `AuthGuard` never logs in by itself (spike fact 1); the documented login guard calls
  `super.canActivate(context)` and then `super.logIn(request)`.

## Goals / Non-Goals

**Goals**

- Sessions whose cookie and record a Node app with express-session can read, and the reverse, with the same secret
  and store contents.
- Closed by default: unguessable identifiers, regeneration on login and logout, a destroyed session cannot come back,
  `HttpOnly` + `SameSite=Lax` + `Secure`, constant-time signatures, rotation, idle and absolute expiry, bounded memory.
- Signed cookies as a general HTTP feature, reused by #43.
- Every limit has a default, a bound and a startup check.

**Non-Goals**

- Redis and ORM stores, per-request `Secure`, CSRF tokens, WebSocket sessions (see proposal Out of scope).
- Matching the original where it is unsafe: its defaults for `SameSite`, `Secure`, `resave`, `saveUninitialized`, its
  unbounded store, its upsert on save.

## Decisions

### 1. One feature, `auth-session`

`auth-session = ["dep:hmac", "dep:subtle"]`, off by default, for signed cookies and sessions (the epic's convention:
one feature per strategy with dependencies; #41 Review note 3 chose one feature for the local strategy and its
hasher). `hmac` 0.12 shares `digest` 0.10 with the `sha2` already in the tree (spike crates table). Signed cookies
live in `corvis::http` (`CookieKeys`, `Request::signed_cookie`), sessions in `corvis::auth::session`.

Alternative: a separate `signed-cookies` feature. Rejected: two features for one small dependency; #43 enables
`auth-session` or depends on it.

### 2. Signed cookies

```rust
let keys = CookieKeys::new([config.get("COOKIE_SECRET")?, config.get("COOKIE_SECRET_OLD")?])?;
let signed: String = keys.sign("value");                 // "s:value.<43 base64 characters>"
let value: Option<String> = keys.unsign(&signed);        // None unless it verifies with one of the secrets
let value: Option<String> = req.signed_cookie("prefs", &keys);
res.cookie("prefs", &keys.sign("dark"), CookieOptions::new().http_only(true));
```

- `CookieKeys::new` takes 1 to 8 secrets; each must have at least 32 bytes (the epic's rule; RFC 2104 asks for a key
  of at least the hash output size). Fewer, more, or a short secret is a `CookieKeysError` whose message has the
  index and the length, never the secret. `Debug` is redacted.
- `sign` uses the first secret. `unsign` requires the `s:` prefix, refuses inputs over 4096 bytes before any HMAC,
  splits at the last `.`, computes the signature with **every** secret (no early exit, so timing does not tell which
  secret matched) and compares the signature strings with `subtle::ConstantTimeEq`. Comparing the re-encoded string,
  as the original does, accepts exactly the strings the original accepts (no base64 leniency).
- `CookieKeys::verified_with_current(&self, signed) -> bool` tells whether the first secret verifies, for re-signing
  after a rotation (decision 6).
- `Request::signed_cookie(name, keys)` is `self.cookie(name).and_then(|v| keys.unsign(&v))`: the equivalent of
  cookie-parser's `req.signedCookies[name]` for string values. `j:` JSON cookies are not decoded.

Rotation procedure (docs): put the new secret first, keep the old one until the longest cookie lifetime has passed
(for sessions, the absolute timeout), then remove it.

### 3. Session options and module

```rust
SessionModule::for_root(
    SessionOptions::new(CookieKeys::new([secret])?)
        .name("connect.sid")
        .cookie(SessionCookie::default()            // Path=/, HttpOnly, SameSite=Lax, Secure
            .secure(false)                          // plain-HTTP development; startup warning
            .same_site(SameSite::Strict)
            .max_age(Duration::from_secs(14 * 24 * 3600))
            .domain("example.com").path("/"))
        .idle_timeout(Duration::from_secs(24 * 3600))
        .absolute_timeout(Duration::from_secs(7 * 24 * 3600))
        .rolling(false).resave(false).save_uninitialized(false)
        .keep_session_info_on_login(false)
        .max_session_bytes(16 * 1024)
        .store(MemorySessionStore::new().max_sessions(100_000).max_total_bytes(256 << 20)),
)
SessionModule::for_root_async(|config: ConfigService| async move { Ok(SessionOptions::new(..)) })
```

`SessionModule` is a global dynamic module (the `ThrottlerModule` pattern) that provides `SessionManager` (options,
keys, store) and installs `SessionMiddleware` for every route through its definition's `configure`. Options are
validated when `SessionManager` is built, so an invalid configuration stops `init()`. Module middleware runs after
global middleware: a global middleware registered with `app.use_middleware` does not see the session; the docs say
so, and `SessionMiddleware` is public so it can be registered globally instead.

### 4. Session identifiers

24 bytes from `rand::rngs::OsRng` (the operating system's generator, not the thread-local one), encoded base64url
without padding: 32 characters, 192 bits, the shape of the original's `uid-safe(24)`. A failure of the generator is a
500; there is no fallback. Incoming identifiers (after the signature check) must be 1 to 128 bytes of visible ASCII;
longer or other bytes are treated as no cookie. An identifier that verifies but is not in the store never becomes the
identifier of a new session: a new one is generated (as the original; this is what makes a planted identifier
useless even to someone who knows a retired secret). `Session`'s `Debug` prints neither the identifier nor the data,
and the framework never logs an identifier.

Store keys are the raw identifiers, as in the original, so stores can be shared with a Node app. Hashing identifiers
in the store would protect against a store dump but break that sharing; noted as a risk.

### 5. Request flow

`SessionMiddleware::use_`:

1. Read the cookie (`Request::cookie(name)`), `unsign` it. No cookie, no `s:` prefix, bad signature, invalid
   identifier → no session loaded, no store access, no error.
2. `store.load(id)`. A store error is a 500 (as the original's `next(err)`), logged without the identifier. A missing
   record → new session. A record past its idle or absolute timeout (decision 8) → `store.destroy(id)` and new
   session.
3. Insert the `Session` handle in the request extensions. If the record has `passport.user` and a session strategy is
   registered, deserialize the user (decision 10).
4. `next.run(req)`.
5. Commit (decision 6) and add `Set-Cookie` to the response with `Response` header appending; the middleware owns the
   response here, so the guard-side `append_response_header` of #136 is not involved.

`Session` is a cheap clone of an `Arc<Mutex<..>>` with synchronous methods: `id()`, `is_new()`, `get::<T>(key)`,
`get_value(key)`, `set(key, value) -> Result<(), SessionError>` (serialization failure, reserved key), `remove(key)`,
`clear()`, `regenerate()` (new identifier now, old one queued for destruction, data kept), `destroy()`. All store
access happens in the middleware before and after the handler, so handlers, guards and strategies need no `await` to
use the session. `Request::session() -> Option<Session>` is `None` when the middleware did not run.

On the `Err` path of `next.run` (no route matched, or an inner middleware failed) no cookie can be attached, because
the response is built later by the global filters: queued destructions are still applied, an existing session is
updated or touched, and a new or regenerated identifier is discarded (the client ends without a session). Guards and
handlers do not run on that path, so only other middleware can have touched the session.

### 6. Saving rules

Same decisions as express-session 1.19.0, on the same inputs (`cookie_id`: the identifier that arrived and loaded;
"modified": the data map differs from the snapshot taken at load, or the identifier changed):

| | New identifier | Existing identifier |
|---|---|---|
| Store write | `create` when modified or `save_uninitialized` | `update` when modified or `resave`; otherwise `touch` |
| `Set-Cookie` | when modified or `save_uninitialized` | when `rolling`, or when the cookie has `max_age` and the session was modified |

Corvis additions, each marked as a divergence in the fixture:

- Defaults `resave = false`, `save_uninitialized = false` (spike Review note 8): an anonymous visit creates nothing.
- `update` and `touch` do not create: if the record is gone (destroyed by a logout in a concurrent request, expired,
  evicted) they return `false`, nothing is written and no cookie is sent. The original's `store.set` would re-create
  the record, which lets a slow request that started before a logout bring the logged-in session back.
- Re-signing: when the incoming cookie verified only with a retired secret, `Set-Cookie` is sent for the existing
  session so the cookie moves to the current secret.
- `Secure` cookies are sent whatever the transport (the original withholds them on plain HTTP); `Request` cannot tell
  (Context).
- A commit that fails (store error, session over `max_session_bytes`, identifier collision after one retry) replaces
  the response with a 500 through the normal exception path and sends no session cookie. The handler's side effects
  have already happened; the docs say so.

The write is awaited before the response is returned, so a client never holds a cookie for a session the store does
not have yet.

### 7. Cookie attributes

| Attribute | Default | Original default | Startup check |
|---|---|---|---|
| Name | `connect.sid` | same | valid cookie name; `__Host-` needs `Secure`, `Path=/`, no `Domain`; `__Secure-` needs `Secure` |
| `Path` | `/` | same | valid cookie path |
| `HttpOnly` | on | on | `http_only(false)` logs a warning |
| `SameSite` | `Lax` | not sent | `None` without `Secure` is an error |
| `Secure` | on | off | `secure(false)` logs a warning |
| `Max-Age` / `Expires` | none (browser-session cookie) | same | at most the absolute timeout |
| `Domain` | none | same | valid cookie domain |

The cookie is written with `serialize_cookie`; name, path and domain are validated at startup with its own
validators, and the value is `s:` + identifier + `.` + base64 signature, which always encodes to a valid value, so
the panics of `serialize_cookie` cannot be reached at request time. With `max_age` the header carries the attributes
the recorded original carries (the fixture decides between `Expires` alone and `Max-Age` with `Expires`).

`Secure` on by default breaks logins over plain HTTP in browsers that do not treat the host as trustworthy (most
accept `http://localhost`). The startup log line states the cookie's attributes, and the error-free way out is
`.secure(false)` with its warning. Alternative: `Secure` off by default as the original (an HTTPS deployment that
forgets the option sends the session over HTTP on the first plain request); rejected, security over parity. An
automatic mode needs `Request::protocol()` (out of scope).

### 8. Idle and absolute timeouts

The record carries a reserved key next to the original's `cookie` key:

```json
{ "cookie": { "originalMaxAge": null, "expires": null, "httpOnly": true, "path": "/", "secure": true,
              "sameSite": "lax" },
  "passport": { "user": 7 },
  "__corvis": { "createdAt": 1794182400000, "lastAccess": 1794186000000 } }
```

- `createdAt` is set when the identifier is generated (so `regenerate`, login and logout start a new absolute
  period); `lastAccess` on every commit (`create`, `update`, `touch`).
- On load the middleware destroys and replaces a session when `now - lastAccess > idle_timeout`,
  `now - createdAt > absolute_timeout`, or `cookie.expires` is in the past. A record without `__corvis` (written by a
  Node app) gets both values set to the time of its first load.
- The `ttl` passed to the store is the smallest of the idle timeout, the remaining absolute time and the cookie's
  remaining `max_age`; stores use it to drop records, but expiry does not depend on the store honoring it.
- `__corvis` and `cookie` are reserved: `Session::set` refuses them, `get` and iteration do not show them. A Node app
  sharing the store sees `__corvis` as ordinary session data and keeps it.
- Defaults: idle 24 hours (the default TTL of `connect-redis` when the cookie has no expiry, so the behavior most
  migrated apps already have), absolute 7 days. Bounds: idle from 60 seconds up to the absolute timeout; absolute up
  to 400 days; `no_absolute_timeout()` is an explicit opt-out with a startup warning.
- Time comes from an internal clock (`pub(crate)` trait, system time by default) so tests move it without sleeping.

Alternative: keep the timestamps outside the JSON (extra arguments of the store trait). Rejected: every shared store
would need its own place for them, and the express-compatible record would stop being the whole truth.

### 9. Store trait and memory store

```rust
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    async fn load(&self, id: &str) -> Result<Option<Value>, SessionStoreError>;
    /// Fails with `SessionStoreError::Exists` when the identifier is taken.
    async fn create(&self, id: &str, record: &Value, ttl: Duration) -> Result<(), SessionStoreError>;
    /// `Ok(false)` when the session no longer exists; never creates it.
    async fn update(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError>;
    async fn touch(&self, id: &str, record: &Value, ttl: Duration) -> Result<bool, SessionStoreError> {
        self.update(id, record, ttl).await
    }
    async fn destroy(&self, id: &str) -> Result<(), SessionStoreError>;
}
```

The original's `get` / `set` / `touch` / `destroy`, with `set` split so that the two guarantees of decision 6 (no
resurrection, no overwrite of an existing identifier) are the store's contract. `SessionStoreError` messages never
include the identifier or the record.

`MemorySessionStore`: a map from identifier to (serialized record, expiry, size) plus a recency order. Bounds:
`max_sessions` (default 100 000, the throttler's default) and `max_total_bytes` (default 256 MiB), both at least 1.
Expired records are dropped on `load` and in an amortized purge on writes (no background task). When a `create`
would exceed a bound after purging, least recently used sessions are evicted until it fits, and a warning is logged
at most once a minute with the number of evictions, without identifiers. A startup log line says that sessions are
kept in memory, are lost on restart and are not shared between instances.

Eviction instead of refusing new sessions: with a full store, refusing blocks every new login until sessions expire,
while eviction ends the longest-idle sessions first. Both are denial of service under a flood of session-creating
requests; the defaults (`save_uninitialized = false`, so only requests that write to the session create one) make the
flood need a route that writes for anonymous visitors, and the docs recommend rate limiting such routes. See Open
question 3.

`max_session_bytes` (default 16 KiB, 1 KiB to 1 MiB) is checked by the middleware on the serialized record before
any store write, for every store.

### 10. Login, logout and the session user

```rust
#[async_trait]
pub trait SessionSerializer: Injectable {
    type User: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    async fn serialize_user(&self, user: &Self::User) -> Result<Value, HttpException>;
    async fn deserialize_user(&self, req: &Request, stored: Value) -> Result<Option<Self::User>, HttpException>;
}
// providers = [UsersService, Provider::strategy::<SessionStrategy<UserSerializer>>()]
```

- `SessionStrategy<S>` resolves `S` through DI, registers itself with the `SessionManager` when built (a second one
  is a startup error) and is the strategy named `"session"`: `Success` when the request has a session user, `Fail`
  otherwise, so `AuthGuard::new("session")` protects a route with the plain 401. The original's `session` strategy
  always passes and apps write an `isAuthenticated()` guard; the reference recorded in the fixture is that guard.
- Restoring: after loading a session with `passport.user`, the middleware calls `deserialize_user`. `Some(user)` →
  the typed user and its JSON are set on the request (what `AuthGuard` does on success in #136). `None` →
  `passport.user` is removed (the session is now modified, so it is saved) and the request is anonymous. `Err` → that
  error is the response, as the original's `next(err)`.
- `Authenticator::log_in(&self, req, &AuthenticatedUser) -> Result<(), HttpException>`: requires a session on the
  request and a registered serializer (otherwise a 500 with a fixed message); converts the user's JSON form to
  `S::User`, calls `serialize_user`, regenerates the identifier, clears the data unless
  `keep_session_info_on_login`, sets `passport.user`, and sets the request user. The old identifier is destroyed in
  the store at commit, before the new record is created.
- `Authenticator::log_out(&self, req)`: removes `passport.user`, regenerates the identifier (old one destroyed),
  clears the data unless `keep_session_info_on_login`, removes the request's JSON user.
- `AuthGuard::new("local").log_in()`: on `Success` the guard calls `Authenticator::log_in` before returning
  `Ok(true)`; a `log_in` error is the guard's error. Without `.log_in()` the guard never touches the session (the
  requirement #136 left here). The startup check of #136 treats `.log_in()` as also naming the `"session"` strategy,
  so a login guard without a registered `SessionStrategy` stops `init()`.
- The key `passport.user` is kept so that sessions are readable both ways during a migration.

`Authenticator` lives in the always-compiled core; the two methods and `AuthGuard::log_in` are
`#[cfg(feature = "auth-session")]`. They reach the `SessionManager` through the `Session` handle on the request, so
the core needs no DI link to the session module.

### 11. Fixtures

Added to `crates/corvis/tests/fixtures/auth/`: `express-session` 1.19.0 and `cookie-parser` (latest 1.4.x at
implementation time, pinned exactly), with `cookie-signature` resolved by the lockfile (1.0.7). All secrets are fake
strings of at least 32 bytes.

- `signed_cookies.mjs` → `signed_cookies.json`: signatures of fixed values with fixed secrets (ASCII, empty, with
  dots, UTF-8); `req.signedCookies` for valid, tampered, unsigned, truncated and `s:`-less inputs; rotation with two
  secrets; the `Set-Cookie` of `res.cookie(name, value, { signed: true })`.
- `session.mjs` → `session.json`: an app with a store wrapper that records every `get` / `set` / `touch` / `destroy`
  and the final record JSON. Sequences (each step: request cookie, response status, body, `Set-Cookie` attributes
  with the identifier replaced by a placeholder, store operations): anonymous request with the defaults of the
  original and with `resave: false, saveUninitialized: false`; first write; unmodified second request; `rolling`;
  `maxAge`; tampered, unsigned and unknown identifiers; two secrets; `regenerate`; `destroy`; login with a custom
  header strategy and the documented login guard; request with the session user; user deleted after login; logout;
  fixation attempt (login while presenting a pre-login cookie); `isAuthenticated()` guard 401. A cookie produced by
  the original with a fixed identifier and a record are included verbatim, so the Rust test feeds them to Corvis
  ("cookie created by the original"), and the Rust test's cookie and record for the same steps are checked against
  the recorded shapes.
- Marked divergences, asserted from the spec: default `SameSite=Lax` and `Secure`; default `resave` /
  `save_uninitialized`; the `__corvis` key; no re-creation by `update`; re-signing after rotation; `Secure` sent over
  plain HTTP; short secrets refused; timeouts; size limits.

### 12. Example

`examples/cats-app`: enables `auth-session`; `SessionModule::for_root_async` with the secret from configuration
(startup failure when shorter than 32 bytes) and `.secure(false)` only under its development configuration;
`UserSerializer` storing the user id; `POST /session/login` with `AuthGuard::new("local").log_in()` (the local
strategy of #41), `GET /session/me` with `AuthGuard::new("session")`, `POST /session/logout`; tests for login, the
changed identifier, logout and the old cookie being anonymous.

## Risks / Trade-offs

- **Depends on unreleased designs** (#136, #41): `AuthGuard` internals, the startup check and `AuthenticatedUser`'s
  JSON accessor may differ; the first task re-checks them.
- **`Secure` by default** costs a line of configuration in plain-HTTP development and can confuse (a login that
  "does not stick"). Mitigated by the startup line and the docs.
- **Raw identifiers as store keys**: whoever reads the store can use the sessions. Same as the original; needed for
  sharing.
- **Last write wins** between concurrent requests of one session, as in the original; only destruction is final.
- **A store read (and a user lookup when logged in) on every request with a session cookie**, static files included
  when they go through the router. Same cost as the original; restricting the middleware to routes is out of scope.
- **A failed commit turns a handled request into a 500** after its side effects. Rare (store outage, oversized
  session) and fail-closed.
- **Eviction under pressure** logs users out (Open question 3).
- **Record compatibility** is pinned to express-session 1.19.0 by the fixture; another major may change the `cookie`
  object.
- **`__corvis` in shared records** is visible to a Node app as session data.
- **Clock changes**: timeouts use wall-clock milliseconds because the record must be readable by another process; a
  clock set backwards extends sessions by that amount.

## Open questions

1. **Redis and ORM stores.** The issue text asks for memory, Redis and ORM stores; this design ships the trait and
   the memory store and proposes two new issues (a Redis store compatible with `connect-redis` 10.0.0 once the
   workspace has a Redis client; a `corvis-orm` store with the `sid` / `sess` / `expire` table of the spike). Confirm
   the split, or say which client crate to use if the Redis store must be part of #44.
2. **`Secure` on by default** (decision 7) versus off as the original, or waiting for an automatic mode.
3. **Full memory store**: evict least recently used sessions (this design) or refuse new sessions with a 503.
4. **Default timeouts**: idle 24 hours and absolute 7 days. Confirm, or pick stricter values (OWASP suggests shorter
   ones for sensitive apps) with the current ones as documented examples.
5. **`passport.user` and `connect.sid`** are kept for cross-reading with a Node app. Confirm these names in the
   defaults are acceptable (they name the Node libraries, not the banned words).
6. **Re-signing after rotation** adds a `Set-Cookie` the original does not send. Keep it, or leave rotation to
   `rolling`?
7. **`log_in` / `log_out` on `Authenticator`** (spike decision 8) with `cfg` on a core type, versus methods on
   `Session`. The design follows the spike.

## Review notes

These settle the open questions and override the design where they differ.

1. The Redis and ORM session stores are out of this change. Two follow-up issues are opened in epic #39 when this
   change is archived; the Redis one waits for the client crate chosen by #36.
2. `Secure` is on by default. The docs say how to turn it off for plain-HTTP development and note that browsers
   differ on `Secure` cookies over `http://localhost`.
3. A full in-memory store evicts the least recently used session, as designed. The docs state that the in-memory
   store is for one process and for development, and that eviction logs a warning (rate limited).
4. Idle expiry 24 h and absolute expiry 7 days are confirmed.
5. `connect.sid` and `passport.user` stay as defaults, so a session written by the original stack can be read.
6. The cookie is re-signed after a secret rotation, as designed; it is a listed difference.
7. `log_in` and `log_out` are `Authenticator` methods behind the feature, as designed.
8. Implemented after #136 and #41 land: the first task compares this design with the merged code (the typed user
   lives in a private slot that `User<T>` reads; `respond_with` only works in the guard phase; a strategy appends
   cookies only on success, because headers a guard appended are also sent on a later 401 or 403) and records the
   adjustments in Implementation notes.
9. The security review is mandatory before the merge (signature verification, fixation, store races, limits).

## Implementation notes

Written after comparing this design with the merged auth code (#136, #42, #41, #45, archived under
`openspec/changes/archive/2026-11-10-e3-*`, including their "Security review" sections) and with the sources of the
original packages, which were read when the generators were written.

### Adjustments to the merged code

1. **API as merged.** `Strategy::name` is a method (the session strategy answers `"session"`);
   `Provider::strategy::<S>()` needs `S: Injectable + Strategy`, so `SessionStrategy<S>` implements `Injectable` by hand,
   like the adapters of #42, #41 and #45; `AuthenticatedUser::new` returns a `Result` and keeps the typed value in a
   private slot that `User<T>` reads. `AuthenticatedUser` gained a crate-private `typed::<T>()` so the session codec can
   read the user the login strategy built, with a conversion from the JSON form when it is another type (the example
   logs in with `LoginUser` and stores a `Member`). `Authenticator` has no public constructor and always exists, so
   `log_in` and `log_out` are inherent methods in `auth/session/login.rs` under the feature, as Review note 7 says.
2. **The session user lives in its own slot.** The middleware installs the restored user like `AuthGuard` does
   (`AuthenticatedUser::install`: typed slot plus JSON user) and also keeps a copy in a `SessionUser` extension, which
   the `"session"` strategy reads. Without it `AuthGuard::chain(["session", ...]).optional()` could not work: an earlier
   optional guard calls `clear_user()` (the original sets `req.user = null` the same way) and the strategy would find
   nothing. The rules, stated in the docs and tested: the user is restored on every route the middleware covers,
   including routes without a guard and `#[public]` ones; a guard that succeeds replaces it; an optional guard or a
   `Pass` clears the request user; the `"session"` strategy still succeeds; `skip_public()` leaves it alone.
3. **Empty users.** `AuthenticatedUser::is_empty` (null, false, 0, "") is applied three times: a stored
   `passport.user` of that kind is removed without calling the serializer, a restored user that is empty is treated as
   gone, and `Authenticator::log_in` refuses an empty user or an empty serialized value with a fixed 500 and stores
   nothing. `AuthenticatedUser::install` refuses an empty user as it already did.
4. **`respond_with` and appended headers are not used.** They exist only in the guard phase and appended headers are
   also sent on a later 401, so the middleware appends `Set-Cookie` to the `Response` it owns, as the design said.
5. **Startup check.** `AuthGuard::startup_problem` also reports `.log_in()` without a registered `"session"` strategy
   (named per route, through `inner_guards` too, because the check walks the same guards) and `.log_in()` on a guard
   that names `"session"`. `credential_query_params` is empty for the session strategy. `WarnLimiter` is reused for the
   store faults (one line per reason per minute, with the count of the suppressed ones) and for the memory store's
   evictions.
6. **Untrusted JSON.** Nothing the client sends is parsed as JSON: the cookie is a signed string and records come from the
   store. `PlainJson` and `parse_untrusted` are therefore not used. A record is read as a `serde_json::Value` by the
   store, and the middleware checks types and timestamps before using any field.
7. **Dependencies.** `hmac` 0.12 is new (same `digest` 0.10 as `sha2`, `cargo tree -d` shows no second generation);
   `subtle` was added by #45 and is shared. `cargo deny check` passes.

### Facts about the original, corrected after reading the sources

8. `cookie-signature` 1.0.7 compares `sha1(sign(str))` with `sha1(val)` using `==`; it is not `timingSafeEqual` (that is
   1.2.x). Corvis compares the re-encoded signature strings in constant time and accepts exactly the same inputs; the
   recorded `read` cases show it.
9. `express-session` writes `Expires` and never `Max-Age`, because it serializes `cookie.data`, which has no `maxAge`.
   The record's `cookie` object has the keys `originalMaxAge`, `expires` (ISO string or null), `secure`, `httpOnly`,
   `domain`, `path`, `sameSite`, the undefined ones omitted. `rolling: false` and a `maxAge` set `Expires` only when the
   session is modified.
10. `req.session.regenerate()` creates an empty session: the data is dropped. The design said "data kept". `Session::
    regenerate()` does what the original does and `Session::regenerate_keeping_data()` is the added variant.
11. The original does nothing for a request whose path does not start with the cookie `Path`, withholds a `Secure`
    cookie on a connection it does not consider secure, and keeps the first of two cookies of one name. Corvis does the
    first, sends `Secure` cookies always and ignores both cookies of a repeated name.
12. Store calls of the original that Corvis does not repeat, normalized by the replay (`normalize` in
    `tests/auth_session.rs`): `regenerate` calls `destroy` even for an identifier that never had a record, a login ends
    with two `set` calls for one record, and `logOut` saves the session (without the user) before destroying it.

### Decisions beyond the design

13. **Repeated session cookies are ignored** (more than one cookie of the session name, in one header or several): the
    request has no session. First wins would let a cookie planted from a sibling subdomain pick the session; rejecting
    can only log the user out of a site that is already under such an attack. `__Host-` is the real fix and is
    documented. Tested with valid cookies in both orders, equal cookies and other cookie names around one.
14. **Ended sessions clear the cookie** when a verified cookie named a session that was destroyed (`destroy()`, an
    idle or absolute expiry in a store that keeps records) and no new session is saved. An unknown identifier does not
    clear it (the original sends nothing either way).
15. **Destruction survives cancellation.** A drop guard on the request future spawns the destruction of the queued
    identifiers if the future is dropped before the commit (a client that disconnects during a logout, a panic).
16. **The memory store evicts anonymous sessions first.** Two recency orders: records without `passport.user` and records
    with one. A flood of anonymous sessions can only evict anonymous ones. A user who can log in can still create
    sessions, so the docs say to rate limit the login route.
17. **Store contract additions.** `SessionStore::in_process()` (the memory warning) and `validate()` (bounds checked at
    startup). `touch` of the memory store renews only the expiry and the `cookie` and `__corvis` keys, so an unmodified
    request cannot overwrite what another request saved. Every call is bounded by `store_timeout` (default 5 s, 100 ms
    to 60 s), also the user lookup. A `Backend` message is never logged (it is the store's text and could hold an
    identifier); the framework logs the kind of failure.
18. **Clock.** `now_ms()` is the wall clock plus the offset of tokio's clock from the real one, so timestamps are
    readable by other processes and tests move time with the paused clock (`tokio::time::advance`) without a test hook
    in the options. Timestamps from the future count as now; unusable ones destroy the record.
19. **Rolling cannot outlive the absolute timeout.** The cookie `Expires` is capped at `createdAt + absolute`, and the ttl
    given to the store is the minimum of idle, remaining absolute and the cookie's remaining life.
20. **`log_out` of a session without a record** only clears its data (and the user): no new identifier, so a visitor
    without a session cannot create records by calling a logout route. `log_in` is reachable only after a successful
    authentication.
21. **`SessionOptions::with_secrets`** returns an error that names the session secret (position and length); it exists
    next to `SessionOptions::new(CookieKeys)` so the startup error of a short secret says what it is about.
    `SessionCookie::without_same_site` reproduces the original's cookie.
22. **The user lookup** (`deserialize_user`) runs for every request that carries a session with a user, before the
    guards, bounded by `store_timeout`.
23. **Not in this change:** `Session::id` is public (the docs say it is a credential); WebSocket and Socket.IO handshakes
    are answered before HTTP middleware, so they never see a session; no CSRF token; `j:` JSON cookies are not decoded;
    no zeroizing of secrets.

### Fixtures and tests

24. `signed_cookies.json`: 20 signatures, 26 `read` cases and 5 `Set-Cookie` headers; `session.json`: 40 steps over 9
    apps (the original's defaults, Corvis's defaults, rolling, `maxAge`, two secrets sharing a store, login with and
    without `keepSessionInfo`), plus a cookie and record produced with a fixed identifier. Both generators give the same
    output on two runs. The replay compares status, body, `Set-Cookie` attributes, the store calls and the records the
    store holds after each step; four steps carry a `divergence`, asserted from the specification.
25. Test files: `tests/signed_cookies.rs`, `tests/auth_session.rs` (replay), `tests/auth_session_http.rs` (scenarios,
    expiry with the paused clock, the logout race with a held handler, cancellation), `tests/auth_session_log.rs` (own
    binary, the logger is process-wide), `tests/session_common/mod.rs` (the shared app and a recording store), unit
    tests in `signed_cookie.rs`, `options.rs`, `session.rs`, `memory_tests.rs`, `request.rs` and property tests in
    `auth/session/fuzz.rs` (CI and nightly run `--lib fuzz` with the feature).
26. **Example.** `cats-app` enables `auth-session`; `SESSION_SECRET` is optional (a random secret per start when unset,
    a value under 32 bytes stops startup); `SESSION_COOKIE_SECURE=false` is the plain-HTTP development switch;
    `POST /session/login`, `GET /session/me`, `POST /session/logout`; tests for the login, the changed identifier, the
    old cookie after a logout and the short secret.
27. **Left to the maintainer's session:** the corvis-parity check (the ports do not enable the feature; the code outside
    it that changed is `AuthGuard`'s optional field and `Request::cookies`, which now share `scan_cookies`), the
    security review, archiving, the CI run of the pull request (task 1.2's last verification), and the follow-up issues
    for the Redis and ORM stores.

## Security review (round 1)

The first review (proofs of concept `item1` to `item8`) found nine items. The maintainer decided each one; this is what
was done and why.

1. **A logout or `destroy()` racing `regenerate_keeping_data()` resurrected the login (Medium).** `commit` destroyed the
   old identifier and created the new one from the loaded data without asking whether the old record still existed.
   `SessionStore::destroy` now returns `Result<bool, _>` (`true`: a record was removed; an expired one counts as gone).
   `State::origin` keeps the identifier of the record the request loaded, also after `regenerate`, `log_in`, `log_out` or
   `destroy`. At commit, if the destroy of the origin reports `false`, or `update` / `touch` find no record, the session
   is treated as destroyed: nothing is created or updated and the cookie is cleared (a `Set-Cookie` that clears it is sent
   when the request can carry one). This applies to every way the request changed the session, `regenerate()`, a login and
   a `destroy()` followed by new data included: a login on a cookie whose record another request just ended is dropped, a
   deliberate trade (the user logs in again) against a rule with exceptions. The check and the write are one step in
   `MemorySessionStore` (a single lock). Tests drive the race with named gates, for logout and `destroy()`, against
   `regenerate_keeping_data()`, `regenerate()`, a data change and a touch. The sentence of the docs, "a slow request that
   started before a logout cannot bring the session back", is now exactly true and was expanded to say how.
2. **The memory store accounted serialized bytes but kept `Value` trees (Medium).** An array of small numbers costs about
   35 times its text. Records are now stored as `Box<str>` and parsed on load (the store serialized them itself); a touch
   merges the timestamps into the parsed record and serializes it again. The accounted size is the allocation (identifier
   + record + user key + `ENTRY_OVERHEAD`). The overhead was measured with a counting allocator at about 520 bytes for the
   cheapest logged-in record (one user each; maps, three order indexes, a per-user list) and set to 640, counted for
   every entry. `tests/auth_session_memory_heap.rs` (its own binary, one test, because the allocator counts the process)
   fills a store with arrays of `0`, of `{}`, of `[]`, of `null`, objects of 1500 short keys, 14 KB strings, records with
   users and tiny records, at counts just past the growth of the hash tables, and asserts live heap at most accounted
   bytes at most `max_total_bytes`.
3. **A login guard followed by a failing guard stayed logged in (Low, fixed).** `AuthGuard::log_in()` now goes through
   `Authenticator::log_in_from_guard`, which saves the state before the login in `State::before_guard_login`. The
   middleware passes the response status to `commit` (`None` for an error, which is a rejection): below 400 the login is
   kept, otherwise the saved state replaces the session, so the pre-login session stays as it was and no logged-in
   cookie is issued. The saved state is the one before the first guard login; an explicit `Authenticator::log_in` by a
   handler drops it (the handler decides, as before). **Deliberate divergence from the original**, which keeps the
   login; listed in the docs divergence table (EN and ES) and in the fixtures' list of differences (this section: the
   recorded fixture has no step for it, so it is asserted from the specification, in `auth_session_review.rs`).
4. **A flood of authenticated sessions evicted other users' sessions (Low, fixed).** The memory store has
   `max_sessions_per_user` (default 16, at least 1, validated at startup), keyed by the serialized stored user. A new
   session beyond the cap evicts that same user's oldest session. Global order when the store is full: anonymous first,
   then the oldest session of the user with the most sessions (a `BTreeSet` of `(count, oldest, user)`, so the choice is
   not a scan). Recency counts loads, as everywhere. Other stores need their own policy; the docs say so.
5. **The default `touch` was a full `update` (Low, fixed).** The default is removed: every store implements `touch`, and
   the trait documents that it must change only the expiry and the timestamps.
6. **Login CSRF in the example (Low, fixed).** `docs/security/csrf.md` only described a middleware recipe, so the example
   gained `SameOriginGuard` (`examples/cats-app/src/auth/same_origin.rs`): a state-changing request is refused with 403
   when `Sec-Fetch-Site` is `cross-site` or, without that header, when `Origin` is present and is neither the request's
   host nor an allowed origin. It runs before the login guard. The docs (EN and ES) say `SameSite=Lax` does not stop login
   CSRF and show the check. It stays in the example: a reusable framework facility belongs to the CSRF epic.
7. **Cookie-name prefixes were matched case-sensitively (Info, fixed).** `__host-` and `__secure-` are matched ignoring
   ASCII case at startup, as browsers do.
8. **A stored record over `max_session_bytes` loaded and then failed every write (Info, fixed).** `load` checks the size
   of the record the store returns (the same serialized length as the write check) and treats a larger one as no session:
   not loaded, not adopted, not deleted, with a warning at most once a minute that names no identifier.
9. **Docs only.** The cookie name is not part of the signature (parity with `cookie-signature`): use separate `CookieKeys`
   for the application's own signed cookies (`docs/http/cookies.md`, `docs/http/session.md`, and a requirement in
   `http/signed-cookies`). The session user is visible on routes without an `AuthGuard`: a prominent warning with the
   hand-written roles guard as the example, pointing to the CSRF page. Cookie tossing: two cookies of the session name
   load no session, so a planted one logs the victim out; use `__Host-`. The `Secure` sentence now says the browser will
   not send the cookie over HTTP while a `Set-Cookie` issued in a response over plain HTTP still travels in clear.

Test notes: `an_active_session_reaches_the_absolute_timeout` asserted the last request exactly on the absolute deadline,
which the real milliseconds spent by the test decide (the store's expiry uses `<=`, the middleware's check `>`); it now
stops one hour short and checks the expiry after two more.

## Security review (round 2)

The second review (proofs of concept `r2_item1` to `r2_item5`) confirmed that the fixes of round 1 held (no resurrection
in 16 interleavings and 100 000-request stress runs, counters exact) and found five items plus documentation points. The
maintainer decided each one.

1. **An exception filter decided whether a guard login was kept (Medium).** Round 1 passed the status of the final
   response to `commit` and discarded the login when it was 400 or more. Route filters run before that, so with
   `#[use_guards(AuthGuard::new("local").log_in(), SecondFactorGuard)]` and a filter that turns the denial into a
   redirect to `/mfa` (or into an "access denied" page with status 200) the response was a 302 or 200 with a logged-in
   cookie: a second-factor bypass. The decision no longer depends on the status. The router sets the marker
   `RequestRejected` (in `common/context.rs`, stored in the request's shared extensions) at the moment of failure: in
   `execute_route` when the pipeline result is an `Err`, a panic is caught, or a guard ended the request with a stored
   response, and in `invoke_handler` when a pipe fails or the handler returns `Err` (so an interceptor that maps the
   error to a normal response cannot hide it). The middleware reads the marker through a clone of the request's
   extensions (a reference count, not a copy of the request) and passes `rejected` to `commit`, which restores the state
   before the login when `rejected` is set or no response exists. A handler that returns normally keeps the login
   whatever status it sets, including a redirect it issues itself: the docs say that a handler implementing a second step
   must not use `.log_in()` on the first step's guard but call `Authenticator::log_in` after the last step. A response
   that is never produced (client disconnect, timeout) discards the login too: the future is dropped, the `CommitGuard`
   destroys the identifiers queued (the one the login replaced), and `create` is never reached, so nothing logged in is
   stored. The previous session is destroyed rather than restored, the safe direction. Always-compiled files touched:
   `common/context.rs` (the marker; `is_marked` exists only with `auth-session`) and `router/mod.rs` (three marking
   calls, executed only on failure paths); an application that does not enable the feature pays one extension insert per
   failed request. The docs example was rewritten (EN and ES) and no longer suggests a filter can recover a login.
2. **The memory store parsed and serialized records under its global lock (Medium, availability).** `load` parsed under
   the lock and `touch` parsed, merged and serialized under it on every unchanged request, so 16 tasks using one 15 KiB
   session made another user's p99 14 to 16 ms (0.32 ms alone) and cut its throughput from 9 368 to 1 600 requests a
   second. An entry now holds `record: Arc<str>` (the text of the last create or update) and `touch: Option<Arc<str>>` (a
   small JSON object with `cookie` and `__corvis`, the only things a touch changes). `load` clones both references under
   the lock and parses outside it, laying the fragment over the record; `create` and `update` build the `Arc<str>`
   before the lock; `touch` serializes the fragment before the lock and under it replaces only that field (the removal
   and reinsertion of the entry is the same work as for any write: O(log n), nothing proportional to the record). The
   fragment is accounted in the entry's size, so the bound still holds; the stored text keeps the documented record
   shape because the composition happens on load. Measured with `tests/auth_session_contention.rs` (debug build, 8
   workers, 16 attackers, 1.5 s per run): before, the victim's p99 went from 0.82 ms (attackers on a tiny session, 2 600
   requests a second) to 28.7 ms (attackers on a 15 KiB session, 51 requests a second); after, 0.20 ms and 10 400 requests
   a second against 0.19 ms and 10 560. Sharding was not needed: no contention shows after the change. The test compares
   the same victim under two loads (tiny against 15 KiB attackers), takes the median of three rounds and asserts a ratio of
   at most 2 on p99 and on throughput; it asserts no absolute time and failed on the old code.
3. **Records the store wrote and could not read (Low).** The body parser accepts JSON nested 127 to 128 levels; inside a
   session record that exceeds the 128 levels `serde_json` reads, so `load` returned `None` (the user was silently logged
   out), the record stayed until its time to live, and `touch` could overwrite it with `"null"`. Now: `commit` refuses to
   write a record nested deeper than `MAX_RECORD_DEPTH` (100, the object of the record counting as level 1, so a value
   nests 99 levels) with the same 500 as an oversized session, through `check_writable`, which checks size and depth with a
   walk that stops at the limit; the limit is in the limits table of the docs. A stored record deeper than the limit (it
   could never be saved again) is no session when read, with a warning, like an oversized one. The memory store destroys a
   record whose text does not parse and answers `None`, with the rate-limited warning. Every fallback that could write a
   corrupted record is gone: `touch` no longer parses at all.
4. **The maps never shrank (Low).** `Inner::shrink` runs after every removal: when a hash table (`entries`, `by_user`)
   holds less than a quarter of its capacity it is rebuilt at twice its length (floor 256), amortized because at least
   three quarters of the entries must be removed first (the ordered maps free their nodes). The counting-allocator test now
   creates 100 000 minimal sessions, destroys them all (live heap goes from 30.1 MB to 46 KB; it was 14.6 MiB before) and
   refills with 14 KB records past a 128 MiB bound (live heap 131.1 MB, accounted 134.2 MB, under the bound).
5. **A third party could make a victim's login fail (Low).** Flooding the store until the victim's pre-login anonymous
   session was evicted made the login answer 200 and clear the cookie ("the origin record is gone" rule of round 1). A
   login that keeps nothing of the previous session now sets `State::fresh_login`, and `commit` skips that rule for it: the
   new session is created, the old identifier is still destroyed. Everything else keeps the rule: a login with
   `keep_session_info_on_login(true)`, `regenerate_keeping_data()`, `regenerate()`, plain saves and touches. Case to think
   about: request A (logged in, cookie C0) logs out while request B on C0 performs a **new** login with credentials
   presented again. B's new session being valid is correct: it authenticated afresh and carries nothing of C0; the
   logged-out C0 stays dead. With data kept, B only clears the cookie, as in round 1. Tests cover both.
6. **`SameOriginGuard` accepted `same-site` and unknown values (Low, example).** It is now an allow-list: a state-changing
   request passes with `Sec-Fetch-Site` `same-origin` or `none`, with its single `Origin` in the list given to
   `allow_origin`, or, when the header is absent, with `Origin` equal to the request's `Host` (or no `Origin`). `same-site`,
   `garbage`, `" cross-site"` and repeated headers are refused. `refuses` takes the values of both headers as slices. EN and
   ES docs and the doc comment of the guard say that the host comparison uses the `Host` the application received and that
   behind a proxy that rewrites it the origins must be listed.
7. **Documented behavior (EN and ES), with the reason:** a logout that loaded X does not end a successor Y that another
   request created by regenerating X (parity with the original; closing it would need a moved-to marker, a candidate
   follow-up); the per-user cap is keyed by the serialized user, so `serialize_user` must be stable across logins; under a
   flood by many accounts users with several sessions lose their oldest first; if the client disconnects during a guard
   login the previous session is destroyed, not restored.

Round 1, item 3 said the login was kept "below 400" and that a handler answering 403 discarded it: that is replaced by item
1 above. A handler that returns `Ok` with a 403 keeps the login (the handler decided); the test of round 1 that exercised
a handler `Err` still holds.

## Security review (round 3)

The third pass confirmed the round 2 fixes (no cross-request leak of the rejection mark over a pipelined connection;
the store does nothing proportional to a record under its lock, victim p99 65 microseconds against 14 to 16 ms; depth
limits exact at 99/100/101; `SameOriginGuard` holds) and found two low issues and a test that overflowed the stack:

1. The rejection mark missed two paths: an interceptor that fails while an OUTER interceptor recovers the failure, and
   a handler that panics or is dropped (a timeout interceptor) while an interceptor then answers normally. A guard
   login was kept there, which also contradicted the docs. `call_interceptors` now marks the request when an
   interceptor returns `Err`, and `invoke_handler` holds a drop guard that marks it unless the handler ends with `Ok`.
   An interceptor that answers without calling the handler and returns `Ok` stands in for the handler and keeps the
   login; documented. Tests: `an_interceptor_that_refuses_leaves_no_login_even_when_another_recovers_it`,
   `a_handler_that_is_dropped_before_it_ends_leaves_no_login`.
2. Round 2's rule "a login that keeps nothing is created even if the origin record is gone" also covered a login made
   over a session that already had a user, so a handler calling `log_in` with the user the session restored could
   bring back a session that another request had just logged out. The exception now applies only when the loaded
   session had no user (the case it was made for: the eviction of a victim's anonymous pre-login session). This
   overrides the round 2 statement that a new login racing a logout of the same cookie is valid: it is dropped, and the
   user logs in again. Test: `a_login_over_a_logged_in_session_that_was_logged_out_meanwhile_stays_logged_out`.
3. The unit test of the depth walk built a value 5,000 levels deep, whose drop overflowed the default stack on Windows
   and took the other unit tests of the binary with it; it uses 300 levels.
4. Documented: a request with neither `Sec-Fetch-Site` nor `Origin` passes `SameOriginGuard`.


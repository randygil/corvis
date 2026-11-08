# Design

## Context

### What Corvis has today (verified in the code)

- **Guards.** `CanActivate` (`crates/corvis/src/common/guard.rs:23-26`) has one method,
  `async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException>`. `run_pipeline`
  (`crates/corvis/src/router/mod.rs:613-618`) runs route guards in order; `Ok(false)` becomes
  `ForbiddenException::new("Forbidden resource")`, an `Err` is passed to the exception filters as is. WebSocket
  handlers run guards the same way (`crates/corvis/src/websockets.rs:711-715`, `WsException("Forbidden resource")`).
- **Guard references.** `#[use_guards(X)]` expands each argument through `enhancer_tokens`
  (`crates/corvis-macros/src/utils.rs:95-109`): a type path becomes `guard_of::<X>()` (resolved through DI, with the
  testing override token and `ResolveCtx::get_or_construct`, macro `enhancer_ctor!` in
  `crates/corvis/src/common/enhancer.rs:38-64`, instantiated for guards at line 68) and any other expression becomes
  `guard_instance(expr)`. So `#[use_guards(AuthGuard::new("jwt"))]` already compiles to an instance guard without
  macro changes.
- **Global guards with DI.** `Provider::app_guard::<T>()` (`crates/corvis/src/di/provider.rs:258-263`) is a
  `use_class` provider whose `enhancer` field casts the instance to `Arc<dyn CanActivate>`;
  `CorvisApplication::init()` (`crates/corvis/src/application.rs:361-397`) walks `container.all_providers()` and
  installs them. This "tagged provider collected in `init()`" mechanism is the model for a strategy registry.
- **Container access from a guard.** `ExecutionContext::module_ref()` (`crates/corvis/src/common/context.rs:136-139`)
  gives a `ModuleRef` whose `get` looks in the module context and then application-wide
  (`crates/corvis/src/di/module_ref.rs:18-37`). An instance guard can find an application-wide registry this way.
- **Request user.** `Request::insert` / `get` store typed extensions shared by all clones
  (`crates/corvis/src/http/request.rs:209-220`); `set_user` stores a JSON value in a private `RequestUser`, read with
  `user()` / `user_as::<T>()` (`request.rs:221-235`). `FromContext` (`crates/corvis/src/common/param.rs:26-29`) is how
  a parameter like `CurrentUser` reads it.
- **401 body.** `UnauthorizedException` converts to `UnauthorizedException::empty()`
  (`crates/corvis/src/http/exception.rs:310-314`), whose body is `{"message":"Unauthorized","statusCode":401}`
  (`exception.rs:112-115`), the same body as the original `new UnauthorizedException()`.
- **JWT.** `JwtService::verify` (`crates/corvis/src/jwt.rs:250-292`) already follows node-jsonwebtoken (configured
  algorithm only, `nbf`, `exp` with `now >= exp`, `iss`/`aud`, numeric claim checks) and is covered by
  `openspec/specs/auth/jwt/spec.md`. It has no `ignore_expiration`, no per-call key and no per-call issuer/audience.
  `verify_or_unauthorized` (`jwt.rs:300-302`) maps every failure to the empty 401.
  `extract_bearer_token` (`jwt.rs:314-318`) uses `split_once(' ')` and `trim()`; passport-jwt's
  `fromAuthHeaderAsBearerToken` uses the regex `/(\S+)\s+(\S+)/` (`passport-jwt/lib/auth_header.js`). They differ:
  `"Bearer\tabc"` gives no token in Corvis and `abc` in passport-jwt; `"Bearer a b"` gives `"a b"` in Corvis and `a`
  in passport-jwt.
- **Guards cannot write a response.** `Res` is only created after the guards and only for handlers declared with
  `#[res]` (`router/mod.rs:619-629`; read back in `execute_route`, `router/mod.rs:576-600`). A guard can only return
  `true`, `false` or an `HttpException`, and `HttpException` (`exception.rs:20-28`) has no headers. So a strategy
  cannot redirect (OAuth2) or set a cookie (sessions) today.
- **Middleware.** `CorvisMiddleware::use_(&self, req, next) -> HandlerResult`
  (`crates/corvis/src/common/middleware.rs:28-31`) wraps the endpoint and can edit the returned `Response`, so a
  session middleware can load the session before the guards and add `Set-Cookie` after the handler.
- **Cookies.** `Request::cookies` parses like cookie-parser (`request.rs:198-207`); `Res::cookie`
  (`crates/corvis/src/http/res.rs:733`) writes them. There is no signed-cookie support and no session.
- **Rate limiting.** `ThrottlerModule` / `ThrottlerGuard` (`crates/corvis/src/modules/throttler.rs:132-165`) already
  limit requests per client with per-route overrides (`THROTTLE_LIMIT`, `THROTTLE_TTL`, `SKIP_THROTTLE`, lines
  29-33); login throttling (#46) builds on it instead of a new limiter.
- **Existing differential-test pattern.** `crates/corvis/tests/fixtures/res_express.oracle.mjs` runs a real Node app
  and writes `res_express.json`; `crates/corvis/tests/res.rs:116` replays the recorded cases against Corvis. Node is
  only needed to refresh the fixture, not for `cargo test`.
- **Dependencies.** `crates/corvis/Cargo.toml` has `jsonwebtoken = "9"` (line 34, optional, feature `jwt`) and
  `sha2 = "0.10"` (line 42); no bcrypt, argon2, hmac, subtle or HTTP client. Default features are
  `["jwt", "websockets"]` (line 46).
- **Docs.** `docs/recipes/passport.md` is `status: unavailable` and documents the hand-made guard;
  `docs/security/authentication.md` builds the JWT flow by hand (lines 133-165) and compares passwords in plain text
  with a warning (lines 96-100).
- **Port apps.** consi and facten do not use Passport: they write `JwtAuthGuard` by hand over `@nestjs/jwt`
  (`corvis-tests/nest/consi-backend/src/auth/jwt-auth.guard.ts`) and hash with `bcryptjs` 3.x; their ports use the
  `bcrypt` 0.17 crate. The epic is opt-in for them.

### What the original does (verified in the Node sources)

Sources read for this spike (default branches on 2026-11-08):

- `@nestjs/passport` 12.0.1, <https://github.com/nestjs/passport> (`lib/auth.guard.ts`,
  `lib/passport/passport.strategy.ts`, `lib/passport.module.ts`, `lib/options.ts`); peer `passport ^0.5 || ^0.6 || ^0.7`.
- `passport` 0.7.0, <https://github.com/jaredhanson/passport> (`lib/middleware/authenticate.js`,
  `lib/sessionmanager.js`).
- `passport-jwt` 4.0.1, <https://github.com/mikenicholson/passport-jwt> (`lib/strategy.js`, `lib/extract_jwt.js`,
  `lib/auth_header.js`).
- `passport-local` 1.0.0, <https://github.com/jaredhanson/passport-local> (`lib/strategy.js`, `lib/utils.js`).
- `passport-oauth2` 1.8.0, <https://github.com/jaredhanson/passport-oauth2> (`lib/strategy.js`).
- `express-session` 1.19.0 with `cookie-signature ~1.0.7`, <https://github.com/expressjs/session> (`index.js`).
- `passport-headerapikey` 1.2.2, `passport-custom` 1.2.1 and `passport-strategy` 1.0.0 (API keys and custom
  strategies; `passport-custom`'s verify callback only reaches success, fail and error, so redirect and pass are
  tested with a direct `passport-strategy` subclass), `passport-google-oauth20` 2.0.0, `passport-github2` 0.1.12 and
  `passport-oauth2-refresh` 2.2.0 (OAuth2 providers and refresh), `connect-redis`
  10.0.0 (Redis session store), `bcryptjs` 3.0.3 (the hashes of the port apps).

Versions confirmed with `npm view <package> version` on 2026-11-08.

Facts the design depends on:

1. **`AuthGuard(type)` always uses the custom callback of `passport.authenticate`** (`auth.guard.ts`,
   `createPassportContext`). Consequences:
   - When every strategy fails, passport calls `callback(null, false, challenge, status)` (`authenticate.js`,
     `allFailed`); the default `handleRequest` throws `err || new UnauthorizedException()`. So the response is the
     empty 401 body, **no `WWW-Authenticate` header, and the strategy's status is ignored** (passport-local's
     `fail(..., 400)` for missing credentials still answers 401).
   - On success the callback path returns before `req.logIn`, so **`session: true` on `AuthGuard` does not log in**;
     only an explicit `this.logIn(request)` (the `LocalAuthGuard` pattern) writes the session.
   - `request.authInfo = info` for every outcome; `request[options.property || 'user'] = user`.
   - `strategy.error(err)` → `callback(err)` → `handleRequest` throws `err` unchanged: an `HttpException` thrown in
     `validate()` keeps its status and body; any other error becomes the 500 `Internal server error`.
   - `strategy.redirect(url, status)` writes `302` (or `status`), `Location`, `Content-Length: 0` and ends the
     response without calling the callback (the guard promise never settles; the client gets the redirect).
   - `strategy.pass()` calls `next()`: the guard resolves with no callback, so `request.user` is set to `undefined`
     and the route runs.
   - An unknown name → `next(new Error('Unknown authentication strategy "x"'))` → 500 at request time.
2. **Chained strategies** (`AuthGuard(['jwt', 'api-key'])`, `authenticate.js` `attempt(i)`): tried in order; the
   first `success` wins; a `fail` moves on; an `error` stops the chain; all failed → the 401 above. With more than
   one name, the callback gets the arrays of challenges and statuses, so `request.authInfo` is an array.
3. **`PassportStrategy(Strategy, name)`** (`passport.strategy.ts`) builds the verify callback from `validate()`: a
   returned array is spread as `done(null, user, info)`, a falsy value means fail, a thrown error means error. The
   instance registers itself in the global passport instance on construction (last `use` with a name wins).
4. **passport-jwt**: no token → `fail("No auth token")`; key provider error or verification error → `fail`;
   `validate` falsy → `fail(info)`. Options `secretOrKey` xor `secretOrKeyProvider(req, rawJwt, done)`,
   `jwtFromRequest`, `issuer`, `audience`, `algorithms`, `ignoreExpiration`, `passReqToCallback`,
   `jsonWebTokenOptions`. Extractors: `fromHeader`, `fromBodyField`, `fromUrlQueryParameter`,
   `fromAuthHeaderWithScheme`, `fromAuthHeaderAsBearerToken`, `fromExtractors` (first non-empty). There is no cookie
   extractor; apps write a function.
5. **passport-local**: fields `username`/`password` (configurable, `a[b]` paths) looked up in `req.body` then
   `req.query`; a non-object value is returned as is (numbers too); a missing or falsy value →
   `fail({ message: 'Missing credentials' }, 400)`.
6. **passport-oauth2**: `?error=access_denied` → fail; other `?error` → `AuthorizationError` (500 through
   `handleRequest`); no `code` → redirect to the authorization URL with `response_type=code`, `redirect_uri`,
   `scope`, `client_id`, optional `state` and PKCE `code_challenge` (`S256` or `plain`); with `code` → state check
   (`fail(state, 403)` on mismatch), token exchange, profile load, `validate(accessToken, refreshToken, [params],
   profile)`. **Default state store is `NullStore`** (no CSRF protection) unless `state: true`, and PKCE requires
   `state: true` (both stores keep data in `req.session`).
7. **express-session**: cookie `connect.sid` (default), value `s:` + `id` + `.` + base64 (standard alphabet, `=`
   removed) of HMAC-SHA256(id, secret) (`cookie-signature`); several secrets accepted for unsigning, the first one
   signs. **passport 0.6+ regenerates the session on `logIn` and `logOut`** (`sessionmanager.js`) and stores the
   serialized user under `req.session.passport.user`.
8. **passport-headerapikey** 1.2.2 (`src/Strategy.ts`, <https://github.com/hydra-newmedia/passport-headerapikey>):
   strategy name `headerapikey`; header `X-Api-Key` by default (looked up lowercased, so case-insensitive) with an
   optional `prefix`; missing header → `fail(BadRequestError('Missing API Key'))`; wrong prefix → `fail(...)`; the
   prefix is stripped with an unescaped `RegExp('^' + prefix)`; `verify` falsy → `fail(info)`. **It reads headers
   only**: an API key in a query parameter (asked for by #45) has no reference in this package; its reference is a
   `passport-custom` 1.2.1 strategy reading `req.query[name]`. Through `AuthGuard`, every `fail` is the plain 401.

### Rust ecosystem (crates.io, checked 2026-11-08)

| Need | Crate | Latest | Notes |
|---|---|---|---|
| bcrypt | `bcrypt` | 0.19.3 | Verifies `$2a$`/`$2b$`/`$2y$` hashes as produced by bcryptjs; ports already use 0.17 |
| argon2 | `argon2` | 0.6.0 | RustCrypto, PHC strings |
| Both, simple API | `password-auth` | 2.0.0 | RustCrypto wrapper (argon2 default, verifies others) |
| Constant-time compare | `subtle` | 2.6.1 | `ConstantTimeEq` |
| HMAC for cookies | `hmac` | 0.13.0 | Must share the `digest` generation of the `sha2 = "0.10"` already in the tree: use `hmac` 0.12 with it, or move both up together |
| OAuth2 client | `oauth2` | 5.0.0 | Authorization code, PKCE, typed token responses; needs an HTTP client |
| OpenID Connect | `openidconnect` | 4.1.0 | Out of scope |
| Sessions | `tower-sessions` | 0.15.0 | Tower layer; its cookie and stores are not express-session compatible |
| Auth sessions | `axum-login` | 0.18.0 | Built on tower-sessions; axum extractors |
| JWT | `jsonwebtoken` | 11.1.0 | Corvis uses 9 |
| HTTP client for OAuth2 | `reqwest` | 0.13.5 | With `rustls`; redirects disabled for the token endpoint |
| Rate limiting | `governor` | 0.10.4 | For #46 (login throttling) |

## Goals / Non-Goals

**Goals**

- One strategy model that local, JWT, OAuth2, sessions-backed and API key strategies (#41-#45) and user-written
  strategies plug into, registered and injected like any provider.
- `AuthGuard` with the same HTTP behavior as `AuthGuard(name)` of the original in every outcome: success, failure,
  error thrown by `validate`, chained strategies, redirect, pass, unknown strategy.
- Typed users in handlers without giving up the existing `Request::user_as` path.
- Each follow-up verified by a differential test against the Node library.
- Safe defaults where the original is unsafe, documented as divergences.

**Non-Goals**

- Implementing the strategies (follow-ups do).
- Emulating the passport global singleton, `passport.use()` at runtime, `failureRedirect`, flash messages,
  `successReturnToOrRedirect`, or the `property` option (in Rust the user's type, not a property name, identifies
  the value).
- Being a drop-in for `axum-login`/`tower-sessions` users.

## Decisions

### 1. A registry of named strategies with an object-safe `Strategy` trait (recommended option A)

Options compared:

- **A. Object-safe `Strategy` trait + named registry built in `init()`** (recommended). `Strategy` is the equivalent of
  `passport.Strategy.prototype.authenticate`:
  ```rust
  #[async_trait]
  pub trait Strategy: Send + Sync + 'static {
      fn name(&self) -> &str;
      async fn authenticate(&self, req: &Request, opts: &AuthenticateOptions) -> StrategyOutcome;
  }
  pub enum StrategyOutcome {
      Success { user: AuthUser, info: Option<Value> },
      Fail { challenge: Option<Value>, status: Option<u16> },
      Redirect { url: String, status: u16 },
      Pass,
      Error(HttpException),
  }
  ```
  The five variants are exactly passport's `success`, `fail`, `redirect`, `pass`, `error`, so the guard logic is a
  direct port of `authenticate.js` and testable without HTTP. Strategies are providers tagged like
  `Provider::app_guard` (decision 3), collected into an application-wide `StrategyRegistry`.
- **B. Generic guards per strategy** (`JwtGuard<V>`, `LocalGuard<V>`), no registry. Simplest types, but no
  `AuthGuard("jwt")` by name, no chaining across kinds and no `default_strategy`; migration would change every route
  attribute. Rejected.
- **C. A `#[strategy("jwt")]` attribute macro** generating the registration. Saves one line per strategy but adds
  macro surface before the runtime model is proven; can be added later on top of A. Deferred.
- **D. Wrap `axum-login` (0.18.0) / `tower-sessions` (0.15.0).** They are tower/axum-centric (Corvis has its own
  router and `CanActivate`), are session-first (JWT and API keys do not fit their `AuthnBackend` model), and their
  cookie format and stores are not express-session compatible, so they cannot meet the parity requirements of #44.
  Rejected as the base; individual RustCrypto crates are used instead (decision 9).

### 2. Typed validators with adapter strategies

`validate()` has a different signature per strategy kind (local: username and password; JWT: payload; OAuth2:
tokens and profile). A single generic trait cannot express that, and a blanket
`impl<T: JwtValidator> Strategy for T` conflicts with `impl<T: LocalValidator> Strategy for T`. So each kind has a
validator trait the user implements, and a generic adapter that implements `Strategy` and `Injectable`:

```rust
#[injectable]
pub struct JwtAuth { users: Arc<UsersService> }

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = AuthPayload;                       // Serialize + Clone + Send + Sync + 'static
    fn options(&self) -> JwtStrategyOptions { ... } // extractors, key, issuer, audience...
    async fn validate(&self, req: &Request, payload: Value) -> Result<Option<Self::User>, HttpException> { ... }
}

#[module(providers = [Provider::strategy::<JwtStrategy<JwtAuth>>()])]
pub struct AuthModule;
```

- `Ok(Some(user))` = success, `Ok(None)` = fail (falsy `validate`), `Err(e)` = error passed through unchanged (a
  thrown exception). `info` is available through a second method with a default (`fn info(..) -> Option<Value>`)
  for the rare "return `[user, info]`" case.
- The request is always passed (`passReqToCallback` always on): it costs nothing in Rust and avoids two signatures.
- The strategy name defaults to the kind (`"jwt"`, `"local"`, `"oauth2"`, `"headerapikey"`) and is overridden with
  an associated const `const NAME: &'static str` on the validator, matching `PassportStrategy(Strategy, 'name')`.
- Custom strategies (#45) implement `Strategy` directly (the `passport-custom` model).

Alternative considered: closures (`JwtStrategy::new(opts, |payload| async { .. })`). Rejected: no DI for the
validator's dependencies, which is the main reason the original uses classes.

### 3. Registration: `Provider::strategy::<S>()` and a registry built in `init()`

- `Provider::strategy::<S: Injectable + Strategy>()` builds a `use_class::<S>` provider with a new tag (a
  `strategy` cast next to `enhancer` in `crates/corvis/src/di/provider.rs:117`), token `STRATEGY:<type name>`.
- `init()` (next to the enhancer loop in `application.rs:364-397`) collects every tagged instance into
  `StrategyRegistry { by_name: HashMap<String, Arc<dyn Strategy>> }`, registered as a core application-wide
  provider so `ModuleRef::get::<StrategyRegistry>()` finds it from any module.
- **Duplicate names are a startup error** naming both types. Divergence: passport silently keeps the last `use`.
  A silent override of an authentication strategy is a security risk.
- Strategies live in the user's module and get DI like any provider; the adapter's `Injectable` constructor
  resolves the validator with `ResolveCtx::get_or_construct::<V>()` (`crates/corvis/src/di/injectable.rs:138`), so
  the validator's own dependencies come from that module.
- Optional `StrategyModule::register(StrategyOptions { default_strategy })` (equivalent of
  `PassportModule.register`) provides the default name(s) for `AuthGuard::default()`; without it,
  `AuthGuard::default()` is a startup error (the original only logs an error).

### 4. `AuthGuard` is an instance guard built on an `Authenticator` service

- `AuthGuard::new("jwt")`, `AuthGuard::chain(["jwt", "api-key"])`, `AuthGuard::default()`, used as
  `#[use_guards(AuthGuard::new("jwt"))]`, which `enhancer_tokens` already expands to `guard_instance(..)`; it finds
  the registry through `ctx.module_ref()`. A global variant is `use_global_guards(AuthGuard::new("jwt"))`.
- Its logic is a port of `authenticate.js` + `auth.guard.ts` (spec `auth/strategies`): try strategies in order;
  first success → store the user, return `Ok(true)`; error → return it unchanged; all failed → `UnauthorizedException`
  (empty body, no `WWW-Authenticate`, strategy status ignored); `Pass` → no user, `Ok(true)`; `Redirect` → decision 6.
- The reusable part is `Authenticator::authenticate(&ctx, names, opts) -> AuthResult { user, info, failures }`, so a
  user's own guard can replace `handleRequest` (custom messages, optional auth) without subclassing. Common cases get
  builder methods: `.optional()` (no user instead of 401, the usual `handleRequest` override) and
  `.on_failure(|failures| HttpException)`.
- Not in the prelude: users commonly define their own `AuthGuard` (docs/guards.md:35); the type lives at
  `corvis::auth::AuthGuard`.

### 5. User storage: typed extension plus the existing JSON user

On success the guard stores the user twice: `request.insert(user.clone())` (typed, read with `request.get::<T>()`)
and `request.set_user(&user)` (JSON, read with `request.user_as::<T>()`, unchanged API). Strategy output is
type-erased as `AuthUser { json: Value, typed: Arc<dyn Fn(&Request) + Send + Sync> }` (a closure that inserts the
typed value). `info` is stored as `AuthInfo(Value)` (`request.auth_info()`), matching `request.authInfo`. A generic
`User<T>` `FromContext` extractor answers the empty 401 when no user of type `T` is present.

Alternative: JSON only. Rejected: typed access is the main ergonomic win over the original and costs one clone.

### 6. Core prerequisite: a guard can short-circuit with a response and add response headers

OAuth2 (redirect) and sessions (`Set-Cookie`) need it, and the router cannot do it today (Context). Options:

- **A. Request-scoped "pending response" extension** (recommended): `Request::respond_with(Response)` stores a
  response; `run_pipeline` checks it after each guard and, if present, returns it without running the rest; and
  `Request::append_response_header(name, value)` collects headers merged into whatever response is finally sent
  (success, exception or short-circuit), like `res.setHeader` before `next()` in Express. Non-breaking; also useful
  for interceptors and filters.
- B. Change `CanActivate::can_activate` to return a richer enum. Breaking for every guard. Rejected.
- C. Give `HttpException` headers and a "redirect" kind. Redirects would go through exception filters, and a `@Catch()`
  filter would turn them into error bodies. Rejected.

`StrategyOutcome::Redirect { url, status }` becomes `respond_with` of `302` (or `status`), `Location: url`,
`Content-Length: 0`, empty body, exactly what `strategy.redirect` writes. Scheduled with the strategy core in #42,
because the `Redirect` outcome is part of `auth/strategies` and custom strategies (#45) can redirect; #43 (OAuth2) is
its first real user and #44 (sessions) the first user of `append_response_header`. Spec: `http/guard-responses`.

### 7. Unknown strategy names are a startup error

The original fails at request time with a 500 (fact 1). Corvis checks at `init()`: every `AuthGuard` attached to a
route (and the `default_strategy`) must name registered strategies, otherwise `init()` fails listing route, guard and
missing name. Mechanism: a default method `fn as_any(&self) -> Option<&dyn Any> { None }` on `CanActivate`
(non-breaking), overridden by `AuthGuard`, so `init()` can find `AuthGuard`s among compiled route guards. A guard the
check cannot see (a user guard calling `Authenticator`) still gets the original's 500 at request time. Divergence
documented: an error that the original reports per request is reported once at startup.

### 8. Sessions are a separate middleware, compatible with express-session

`SessionModule` installs a `CorvisMiddleware` that reads `connect.sid` (configurable name), unsigns it with
HMAC-SHA256 against a list of secrets (constant-time comparison), loads the session from a `SessionStore` (memory,
Redis with `sess:` keys and the JSON format of `connect-redis`, ORM table), exposes it as `request.session()`, and
writes the cookie and the store after the handler. Login and logout go through
`Authenticator::log_in(req, user)` / `log_out(req)` which, like passport 0.6+, regenerate the session id and store the
serialized user under `passport.user` (key name kept for cross-reading sessions with a Node app during migration).
`AuthGuard` never logs in by itself (fact 1); a `.log_in()` builder flag covers the `LocalAuthGuard` pattern. User
serialization is a `SessionSerializer` trait (`serialize_user` / `deserialize_user`). The middleware restores the
session user before the guards (the work of `passport.session()`), and `SessionModule` registers a built-in strategy
named `"session"` that succeeds when the request has a session user and fails otherwise, so a route protects itself
with `AuthGuard::new("session")`. The ORM store uses a table shaped like the one of `connect-pg-simple` (`sid`,
`sess` JSON, `expire`), so an existing table can be reused; its schema is fixed in #44.

Alternative: `tower-sessions`. Rejected for the cookie and store format (decision 1 D).

### 9. Crates and features

- New cargo features on `corvis` (all off by default except what today's defaults include): `auth` (registry,
  `AuthGuard`, `Authenticator`, `Strategy`, custom and API key strategies; only `subtle` added, for constant-time key comparison), `auth-jwt`
  (`auth` + `jwt`), `auth-local` (`auth` + `bcrypt`, `argon2`, `subtle`), `auth-oauth2` (`auth` + `oauth2` 5.x +
  an HTTP client), `auth-session` (`auth` + `hmac`; Redis store behind `auth-session-redis`).
- Password hashing: `PasswordHasher` with `bcrypt` (verifies bcryptjs `$2a$/$2b$/$2y$` hashes, configurable cost,
  default 10 like bcryptjs) and `argon2` (argon2id PHC, default for new hashes is a maintainer decision: see Open
  questions); `verify` dispatches on the hash prefix so a migrated database with bcrypt hashes keeps working while
  new hashes use argon2 (`needs_rehash`). Hashing a password longer than 72 bytes with bcrypt is an error rather
  than a silent truncation; verification follows `bcryptjs` exactly so existing hashes keep working.
- OAuth2 HTTP client: `oauth2` 5.x accepts any client implementing its `AsyncHttpClient`; use `reqwest` with
  `rustls` and redirects disabled (the `oauth2` docs require this to avoid SSRF through token endpoint redirects).

### 10. Divergences from the original, on purpose (security over parity)

| Original | Corvis | Why |
|---|---|---|
| OAuth2 `state` off by default (`NullStore`) | `state` required; stored in the session or, without sessions, in a signed short-lived cookie | Login CSRF |
| PKCE needs `state: true` and a session | PKCE `S256` on by default; `plain` rejected | RFC 9700 guidance |
| Last `passport.use(name)` wins | Duplicate names fail at startup | Silent override |
| Unknown strategy → 500 per request | Startup error | Fail early |
| String comparisons of API keys | Constant-time comparison | Timing |
| `AuthGuard()` without default only logs | Startup error | Fail early |
| API key `prefix` used as an unescaped regular expression | `prefix` compared literally | A prefix such as `Key+` must not change matching |

Every other observable behavior (status codes, bodies, headers, which strategy wins, what is extracted from where)
matches the original and is pinned by the differential tests.

### 11. Differential tests: Node oracle scripts and recorded fixtures

Options compared:

- **A. Oracle script + recorded fixture, replayed by `cargo test`** (recommended). The pattern already in the repo
  (Context: `tests/fixtures/res_express.oracle.mjs` → `res_express.json` → `tests/res.rs:116`). Each follow-up adds
  `crates/corvis/tests/fixtures/auth/<strategy>.oracle.mjs`, a minimal Node app with `@nestjs/passport` 12.0.1 and
  the strategy package at the versions of this design, that sends a fixed list of requests and writes
  `<strategy>.json` (request; response status, the headers that matter, body; for sessions also the `Set-Cookie`
  values and the store contents). A Rust test (`crates/corvis/tests/auth_<strategy>.rs`) declares the same routes
  and asserts every recorded case. `cargo test` and CI need no Node; refreshing a fixture is a documented manual
  step (`node <file>.oracle.mjs`) with the versions pinned in the script header.
- B. Two live apps compared by the apidiff tool of the port apps (`corvis-tests/tools/apidiff/apidiff.mjs`). That
  tool lives in a separate repository, needs both servers and Node at test time, and is built for whole apps.
  Rejected for the epic; the port apps keep using it for the parity check.
- C. Rust-only tests with expected values typed by hand. No evidence that the expectations match the original.
  Rejected.

Cases that depend on time or randomness (JWT `exp`, session ids, OAuth2 `state`) are made deterministic in the oracle
(fixed keys, tokens signed with fixed `iat`/`exp` far in the past or future, ids compared by shape, not value).
OAuth2 needs a fake provider (authorization, token and profile endpoints) started by both the oracle and the Rust
test on a loopback port (`127.0.0.1`). Where Corvis diverges on purpose (decision 10) the fixture case carries
`"divergence": "<reason>"` and the Rust test asserts the Corvis behavior written in the spec instead.

### 12. Interaction with route posture (#24)

`AuthGuard` is a guard like any other for the posture check of #26: a route with `AuthGuard` is protected. A global
`AuthGuard` does not skip `#[public]` routes by itself (the original does not either; users write the `isPublic`
check); `AuthGuard::new("jwt").skip_public()` is offered as a convenience once #26 defines the public metadata
key. See Open questions.

## Risks / Trade-offs

- **Core router change (decision 6) touches every request path.** Mitigation: the check is one extension lookup
  after each guard; regression tests and the parity check of the three ports in #42, where it lands.
- **Type erasure of the user (decision 5)** adds a closure allocation per authenticated request. Acceptable; measure
  in the #12 benchmark if it shows up.
- **Session store compatibility** with `connect-redis` depends on its major version (10.0.0 today; key prefix and TTL
  handling changed between majors). Mitigation: pin the version in the #44 oracle and document the supported one.
- **bcrypt edge cases**: bcrypt only uses the first 72 bytes of a password; bcryptjs 3.x and the `bcrypt` crate
  (which has truncating and non-truncating functions) may not handle longer passwords the same way. The #41 fixture
  must cover a 73+ byte password in both directions (hash in Node, verify in Rust and the reverse) and the spec
  decides the behavior.
- **Recorded fixtures go stale** when a Node package releases a new version. Mitigation: the versions are pinned in
  each oracle header; refreshing is a deliberate step reviewed like code.
- **Feature flag sprawl**: five new features. Trade-off accepted to keep `reqwest`/crypto out of apps that do not need
  them.
- **`as_any` on `CanActivate`** is a new public trait method (with a default). Small API surface; may be superseded
  by whatever introspection #26 adds.
- **Unknown-strategy check blind spots**: user guards calling `Authenticator` are only checked at request time.

## Open questions

1. **Name of the module and option struct.** This design uses `corvis::auth`, `StrategyModule`, `StrategyOptions`.
   "Passport" is allowed by the naming rule (it is the Node library), and `PassportModule` would ease migration, but
   it ties the API to another project's brand. Maintainer to confirm.
2. **Default hasher for new passwords**: argon2id (modern default) or bcrypt cost 10 (same hashes as the apps being
   migrated). The design recommends argon2id for new hashes with transparent bcrypt verification.
3. **Which follow-up carries the strategy core** (registry, `Strategy`, `AuthGuard`, `Authenticator`). `tasks.md`
   puts it in #42 (JWT, the most used strategy and the one that reuses existing code), so #42 must be done first;
   alternatively a new small issue "strategy core" could be created.
4. **`skip_public()` on `AuthGuard`** depends on the metadata key #26 defines; confirm once #26 is designed.
5. **Where the oracle's `node_modules` come from.** `res_express.oracle.mjs` hard-codes a path into a port app's
   `node_modules`. The auth oracles need packages the ports do not have (`@nestjs/passport`, the strategies), so the
   design proposes a small `crates/corvis/tests/fixtures/auth/package.json` with exact versions and a lockfile,
   installed only when refreshing fixtures. Maintainer to confirm that adding a `package.json` under `tests/` is
   acceptable.
6. **API key in a query parameter.** `passport-headerapikey` 1.2.2 reads headers only (fact 8). The design matches
   it for headers and uses a `passport-custom` strategy reading `req.query[name]` as the reference for the query
   variant. Confirm that keys in URLs should be offered at all (they end up in access logs); the design offers it
   but documents the risk.
7. **Cookie extractor for JWT** (#42 mentions it): passport-jwt has none; Corvis can ship `from_cookie(name)` as an
   extra extractor whose differential reference is the usual hand-written function (`req => req.cookies?.[name]`).
8. **Smaller divergences fixed by the spec deltas**, to confirm: a non-string `username`/`password` (for example a
   JSON number) is treated as missing credentials (`auth/local`); bcrypt hashing of passwords over 72 bytes is an
   error (`auth/local`); `resave` and `save_uninitialized` default to `false` (`auth/sessions`); the API key prefix is
   compared literally (`auth/api-key`). Each is marked as a divergence in its fixture.
9. **Where `Provider::strategy` lives when the `auth` feature is off.** The design gates it behind `auth`; the DI
   tag it needs in `Provider` (`crates/corvis/src/di/provider.rs:117`) is a field on a core type, so #42 must decide
   whether the field exists unconditionally (simpler, one `Option` per provider) or behind `cfg`.

## Review notes (2026-11-08)

1. `PassportModule` in `corvis::auth`, like `BullModule` for queues: same names as the original module make migration
   mechanical (the library name is not one of the banned words).
2. New hashes use argon2id by default; verification accepts bcrypt hashes too, so migrated user tables keep working.
3. The strategy core and guard responses get their own issue, #136, done first; order: #136 → #42 → #41 → #45 → #44 →
   #43 → #46 → #47.
4. `skip_public()` uses the `isPublic` key settled in #25/#26.
5. A `package.json` in `tests/fixtures/auth/` for the Node oracles is accepted (fixtures are recorded; `cargo test`
   needs no Node).
6. API keys in the query string only when explicitly enabled, with a docs warning about logs.
7. `from_cookie` as an extra JWT extractor is accepted (documented as an addition to the original).
8. The minor divergences of Decision 5 are accepted, each documented in its spec.
9. The strategy tag in `Provider` is always present (no `cfg`).
- `AuthGuard` on WebSockets, OIDC/SAML/2FA, flash messages and CSRF tokens stay out of scope for epic #39.

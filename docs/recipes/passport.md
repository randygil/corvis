---
status: partial
---

# Passport (authentication)

Corvis has the **strategy model** of Passport and its integration for the original framework: strategies registered by name, an `AuthGuard` that runs them, the user on the request and the `401` the original answers when every strategy fails. Its behavior is checked against the original passport integration (12.0.1) over `passport` 0.7.0 with recorded responses.

What is available now is the core: the `Strategy` trait you implement for your own strategies, `AuthGuard`, `PassportModule` and the `User<T>` parameter, and the [JWT strategy](#the-jwt-strategy), the [local strategy](#local-strategy-username-and-password) (username and password, with the `auth-local` feature) and [API keys](#api-keys) (the `auth-api-key` feature), plus the guide to [writing a strategy](#writing-a-strategy) of your own. The other strategies arrive one by one: sessions and OAuth2 are tracked in their own issues (#44 and #43). A [guard of your own over `JwtModule`](#alternative-jwtmodule-guards) stays available.

## The model

| Original | Corvis |
|---|---|
| `class JwtStrategy extends PassportStrategy(Strategy, 'jwt')` | a type that implements `Strategy` (`name()` is `"jwt"`), registered with `Provider::strategy::<JwtStrategy>()` |
| `validate(payload)` returning the user | `authenticate(ctx, options)` returning a `StrategyOutcome::Success` |
| `this.success(user, info)`, `fail`, `redirect`, `pass`, `error` | `StrategyOutcome::{Success, Fail, Redirect, Pass, Error}` |
| `@UseGuards(AuthGuard('jwt'))` | `#[use_guards(AuthGuard::new("jwt"))]` |
| `AuthGuard(['jwt', 'apikey'])` | `AuthGuard::chain(["jwt", "apikey"])` |
| `AuthGuard()` with `PassportModule.register({ defaultStrategy })` | `AuthGuard::default()` with `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` |
| `handleRequest` returning `user \|\| null` | `.optional()` |
| `handleRequest` throwing another exception | `.on_failure(\|failures\| UnauthorizedException::new("Token required"))` |
| `if (isPublic) return true` before `super.canActivate` | `.skip_public()` (reads `#[public]`) |
| `req.user` | `request.user()` (JSON) or the typed value: a `User<T>` parameter |
| `req.authInfo` | `request.auth_info()` |
| a `@User()` parameter decorator | the `User<T>` parameter |

## Writing a strategy

A strategy turns the credentials of a request into a user. It is a normal provider: it receives its dependencies by injection, and can live in any module.

```rust
#[injectable]
pub struct DeviceStrategy {
    devices: Arc<DevicesService>,
}

#[async_trait]
impl Strategy for DeviceStrategy {
    fn name(&self) -> &str {
        "device"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        // Read the credential, bounded, from the places this strategy accepts and no other.
        let Some(id) = ctx.request().header("x-device-id").filter(|id| !id.is_empty() && id.len() <= 64) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match self.devices.find(id).await {
            Ok(Some(device)) => {
                let info = (id == "dev-info").then(|| json!({ "scope": "telemetry" }));
                match AuthenticatedUser::new(device) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            // An error ends the request: it is never a way in.
            Err(DevicesError::Blocked) => StrategyOutcome::Error(ForbiddenException::new("Device blocked")),
            Err(e) => StrategyOutcome::Error(HttpException::internal(e)),
        }
    }
}

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("device"))],
    providers = [Provider::class::<DevicesService>(), Provider::strategy::<DeviceStrategy>()],
    controllers = [DevicesController],
)]
pub struct AppModule;
```

This is the code of `crates/corvis/tests/auth_custom.rs`, which a test replays against a `passport-custom` strategy of the original. No cargo feature is needed.

`Provider::strategy::<S>()` registers the strategy under the name `S::name()`. The application collects every strategy when it initializes; two strategies with the same name stop startup with an error that names both types (the original keeps the last one silently), and so does an empty name, one over 128 bytes or one with a control character.

A strategy ends an attempt in one of five ways, the calls a Passport strategy can make:

| Outcome | Effect |
|---|---|
| `Success { user, info }` | the user is stored for the handler, `info` becomes `request.auth_info()` |
| `Fail { challenge, status }` | not authenticated; the next strategy of a chain runs. If all fail, the answer is the `401` below |
| `Redirect { url, status }` | the request ends with a redirect (302 unless `status` is set): the URL is sent as given, with `Content-Length: 0` and no body; a `status` outside `3xx` is an internal error (`500`) |
| `Pass` | the route runs without a user |
| `Error(exception)` | the chain stops and the exception is the answer, unchanged |

`AuthenticatedUser::new(user)` serializes the user to JSON once and keeps the typed value. A user that cannot be serialized is an internal error (a `500`): the request does not continue with a typed user and no JSON one.

**Redirect URLs are sent as given.** Build the URL from trusted data (a configured identity provider address, a fixed path). If a redirect target comes from the request (a `return_to` or `next` parameter), validate it against an allowlist, or accept only a relative path that starts with a single `/`, is not followed by `/` or `\`, and contains no backslash or control character (browsers treat `\` as `/`, so `/\evil.com` becomes `//evil.com`), before you put it in `Redirect`: otherwise the login becomes an open redirect.

Headers a strategy or guard appended with `append_response_header` are sent on every response of that request, including a later `401` or `403` from the same chain or another guard. Append a cookie only once the attempt succeeded.

A strategy can also set cookies or end the request itself with `ctx.switch_to_http().append_response_header(..)` and `respond_with(..)`, see [Guards](/guards#ending-the-request-and-appending-headers).

### Coming from `passport-custom` and `passport-strategy`

| Original | Corvis |
|---|---|
| `done(null, user, info)` in a `passport-custom` callback | `StrategyOutcome::Success { user, info }` |
| `done(null, false)` | `StrategyOutcome::Fail { challenge: None, status: None }`: the `401` |
| `done(err)` with a `ForbiddenException` | `StrategyOutcome::Error(ForbiddenException::new(..))`: that exception's response |
| `done(err)` with any other error, or a callback that throws | `StrategyOutcome::Error(HttpException::internal(err))`: the `500`, the same body for every request |
| `this.redirect(url, status)` of a `passport-strategy` subclass | `StrategyOutcome::Redirect { url, status }` (`passport-custom` cannot reach it) |
| `this.pass()` | `StrategyOutcome::Pass` (`passport-custom` cannot reach it) |
| a `PassportStrategy(CustomStrategy, 'device')` class with injected services | a type that implements `Strategy`, with the services as fields |

The responses are checked against `passport-custom` 1.2.1 behind the original guard (`crates/corvis/tests/fixtures/auth/custom.mjs`, 15 cases: success with info, no user, `ForbiddenException`, a plain `Error`, alone and second in a chain).

### What the framework guarantees around a strategy

- **An error never lets a request through.** `Error(..)` ends the chain and is the answer, also behind `.optional()`: an optional guard lets a request through only when every strategy *failed*.
- **A panic is an error.** A panic inside `authenticate` is caught and answered as a `500 {"statusCode":500,"message":"Internal server error"}`, the same for every request, behind an optional guard too and without running later strategies of the chain. The panic payload is never read, so a credential in a panic message is not copied anywhere by the framework. The application logs one warning per minute per strategy (with the count of the ones skipped), never the message.
- **Names are checked at startup.** `AuthGuard::new("devise")` for a strategy named `device` stops `init()`, also inside a guard that lists its inner guards in `CanActivate::inner_guards`.
- **Responses during the guard phase.** A strategy ends a request itself only through `Redirect` (3xx statuses only) and, for cookies, `append_response_header`. `respond_with` stores a response that wins over everything else the chain does: do not call it from a strategy to say "no".
- **There is no timeout around a strategy.** A strategy that waits for a database or a service must bound the wait itself (`tokio::time::timeout`), or one slow dependency holds every request. The [API key strategy](#api-keys) does it for its validator.

### Testing a route that uses a strategy

`override_strategy` replaces a registered strategy in a testing module. Give the stub the name of the strategy it replaces, so the routes and the startup checks stay the same:

```rust
let module = Test::create_testing_module()
    .import::<DevicesModule>()
    .override_strategy::<DeviceStrategy>(StubDevice) // StubDevice::name() is "device"
    .compile()
    .await?;
let mut app = module.create_corvis_application();
app.init().await?;
```

### Security checklist for a strategy

- **Fail, never pass.** Return `Fail` (or `Error`) when credentials are missing, malformed or wrong. `Pass` lets the request through with no user: use it only for a strategy that is meant to skip authentication.
- **Compare secrets in constant time.** `==` stops at the first different byte. With the `auth-api-key` feature, `corvis::auth::api_key::constant_time_eq(a, b)` compares SHA-256 digests with a constant-time primitive; store secrets hashed.
- **Keep request data out of errors and logs.** An `Error` message, a `Fail` challenge and a log line all end up somewhere. Fixed strings only, and never a credential or part of one.
- **Bound what you read.** Check the length of a credential before you hash it, compare it or send it to a database, and read only the headers, parameters or cookies you document. Read one place, or define which wins and fail when two disagree.
- **Mark credentials read from the URL** with `request.redact_query_param("name")` before you read them, so `LoggingInterceptor` hides them (see [API keys](#api-keys)). Prefer a header: a URL is written to proxy and access logs.
- **Never build a redirect target from request data** (see the warning above).
- **Treat a slow or failing dependency as an error**, with the same answer for every credential: the client must not learn from the status or the body which key or user it was.

## `AuthGuard`

```rust
#[controller("cats")]
#[use_guards(AuthGuard::new("header"))]
pub struct CatsController;

#[routes]
impl CatsController {
    #[get("me")]
    fn me(&self, user: User<CatUser>) -> Value {
        json!({ "id": user.id })
    }
}
```

`AuthGuard` is an instance guard: `#[use_guards(AuthGuard::new("jwt"))]` on a controller or route, or `app.use_global_guards(AuthGuard::new("jwt"))`. It is in `corvis::auth`, not in the prelude (many applications have a guard of their own with that name).

| Constructor or method | What it does |
|---|---|
| `AuthGuard::new("jwt")` | one strategy |
| `AuthGuard::chain(["jwt", "apikey"])` | strategies tried in order: the first success wins, a failure moves on, an error stops the chain, a pass or a redirect ends it |
| `AuthGuard::default()` | the strategies of `PassportModule::register(..)` |
| `.optional()` | the route runs without a user when every strategy fails; errors are still passed on. An optional guard does not count as access control for [route posture](/guards#require-a-guard-on-every-route) |
| `.on_failure(\|failures\| ..)` | the exception to answer when every strategy fails, built from each failure's strategy name, challenge and status |
| `.skip_public()` | lets `#[public]` routes through without running a strategy: the handler's value of `PUBLIC_METADATA`, else the controller's. Without it, public routes are authenticated like any other, as in the original |
| `.with_options(json!({ .. }))` | options handed to the strategies (`AuthenticateOptions::get`) |

When every strategy fails, the answer is `401 {"message":"Unauthorized","statusCode":401}` through the exception filters, with no `WWW-Authenticate` header, whatever status or challenge the strategies attached to their failure (a strategy that fails with status `400` still gives a `401`). The challenge is kept as `request.auth_info()`: the challenge of the failure for one strategy, or the list of challenges, `null` where a strategy gave none, for a chain.

`User<T>` reads the typed user the guard stored. On a route where there is none (no guard, or an optional guard that found no credentials) it answers the same `401`. `Request::user()` and `user_as::<T>()` read the JSON form.

### Differences from the original

- **Startup errors.** The original answers `500 Internal server error` on each request when a guard names an unknown strategy, and only logs an error when `AuthGuard()` has no default. Corvis stops `init()` before middleware `configure` and the init hooks, with one error that lists every route (method and path) and the missing name. This also covers an empty `AuthGuard::chain`, a default that is not registered, and two `PassportModule::register` calls with different defaults. A guard of your own that calls the `Authenticator` with an unknown name still gets the original's `500` at request time. A guard that wraps an `AuthGuard` (a composite guard) lists it in `CanActivate::inner_guards`, which the check follows at any depth, so a typo in the wrapped name also stops `init()`: `fn inner_guards(&self) -> Vec<&dyn CanActivate> { vec![&self.jwt] }`. Do not return the inner guard from `as_any`: that one is for the guard itself, and it would make the gateway warning below fire for a wrapper that lets public targets through.
- **No `AuthGuard` on WebSocket gateways.** A global `AuthGuard` also reaches gateway messages by default, where the original does not work either. It denies the message with `Forbidden resource` without running a strategy, and the application logs one warning per gateway at startup. Authenticate sockets in `handle_connection`, or call `apply_global_enhancers_to_gateways(false)`, which removes every global enhancer from gateways, not only the `AuthGuard`.
- **The typed user lives in its own slot.** `User<T>` reflects only what an `AuthGuard` stored, never another request extension of type `T`, and `Request::set_user` drops it, so a later guard that sets its own user makes `User<T>` answer `401`. When a strategy passes, or an optional guard finds no credentials, both the JSON user and the typed user are removed (`req.user` is `undefined` or `null` in the original).
- **No session login.** `AuthGuard` never logs in to a session by itself; sessions come with their own issue.
- **No `property`, `failureRedirect` or flash messages.** The user is always stored for `User<T>` and `request.user()`.

## The JWT strategy

`corvis::auth::jwt` (under the `jwt` feature, enabled by default) is the port of `passport-jwt` 4.0.1 over `jsonwebtoken` 9. You write the class of the original, `PassportStrategy(Strategy)` with its options and `validate`, as a `JwtValidator`, and register `JwtStrategy<YourValidator>`:

```rust
use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtStrategy, JwtStrategyOptions, JwtValidator};

#[injectable]
pub struct JwtAuth {
    config: Arc<ConfigService>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Claims; // what handlers receive as User<Claims>

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.config.get::<String>("JWT_SECRET").unwrap_or_default()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
        Ok(serde_json::from_value(payload).ok()) // None answers 401
    }
}

#[module(
    providers = [JwtAuth, Provider::strategy::<JwtStrategy<JwtAuth>>()],
    controllers = [CatsController],
)]
pub struct AuthModule;

// on a controller, a route or globally
#[use_guards(AuthGuard::new("jwt"))]
```

`options()` runs once, when the application starts, and every problem in it stops startup with an error that names the strategy (`Error while instantiating JwtStrategy "jwt" (..): invalid JWT strategy options: ..`). `validate` runs only for a token that was found and verified; the payload is a JSON object. It returns the user (`Some`), no user (`None`, a `401`) or an exception (`Err`, passed on unchanged, as `done(err)` does in the original). The strategy is called `"jwt"`; set `const NAME: &'static str = "jwt-refresh";` in a second validator to register another strategy next to it (each with its own key). `fn info(&self, user)` can return the auth info of a success.

`passReqToCallback` does not exist: the request is always the first argument of `validate`.

### Options

| `passport-jwt` | Corvis | Notes |
|---|---|---|
| `jwtFromRequest` | `.jwt_from_request(ExtractJwt::..)` | required; there is no default place |
| `secretOrKey` | `.secret_or_key(JwtKey::..)` | `JwtKey::secret`, `rsa_pem`, `ec_pem`, `ed_pem` |
| `secretOrKeyProvider` | `.secret_or_key_provider(provider)` | an async `JwtKeyProvider`; give a key or a provider, not both |
| `issuer` | `.issuer("https://auth.example")` or a list | the token's `iss` must equal one of them |
| `audience` | `.audience("web")` or a list | the token's `aud` (string or list) must contain one of them |
| `algorithms` | `.algorithms([..])` | see [Algorithms and keys](#algorithms-and-keys) |
| `ignoreExpiration` | `.ignore_expiration(true)` | logs a startup warning |
| `jsonWebTokenOptions.clockTolerance` | `.clock_tolerance(Duration)` | 0 by default, at most 300 s |
| `passReqToCallback` | | always on |
| | `.max_token_length(bytes)` | 8192 by default, checked before decoding |
| | `.allow_missing_exp()` | accept tokens without `exp`; logs a startup warning |

`maxAge`, `subject`, `jwtid`, `nonce`, `complete`, `clockTimestamp` and regular-expression audiences are not supported.

### Where the token is read from

| `ExtractJwt` in `passport-jwt` | Corvis |
|---|---|
| `fromAuthHeaderAsBearerToken()` | `ExtractJwt::from_auth_header_as_bearer_token()` |
| `fromAuthHeaderWithScheme(s)` | `ExtractJwt::from_auth_header_with_scheme(s)` |
| `fromHeader(name)` | `ExtractJwt::from_header(name)` |
| `fromUrlQueryParameter(name)` | `ExtractJwt::from_url_query_parameter(name)` |
| `fromBodyField(name)` | `ExtractJwt::from_body_field(name)` |
| `fromExtractors([..])` | `ExtractJwt::from_extractors([..])`: the first non-empty token wins, with no fallback to the next one |
| `req => req.cookies?.[name]` | `ExtractJwt::from_cookie(name)` |
| any function | `ExtractJwt::custom(\|req\| ..)` |

The `Authorization` header is parsed exactly as `passport-jwt` does: the first `Authorization` value is matched with `(\S+)\s+(\S+)` (so a tab can separate the scheme and the token, and words after the token are ignored) and the scheme is compared ignoring case. `extract_bearer_token` keeps its own, stricter behavior. An empty value, or a value that is not a string (a number in a JSON body), is no token. Tokens are read only from the places you configure: a valid token in the query or in a cookie does nothing unless an extractor reads it.

::: warning Order matters in `from_extractors`
The first non-empty token is the only one tried: if it fails, the next extractors are not consulted. With a cookie extractor placed before the header one, an attacker who can plant a cookie (from a sibling subdomain, for example) overrides the `Authorization` header of the user. Put the most trusted place first, and read only the places you need.
:::

::: warning A token in a cookie is an ambient credential
The browser sends a cookie on its own, so a cookie token needs the protections of any cookie session: mitigate CSRF (a token or custom header on state-changing requests, or an `Origin` check) and set `SameSite=Lax` or `Strict`, `Secure` and `HttpOnly` when you issue it.
:::

::: warning Tokens in URLs
A token in the query string ends up in access logs, browser history and `Referer` headers. The strategy logs a warning at startup when `from_url_query_parameter` is configured; prefer the `Authorization` header or a cookie.
:::

### Algorithms and keys

A token is accepted only when its header `alg` is in the allow-list **and** belongs to the family of the key used for it, so a public key can never be used as an HMAC secret, and `none` (in any letter case) can never be accepted: there is no way to write it.

| Key | Allowed by default |
|---|---|
| `JwtKey::secret(..)` | `HS256`, `HS384`, `HS512` |
| `JwtKey::rsa_pem(..)` | `RS256`, `RS384`, `RS512`, `PS256`, `PS384`, `PS512` |
| `JwtKey::ec_pem(..)` | `ES256`, `ES384` |
| `JwtKey::ed_pem(..)` | none: write `.algorithms([JwtAlgorithm::EdDSA])` |

With a fixed key, `.algorithms([..])` must be non-empty and belong to the key's family (an HMAC algorithm with an RSA key, or a list that mixes families, stops startup). With a key provider, the list (or every default one) is narrowed to the family of the key each request gets; an empty result is a `401`. `jsonwebtoken` 9 has no `ES512`, so an `ES512` token always fails.

An HMAC secret must be at least 32 bytes (RFC 7518 section 3.2): a shorter one, or an empty one, stops startup, and a key provider that returns one fails that request (`JwtService` itself keeps accepting shorter secrets). Generate one with `openssl rand -hex 32`.

A key provider receives the request and the raw token, which is **not verified yet**: use its `iss` or `kid` only to choose a key, never to trust a value. It runs for every request, so cache what it fetches; an error is a `401`, not a `500`. The error text is logged at debug level, so a provider must never put the raw token in its error. Before the provider runs, the strategy refuses a token longer than `max_token_length`, one with a `crit` header (RFC 7515 section 4.1.11; Corvis supports no critical extensions) and one that is not exactly three non-empty base64url segments.

```rust
struct ByIssuer {
    keys: HashMap<String, JwtKey>,
}

#[async_trait]
impl JwtKeyProvider for ByIssuer {
    async fn key(&self, _req: &Request, raw_token: &str) -> anyhow::Result<JwtKey> {
        let iss = unverified_issuer(raw_token).ok_or_else(|| anyhow::anyhow!("no issuer"))?;
        self.keys.get(&iss).cloned().ok_or_else(|| anyhow::anyhow!("unknown issuer"))
    }
}
```

### Claims and time

`exp` and `nbf` follow `jsonwebtoken` 9: with `now` in whole seconds and the tolerance `t`, a token fails when `now >= exp + t` (`jwt expired`) or `nbf > now + t` (`jwt not active`), and when `exp`, `nbf` or `iat` is present and not a number. `ignore_expiration` skips the `exp` check and requirement. The tolerance defaults to 0 s and is capped at 300 s: a larger value stops startup.

### Differences from `passport-jwt`

Each one is on purpose, and the differential test marks it:

- **`exp` is required.** The original accepts a token without `exp`, which then never expires. Add `.allow_missing_exp()` to accept them (it logs a startup warning); tokens from `JwtService::sign` carry `exp` whenever `expires_in` is set.
- **A non-numeric `iat` fails.** `jsonwebtoken` checks `iat` only with `maxAge`.
- **The payload must be a JSON object.** The original passes a string payload to `validate`.
- **Header names are case-insensitive** in `from_header`; the original only finds names written in lowercase.
- **Tokens are limited to 8192 bytes** by default (`.max_token_length(..)`).
- **The configuration is checked at startup**: an HMAC secret under 32 bytes, an empty issuer or audience, a tolerance above 300 s and the other combinations above do not start.
- **`ES512` is not available** (`jsonwebtoken` 9).
- **A token with a `crit` header, or not made of three base64url segments, fails** before the key provider runs.

`typ` is not validated, and the strategy has no option for it. Do not sign other kinds of token (refresh tokens, e-mail verification, password reset) with the same key and audience as the access tokens: one would be accepted as the other. Use a different key or `aud` for each kind. A `typ` check could come later as an option.

A failed attempt keeps its reason as `request.auth_info()` (`{"message": "jwt expired"}`, `invalid signature`, `invalid algorithm`, `jwt malformed`, `No auth token`, `token too long`, ..), never in the response, which is the usual `401`. The reasons never repeat anything from the token.

### Testing against the original

`crates/corvis/tests/fixtures/auth/jwt.mjs` runs `passport-jwt` 4.0.1 (over `jsonwebtoken` 9.0.x) behind the original guard with one route per strategy and records 65 cases (`jwt.json`): extractors, `Authorization` parsing, algorithms (`none`, confusion between families, explicit lists), time claims, audience and issuer, key providers, RS256, PS256 and ES256. `crates/corvis/tests/auth_jwt.rs` replays them against the same routes, signing the tokens again, with no Node needed at test time.

## Local strategy (username and password)

The local strategy is the port of `passport-local` 1.0.0. It needs the `auth-local` cargo feature, which also brings [`PasswordHasher`](/security/encryption-and-hashing). You write the validator, the class `PassportStrategy(Strategy)` is in the original, and register `LocalStrategy<YourValidator>`:

```rust
use corvis::auth::local::{LocalStrategy, LocalValidator, PasswordError, PasswordHasher};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

#[injectable]
pub struct LocalAuth {
    users: Arc<UsersService>,
    hasher: Arc<PasswordHasher>,
}

#[async_trait]
impl LocalValidator for LocalAuth {
    type User = PublicUser; // Serialize + Clone; leave the password hash out of it

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<PublicUser>, HttpException> {
        let user = self.users.find_by_username(username).await;
        // one real verification whether the user exists or not
        let ok = self
            .hasher
            .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
            .await
            .map_err(PasswordError::into_http_exception)?; // Busy is a 503, the same for every user
        Ok(user.filter(|_| ok).map(PublicUser::from))
    }
}

#[module(
    providers = [
        Provider::class::<PasswordHasher>(),
        Provider::class::<LocalAuth>(),
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
    ],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth")]
pub struct AuthController;

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<PublicUser>) -> Value {
        json!({ "username": user.username })
    }
}
```

`validate` returns the user (a success), `Ok(None)` (the credentials are refused: `401`) or an exception, which is answered as it is (a `ForbiddenException::new("Account locked")` gives the `403`). The strategy is named `"local"`; set `const NAME` in the validator for another name.

| `passport-local` | Corvis (`LocalValidator::options`) |
|---|---|
| `usernameField` (`username`) | `LocalStrategyOptions::default().username_field("email")` |
| `passwordField` (`password`) | `.password_field("pass")` |
| `passReqToCallback` | `validate` always receives the `Request` |
| none | `.max_username_bytes(1024)`, `.max_password_bytes(1024)` |
| reading the query string | `.allow_query_credentials()`, off by default |

Credentials are read like `passport-local` does: each field is looked up in the parsed body (JSON or `application/x-www-form-urlencoded`). Unlike the original, the query string is read only with `.allow_query_credentials()`; then a field the body has no value for is looked up there too. A field name like `user[email]` is a path (`body.user.email`); a segment of an array is an index; a string met before the end of the path is the value; an object or array at the end gives no value. A value that is absent, empty, or not a string is missing, and when either credential is missing the strategy fails without calling `validate`. `AuthGuard` answers the same `401` for every failure; the failure itself (`{"message":"Missing credentials"}`, status `400`) is available to `.on_failure(..)`. An empty field name stops the application at startup.

Do not send credentials in the query string unless you have to: URLs end up in access logs, browser history and `Referer` headers. That is why reading them is opt-in, and a warning is logged at startup when it is on.

### Differences from passport-local

- **The query string is not read by default.** The original takes the username and the password from the query when the body has none; Corvis does so only with `.allow_query_credentials()`, because a password in a URL is written to logs. The recorded fixture replays these cases both ways.
- **Non-string credentials are missing.** The original passes a number or a boolean to `validate()`; Corvis treats them as missing, so a user lookup never sees a type it did not expect.
- **Credentials are bounded.** A username or a password over 1024 bytes (configurable) fails without calling `validate`. The original has no limit besides the body size. Raise the limits with `.max_password_bytes(4096)` if your users need it; the hasher has its own `max_password_bytes`.
- **Failures are uniform.** Missing credentials, wrong passwords and unknown users all give the same `401`, and the validator above spends one hash verification in every case. A stored hash the hasher refuses (an account without a password, an unsupported format) is a wrong password too, not a `500`. When the hasher is saturated (`PasswordError::Busy`) the answer is `503 Service Unavailable` for every user; see [a busy hasher](/security/encryption-and-hashing#a-busy-hasher).
- **Login throttling is separate.** Pair the login route with the [throttler](/security/rate-limiting) (the cats example allows five attempts a minute per client); hashing is expensive on purpose, and a flood queues behind the hasher's `max_concurrent` slots.

## API keys

The API key strategy is the port of `passport-headerapikey` 1.2.2 for machine clients (webhooks, internal services, partner integrations). It needs the `auth-api-key` cargo feature (one extra dependency, `subtle`). You write the validator, the `verify` callback of the original, and register `ApiKeyStrategy<YourValidator>`:

```toml
corvis = { version = "0.1", features = ["auth-api-key"] }
```

```rust
use corvis::auth::api_key::{ApiKey, ApiKeySet, ApiKeyStrategy, ApiKeyValidator, ExpectedApiKey};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

/// The keys of the services that call this application, from configuration.
pub struct ServiceKeys(ApiKeySet<Service>);

#[injectable]
pub struct ServiceKeyAuth {
    keys: Arc<ServiceKeys>,
}

#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = Service; // Serialize + Clone; leave the key out of it

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
        Ok(self.keys.0.lookup(key).cloned()) // None answers 401
    }
}

#[module(
    providers = [
        // a key shorter than 32 bytes is an error here, and the application does not start
        Provider::use_factory(Token::of::<ServiceKeys>(), |ctx| async move {
            let config = ctx.get_or_construct::<ConfigService>().await?;
            let mut keys = ApiKeySet::new();
            keys.insert(ExpectedApiKey::new(config.get::<String>("BILLING_API_KEY").unwrap_or_default())?, Service::billing());
            Ok(ServiceKeys(keys))
        }),
        Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>(),
    ],
    controllers = [DataController],
)]
pub struct MachineModule;

#[get("data")]
#[use_guards(AuthGuard::new("api-key"))]
fn data(&self, service: User<Service>) -> Value { /* .. */ }
```

The strategy is named `"api-key"` (the original names it `headerapikey`: set `const NAME: &'static str = "headerapikey";` in the validator to port a configuration unchanged, or another name to have several API key strategies). `validate` runs only for a key that was found in the configured places and is within the limits. It returns the user (`Some`), no user (`None`, a `401`) or an exception (passed on, see [failures](#failures-of-the-validator)). `fn info(&self, user)` can return the auth info of a success: never put the key in it.

A route for people and machines tries the user strategy first: `AuthGuard::chain(["jwt", "api-key"])`.

### Where the key is read from

`ApiKeyStrategyOptions::default()` reads the `X-Api-Key` header and nothing else. Add the places you accept with `fn options(&self)`:

```rust
fn options(&self) -> ApiKeyStrategyOptions {
    ApiKeyStrategyOptions::new()
        .source(ApiKeySource::header("X-Api-Key"))
        .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
}
```

| Source | Reads | Same as |
|---|---|---|
| `ApiKeySource::header(name)` | every value of the header, read as latin1 and joined with `", "` as Node does; the name is case-insensitive | `passport-headerapikey` (`header`) |
| `.prefix("Api-Key ")` on a header source | the value must start with the prefix (compared literally, case-sensitively), which is removed | `passport-headerapikey` (`prefix`), but literal |
| `ApiKeySource::bearer()` | the first `Authorization` value, parsed like `passport-jwt`'s bearer extractor: `(\S+)\s+(\S+)`, the scheme compared ignoring case | a `passport-custom` strategy with `ExtractJwt.fromAuthHeaderAsBearerToken()` |
| `ApiKeySource::query(name)` | the query parameter, when it is a single string | a `passport-custom` strategy reading `req.query[name]` |

There is no default for the bearer or query sources, and the request body and cookies are never a source (a cookie is an ambient credential, and CSRF comes with it; write a [strategy of your own](#writing-a-strategy) if you need one). Other places are not read even when the key is there: a key in the query is ignored unless a query source is configured.

Every configured source is read. A source whose value is **present but unusable** (too long, with a control character, a repeated query parameter, or a bracketed one when the application uses the extended query parser) fails the attempt, and **two sources that carry different keys fail it too**, without calling the validator. That closes the case where a proxy or a firewall authenticates or rate-limits by one place and the application trusts another. A source that finds nothing (no header, an empty value, another scheme, a value without the prefix) is skipped, and the same key in two places is one key. At most 8 sources.

::: warning Keys in URLs
`ApiKeySource::query("api_key")` writes keys to proxy logs, access logs, browser history and `Referer` headers, none of which Corvis controls. It is off by default and the strategy logs a warning at startup for each query parameter. Corvis hides the value in the URL that `LoggingInterceptor` logs (`GET /data?page=2&api_key=REDACTED 200`), but a middleware that logs `request.url()` before the guards run, your reverse proxy and your CDN still see it. Prefer a header.
:::

### Limits

| Limit | Default | Notes |
|---|---|---|
| `max_key_bytes` | 512 | 1 to 4096, else startup fails. A longer key fails before it is hashed, compared or given to the validator. The prefix of a header source does not count |
| Characters | no byte `0x00` to `0x1F` or `0x7F` | a key with one fails; a tab in a header counts |
| Sources | at most 8, none by default for `ApiKeyStrategyOptions::new()` | an empty list stops startup |
| `validator_timeout` | 5 s | 1 ms to 300 s. A validator that takes longer is a `503` |
| Source names and prefixes | not empty; header names valid; `Cookie` refused; prefixes 1 to 64 printable ASCII characters | each problem stops startup with an error that names the strategy and the option |

Repeated header values are not copied past the limit (a request with thousands of `X-Api-Key` headers stops at the first one that crosses it), and the bearer source reads at most `max_key_bytes + 128` bytes of leading space, scheme and separator before the key.

### Keeping keys

An API key is a long-lived bearer secret, so store only a digest of it and compare digests:

1. **Generate** at least 32 random bytes per key: `openssl rand -hex 32`, with a public id in front if you have many keys (`ck_live_3f9a.<secret>`). `ExpectedApiKey::new` refuses a secret under 32 bytes; a key you generate yourself is yours to get right, so make the generator long enough.
2. **Store** `ApiKey::sha256_hex()` of the key, not the key. A digest of a random 32-byte secret cannot be turned back into the key, and it needs no slow hash (argon2 and bcrypt are for passwords). Show the key to its owner once.
3. **Compare** with the helpers, never with `==`:

| Helper | Use |
|---|---|
| `ExpectedApiKey::new(secret)` | one key from configuration; only its digest is kept in memory |
| `ExpectedApiKey::from_sha256_hex(hex)` | one key from a stored digest |
| `ExpectedApiKey::matches(&key)` | one comparison |
| `ApiKeySet::lookup(&key)` | a handful of keys with their owners; compares every entry with no early exit, so the time depends on how many keys there are, not on which one matched |
| `ApiKey::ct_eq(bytes)` and `constant_time_eq(a, b)` | any other secret: both sides are hashed with SHA-256 and the digests compared with `subtle`, so neither the position of the first different byte nor the length of the stored secret shows in the time |

For **many keys** in a database, look the key up by a public id, then compare the digest. An unknown id costs one lookup and a dummy comparison, so unknown and wrong keys take the same time:

```rust
async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
    let Some(id) = key.id('.') else { return Ok(None) }; // `<id>.<secret>`, id 1 to 64 of A-Za-z0-9_-, public
    let record = self.keys.find(id).await.map_err(|_| ServiceUnavailableException::empty())?;
    let stored = record.as_ref().map(|r| &r.expected); // ExpectedApiKey::from_sha256_hex(&row.digest)
    Ok(ExpectedApiKey::matches_or_dummy(stored, key).then(|| record.map(|r| r.service)).flatten())
}
```

Without an id, store the digest in an indexed column and look the digest of the presented key up: `self.keys.find_by_digest(&key.sha256_hex())`. That lookup is not constant-time, but what it can leak is a prefix of the digest of a random secret, which does not help to find the key.

`ApiKey`, `ExpectedApiKey` and `ApiKeySet` print `<redacted>` with `{:?}`, so a derived `Debug` on a struct that holds them cannot write a key to a log; they have no `Display`, `Serialize`, `Clone` or `PartialEq` (an `==` between keys is the comparison that leaks). Call `key.expose()` only to pass the key to something that needs the text, and never log it, put it in an error message, a response or the auth info.

### Failures

Missing, malformed, unknown, revoked and expired keys all answer the same `401 {"message":"Unauthorized","statusCode":401}` through `AuthGuard`, with no `WWW-Authenticate` header. Return `Ok(None)` from the validator for every key that is not valid, whatever the reason. The reason is kept as `request.auth_info()` and never sent: `Missing API Key`, `Conflicting API keys`, `API key too long`, `Invalid API key characters` or `Ambiguous API key` (a repeated or bracketed query parameter). A key the validator refused has no reason. None of them repeats request data.

#### Failures of the validator

- An exception with a **4xx** status is passed on as it is, like `done(err)` in the original (`ForbiddenException::new("Key suspended")` gives the `403`). Use it only for an answer you want the client to see; it tells the client something about the key.
- An exception with a status of **500 or above** (or below 400) is replaced by the plain exception of that status: `{"statusCode":503,"message":"Service Unavailable"}`, `{"statusCode":500,"message":"Internal server error"}`. A database error that quotes the key in its text, or a message that differs per key, never reaches the client. The application logs one warning per minute with the status, never the message.
- A validator that runs longer than `validator_timeout` is a `503` with that same body, and a **panic** in the validator is a `500`: both for every key, also behind `.optional()`. Neither is a way in.

An infrastructure failure (the database is down) should be an error, not `Ok(None)`: a `401` tells the machine client its key is wrong, a `503` tells it to retry.

### Differences from passport-headerapikey

Each one is on purpose and the differential test marks it:

- **The name is `"api-key"`**, not `headerapikey`, and the key can come from more places than a header.
- **The default reads one header and nothing else.** The query is a source only when configured, with a warning.
- **A prefix is a literal string.** The original builds an unescaped regular expression from it: with the prefix `Key+`, `Key+<key>` reaches its `verify` as `+<key>`.
- **Different keys in two sources, or an unusable value in one, fail** (the original has one source).
- **Keys over `max_key_bytes` (512) or with control characters fail** before the validator (the original has no limit besides the header size).
- **An empty key, or a prefix with nothing after it, never reaches the validator.**
- **A validator failure of status 500 or above has a fixed body, and a slow validator is a `503`.**

`passport-headerapikey` answers a missing header with a `BadRequestError('Missing API Key')` challenge; through the original guard that is the same `401`, which is what is recorded.

The `404` for a route that does not exist is `Cannot GET /x?api_key=...` with the URL as sent, as in the original; no strategy runs for it and it goes only to the sender.

### Testing against the original

`crates/corvis/tests/fixtures/auth/api_key.mjs` runs `passport-headerapikey` 1.2.2, and `passport-custom` 1.2.1 strategies for the query and bearer variants, behind the original guard and records 55 cases (`api_key.json`): header names, prefixes, repeated values, wrong, missing and empty keys, the validator returning no user, an exception and a plain error, chains. `crates/corvis/tests/auth_api_key.rs` replays them against the same routes with no Node needed at test time; the five marked divergences (an oversized key twice, a control character, conflicting keys, the literal prefix) are asserted from the specification.


## Differential testing

Every case above is recorded from the original: `crates/corvis/tests/fixtures/auth/strategies.mjs` runs the original passport integration (12.0.1) over `passport` 0.7.0 with custom `passport-strategy` 1.0.0 strategies driven to each outcome, and `tests/auth_strategies.rs` replays the recorded responses against the same routes in Corvis, with no Node needed at test time. Each strategy kind that arrives later adds its own cases.

## Alternative: `JwtModule` + guards

The JWT strategy above is the main path. The pieces under it stay available when you prefer to write the guard yourself, and cover the most common case, an API with JWT:

- `JwtModule::register(JwtModuleOptions::new(secret))` or `JwtModule::register_async(global, |ctx| ...)` to configure signing;
- `JwtService::sign` and `JwtService::verify` (or `verify_or_unauthorized`, which already returns a `401`);
- `extract_bearer_token(&request)` to read the `Authorization: Bearer ...` header;
- a [guard](/guards) that verifies the token and leaves the user on the request with `request.insert(user)`;
- a `FromContext` parameter (the equivalent of a `@User()` decorator) that retrieves it in the handler.

The `examples/tertulia` guard (`src/auth/auth_guard.rs`) does exactly that:

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

The full flow (login, guard, current user, roles) is explained in [Authentication](/security/authentication) and [Authorization](/security/authorization). For a username and password login, Passport's "local step" is an endpoint of your own that verifies the password (see [Encryption and hashing](/security/encryption-and-hashing)) and returns the token.

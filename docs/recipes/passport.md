---
status: documented
---

# Passport (authentication)

Corvis has the **strategy model** of Passport and its integration for the original framework: strategies registered by name, an `AuthGuard` that runs them, the user on the request and the `401` the original answers when every strategy fails. Its behavior is checked against the original passport integration (12.0.1) over `passport` 0.7.0 with recorded responses.

Five strategies are available, each behind a cargo feature, and the `Strategy` trait lets you [write your own](#writing-a-strategy):

| Strategy | Cargo feature | Port of | Section |
|---|---|---|---|
| JWT | `jwt` (on by default) | `passport-jwt` 4.0.1 | [The JWT strategy](#the-jwt-strategy) |
| Local (username and password) | `auth-local` | `passport-local` 1.0.0 | [Local strategy](#local-strategy-username-and-password) |
| API key | `auth-api-key` | `passport-headerapikey` 1.2.2 | [API keys](#api-keys) |
| Session | `auth-session` | `passport.session()` over `express-session` | [Sessions](#sessions) |
| OAuth2 (Google, GitHub and any provider) | `auth-oauth2` (brings `auth-session`) | `passport-oauth2` 1.8.0 and its Google and GitHub profiles | [OAuth2](#oauth2-google-github-and-others) |

This page is the reference. It is organized as the model, writing a strategy, `AuthGuard`, then one section per strategy with its options, limits and the differences from its original, then the [index of the differences](#differences-from-the-original-by-strategy), how the port is tested, and the [alternative](#alternative-jwtmodule-guards) of a guard of your own over `JwtModule`. To choose a strategy and see a working application, read the guide, [Authentication](/security/authentication) (the code is in `examples/auth-app`); to bring a Passport setup over, [Migrating from Passport](/recipes/passport-migration); for what the module protects and what it leaves to you, [Securing authentication](/security/authentication-security) and the [threat model](/security/authentication-threat-model).

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

The responses are checked against `passport-custom` 1.2.1 behind the original guard (`crates/corvis/tests/fixtures/auth/custom.mjs`, 24 cases: success with info, no user, `ForbiddenException`, a plain `Error`, alone and second in a chain, and a user that is `false`, `0`, `""`, `null`, `undefined` or `NaN`).

::: danger The user must be a real value
A strategy that succeeds with a user that is JSON `null`, `false`, the number `0` (also `-0` and `0.0`) or the empty string is **not authenticated**. `Authenticator::authenticate` turns that `Success` into a failure of the strategy (the chain goes on, the `auth_info` is `{"message":"The strategy returned an empty user"}`, and the answer is the usual `401`) and logs a warning, at most one per minute per strategy, that says the validator should have returned `None`. This applies to every strategy: JWT, local, API key and your own. The originals fail closed the same way (`if (!user) return self.fail()` in the strategies, and `handleRequest` throws a `401` for a falsy user).

The usual way to hit it is a validator that returns what it computed. `type User = bool; Ok(Some(expected.matches(key)))` and `type User = Option<Service>; Ok(Some(repo.find(id)))` look right, but they say "found" for every key, and the user is `false` or `null` when the key is wrong. Return `Ok(None)` for "no user": `Ok(expected.matches(key).then_some(service))`, `Ok(repo.find(id))`.

If an application's users can legitimately be `0` or `""` (a numeric id as the whole user), wrap them in a struct: `struct Account { id: u64 }` serializes to an object and is never empty. An empty object `{}`, an empty array and the strings `"0"` and `"false"` are real users, as in JavaScript.
:::

### What the framework guarantees around a strategy

- **An error never lets a request through.** `Error(..)` ends the chain and is the answer, also behind `.optional()`: an optional guard lets a request through only when every strategy *failed*.
- **A panic is an error.** A panic inside `authenticate` is caught and answered as a `500 {"statusCode":500,"message":"Internal server error"}`, the same for every request, behind an optional guard too and without running later strategies of the chain. Corvis never reads the panic payload and never logs it. It is not hidden from everything else: the default panic hook of the standard library prints the panic message to stderr, so a credential in a panic message ends up in your process logs. A strategy must not put credentials in panic messages (`unwrap`, `expect` and `panic!` on a value that holds one) or in errors. Corvis logs one warning per minute per strategy (with the count of the ones skipped), and the default exception filter does not add a line per request.
- **An empty user is a failure.** See the warning below.
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
- **Refuse an empty or unset secret before you compare.** `constant_time_eq(b"", b"")` is `true`: an expected secret that is missing from the configuration (an unset environment variable read as `""`) would accept a request with an empty credential. Check that the expected secret is not empty (and long enough) at startup or before the comparison, and fail when it is.
- **Return `None`, not an empty user.** `Ok(Some(false))`, `Ok(Some(None))`, `0` and `""` are treated as a failure with a warning (see above); say "no user" with `None` or `Fail`.
- **Keep credentials out of panic messages.** The default panic hook prints the message to stderr, which Corvis cannot intercept.
- **Keep request data out of errors and logs.** An `Error` message, a `Fail` challenge and a log line all end up somewhere. Fixed strings only, and never a credential or part of one.
- **Bound what you read.** Check the length of a credential before you hash it, compare it or send it to a database, and read only the headers, parameters or cookies you document. Read one place, or define which wins and fail when two disagree.
- **Register credentials read from the URL.** A strategy lists the query parameters it reads in `Strategy::credential_query_params`, and `LoggingInterceptor` hides their values on every request (see [API keys](#api-keys)); a guard of your own marks one with `request.redact_query_param("name")` before it reads it. Prefer a header: a URL is written to proxy and access logs, and nothing outside Corvis hides it.
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

`User<T>` reads the typed user slot. The slot holds what an `AuthGuard` stored, or what the session middleware restored from the session (or `Authenticator::log_in` stored). On a route where the slot is empty (an optional guard that found no credentials, or no session user) it answers the same `401`. On a route without an `AuthGuard`, `User<T>` returns the session's user when the session restored one, as `Request::user()` does, and nothing about that request checked it: put an `AuthGuard` on every route that needs an authenticated user (see [the session user on routes without an `AuthGuard`](/security/authentication-threat-model#the-session-user-on-routes-without-an-authguard)). `Request::user()` and `user_as::<T>()` read the JSON form. A guard that runs after an `AuthGuard`, a roles guard for example, reads the same slot with `Request::typed_user::<T>()`, which returns a clone of the typed user and never another request extension of the same type, so it does not deserialize the JSON user.

### Differences from the original

- **Startup errors.** The original answers `500 Internal server error` on each request when a guard names an unknown strategy, and only logs an error when `AuthGuard()` has no default. Corvis stops `init()` before middleware `configure` and the init hooks, with one error that lists every route (method and path) and the missing name. This also covers an empty `AuthGuard::chain`, a default that is not registered, and two `PassportModule::register` calls with different defaults. A guard of your own that calls the `Authenticator` with an unknown name still gets the original's `500` at request time. A guard that wraps an `AuthGuard` (a composite guard) lists it in `CanActivate::inner_guards`, which the check follows at any depth, so a typo in the wrapped name also stops `init()`: `fn inner_guards(&self) -> Vec<&dyn CanActivate> { vec![&self.jwt] }`. Do not return the inner guard from `as_any`: that one is for the guard itself, and it would make the gateway warning below fire for a wrapper that lets public targets through.
- **No `AuthGuard` on WebSocket gateways.** A global `AuthGuard` also reaches gateway messages by default, where the original does not work either. It denies the message with `Forbidden resource` without running a strategy, and the application logs one warning per gateway at startup. Authenticate sockets in `handle_connection`, or call `apply_global_enhancers_to_gateways(false)`, which removes every global enhancer from gateways, not only the `AuthGuard`.
- **The typed user lives in its own slot.** `User<T>` reflects what an `AuthGuard` stored or the session restored (never another request extension of type `T`; see above for routes without a guard), and `Request::set_user` drops it, so a later guard that sets its own user makes `User<T>` answer `401`. When a strategy passes, or an optional guard finds no credentials, both the JSON user and the typed user are removed (`req.user` is `undefined` or `null` in the original).
- **Session login is opt-in.** `AuthGuard` never writes to a session unless it is built with `.log_in()`; see [Sessions](#sessions).
- **No `property`, `failureRedirect` or flash messages.** The user is always stored for `User<T>` and `request.user()`.

## Sessions

The `auth-session` feature adds the session part of Passport on top of [server-side sessions](/http/session): the user
is stored in the session at login and restored on later requests.

| Original | Corvis |
|---|---|
| `passport.serializeUser` and `passport.deserializeUser` | `SessionSerializer::serialize_user` and `deserialize_user` |
| `app.use(passport.session())` | `SessionModule` (its middleware) and `SessionStrategy<S>`, the strategy named `"session"` |
| `req.logIn(user)` | `AuthGuard::new("local").log_in()`, or `Authenticator::log_in(&req, &authenticated_user).await?` |
| `req.logout()` | `Authenticator::log_out(&req).await?` |
| a guard on `req.isAuthenticated()` | `AuthGuard::new("session")`: `401 {"message":"Unauthorized","statusCode":401}` when anonymous. The session strategy of the original always passes |
| `keepSessionInfo` | `SessionOptions::keep_session_info_on_login(true)` (an option of the module, not of each call) |

A complete example, with the [local strategy](#local-strategy-username-and-password) for the login:

```rust
use corvis::auth::session::{SessionModule, SessionSerializer, SessionStrategy};
use corvis::auth::{AuthGuard, Authenticator, User};

#[injectable]
pub struct UserSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    // what goes into `passport.user`: an object, never a bare `0` or `""`
    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    // `None` when the user no longer exists: the request is anonymous and the stale user is dropped
    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64) else { return Ok(None) };
        Ok(self.users.find(id).await)
    }
}

#[module(
    imports = [SessionModule::for_root(session_options())],
    providers = [
        UsersService,
        Provider::class::<PasswordHasher>(),
        Provider::class::<LocalAuth>(),
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
        Provider::strategy::<SessionStrategy<UserSerializer>>(),
        UserSerializer,
    ],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth")]
pub struct AuthController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Account {
        user.0
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(204)]
    async fn logout(&self, #[req] req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}
```

`session_options()` builds the `SessionOptions`, see [Session](/http/session#setup). `Authenticator::log_in` takes an
`AuthenticatedUser` (`AuthenticatedUser::new(user)?`). Logging in generates a new session identifier and destroys the old
one, which prevents session fixation. The middleware restores the user on every route, public ones included, before the
guards run. `AuthGuard::new("local").log_in()` is a startup error when no `SessionStrategy` is registered, and when it is
combined with the `"session"` strategy. The rest of the rules (what an optional guard does to the session user,
`AuthGuard::chain(["session", "jwt"])`, the `500` of a failed save) are in
[Session](/http/session#guards-and-the-session-user).

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
| | `.key_provider_timeout(Duration)` | 5 s by default, 1 ms to 300 s: a key provider that takes longer is a `503` |
| | `.validator_timeout(Duration)` | 5 s by default, 1 ms to 300 s: a validator that takes longer is a `503` |

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

The `Authorization` header is parsed exactly as `passport-jwt` does: the first `Authorization` value is matched with `(\S+)\s+(\S+)` (so a tab can separate the scheme and the token, and words after the token are ignored) and the scheme is compared ignoring case. `extract_bearer_token` is not the strategy's parser and keeps its own rules: it splits at the first space, compares the scheme ignoring case and trims the token. `ExtractJwt::from_auth_header_as_bearer_token` follows `passport-jwt`. An empty value, or a value that is not a string (a number in a JSON body), is no token. Tokens are read only from the places you configure: a valid token in the query or in a cookie does nothing unless an extractor reads it.

::: warning Order matters in `from_extractors`
The first non-empty token is the only one tried: if it fails, the next extractors are not consulted. With a cookie extractor placed before the header one, an attacker who can plant a cookie (from a sibling subdomain, for example) overrides the `Authorization` header of the user. Put the most trusted place first, and read only the places you need.
:::

::: warning A token in a cookie is an ambient credential
The browser sends a cookie on its own, so a cookie token needs the protections of any cookie session: mitigate CSRF (a token or custom header on state-changing requests, or an `Origin` check) and set `SameSite=Lax` or `Strict`, `Secure` and `HttpOnly` when you issue it. A cookie issued without `SameSite` is sent on cross-site `POST`s by browsers that do not default to `Lax`.

A page on a sibling subdomain can set a cookie for the whole domain, in front of your own. When the cookie name appears more than once in a request, `from_cookie` yields no token (the usual `401`), as the session cookie does, instead of trusting the first value. A cookie planted next to the user's own therefore logs that user out of the cookie login for as long as it lives; one that is the only cookie of that name still authenticates as its owner, so name the cookie with the `__Host-` prefix (`Secure`, `Path=/`, no `Domain`) so that a browser refuses one set from a sibling subdomain. The strategy logs a startup warning naming the strategy and the cookie when a cookie extractor's name has no `__Host-` prefix; the prefix is accepted in any letter case (`__HOST-jwt`), as browsers do.
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

A key provider receives the request and the raw token, which is **not verified yet**: use its `iss` or `kid` only to choose a key, never to trust a value. It runs for every request, so cache what it fetches; an error is a `401`, not a `500`. The error text is logged at debug level, so a provider must never put the raw token in its error. The provider runs for any well-formed token, forged ones included, so it has a deadline: `key_provider_timeout` (5 s by default); one that takes longer is dropped (its future is cancelled) and the request is answered with a `503 Service Unavailable` that is the same for every token, with one warning a minute. A provider that fails, by contrast, is a failed attempt, so a chain goes on to the next strategy. Before the provider runs, the strategy refuses a token longer than `max_token_length`, one with a `crit` header (RFC 7515 section 4.1.11; Corvis supports no critical extensions) and one that is not exactly three non-empty base64url segments.

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
- **A cookie token sent twice is no token.** `from_cookie` refuses a name that appears more than once in the request, instead of reading the first value (see the warning above).
- **The callbacks have deadlines.** The key provider and the validator are cut off after `key_provider_timeout` and `validator_timeout` (5 s) with a `503`; the original waits for them forever.
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
| none | `.validator_timeout(Duration)`, 5 s by default (1 ms to 300 s): a validator that takes longer is dropped and answered with a `503` that is the same for every caller, with one warning a minute |

Credentials are read like `passport-local` does: each field is looked up in the parsed body (JSON or `application/x-www-form-urlencoded`). Unlike the original, the query string is read only with `.allow_query_credentials()`; then a field the body has no value for is looked up there too. A field name like `user[email]` is a path (`body.user.email`); a segment of an array is an index; a string met before the end of the path is the value; an object or array at the end gives no value. A value that is absent, empty, or not a string is missing, and when either credential is missing the strategy fails without calling `validate`. `AuthGuard` answers the same `401` for every failure; the failure itself (`{"message":"Missing credentials"}`, status `400`) is available to `.on_failure(..)`. An empty field name stops the application at startup.

Do not send credentials in the query string unless you have to: URLs end up in access logs, browser history and `Referer` headers. That is why reading them is opt-in, and a warning is logged at startup when it is on.

### Differences from passport-local

- **The query string is not read by default.** The original takes the username and the password from the query when the body has none; Corvis does so only with `.allow_query_credentials()`, because a password in a URL is written to logs. The recorded fixture replays these cases both ways.
- **Non-string credentials are missing.** The original passes a number or a boolean to `validate()`; Corvis treats them as missing, so a user lookup never sees a type it did not expect.
- **Credentials are bounded.** A username or a password over 1024 bytes (configurable) fails without calling `validate`. The original has no limit besides the body size. Raise the limits with `.max_password_bytes(4096)` if your users need it; the hasher has its own `max_password_bytes`.
- **Failures are uniform.** Missing credentials, wrong passwords and unknown users all give the same `401`, and the validator above spends one hash verification in every case. A stored hash the hasher refuses (an account without a password, an unsupported format) is a wrong password too, not a `500`. When the hasher is saturated (`PasswordError::Busy`) the answer is `503 Service Unavailable` for every user; see [a busy hasher](/security/encryption-and-hashing#a-busy-hasher).
- **Login throttling is separate.** Pair the login route with the [throttler](/security/rate-limiting) (the cats example allows five attempts a minute per client); hashing is expensive on purpose, and a flood queues behind the hasher's `max_concurrent` slots. To count attempts per account as well, use `login_tracker` with the recipe in [Protecting a login route](/security/rate-limiting#protecting-a-login-route): the throttler must run before the `AuthGuard`, or the password is hashed before the limit is checked.

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
| `.prefix("Api-Key ")` on a header source | the value must start with the prefix (compared literally, case-sensitively), which is removed; a value that does not start with it is absent, whatever its length or content, so an unrelated `Authorization` header never fails a key found elsewhere | `passport-headerapikey` (`prefix`), but literal |
| `ApiKeySource::bearer()` | the first `Authorization` value, parsed like `passport-jwt`'s bearer extractor: `(\S+)\s+(\S+)`, the scheme compared ignoring case | a `passport-custom` strategy with `ExtractJwt.fromAuthHeaderAsBearerToken()` |
| `ApiKeySource::query(name)` | the query parameter, when it is a single string | a `passport-custom` strategy reading `req.query[name]` |

There is no default for the bearer or query sources, and the request body and cookies are never a source (a cookie is an ambient credential, and CSRF comes with it; write a [strategy of your own](#writing-a-strategy) if you need one). Other places are not read even when the key is there: a key in the query is ignored unless a query source is configured.

Every configured source is read. A source whose value is **present but unusable** (too long, with a control character, a repeated query parameter, or a bracketed one when the application uses the extended query parser) fails the attempt, and **two sources that carry different keys fail it too**, without calling the validator. That closes the case where a proxy or a firewall authenticates or rate-limits by one place and the application trusts another. A source that finds nothing (no header, an empty value, another scheme, a value without the prefix) is skipped, and the same key in two places is one key. At most 8 sources.

::: warning Keys in URLs
`ApiKeySource::query("api_key")` puts the key in the URL, and **we recommend against it**: use a header. It is off by default and the strategy logs a warning at startup for each query parameter.

What is hidden: when the application starts, every strategy that reads a query parameter registers its name, and `LoggingInterceptor` and `Request::redacted_url()` replace the value of that parameter with `REDACTED` on **every** request, whether or not that strategy runs for it (`GET /data?page=2&api_key=REDACTED 200`), including percent-encoded names and, with the extended query parser, `api_key[x]=`. A query string with more than 1000 pairs has every value hidden. The `Debug` form of a `Request` prints the same redacted URL. A parameter name that contains `%` is refused at startup, because it cannot be hidden reliably.

What is not: anything outside Corvis (the access logs of your reverse proxy, load balancer and CDN, the browser history, the `Referer` header that the page sends to other sites) and anything inside it that prints `request.url()`, `request.uri()` or the query itself (a middleware, your own logging, an error report) instead of `redacted_url()`. Only the names the strategies declare are known: a strategy that reads the query in its own code, such as `ExtractJwt::custom` calling `query_param`, or a custom strategy that does not override `credential_query_params()`, registers nothing, so its parameter is printed as is. Call `request.redact_query_param(name)` there, or override the method.
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

Missing, malformed, unknown, revoked and expired keys all answer the same `401 {"message":"Unauthorized","statusCode":401}` through `AuthGuard`, with no `WWW-Authenticate` header. Return `Ok(None)` from the validator for every key that is not valid, whatever the reason. The reason is kept as `request.auth_info()` and never sent: `Missing API Key`, `Conflicting API keys`, `API key too long`, `Invalid API key characters` or `Ambiguous API key` (a repeated or bracketed query parameter). A key the validator refused has no reason. None of them repeats request data. A validator that answers `Ok(Some(user))` with an empty user (`false`, `null`, `0`, `""`) is a failure too, with the reason `The strategy returned an empty user`: answer `Ok(None)` instead (see [the user must be a real value](#writing-a-strategy)).

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


## OAuth2 (Google, GitHub and others)

The OAuth2 strategy is the port of `passport-oauth2` 1.8.0 with the Google and GitHub profiles of `passport-google-oauth20` 2.0.0 and `passport-github2` 0.1.12, and the refresh request of `passport-oauth2-refresh` 2.2.0. It does the authorization code flow only, and needs the `auth-oauth2` cargo feature (it brings `auth-session`; the provider client uses `tokio-rustls` and the bundled web PKI roots, no other crate):

```toml
corvis = { version = "0.1", features = ["auth-oauth2"] }
```

You write the validator (the `verify` callback of the original, receiving everything the provider gave as one `OAuth2Grant`) and register `OAuth2Strategy<YourValidator>`. Two routes carry the same guard: the login route, which redirects to the provider, and the callback route, whose handler runs with the user your validator returned.

```rust
use corvis::auth::oauth2::{OAuth2Grant, OAuth2Options, OAuth2Strategy, OAuth2Validator, return_to};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

#[injectable]
pub struct GoogleAuth {
    users: Arc<UsersService>,
    config: Arc<ConfigService>,
}

#[async_trait]
impl OAuth2Validator for GoogleAuth {
    const NAME: &'static str = "google";
    type User = SessionUser;

    fn options(&self) -> OAuth2Options {
        let get = |name: &str| self.config.get::<String>(name).unwrap_or_default();
        OAuth2Options::google(get("GOOGLE_CLIENT_ID"), get("GOOGLE_CLIENT_SECRET"), "https://app.example/auth/google/callback")
            .scope(["openid", "email", "profile"])
            .state_cookie_secrets([get("OAUTH_STATE_SECRET")])
            .return_to_param("return_to")
            .failure_redirect("/login?error=oauth")
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<SessionUser>, HttpException> {
        let Some(profile) = grant.profile else { return Ok(None) };
        // Identify the user by (provider, id), never by e-mail alone. Link an existing account by e-mail only through
        // verified_email().
        let user = self.users.find_or_create(&profile.provider, &profile.id, profile.verified_email()).await?;
        Ok(Some(user))
    }
}

#[module(
    providers = [GoogleAuth, Provider::strategy::<OAuth2Strategy<GoogleAuth>>()],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth/google")]
pub struct AuthController;

#[routes]
impl AuthController {
    #[get("login")]
    #[public]
    #[use_guards(AuthGuard::new("google"))] // redirects to Google
    fn login(&self) {}

    #[get("callback")]
    #[public]
    #[use_guards(AuthGuard::new("google"))] // the handler runs with the user `validate` returned
    fn callback(&self, req: Request, user: User<SessionUser>) -> Value {
        json!({ "user": user.0, "return_to": return_to(&req) })
    }
}
```

`validate` returns `Ok(Some(user))` for a success, `Ok(None)` for a 401 and `Err(exception)` to answer that exception, as every strategy does. Invalid options stop `init()` with an error that names the strategy and the option. The presets are `OAuth2Options::google(..)` and `OAuth2Options::github(..)`; `OAuth2Options::new(OAuth2Endpoints::new(authorization_url, token_url).userinfo_url(url), ..)` is any other provider (without a userinfo URL the grant has no profile). `OAuth2Options::endpoints(..)` replaces the URLs of a preset (GitHub Enterprise, a proxy, a test double). Register the redirect URI you configured with the provider. Google needs a scope; GitHub's `user:email` gives the verified e-mail addresses.

::: warning Give OAuth2 its two routes
An OAuth2 strategy starts a login (a redirect to the provider) for any request that is not a callback. In a chain it therefore ends the chain with that redirect: `AuthGuard::chain(["github", "api-key"])` never reaches the key, and in the last place it turns every anonymous request into a login start. Keep OAuth2 strategies out of chains, and rate limit both routes per client, see [Protecting a login route](/security/rate-limiting#protecting-a-login-route). The [threat model](/security/authentication-threat-model#combinations-of-strategies) lists what this and the other combinations of strategies do.
:::

### What the validator receives

`OAuth2Grant` has `access_token` and `refresh_token: Option<_>` (both `OAuth2Token`: no `Display`, no `Serialize`, `Debug` prints nothing; call `expose()` on purpose), `params` (the token response without the tokens: `get(key)`, `expires_in()`, `scope()`, `id_token()`) and `profile: Option<OAuth2Profile>`. The framework stores no token. If you keep one to call the provider later, store it encrypted at rest with the user record, never in a cookie or in the browser's session.

`OAuth2Profile` serializes with the keys of the original (`provider`, `id`, `displayName`, `username`, `name`, `emails`, `photos`, `profileUrl`, `_json`) and adds a `verified` flag to every e-mail (and `primary` for GitHub). **Identify a user by `(provider, id)`, never by e-mail alone**: Google's `sub`, GitHub's numeric id as a string. `provider` is `"google"` or `"github"` only when the strategy uses that preset's own endpoints unchanged; when any endpoint was replaced with `endpoints(..)` (GitHub Enterprise, a proxy) and for a generic strategy it is the strategy's name, which is unique in the application. Two instances of the same software therefore never share a namespace (a user number of your GitHub Enterprise server is not the github.com account with that number), as long as you give each strategy its own name. `profile.issuer` is the host of the authorization endpoint, for logs and audits. The `id` is the provider's value as a string, compared exactly (`"00012"` and `"12"` are different users): store it as the provider gives it. **Link an account by e-mail only through `verified_email()`**: the primary address the provider reports as verified (Google: `email_verified: true`; GitHub: the `/user/emails` entry that is both `primary` and `verified`). Linking by an unverified address lets anyone who registers it at the provider take over the account. A strategy that is not a preset but is **named** like one (`github`, `google`) gets `name@issuer` as its `provider` (for example `github@ghe.corp.example`) and a warning at startup, so the ids of another server never share the namespace of github.com or Google; give such a strategy a name of its own to choose the value.

::: warning The `id_token` is not verified
With the `openid` scope Google returns an ID token. `grant.params.id_token()` hands it over as it came: its signature, issuer, audience, expiry and nonce are **not** checked, so read no claim from it. The profile comes from the userinfo endpoint over TLS, authenticated by the token the server obtained with its client secret and PKCE verifier; use that.
:::

### `state`, PKCE and where they live

Each login generates a `state` and a PKCE verifier, 32 bytes each from the operating system's generator (43 base64url characters), and sends `code_challenge` with `code_challenge_method=S256` (`plain` cannot be expressed). `state` cannot be turned off. `.pkce(false)` turns the challenge off for a provider that refuses unknown parameters and logs a warning at startup.

You **choose where the state of a login lives**; choosing nothing stops `init()`:

| | `state_cookie_secrets(..)` / `state_cookie(keys)` | `state_in_session()` |
|---|---|---|
| Stored in | a cookie signed with `CookieKeys` (secrets of at least 32 bytes; use one of your own, not the session's) | the session of the request |
| Server state | none: a visitor who starts a login creates nothing | a session record per login start (rate limit the login route) |
| Needs | nothing else | `SessionModule` installed, session cookie not `SameSite=Strict` (startup error otherwise) |
| Single use | the cookie is deleted and expires; a client that keeps it can present the same record again within the lifetime, and this instance refuses the replayed `state` before any request to the provider (see below) | the record is removed before any request to the provider: a replay fails whatever the client sends |
| PKCE verifier | derived on the server from the `state` and the first cookie key: it is never in the cookie | stored in the session record |

The state cookie is `__Host-oauth2.<name>`, `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax` (the callback is a top-level cross-site navigation, so `Strict` would never be sent), no `Domain`, and lives as long as the state (600 s by default). The `__Host-` prefix makes a browser refuse a cookie set by a sibling subdomain, which closes login CSRF by cookie tossing. `.state_cookie_secure(false)` (plain-HTTP development) drops `Secure` and the prefix and logs a startup warning. A `state_cookie_name(..)` without the `__Host-` prefix loses that protection and logs a startup warning naming the strategy and the cookie (it is not an error: plain-HTTP development and applications with no untrusted subdomain are legitimate). After a key rotation, a login that started before it fails once (the verifier is derived with the first key).

With the session as the store, the same attack needs the session cookie to be set by someone else (a sibling subdomain, or a network attacker on plain HTTP): give the session cookie the `__Host-` prefix (`SessionOptions::name("__Host-sid")`) and keep it `Secure`. With the default name `connect.sid` the strategy logs a startup warning that says so.

**A flow is bound to the session it started under.** With the state in a cookie, a browser can be shared: user A starts an account link, logs out, user B logs in, and A's pending callback arrives. The flow therefore records a keyed digest of the session identifier of the request that started it (or a mark that there was none), and the callback is refused with the uniform failure when the session it arrives with differs: another session, a session where there was none, or none where there was one. The digest is a SHA-256 of an HMAC made with a state cookie key, so the identifier cannot be recovered from the cookie. The check is skipped when no `SessionModule` runs, a session that the start request itself creates (anything written to it during that request, or `save_uninitialized(true)`) makes a flow that began without one fail, and a key rotation between the redirect and the callback fails that login once.

**A flow is bound to the user that started it.** The session binding does not cover an identity that does not live in the session: a token or an API key. So the flow also records a keyed digest of the user the start request carried (or that there was none), with either state store, and a callback whose request carries a user completes only a flow that the same user started: another user's, or one that nobody started, gets the uniform failure. A callback that carries no user is a plain login and is not checked. For account linking this means: run the same guards on the route that starts the flow as on the callback (a link started on an unauthenticated route cannot be completed as a user), and link the provider account to `Request::user()` only after the guards have authenticated the request. The comparison is of the serialized user, so a user whose serialized form changes between the two requests (a field taken from the token's issue time, for example) makes its own flow fail and start again.

**A `state` is redeemed once per instance.** With the cookie store, a client that ignores the clearing cookie can present the same record again and again. Each strategy instance therefore keeps a bounded set of the verified states it has presented to the token endpoint (for the state lifetime, `redeemed_states_max(..)` entries, 100000 by default; when it is full the oldest are forgotten). A callback whose state is in the set, or is being redeemed right now, gets the uniform 401 before any request to the provider, so concurrent replays of one flow cost one token request. Only states that matched the browser's record enter the set, so forged callbacks grow nothing. This is a cost control per instance, not a guarantee across instances: a load balancer sends the replay to an instance that has not seen the state, where only the provider's refusal of the used code stops it. The session store removes the record itself, which several instances need a shared session store to do consistently.

**A repeated state cookie is no state.** A callback that carries two cookies with the state cookie's name is refused as a callback without a state (the 401), before any request to the provider, as the session cookie and the JWT cookie extractor already treat a repeated cookie. A login start with two such cookies begins from an empty set of flows.

A callback is checked in this order, and nothing reaches the provider until the last check passes: `code`, `state` and `error` are each absent or one string within their limits (`code` and `error` together fail); the flow of that `state` is taken out of the store (a forged state spends nothing of the browser's other flows); the flow must exist for this strategy and be inside its lifetime; its `state` equals the query's (constant time). A browser can have up to four logins in progress for a strategy (two tabs do not break each other); the oldest is dropped when a fifth starts, and an unclaimed flow expires with its lifetime.

### The redirect URI

`redirect_uri` is a required argument: an absolute URL (`https`, or `http` for a loopback host), without fragment or user information. The same string goes in the authorization and the token request. **Nothing is built from the request**: the original resolves a relative callback URL against the `Host` header, so a forged header decides where the provider sends the code. Corvis refuses a relative URI at startup and has no per-request redirect URI; a multi-domain application runs one strategy per domain.

### `return_to` and `failure_redirect`

`.return_to_param("return_to")` makes the **login start** read that query parameter and keep it only when it is one string of at most 1024 bytes that starts with a single `/` (the second character is not `/` or `\`) with no backslash, control character or non-ASCII byte (the rule of [the redirect URLs of a strategy](#writing-a-strategy); `is_safe_relative_path` is public). Anything else is dropped and the login goes on. The value travels in the state record, not in any URL the provider sees, and `oauth2::return_to(&req)` returns it in the callback handler of a successful login (never from the callback's own query). `.failure_redirect(url)` answers every failed callback, including one your validator refuses (`Ok(None)` or an empty user; without it that is a 401), with a 302 to a fixed URL (a relative path under the same rule, or an absolute `https` URL), nothing added from the request; errors of the provider or the configuration (500, 503) are not redirected.

### Failures

| The callback | Answer |
|---|---|
| not a `GET`; repeated, oversized or malformed parameters; `code` with `error` | 401 |
| no state record, another strategy's record, expired, `state` different | 401 |
| `error=access_denied` | 401 |
| any other `error` | 401 (the original: 500) |
| the token endpoint answers 400 or 401 (`invalid_grant`, a reused code), or 200 with an `error` | 401 (the original: 500) |
| concurrency limit reached | 503 `Service Unavailable`, body `{"statusCode":503,"message":"Service Unavailable"}` (the same body as every other 503 of the authentication module) |
| timeout, connect or TLS failure, redirect, 5xx or other status, oversized or unparseable answer, no `access_token`, unknown `token_type`, profile request failure | 500 `Internal server error` |
| `validate` returns `None` / an exception | 401 / that exception |

Provider text never reaches a response: a failure's challenge (`Request::auth_info()`, `AuthGuard::on_failure`) is `{"message": <fixed reason>}` with one of `state mismatch`, `access denied`, `provider error`, `code rejected`, `invalid callback`, plus `"error": <code>` only when the provider's code is 1 to 64 characters of `[A-Za-z0-9_.-]`. `error_description` and `error_uri` are dropped unread. The callback response has `Cache-Control: no-store` and `Referrer-Policy: no-referrer`; redirect from the handler so the `code` leaves the address bar.

### The provider client and its limits

Token, refresh and profile requests go through a private client over `hyper` and `tokio-rustls`: TLS 1.2 and 1.3 with the bundled web PKI roots (plus `OAuth2HttpOptions::extra_root_certificate(der)` for a private CA), host name verified, **no way to turn verification off**. `http` is accepted only for a loopback host whose resolved addresses are all loopback (a mock provider on `127.0.0.1`). A redirect is a failed request and is never followed, proxy variables are ignored, responses are never decompressed, and an error never holds text from the provider.

| Setting | Default | Bounds |
|---|---|---|
| `state_ttl` | 600 s | 60 s to 3600 s |
| PKCE | `S256`, on | `.pkce(false)` warns |
| `max_code_bytes` (callback `code`) | 2048 | 16 to 8192 |
| callback `state` | 43 base64url characters | fixed |
| connect timeout (TCP and TLS) | 5 s | 1 s to 30 s |
| total timeout per request | 10 s | 1 s to 60 s |
| token response | 64 KiB | 1 KiB to 1 MiB |
| each profile response | 256 KiB | 1 KiB to 4 MiB |
| response header block | 16 KiB | fixed |
| an access, refresh or ID token | 16 KiB | fixed |
| requests to the provider in flight, per strategy | 32 | 1 to 1024 (over: 503 at once) |
| in-flight slots one client address may hold (`max_in_flight_per_client`) | a quarter of the cap, at least 1 | 1 to 1024 (over: 503 for that address only) |
| redeemed states remembered per strategy (`redeemed_states_max`) | 100000 | 16 to 10000000 |
| scope | none (Google: required) | 64 values of 1 to 128 visible ASCII characters without space, `"` or `\` (GitHub: no `,` either) |
| extra authorization parameters | none | 16, names and values at most 256 bytes; `response_type`, `client_id`, `redirect_uri`, `scope`, `state`, `code_challenge`, `code_challenge_method` refused |
| client authentication | request body | `ClientAuth::Basic` (form-encoded id and secret, nothing in the body) |
| flows in progress per browser and strategy | 4 | fixed |

An option outside its bound stops `init()` with an error that names the strategy and the option and never holds the client secret.

### Refresh

`OAuth2Client::new(options)?` validates the options like the strategy and gives `refresh(refresh_token)`: `grant_type=refresh_token` with the client credentials (and `refresh_scope(..)` if set) through the same client and limits, returning an `OAuth2TokenSet`. Build it from the same `options()` as your validator. Nothing is stored and nothing refreshes by itself. A rejected refresh is an error whose text holds only the provider's sanitized code (`invalid_grant`).

### Logs, proxies and rate limits

The callback registers `code` and `state` as credential query parameters, so `LoggingInterceptor` and `Request::redacted_url()` hide them (a middleware that logs `Request::url()` does not: use `redacted_url()`). A reverse proxy's access log still records the callback URL; the `code` is single use and bound to the PKCE verifier. Log lines hold the strategy name, the provider's host, the status and the error kind, at most once a minute per reason, never a query, body, header, token or secret. **Rate limit the login route and the callback route, each per client** (see [rate limiting](/security/rate-limiting)): every login start is a request anyone can make (with the session as the store each creates a session record), and every callback that passes the state check spends a request at the provider and a slot of the in-flight cap. The cap (`max_in_flight`, 32 by default) answers 503 once the slots are taken, and the per-address share (`max_in_flight_per_client`, a quarter of the cap by default) keeps one client, found by the address the application resolves (so set `trust proxy` behind a proxy), from holding all of them; neither replaces a rate limit, which also protects your application's quota at the provider.

### Differences from `passport-oauth2`

- `state` and PKCE are always on; the original sends neither unless `state: true` and `pkce: true` are set with a session.
- The redirect URI is absolute and configured; the original resolves a relative one from `Host`.
- A provider `error` other than `access_denied`, and a token endpoint that rejects the code, answer 401 (the original 500).
- Profile requests send the token in an `Authorization: Bearer` header; the original sends Google's in the URL. They also send `Accept: application/json` and the client's own `User-Agent`.
- GitHub's scope is joined with commas, as `passport-github2` does; the e-mails keep every address of `/user/emails` as `{ value, primary, verified }` (the original: only the primary one as `{ value }`, or all with `allRawEmails`, adding `visibility`). A failed e-mail request fails the login, an empty list too.
- Every e-mail has a `verified` flag and `verified_email()` exists; the original omits the flag for GitHub.
- The token response is handed over without `access_token`, `refresh_token` and `id_token` in `params` (they are typed values of their own); the ID token is unverified.
- A callback with a state that is not this browser's answers 401 through `AuthGuard` (the original answers the 403 the strategy gave, through plain `passport`).

### Testing against the original

`crates/corvis/tests/fixtures/auth/oauth2.mjs` runs the four packages against a fake provider and records the authorization redirects, the token and profile requests, the profiles handed to `validate()`, 14 callback outcomes and the refresh request (`oauth2.json`); `crates/corvis/tests/auth_oauth2_original.rs` replays them against a mock provider on `127.0.0.1` with no Node and no network, and asserts the marked divergences from the specification. `examples/cats-app` has a GitHub login whose endpoints come from configuration, tested against a local mock provider.

## Differences from the original, by strategy

Every strategy keeps its own list of deliberate differences next to its options. This is the index; the
[migration page](/recipes/passport-migration#what-answers-differently-on-purpose) has the same differences as tables
(what the original does, what Corvis does and why), built from the cases the differential tests mark as divergences.

| Area | Where its list is |
|---|---|
| Strategy core and `AuthGuard` (startup errors, WebSocket gateways, the typed user, sessions opt-in) | [Differences from the original](#differences-from-the-original) |
| JWT | [Differences from `passport-jwt`](#differences-from-passport-jwt) |
| Local | [Differences from passport-local](#differences-from-passport-local) |
| API keys | [Differences from passport-headerapikey](#differences-from-passport-headerapikey) |
| OAuth2 | [Differences from `passport-oauth2`](#differences-from-passport-oauth2) |
| Sessions | [Divergences from the original](/http/session#divergences-from-the-original) |
| Signed cookies | [Signed cookies](/http/cookies#signed-cookies) |

## Differential testing

Every case above is recorded from the original: `crates/corvis/tests/fixtures/auth/strategies.mjs` runs the original passport integration (12.0.1) over `passport` 0.7.0 with custom `passport-strategy` 1.0.0 strategies driven to each outcome, and `tests/auth_strategies.rs` replays the recorded responses against the same routes in Corvis, with no Node needed at test time. Each strategy has its own recorded cases, described in its section ("Testing against the original").

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

Prefer the strategies above: they give you the startup checks, the uniform `401`, the deadlines, chains and the redacted request log. The path with a guard of your own is the right one when you need a rule no strategy expresses. The full flow with strategies (login, guard, current user, roles) is explained in [Authentication](/security/authentication) and [Authorization](/security/authorization).

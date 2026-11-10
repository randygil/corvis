---
status: partial
---

# Passport (authentication)

Corvis has the **strategy model** of Passport and its integration for the original framework: strategies registered by name, an `AuthGuard` that runs them, the user on the request and the `401` the original answers when every strategy fails. Its behavior is checked against the original passport integration (12.0.1) over `passport` 0.7.0 with recorded responses.

What is available now is the core: the `Strategy` trait you implement for your own strategies, `AuthGuard`, `PassportModule` and the `User<T>` parameter. The strategies themselves arrive one by one: JWT, local (username and password), API keys, sessions and OAuth2 are tracked in their own issues (#42, #41, #45, #44 and #43). Until then, [`JwtModule` and a guard of your own](#alternative-jwtmodule-guards) cover the common token case.

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
pub struct HeaderStrategy {
    users: Arc<UsersService>,
}

#[async_trait]
impl Strategy for HeaderStrategy {
    fn name(&self) -> &str {
        "header"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        let Some(user) = ctx.request().header("x-user").and_then(|id| self.users.find(id)) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match AuthenticatedUser::new(user) {
            Ok(user) => StrategyOutcome::Success { user, info: None },
            Err(error) => StrategyOutcome::Error(error),
        }
    }
}

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("header"))],
    providers = [Provider::strategy::<HeaderStrategy>()],
    controllers = [CatsController],
)]
pub struct AppModule;
```

`Provider::strategy::<S>()` registers the strategy under the name `S::name()`. The application collects every strategy when it initializes; two strategies with the same name stop startup with an error that names both types (the original keeps the last one silently).

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

- **Startup errors.** The original answers `500 Internal server error` on each request when a guard names an unknown strategy, and only logs an error when `AuthGuard()` has no default. Corvis stops `init()` before middleware `configure` and the init hooks, with one error that lists every route (method and path) and the missing name. This also covers an empty `AuthGuard::chain`, a default that is not registered, and two `PassportModule::register` calls with different defaults. A guard of your own that calls the `Authenticator` with an unknown name still gets the original's `500` at request time. A guard that wraps an `AuthGuard` can return it from `CanActivate::as_any` to be checked too.
- **No `AuthGuard` on WebSocket gateways.** A global `AuthGuard` also reaches gateway messages by default, where the original does not work either. It denies the message with `Forbidden resource` without running a strategy, and the application logs one warning per gateway at startup. Authenticate sockets in `handle_connection`, or call `apply_global_enhancers_to_gateways(false)`, which removes every global enhancer from gateways, not only the `AuthGuard`.
- **The typed user lives in its own slot.** `User<T>` reflects only what an `AuthGuard` stored, never another request extension of type `T`, and `Request::set_user` drops it, so a later guard that sets its own user makes `User<T>` answer `401`. When a strategy passes, or an optional guard finds no credentials, both the JSON user and the typed user are removed (`req.user` is `undefined` or `null` in the original).
- **No session login.** `AuthGuard` never logs in to a session by itself; sessions come with their own issue.
- **No `property`, `failureRedirect` or flash messages.** The user is always stored for `User<T>` and `request.user()`.

## Differential testing

Every case above is recorded from the original: `crates/corvis/tests/fixtures/auth/strategies.mjs` runs the original passport integration (12.0.1) over `passport` 0.7.0 with custom `passport-strategy` 1.0.0 strategies driven to each outcome, and `tests/auth_strategies.rs` replays the recorded responses against the same routes in Corvis, with no Node needed at test time. Each strategy kind that arrives later adds its own cases.

## Alternative: `JwtModule` + guards

Until the JWT strategy arrives, the pieces for authenticating with tokens are in the framework and cover the most common case, an API with JWT:

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

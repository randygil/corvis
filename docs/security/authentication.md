---
status: documented
---

# Authentication

Corvis does not impose an authentication strategy, but it ships the pieces for the most common one in APIs:
JWT tokens signed by the server and verified by a [guard](/guards). This page builds that complete
flow with `JwtModule`, `JwtService` and a `CanActivate` of your own, following the
`examples/cats-app` example.

The flow is as follows:

1. The client sends its credentials to `POST /auth/login`.
2. A service checks the credentials and signs a JWT with `JwtService::sign`.
3. On every later request the client sends `Authorization: Bearer <token>`.
4. A guard extracts the token, verifies it and leaves the *claims* on the request for the handler.

::: tip Requirement
`JwtModule` and `JwtService` are behind the `jwt` feature of the `corvis` crate, which is enabled
by default.
:::

## Registering `JwtModule`

`JwtModule::register` takes a `JwtModuleOptions` and returns a dynamic module that exports
`JwtService`. The usual approach is to read the secret from configuration with `register_async`, which takes
first whether the module is global and then an async factory with access to the container:

```rust
use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};

#[module(
    imports = [
        UsersModule,
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let secret = config
                .get::<String>("JWT_SECRET")
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
            Ok(JwtModuleOptions::new(secret).expires_in(config.get_or("JWT_EXPIRES_IN", "1h".to_string())))
        }),
    ],
    controllers = [AuthController],
    providers = [AuthService],
    exports = [AuthService],
)]
pub struct AuthModule;
```

With the first argument set to `true`, `JwtService` is visible across the whole application without importing the
module everywhere (see [Modules](/modules)). If the factory returns an error, the application does not
start: that is preferable to signing tokens with a known secret.

`JwtModuleOptions` options:

| Method | Effect |
|---|---|
| `JwtModuleOptions::new(secret)` | HMAC secret (`HS256` by default) |
| `.expires_in("15m")` | Adds `exp`. Accepts the grammar of the `ms` package (`"60s"`, `"1.5h"`, `"2 days"`); a number without a unit is seconds. A value that is not understood makes `sign` fail |
| `.algorithm(JwtAlgorithm::RS256)` | Algorithm (`corvis::jwt::JwtAlgorithm`, re-exported from `jsonwebtoken`) |
| `.private_key(pem)` / `.public_key(pem)` | PEM keys for `RS*`, `PS*`, `ES*` and `EdDSA`. Use `JwtModuleOptions::new("")`: a non-empty secret takes priority over the keys |
| `.issuer("api")` / `.audience("web")` | Adds `iss`/`aud` when signing and requires them when verifying |
| `.global(true)` | Only for `register`: registers the module as global |

## Signing the token

`JwtService` is injected like any provider. `sign` serializes the payload, adds `iat` (and `exp`
if you configured `expires_in`) and returns the token:

```rust
use corvis::JwtService;
use corvis::prelude::*;

#[injectable]
pub struct AuthService {
    users_service: Arc<UsersService>,
    jwt_service: Arc<JwtService>,
}

impl AuthService {
    pub async fn sign_in(&self, username: &str, pass: &str) -> HttpResult<Value> {
        let user = self.users_service.find_one(username).await;
        let Some(user) = user.filter(|u| u.password == pass) else {
            return Err(UnauthorizedException.into());
        };
        let payload = JwtPayload { sub: user.user_id, username: user.username, roles: user.roles };
        let token = self.jwt_service.sign(&payload).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "access_token": token }))
    }
}
```

::: warning Passwords
The example compares passwords in plain text so as not to distract from the flow. In a real application,
store a hash and compare it with a slow derivation function; see
[Encryption and hashing](/security/encryption-and-hashing).
:::

For per-call options there is `sign_with(&payload, JwtSignOptions { .. })`, with `expires_in`,
`secret`, `subject` and `private_key`. There are also `sign_async` and `verify_async`, which do the same
as their synchronous versions.

## The login endpoint

```rust
#[controller("auth")]
pub struct AuthController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[set_metadata(IS_PUBLIC_KEY, true)]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        self.auth_service.sign_in(&dto.username, &dto.password).await
    }

    #[get("profile")]
    fn get_profile(&self, user: CurrentUser) -> JwtPayload {
        user.0
    }
}
```

`#[http_code(HttpStatus::OK)]` changes the `201` that a `POST` returns by default. The `user: CurrentUser`
parameter has no attribute: it is resolved with `FromContext` (see below).

## The authentication guard

The guard reads the header with `extract_bearer_token`, verifies the token and attaches the payload to the
request:

```rust
use corvis::prelude::*;
use corvis::{JwtService, extract_bearer_token};

#[injectable]
pub struct AuthGuard {
    jwt_service: Arc<JwtService>,
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool = self
            .reflector
            .get_all_and_override(IS_PUBLIC_KEY, &[ctx.get_handler(), ctx.get_class()])
            .unwrap_or(false);
        if is_public {
            return Ok(true);
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or(UnauthorizedException)?;
        let payload: JwtPayload = self.jwt_service.verify_or_unauthorized(&token)?;
        request.set_user(&payload);
        Ok(true)
    }
}
```

- `verify::<T>(token)` checks the signature (only with the configured algorithm), `nbf`, `exp`
  (expired when `now >= exp`), `iss`/`aud` if you configured them, and deserializes the *claims* into
  `T`. It returns a `corvis::Result<T>`.
- `sign` and `verify` fail with `secretOrPrivateKey must have a value` when there is no key. `verify` also rejects a token that is not yet valid (`jwt not active`, from `nbf`) and one whose `exp`, `nbf` or `iat` is not a number.
- `verify_or_unauthorized::<T>(token)` does the same and turns any failure into a `401`.
- `decode(token)` returns the payload as a `Value` **without verifying the signature**. Do not use it to
  decide permissions.

There are two ways to store the user on the request:

- `request.set_user(&payload)` serializes it to JSON; it is read with `request.user_as::<JwtPayload>()` or
  `request.user()`.
- `request.insert(value)` stores a typed value (it must be `Clone`); it is read with
  `request.get::<T>()`. This is what `examples/tertulia` does.

## Enabling authentication on all routes

Registered as a provider with `Provider::app_guard`, the guard is applied to all controller routes and
can receive dependencies by injection:

```rust
#[module(
    imports = [AuthModule, UsersModule, CatsModule],
    providers = [Provider::app_guard::<AuthGuard>()],
)]
pub struct AppModule;
```

Open routes are marked with metadata that the guard checks through `Reflector`:

```rust
pub const IS_PUBLIC_KEY: &str = "isPublic";

#[get]
#[set_metadata(IS_PUBLIC_KEY, true)]
fn find_all(&self) -> Vec<Cat> { /* ... */ }
```

`#[set_metadata]` can also go below `#[controller]` to open a whole controller.

::: warning Scope of global guards
`Provider::app_guard` and `app.use_global_guards` only protect controller routes. They do not apply
to WebSocket gateways, to the `upgrade` listeners of `corvis::ws`, to static files or to
the Swagger/Scalar routes. Protect those surfaces separately.
:::

## Reading the user in the handler

A type of your own that implements `FromContext` works like a parameter decorator: you just
declare it as a handler argument, without an attribute.

```rust
pub struct CurrentUser(pub JwtPayload);

#[async_trait]
impl FromContext for CurrentUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request()
            .user_as::<JwtPayload>()
            .map(CurrentUser)
            .ok_or_else(|| UnauthorizedException.into())
    }
}
```

See [Custom decorators](/custom-decorators) for more details.

## Authentication in WebSockets

A browser cannot send headers when opening a WebSocket, so the token usually travels in the
query. `examples/tertulia` verifies it in `handle_connection` and closes the connection if it is not valid:

```rust
async fn handle_connection(&self, client: WsClient) {
    let handshake = client.handshake();
    let token = handshake.query_param("token").map(str::to_string).or_else(|| extract_bearer_token(handshake));
    let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
        client.disconnect_with(4401, "Unauthorized");
        return;
    };
    client.insert(user);
}
```

Tertulia's same `AuthGuard` works for HTTP and for WebSocket: it checks `ctx.get_type()` and, in
`ContextType::Ws`, reads the user stored on the client. See [WebSocket guards](/websockets/guards).

## What Corvis does not include

- There are no Passport-style strategies (local, OAuth, OpenID Connect) or server-side sessions. See
  [Passport](/recipes/passport) and [Session](/http/session).
- There are no built-in *refresh tokens*: you implement them by signing a second token with a different lifetime
  (`sign_with` with `JwtSignOptions { expires_in: Some("7d".into()), .. }`) and an endpoint that redeems it.

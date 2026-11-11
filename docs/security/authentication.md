---
status: documented
---

# Authentication

Authentication answers *who is making the request*. In Corvis it is done by **strategies**: small pieces that read a credential
from a request (a bearer token, a username and a password, an API key, a session cookie, the answer of an OAuth2 provider) and
turn it into a user. A [guard](/guards) called `AuthGuard` runs the strategies you name on a route, stores the user for the
handler and answers the plain `401` when none of them accepts the request. The model is the one of Passport and of its
integration for the original framework, and it is checked against them: see [Passport](/recipes/passport) for the reference and
[Migrating from Passport](/recipes/passport-migration) if you are bringing a setup over.

This page is the guide: how it works, which strategy to choose, one short path for each, how they combine, and what is not
included. Every option, default and difference lives in the reference, and every threat in the
[threat model](/security/authentication-threat-model); this page links to them instead of repeating them. What you must do
yourself before you deploy is in [Securing authentication](/security/authentication-security).

The samples come from `examples/auth-app`, a small application that is only about authentication and runs with
`cargo run -p auth-app -- --dev`, with no external account. Each sample starts with a comment that names its file; `// ...`
marks code left out.

## How it works

```text
request ──> ThrottlerGuard ──> AuthGuard::new("jwt") ──> your handler
            (global, first)        │
                                   ├─ the strategy "jwt" reads the credential  ──> validator ──> user
                                   ├─ success: the user is stored (User<T>, Request::user())
                                   └─ every strategy failed: 401 {"message":"Unauthorized","statusCode":401}
```

- A **strategy** reads one kind of credential. You do not write it: you write the **validator**, the one method that decides
  whether the credential belongs to a user (the `verify` callback of Passport), and register the strategy with
  `Provider::strategy::<JwtStrategy<YourValidator>>()`.
- `AuthGuard::new("jwt")` goes on a controller or a route (or globally, see [below](#protecting-everything-by-default)) and
  runs the strategy of that name.
- The handler reads the user with the `User<T>` parameter.
- A validator returns `Ok(Some(user))`, `Ok(None)` (the credential is refused) or an error. **Say "no user" with `None`, never
  with a value that serializes to `null`, `false`, `0` or `""`**: such a user never authenticates, and the guard logs a warning
  that tells you which validator to fix ([why](/recipes/passport#writing-a-strategy)).

You can also [write a strategy](/recipes/passport#writing-a-strategy) of your own for any other credential.

## Choosing a strategy

| Strategy | For | The credential travels in | Cargo feature | Reference |
|---|---|---|---|---|
| `jwt` | single-page apps, mobile apps and APIs that send a token | the `Authorization: Bearer` header | `jwt` (on by default) | [JWT](/recipes/passport#the-jwt-strategy) |
| `local` | the login form: it exchanges a username and a password for a token or a session | the body of the login request | `auth-local` | [Local](/recipes/passport#local-strategy-username-and-password) |
| `api-key` | machine clients: webhooks, internal services, partners | a header such as `X-Api-Key` | `auth-api-key` | [API keys](/recipes/passport#api-keys) |
| `session` | browsers on the same site as the server | a signed cookie | `auth-session` | [Sessions](/recipes/passport#sessions) |
| OAuth2 (any provider, with presets for Google and GitHub) | "Sign in with ..." | a redirect to the provider and back | `auth-oauth2` | [OAuth2](/recipes/passport#oauth2-google-github-and-others) |

A few rules of thumb:

- An API consumed by code you do not control uses `jwt`, and `api-key` for the services that call it.
- A browser application served from your own domain uses `session`: the server can end a session, a token cannot be taken back
  before it expires. A cookie is an ambient credential, so it needs the [CSRF](#cookies-and-csrf) care below.
- `local` and OAuth2 are *login* strategies. They check who someone is once and then hand over to a token or a session; the
  example shows both outcomes.
- You rarely use only one. [Combining strategies](#combining-strategies) shows how a route accepts several.

## JWT

Enable nothing: the `jwt` feature is on by default. The validator receives the payload of a token whose signature, algorithm
and expiry were already verified:

```rust
// examples/auth-app/src/auth/jwt.rs
#[injectable]
pub struct JwtAuth {
    settings: Arc<Settings>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.settings.jwt_secret.clone()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        // ...
        let claims: Option<Claims> = corvis::serde_json::from_value(payload).ok();
        Ok(claims.map(|c| Account { id: c.sub, username: c.username, roles: c.roles }))
    }
}
```

Register the strategy next to its validator, and put the guard on a route:

```rust
// examples/auth-app/src/app_module.rs
#[module(
    // ...
    providers = [
        // ...
        JwtAuth,
        Provider::strategy::<JwtStrategy<JwtAuth>>(),
        // ...
    ],
)]
pub struct AppModule;
```

```rust
// examples/auth-app/src/controllers.rs
    #[get("me")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }
```

```sh
curl -s http://127.0.0.1:3000/me -H "Authorization: Bearer $TOKEN"
# {"id":1,"username":"john","roles":["admin"]}
curl -s http://127.0.0.1:3000/me
# {"message":"Unauthorized","statusCode":401}
```

Tokens are signed by `JwtService` (the example gives them a short life; a stolen token works until it expires, and there is
no revocation). Where the token is read from, the algorithms, the key providers, the issuer and audience checks and every
difference from `passport-jwt` are in [the JWT reference](/recipes/passport#the-jwt-strategy). What `JwtService` itself does
not check by default is tracked in #174; to authenticate requests use the
strategy, not `JwtService::verify`.

## Username and password

The `local` strategy reads the username and the password from the body of the login request. Your validator checks them
against the stored hash and returns the user. It must spend one real verification whether the user exists or not, so the
response time does not say which usernames exist, and it replaces hashes made by an older algorithm after a successful login:

```rust
// examples/auth-app/src/auth/local.rs
#[async_trait]
impl LocalValidator for LocalAuth {
    type User = Account;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        let found = self.users.credentials(username);
        let stored = found.as_ref().map(|(_, hash)| hash.as_str());
        // ...
        let ok = self.hasher.verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        let Some((account, hash)) = found.filter(|_| ok) else { return Ok(None) };
        if self.hasher.needs_rehash(&hash) {
            let upgraded = self.hasher.hash(password).await.map_err(PasswordError::into_http_exception)?;
            self.users.set_password_hash(account.id, upgraded);
        }
        Ok(Some(account))
    }
}
```

The login route puts the strategy first and the [throttler](/security/rate-limiting#protecting-a-login-route) before it. The
handler runs only when the credentials were right, and it issues the token:

```rust
// examples/auth-app/src/controllers.rs
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<Account>) -> HttpResult<Value> {
        Ok(json!({ "access_token": issue_token(&self.jwt, &user)? }))
    }
```

```sh
curl -s -X POST http://127.0.0.1:3000/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"john","password":"john-demo-pass"}'
# {"access_token":"eyJ..."}
```

A wrong password, an unknown user and a missing field all answer the same `401`. The hash algorithms, `needs_rehash`, what to
set while most stored hashes are still bcrypt, and the blocking methods that are only for code outside the Tokio runtime are
in [Encryption and hashing](/security/encryption-and-hashing); the credential places, limits and differences from
`passport-local` are in [the local reference](/recipes/passport#local-strategy-username-and-password).

## API keys

A machine client sends a long random key in a header. The validator compares it as a digest, in constant time, and answers
`None` for a wrong key (`then_some`, never a `Some(false)`):

```rust
// examples/auth-app/src/auth/api_key.rs
#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = ServiceCaller;

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<ServiceCaller>, HttpException> {
        // ...
        Ok(self.keys.expected.matches(key).then_some(ServiceCaller { service: "reports" }))
    }
}
```

The example registers it as `Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>()` and accepts it on a route for people and
machines at once (a [chain](#combining-strategies)):

```rust
// examples/auth-app/src/controllers.rs
    #[get("reports")]
    #[use_guards(AuthGuard::chain(["jwt", "api-key"]))]
    fn reports(&self, req: Request) -> Value {
        json!({ "caller": req.user() })
    }
```

```sh
curl -s http://127.0.0.1:3000/reports -H "X-Api-Key: $KEY"
# {"caller":{"service":"reports"}}
```

Keep the key in a header: a key in the query string is opt-in, ends up in access logs, and is hidden from the request log only
for the parameters a strategy declares. Store only a digest of each key. Where keys can be read from, the limits and how to look
up many keys are in [the API key reference](/recipes/passport#api-keys).

## Sessions

A session keeps the user on the server and gives the browser a signed cookie. Two pieces: a `SessionSerializer` that says what
is stored for the user (the id, never the whole user) and how to read it back, and a login route that logs in with
`.log_in()`:

```rust
// examples/auth-app/src/auth/session.rs
#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { return Ok(None) };
        Ok(self.users.find_by_id(id))
    }
}
```

```rust
// examples/auth-app/src/controllers.rs
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(HttpStatus::NO_CONTENT)]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("session"))]
    async fn logout(&self, req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
```

```sh
curl -s -c jar -X POST http://127.0.0.1:3000/session/login -H 'Content-Type: application/json' \
  -d '{"username":"maria","password":"maria-demo-pass"}'
# {"username":"maria"}
curl -s -b jar http://127.0.0.1:3000/session/me
# {"id":2,"username":"maria","roles":["user"]}
```

Logging in changes the session identifier, which prevents session fixation. Two rules matter:

- **A login made by a guard is kept only when the request is not rejected afterwards.** A route that has a second step (a
  second factor, a terms page) must not use `.log_in()`: authenticate with the strategy and call `Authenticator::log_in` after
  the last step, see [Session](/http/session#login-and-logout).
- **The session user is visible on routes that have no `AuthGuard`.** A guard of your own that reads `Request::user()` there
  is authenticated by the cookie alone. See [Combining strategies](#combining-strategies) and
  [CSRF](#cookies-and-csrf).

The store of the example is the memory of the process, which is lost on restart and not shared between instances. The cookie
options, the secrets and their rotation, the store contract and the timeouts are in [Session](/http/session).

## OAuth2

"Sign in with a provider" is the authorization code flow, with `state` and PKCE always on. Two routes carry the same guard: the
one that redirects to the provider and the callback. The validator receives what the provider gave and returns the user:

```rust
// examples/auth-app/src/auth/provider.rs
#[async_trait]
impl OAuth2Validator for ProviderAuth {
    const NAME: &'static str = "provider";
    type User = Account;

    fn options(&self) -> OAuth2Options {
        let client = &self.settings.oauth;
        let endpoints = OAuth2Endpoints::new(&client.authorize_url, &client.token_url).userinfo_url(&client.userinfo_url);
        OAuth2Options::new(endpoints, &client.client_id, &client.client_secret, &client.redirect_uri)
            .scope(["openid", "email", "profile"])
            // ...
            .state_cookie_secrets([self.settings.oauth_state_secret.clone()])
            .state_cookie_secure(!self.settings.dev)
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<Account>, HttpException> {
        let Some(profile) = grant.profile else { return Ok(None) };
        // ...
        let name = profile.display_name.as_deref().unwrap_or("user");
        Ok(Some(self.users.for_provider_user(&profile.provider, &profile.id, profile.verified_email(), name)))
    }
}
```

```rust
// examples/auth-app/src/controllers.rs
    #[get]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn start(&self) -> Value {
        json!({})
    }

    #[get("callback")]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn callback(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }
```

The rules that this code follows, and that you must follow in yours:

- **Identify a user by `(provider, id)`, never by e-mail alone.** An address is a claim, and one that changes hands. Link a
  provider user to an account that already exists only through `verified_email()`, which holds the address only when the
  provider says it verified it. The example does exactly that in `src/users.rs`, and a test pins both cases.
- **Run the same guards on the route that starts a flow and on the callback.** A flow completes only under the session, and for
  the user, that started it. This is what stops a login (or an account link) started by one user from being completed as
  another on a shared browser.
- Keep the state of a login in a cookie signed with a secret of its own, give both routes a rate limit, and keep OAuth2
  strategies out of chains (they start a login for any request that is not a callback).
- The `id_token` that some providers return is handed over unverified: read no claim from it.

The example needs no account anywhere: `--dev` starts a mock provider on `127.0.0.1`, and pointing the `OAUTH_*` settings at a
real one runs the same code (see its README). The state and PKCE mechanics, the redirect URI rules, `return_to`, what every
failure answers and the limits are in [the OAuth2 reference](/recipes/passport#oauth2-google-github-and-others).

## Combining strategies

**A chain** tries strategies in order: the first that succeeds decides the user, a failure moves on to the next, and an error
(a database that is down, a provider that failed) stops the chain and is the answer, so an outage never falls through to a
weaker strategy.

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

**A session cookie and a token on the same request.** Whichever strategies the route's `AuthGuard` lists decide the user:
`AuthGuard::new("jwt")` answers `401` to a cookie on its own, `AuthGuard::chain(["jwt", "session"])` accepts either (the token
first), and an `AuthGuard` that succeeds replaces the user the session had restored. Say on each route which credentials it
accepts.

**Optional authentication** personalizes a route that anyone can call. An optional guard lets the request through when every
strategy *failed*, and an error still ends it. It is not access control, so the route is marked `#[public]` too:

```rust
// examples/auth-app/src/controllers.rs
    #[get("feed")]
    #[public]
    #[use_guards(AuthGuard::chain(["jwt", "session"]).optional())]
    fn feed(&self, req: Request) -> Value {
        match req.user_as::<Account>() {
            Some(user) => json!({ "message": format!("Hello, {}", user.username) }),
            None => json!({ "message": "Hello, stranger" }),
        }
    }
```

**A default strategy.** `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` lets a route write
`AuthGuard::default()`. An application with one main strategy uses it; one with several names them on each route.

**Order of guards.** Guards run global first, then the controller's, then the route's, each left to right. Two rules follow:

- An `AuthGuard` goes **before** any guard that reads the user. The roles guard of the example is listed after the chain, and
  [Authorization](/security/authorization) shows why: on a route with no `AuthGuard`, `Request::user()` is whatever the session
  middleware restored from the cookie, which nothing about the request has checked.
- The throttler goes **before** the `AuthGuard`, so a request over the limit costs no password hash. Register
  `ThrottlerGuard` with `Provider::app_guard` and startup warns when a route runs it after an `AuthGuard`. See
  [Rate limiting](/security/rate-limiting#protecting-a-login-route).

**Token or session for the login routes?** The same `local` strategy serves both. A login route that issues a token returns it
in the body and logs nothing in; one that opens a session uses `.log_in()` and answers with a cookie. A single-page application
that also needs the cookie's revocation uses the session; a mobile client uses the token.

## Protecting everything by default

Most applications want every route authenticated except a few. Register one global `AuthGuard` built with `.skip_public()`, and
mark the open routes `#[public]`:

```rust
app.use_global_guards(AuthGuard::new("jwt").skip_public());
```

```rust
#[get]
#[public]
fn find_all(&self) -> Vec<Cat> { /* ... */ }
```

`.skip_public()` is what makes `#[public]` mean something to the guard: without it, public routes are authenticated like any
other. A global guard trusts every `#[public]` mark, so review them. To make the application refuse to start when a route has
neither a guard nor the mark, turn on [route posture](/security/route-posture). `examples/auth-app` has no global guard and
enforces the posture instead, so each strategy shows on its own route; `examples/cats-app` shows the global pattern.

`AuthGuard` denies WebSocket messages (see [below](#websockets)), so a global guard is not the way to protect a gateway.

## Reading the user

- `User<T>` in the handler's parameters is the typed value the guard stored. On a route where there is none (no guard, or an
  optional guard that found no credentials) it answers the same `401`.
- `Request::user()` is the JSON form, and `Request::user_as::<T>()` deserializes it. This is what a guard of your own reads.
- `Request::auth_info()` is the reason of a failed attempt or the info of a success. Never echo it to the client.

## Authorization

Authentication says who; what the user may do is [Authorization](/security/authorization): a guard listed after the `AuthGuard`
that reads the user, as `RolesGuard` does in the example.

## Cookies and CSRF

A session cookie, or a token you keep in a cookie, is sent by the browser on its own. That is an ambient credential, and it
brings two things to do:

- **Refuse state-changing requests that come from another site.** `SameSite=Lax` does not stop *login* CSRF: a cross-site form
  `POST` to your login route is answered, and the browser stores the cookie that logs the victim in as the attacker. The example
  puts a `SameOriginGuard` (in `src/same_origin.rs`) before the credentials are checked; its test sends the cross-site request.
  See [CSRF protection](/security/csrf).
- **Prefix the cookies with `__Host-`.** The prefix makes a browser refuse a cookie set from a sibling subdomain (cookie
  tossing). The session cookie, the OAuth2 state cookie and a cookie that holds a JWT should all have it, and a request that
  carries the same cookie name twice is treated as having no credential. Only `--dev`, which runs over plain HTTP, drops it.

## WebSockets

`AuthGuard` denies every WebSocket message with `Forbidden resource` and does not run a strategy: a global `AuthGuard`
therefore blocks a public gateway, and the application warns once per gateway at startup. Strategies do not run at the
WebSocket handshake either (#173): a browser cannot send headers when it opens
a socket, so the gateway authenticates in `handle_connection`, as `examples/tertulia` does, and closes the connection when the
token is not valid:

```rust
// examples/tertulia/src/chat/chat_gateway.rs
    async fn handle_connection(&self, client: WsClient) {
        let handshake = client.handshake();
        let token = handshake.query_param("token").map(str::to_string).or_else(|| extract_bearer_token(handshake));
        let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
            // ...
            client.disconnect_with(CLOSE_UNAUTHORIZED, "Unauthorized");
            return;
        };
        client.insert(user.clone());
        // ...
    }
```

Until strategies run at the handshake, `JwtService::verify` is looser than the JWT strategy (#174):
apply the strategy's rules (an expiry, a minimum secret length) in the handler. A token in the query string reaches your
proxy's logs, so prefer a header or a first message. In Socket.IO, a connect middleware on the main namespace does not cover
the other namespaces (#178): register it on every namespace that has handlers.
The [WebSocket guards](/websockets/guards) page covers guards on messages.

## Without strategies

The strategies are the main path, but the pieces under them stay available: `JwtModule`, `JwtService::sign` and `verify`,
`extract_bearer_token` and a `CanActivate` of your own that leaves the user on the request. That path is described in
[Passport: the alternative](/recipes/passport#alternative-jwtmodule-guards). Prefer a strategy: it gives you the startup
checks, the uniform `401`, the deadlines, the chains and the redacted logs for free.

## What is not included

- **OpenID Connect.** The OAuth2 strategy does the authorization code flow only, and the ID token is handed over unverified.
- **CSRF tokens.** Nothing generates or checks synchronized tokens; see [CSRF protection](/security/csrf).
- **Multi-factor authentication, account lockout, password policies and breached-password checks.** They are application
  policy. A second factor is a second step that calls `Authenticator::log_in` last; the throttler can limit attempts per account
  but locks nothing.
- **Refresh tokens.** There is no store: sign a second token with a longer life and an endpoint that redeems it. `OAuth2Client`
  can refresh a provider's token, and nothing refreshes by itself.
- **A session store other than memory.** Redis and ORM stores are follow-up work (#170,
  #171); write the store contract meanwhile (see [Session](/http/session#writing-a-store)).
- **A redirect URI allow-list** for OAuth2 (#172).
- **Strategies at the WebSocket handshake** (#173).
- **A response cache that is safe for authenticated routes.** Do not register `CacheInterceptor` for the whole application when
  it has authenticated `GET` routes: it keys by URL, so it replays one user's response to another
  (#177, see [Caching](/data/caching)).

Next: [Securing authentication](/security/authentication-security) before you deploy, the
[threat model](/security/authentication-threat-model) if you judge the module, and
[Migrating from Passport](/recipes/passport-migration) if you are bringing a setup over.

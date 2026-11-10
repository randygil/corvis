# auth/strategies Specification

## Purpose
Named authentication strategies and `AuthGuard`, with the HTTP behavior of `AuthGuard(name)` of `@nestjs/passport`
12.0.1 over `passport` 0.7.0 in every outcome, except where this specification diverges on purpose for safety (each
divergence is named in its requirement). Strategy kinds (JWT, local, API key, sessions, OAuth2) build on this
capability in their own specifications.

## Requirements

### Requirement: Strategies are registered by name
The framework SHALL collect every provider registered with `Provider::strategy::<S>()` into a strategy registry when
the application is initialized, keyed by the name `S` reports through `Strategy::name`. Strategies MUST receive their
dependencies through dependency injection like any other provider, and the registry MUST be reachable from any module.

#### Scenario: Strategy found by name
- **WHEN** a module registers `Provider::strategy::<HeaderStrategy>()` whose name is `"header"`, `HeaderStrategy`
  injects a `UsersService`, and a route of another module uses `AuthGuard::new("header")`
- **THEN** a request to that route is authenticated by `HeaderStrategy` with its injected `UsersService`

#### Scenario: Two strategies of the same type with different names
- **WHEN** two strategies are registered under the names `"a"` and `"b"`, and two routes use `AuthGuard::new("a")` and
  `AuthGuard::new("b")`
- **THEN** each route is authenticated by its own strategy

### Requirement: Duplicate strategy names fail at startup
Application initialization MUST fail when two registered strategies report the same name, with an error naming the
name and both strategy types. This is a documented divergence: the original keeps the last registration silently.

#### Scenario: Duplicate name
- **WHEN** two strategy providers both report the name `"jwt"`
- **THEN** `init()` returns an error that names `"jwt"` and both strategy types, and the application does not start

### Requirement: Unknown strategy names fail at startup
Application initialization MUST fail, with an error naming the route (method and path) and the missing name, when an
`AuthGuard` attached to a route (global, controller or route level) names a strategy that is not registered. The
check MUST run before middleware `configure`, gateway `after_init` and the init hooks. This is a documented
divergence: the original answers 500 at request time.

#### Scenario: Typo in a guard
- **WHEN** a route `GET /cats` uses `AuthGuard::new("jtw")` and no strategy is named `"jtw"`
- **THEN** `init()` returns an error naming `GET /cats` and `"jtw"`, and no init hook has run

#### Scenario: Name resolved only at request time
- **WHEN** a user-written guard calls `Authenticator::authenticate` with a name that is not registered
- **THEN** that request answers 500 with the body `{"statusCode":500,"message":"Internal server error"}`, as the
  original does for an unknown strategy

### Requirement: Guards without usable names fail at startup
Application initialization MUST fail, naming the route, when an `AuthGuard::default()` is attached and no default
strategy is configured, when the configured default strategy is not registered, or when `AuthGuard::chain` is given
no names. The original only logs an error for a missing default.

#### Scenario: Empty chain
- **WHEN** a route uses `AuthGuard::chain(Vec::<String>::new())`
- **THEN** `init()` returns an error naming the route

#### Scenario: Default guard without a default strategy
- **WHEN** a route uses `AuthGuard::default()` and `PassportModule::register` was not imported
- **THEN** `init()` returns an error naming the route and saying that no default strategy is configured

### Requirement: Default strategy
`PassportModule::register(PassportModuleOptions)` SHALL configure the default strategy name, or an ordered list of
names, used by `AuthGuard::default()`, as `PassportModule.register({ defaultStrategy })` does.

#### Scenario: Default strategy used
- **WHEN** the application imports `PassportModule::register(PassportModuleOptions::new().default_strategy("header"))`
  and a route uses `AuthGuard::default()`
- **THEN** the route is authenticated by the strategy named `"header"`

### Requirement: Strategy outcomes
A strategy SHALL end each authentication attempt with exactly one of five outcomes, the equivalents of passport's
`success`, `fail`, `redirect`, `pass` and `error`: success with a user and optional info; failure with an optional
challenge and an optional status; redirect with a URL and an optional status; pass; or error with an exception.

#### Scenario: Each outcome is reachable from a custom strategy
- **WHEN** a custom strategy returns each of the five outcomes in turn for the same route
- **THEN** the responses equal the recorded responses of a `passport-strategy` 1.0.0 subclass calling `success`,
  `fail`, `redirect`, `pass` and `error` behind `AuthGuard` of the original

### Requirement: Successful authentication exposes the user to the handler
When a strategy succeeds, `AuthGuard` SHALL store the user on the request both as a typed value (readable through a
`User<T>` parameter extractor, which reads only the value an `AuthGuard` stored and never another request extension of
the same type) and as JSON (readable with `Request::user` and
`Request::user_as`), SHALL store the strategy's info as the request's auth info (`Request::auth_info()`), and SHALL
let the route continue.

#### Scenario: Foreign extension is not a user
- **WHEN** a guard that is not an `AuthGuard` inserts a `String` into the request and the handler takes `User<String>`
- **THEN** the response is the 401 `Unauthorized` response

#### Scenario: Handler reads the user
- **WHEN** a strategy succeeds with a user `{ "id": 7, "role": "admin" }` and info `{ "scope": "read" }`
- **THEN** the handler receives the same user through `User<AuthPayload>` and through
  `Request::user_as::<AuthPayload>()`, `Request::auth_info()` is `{ "scope": "read" }`, and the response of a handler
  returning `{ user, authInfo }` matches the recorded original handler returning `{ user: req.user, authInfo:
  req.authInfo }`

### Requirement: User extraction and serialization failures
A `User<T>` extractor on a request without a user of type `T` MUST answer the 401 of the failure requirement. A user
that a strategy returns and that cannot be serialized to JSON MUST make the request fail with the 500
`Internal server error` response instead of continuing without a JSON user.

#### Scenario: Extractor without a user
- **WHEN** a handler declares a `User<AuthPayload>` parameter and the route has no `AuthGuard`
- **THEN** the response is status 401 with body `{"message":"Unauthorized","statusCode":401}`

#### Scenario: User that does not serialize
- **WHEN** a strategy succeeds with a user whose `Serialize` implementation returns an error
- **THEN** the response is status 500 with body `{"statusCode":500,"message":"Internal server error"}` and the handler
  is not called

### Requirement: Failed authentication answers the original 401
When every strategy of an `AuthGuard` fails, the response MUST be status 401 with body
`{"message":"Unauthorized","statusCode":401}`, MUST NOT include a `WWW-Authenticate` header, and MUST ignore any
status and challenge a strategy attached to its failure (for example the 400 of missing local credentials), exactly
as the original `AuthGuard` does. The 401 MUST go through the exception filters like any guard exception, and the
request's auth info MUST be the failure's challenge.

#### Scenario: Strategy failure with its own status
- **WHEN** a strategy fails with status 400 and challenge `{ "message": "Missing credentials" }`
- **THEN** the response is status 401, body `{"message":"Unauthorized","statusCode":401}`, with no
  `WWW-Authenticate` header, identical to the recorded original response

#### Scenario: Exception filter sees the failure
- **WHEN** a global exception filter catches `UnauthorizedException` and answers 401 with `{"error":"login"}`, and an
  `AuthGuard` fails
- **THEN** the response is the filter's, identical to the recorded original with the same filter

### Requirement: Errors raised by a strategy pass through unchanged
When a strategy reports an error, `AuthGuard` MUST stop and pass that exception on unchanged: an `HttpException`
keeps its status and body, and an internal error becomes the 500 `Internal server error` response. Later strategies
of a chain MUST NOT run.

#### Scenario: Strategy reports a specific exception
- **WHEN** the strategy reports `ForbiddenException::new("Account locked")`
- **THEN** the response is status 403 with body `{"message":"Account locked","error":"Forbidden","statusCode":403}`,
  identical to the original's when `validate()` throws `new ForbiddenException('Account locked')`

#### Scenario: Strategy reports an internal error
- **WHEN** the strategy reports an internal error
- **THEN** the response is status 500 with body `{"statusCode":500,"message":"Internal server error"}`, identical to
  the original's when `validate()` throws `new Error('db down')`

### Requirement: Chained strategies are tried in order
`AuthGuard::chain([...])` SHALL try the named strategies in the given order: the first success wins and later
strategies are not run; a failure moves on to the next strategy; an error stops the chain and is passed on; a pass or
a redirect ends the chain with that outcome; when all fail, the 401 of the failure requirement is sent.

#### Scenario: Second strategy succeeds
- **WHEN** a route uses `AuthGuard::chain(["a", "b"])`, `"a"` fails and `"b"` succeeds
- **THEN** the handler runs with the user of `"b"`

#### Scenario: First strategy errors
- **WHEN** the first strategy of a chain reports an error and the second would succeed
- **THEN** the second strategy is not run and the error is the response

### Requirement: Auth info of a failed chain
When more than one strategy of a chain failed, the request's auth info SHALL be the list of the strategies'
challenges in order, with `null` for a failure without a challenge, as in the original.

#### Scenario: Auth info of a failed chain
- **WHEN** a route uses `AuthGuard::chain(["a", "b"]).optional()`, `"a"` fails with challenge `"no token"` and `"b"`
  fails without a challenge
- **THEN** `Request::auth_info()` is `["no token", null]`, as `req.authInfo` in the recorded original

### Requirement: Pass and optional authentication
When a strategy passes, `AuthGuard` SHALL let the route run with no JSON user set by this guard (a JSON user stored
earlier on the request is removed, as the original assigns `undefined` to `req.user`). `AuthGuard::optional()` SHALL
let the route run with no JSON user when all strategies fail, the usual override of `handleRequest` in the original;
errors MUST still be passed on.

#### Scenario: Optional guard without credentials
- **WHEN** a route uses `AuthGuard::new("a").optional()` and `"a"` fails
- **THEN** the handler runs, `Request::user()` is `None`, and the response matches the original route whose guard
  overrides `handleRequest` to return `user ?? null`

#### Scenario: Strategy passes
- **WHEN** the strategy passes
- **THEN** the handler runs, `Request::user()` is `None`, and the response matches the recorded original

### Requirement: Custom failure exception
`AuthGuard::on_failure(f)` SHALL replace the 401 of a failed authentication with the exception `f` builds from the
list of failures (strategy name, challenge and status of each), the other usual override of `handleRequest`.

#### Scenario: Custom failure exception
- **WHEN** a route uses `AuthGuard::new("a").on_failure(|_| UnauthorizedException::new("Token required"))` and `"a"`
  fails
- **THEN** the response is status 401 with body `{"message":"Token required","error":"Unauthorized","statusCode":401}`

### Requirement: Redirect outcome
When a strategy redirects, `AuthGuard` MUST end the request with a response of status 302 (or the status the strategy
gives), a `Location` header with the URL exactly as given, `Content-Length: 0` and an empty body, through
`HttpArgumentsHost::respond_with`, and MUST NOT run later guards, interceptors, pipes or the handler. A URL that is not
a valid header value MUST make the request fail with the 500 `Internal server error` response, as Node's
`res.setHeader` throws for it. A status outside 300-399, or a redirect outside the guard phase (see `respond_with`),
MUST make the request fail the same way.

#### Scenario: Strategy redirects
- **WHEN** a custom strategy redirects to `https://idp.example/authorize?x=1`
- **THEN** the response is status 302, `Location: https://idp.example/authorize?x=1`, `Content-Length: 0`, empty
  body, the handler is not called, and the response equals the recorded original

#### Scenario: Redirect status outside 3xx
- **WHEN** a custom strategy redirects with status 200
- **THEN** the response is the 500 `Internal server error` response and the handler is not called

#### Scenario: Redirect outside the guard phase
- **WHEN** an `AuthGuard` whose strategy redirects is run from an interceptor
- **THEN** the request answers 500 and the handler is not called

#### Scenario: Redirect URL with a line break
- **WHEN** a custom strategy redirects to `"/a\r\nSet-Cookie: x=1"`
- **THEN** the response is the 500 `Internal server error` response and carries no `Set-Cookie`

### Requirement: Public routes can be skipped
`AuthGuard::skip_public()` SHALL let a request through without running any strategy when the route is public by the
rule of the route posture specification ("Public resolution order": the handler's boolean `PUBLIC_METADATA` value, else
the controller's, as `Reflector::get_all_and_override` resolves it). Without `skip_public()`, `AuthGuard` MUST
authenticate public routes like any other route, as the original `AuthGuard` does.

#### Scenario: Global guard and a public route
- **WHEN** the application calls `use_global_guards(AuthGuard::new("a").skip_public())` and a handler is marked
  `#[public]`
- **THEN** a request without credentials to that handler runs it, and the same request to an unmarked handler answers
  the 401

#### Scenario: Handler overrides a public controller
- **WHEN** a controller is marked `#[public]` and one of its handlers has `#[set_metadata(PUBLIC_METADATA, false)]`,
  under `AuthGuard::new("a").skip_public()`
- **THEN** that handler requires authentication

### Requirement: AuthGuard and route posture
`AuthGuard` SHALL count as a guard that controls access for the route posture check, except when built with
`.optional()`, which lets every request through and MUST report that it does not control access.

#### Scenario: Optional guard does not protect a route
- **WHEN** route posture is enforced and the only guard of a route is `AuthGuard::new("a").optional()`
- **THEN** `init()` fails listing that route as unprotected

### Requirement: AuthGuard fails closed outside HTTP
In a WebSocket context, `AuthGuard` MUST deny the message without running any strategy, and the framework SHALL log
one warning per gateway at startup (not per connection or message) naming the gateway and the guard. Handshake
authentication through strategies is not supported.

#### Scenario: Global AuthGuard applied to a gateway
- **WHEN** the application calls `use_global_guards(AuthGuard::new("a"))`, global enhancers apply to gateways (the
  default), and a client sends a message to a gateway handler
- **THEN** the message is denied with `Forbidden resource` and no strategy runs

### Requirement: Each strategy is verified against the original
Every strategy kind of the epic MUST have a differential test: a Node generator in `crates/corvis/tests/fixtures/auth/`
with `@nestjs/passport` 12.0.1, `passport` 0.7.0 and the strategy's reference package at an exact version records
request/response pairs into a JSON fixture, and a Rust test replays every case against an equivalent Corvis app,
comparing status, the headers named in the fixture and the body.

#### Scenario: Fixture replay
- **WHEN** `cargo test --workspace` runs
- **THEN** every recorded case of every auth fixture is replayed without Node, and a difference from the recorded
  original fails the test

#### Scenario: Marked divergence
- **WHEN** a fixture case carries a `divergence` field with its reason
- **THEN** the Rust test asserts the behavior this specification states for that case instead of the recorded one

### Requirement: Composite guards expose inner guards to the startup check
A guard SHALL be able to list the guards it delegates to (`CanActivate::inner_guards`); the startup check of strategy names MUST look inside them, up to 32 levels deep; a deeper nesting stops startup with an error naming the route. Listing an inner guard MUST NOT make the gateway warning fire for the wrapper.

#### Scenario: Wrapper with a typo
- **WHEN** a guard that lists an inner `AuthGuard::new("headr")` is used on a route and no strategy has that name
- **THEN** the application does not start and the error names the route and the name

#### Scenario: Guard that returns itself
- **WHEN** a guard lists itself in `inner_guards`
- **THEN** the application does not start and the error names the route

#### Scenario: Wrapper with a registered name
- **WHEN** the inner guard names a registered strategy
- **THEN** the application starts

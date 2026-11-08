# Spec Delta

## ADDED Requirements

### Requirement: Strategies are registered by name
The framework SHALL collect every provider registered with `Provider::strategy::<S>()` into a strategy registry when
the application is initialized, keyed by the strategy name. Strategies MUST receive their dependencies through
dependency injection like any other provider.

#### Scenario: Strategy found by its default name
- **WHEN** a module registers `Provider::strategy::<JwtStrategy<JwtAuth>>()` and a route uses
  `AuthGuard::new("jwt")`
- **THEN** a request to that route is authenticated by `JwtAuth`

### Requirement: Strategy names default to the kind
A strategy's name SHALL default to its kind (`"jwt"`, `"local"`, `"oauth2"`, `"headerapikey"`, or the name a custom
strategy returns) and MUST be overridable per validator, as with `PassportStrategy(Strategy, 'name')`.

#### Scenario: Two strategies of the same kind with different names
- **WHEN** two JWT strategies are registered, one with the default name and one whose validator sets the name
  `"jwt-refresh"`
- **THEN** `AuthGuard::new("jwt")` and `AuthGuard::new("jwt-refresh")` each use their own validator

### Requirement: Duplicate strategy names fail at startup
Application initialization MUST fail when two registered strategies share a name, naming both types. This is a
documented divergence: the original keeps the last registration silently.

#### Scenario: Duplicate name
- **WHEN** two strategy providers both resolve to the name `"jwt"`
- **THEN** `init()` returns an error that names both strategy types and the application does not start

### Requirement: Unknown strategy names fail at startup
Application initialization MUST fail when an `AuthGuard` attached to a route, or the configured default strategy,
names a strategy that is not registered, naming the route and the missing name, and when an `AuthGuard` without a
name is used and no default strategy is configured. This is a documented divergence: the original answers 500 at
request time.

#### Scenario: Typo in a guard
- **WHEN** a route uses `AuthGuard::new("jtw")` and no strategy is named `"jtw"`
- **THEN** `init()` returns an error naming the route and `"jtw"`

#### Scenario: Name resolved only at request time
- **WHEN** a user-written guard calls the authenticator with a name that is not registered
- **THEN** that request answers 500 with the body `{"statusCode":500,"message":"Internal server error"}`, as the
  original does for an unknown strategy

### Requirement: Validator results map to strategy outcomes
For every strategy kind with a validator, a validator result of `Ok(Some(user))` SHALL be a success, `Ok(None)` SHALL
be a failure, and `Err(exception)` SHALL be an error, matching how `PassportStrategy` maps a returned user, a falsy
value and a thrown error. A validator MAY also return authentication info, which SHALL be stored as the request's
auth info.

#### Scenario: Validator returns no user
- **WHEN** the validator of the only strategy of a guarded route returns `Ok(None)`
- **THEN** the response is the same as the original's when `validate()` returns `null`: status 401 and body
  `{"message":"Unauthorized","statusCode":401}`

### Requirement: Successful authentication exposes the user to the handler
When a strategy succeeds, `AuthGuard` SHALL store the user on the request both as a typed value (readable by type,
and through a `User<T>` parameter extractor) and as JSON (readable with `Request::user` and `Request::user_as`), SHALL
store the strategy's info as the request's auth info, and SHALL let the route continue. A `User<T>` extractor on a
request without a user of type `T` MUST answer the 401 of the failure requirement.

#### Scenario: Handler reads the user
- **WHEN** a JWT strategy validates a token and returns a user `{ "id": 7, "role": "admin" }`
- **THEN** the handler receives the same user through `User<AuthUser>` and through `Request::user_as::<AuthUser>()`,
  and the response matches the original handler that returns `req.user`

### Requirement: Failed authentication answers the original 401
When every strategy of an `AuthGuard` fails, the response MUST be status 401 with body
`{"message":"Unauthorized","statusCode":401}`, MUST NOT include a `WWW-Authenticate` header, and MUST ignore any
status a strategy attached to its failure (for example the 400 of missing local credentials), exactly as the
original `AuthGuard` does. The response MUST still go through the exception filters, like any guard exception.

#### Scenario: Strategy failure with its own status
- **WHEN** a strategy fails with status 400 and challenge `{ "message": "Missing credentials" }`
- **THEN** the response is status 401, body `{"message":"Unauthorized","statusCode":401}`, with no
  `WWW-Authenticate` header, identical to the recorded original response

#### Scenario: Exception filter sees the failure
- **WHEN** a global exception filter catches `UnauthorizedException` and an `AuthGuard` fails
- **THEN** the filter receives the 401 exception and its response is the one sent

### Requirement: Errors raised by a strategy pass through unchanged
When a strategy reports an error (an exception returned by the validator, or any internal error), `AuthGuard` MUST
stop and pass that exception on unchanged: an `HttpException` keeps its status and body, and any other error becomes
the 500 `Internal server error` response.

#### Scenario: Validator throws a specific exception
- **WHEN** the validator returns `Err(ForbiddenException::new("Account locked"))`
- **THEN** the response is status 403 with body `{"message":"Account locked","error":"Forbidden","statusCode":403}`,
  identical to the original's when `validate()` throws `new ForbiddenException('Account locked')`

### Requirement: Chained strategies are tried in order
`AuthGuard::chain([...])` SHALL try the named strategies in the given order: the first success wins and later
strategies are not run; a failure moves on to the next strategy; an error stops the chain and is passed on; when all
fail, the 401 of the failure requirement is sent. With more than one strategy, the auth info of a failed request
SHALL be the list of the strategies' challenges, as in the original.

#### Scenario: Second strategy succeeds
- **WHEN** a route uses `AuthGuard::chain(["jwt", "headerapikey"])`, the request has no bearer token and a valid
  API key
- **THEN** the API key strategy authenticates the request and the handler runs with its user

#### Scenario: First strategy errors
- **WHEN** the first strategy of a chain reports an error and the second would succeed
- **THEN** the second strategy is not run and the error is the response

### Requirement: Pass and optional authentication
When a strategy passes (neither success nor failure, as `strategy.pass()` in the original), `AuthGuard` SHALL let the
route run with no user set. `AuthGuard::optional()` SHALL let the route run with no user when all strategies fail,
the usual override of `handleRequest` in the original; errors MUST still be passed on.

#### Scenario: Optional guard without credentials
- **WHEN** a route uses `AuthGuard::new("jwt").optional()` and the request has no token
- **THEN** the handler runs, `Request::user()` is `None`, and the response matches the original route whose guard
  overrides `handleRequest` to return `user ?? null`

### Requirement: Redirect outcome
When a strategy redirects, `AuthGuard` MUST end the request with the redirect response, with status 302 unless the
strategy gives another one, a `Location` header with the URL, `Content-Length: 0` and an empty body, and MUST NOT run
later guards, interceptors, pipes or the handler.

#### Scenario: Strategy redirects
- **WHEN** a custom strategy redirects to `https://idp.example/authorize?x=1`
- **THEN** the response is status 302, `Location: https://idp.example/authorize?x=1`, `Content-Length: 0`, empty
  body, and the handler is not called

### Requirement: AuthGuard does not log in to a session by itself
A successful `AuthGuard` MUST NOT write the user into a session unless it was built with `.log_in()`, matching the
original, where `AuthGuard` uses the `passport.authenticate` callback and therefore skips `req.logIn`.

#### Scenario: Session installed, guard without log_in
- **WHEN** sessions are enabled and a route with `AuthGuard::new("local")` authenticates a user
- **THEN** no session cookie is created by the guard and the session store is unchanged

### Requirement: Each strategy feature is verified against the original
Every strategy of this epic MUST have a differential test: a Node oracle with `@nestjs/passport` 12.0.1, `passport`
0.7.0 and the strategy's reference package at a pinned version records request/response pairs into a fixture, and a
Rust test replays every case against an equivalent Corvis app, comparing status, relevant headers and body.

#### Scenario: Fixture replay
- **WHEN** `cargo test --workspace` runs
- **THEN** every recorded case of every strategy fixture is replayed without needing Node, and a difference from
  the recorded original fails the test

#### Scenario: Marked divergence
- **WHEN** a fixture case is marked as a divergence with its reason
- **THEN** the Rust test asserts the behavior this specification states for that case instead of the recorded one

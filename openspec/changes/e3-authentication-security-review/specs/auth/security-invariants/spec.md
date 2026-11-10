# Spec Delta

## Purpose

Properties of authentication that must hold for every combination of the strategies of `corvis::auth` (JWT, local,
API key, session, OAuth2, custom), of guards and of transports. Each strategy has its own specification; these
requirements cover what only appears when they are used together. Every scenario is a regression test, and the fuzz
targets exercise the same properties with random input.

## ADDED Requirements

### Requirement: The handler's user comes from the route's guard
After an `AuthGuard` has run on a route, the user the handler sees (`User<T>`, `Request::user()`) SHALL be the one
that guard established in this request, or none. A user restored earlier in the request by the session middleware
MUST NOT survive a guard that did not authenticate it.

#### Scenario: Session of one user and token of another
- **WHEN** a request carries the session cookie of user A and a valid token of user B to a route guarded by
  `AuthGuard::new("jwt")`
- **THEN** the handler sees user B through `User<T>` and through `Request::user()`

#### Scenario: Optional token guard with a session cookie
- **WHEN** a request carries the session cookie of user A and an invalid token to a route guarded by
  `AuthGuard::new("jwt").optional()`
- **THEN** the handler runs with no user

#### Scenario: Session strategy in the chain
- **WHEN** the same request reaches a route guarded by `AuthGuard::chain(["jwt", "session"])`
- **THEN** the handler sees user A, authenticated by the session strategy

#### Scenario: Token-only route with only a session cookie
- **WHEN** a request carries only the session cookie of user A to a route guarded by `AuthGuard::new("jwt")`
- **THEN** the response is the plain `401` and the handler does not run

### Requirement: Routes without an AuthGuard are documented as seeing the session user
On a route with no `AuthGuard`, the session user restored by the middleware SHALL be visible to later guards and to
the handler, and the threat model and the session documentation MUST state it, with the consequence for a
hand-written authorization guard used without an `AuthGuard`.

#### Scenario: Roles guard without an AuthGuard
- **WHEN** a request with the session cookie of an administrator reaches a route guarded only by a hand-written guard
  that reads `Request::user()`
- **THEN** the guard sees the administrator, and the test that pins this behavior is named in the threat model next
  to the documented rule "put an `AuthGuard` first"

### Requirement: Chains fail closed
In a chain, an `Error` outcome SHALL end the request with that error and MUST NOT let a later strategy run. No
strategy shipped by the framework may return `Pass`. A request reaches the handler of a non-optional guard only after
a `Success`.

#### Scenario: Failing dependency before a weaker strategy
- **WHEN** the first strategy of `AuthGuard::chain(["oauth", "api-key"])` returns an error because its provider is
  unreachable and the request also carries a valid API key
- **THEN** the response is that error and the API key validator is not called

#### Scenario: Built-in strategies never pass
- **WHEN** the gate fuzz target sends random requests to routes guarded by each built-in strategy
- **THEN** no handler of a non-optional guard runs without a user

### Requirement: One failure response for every reason
For every built-in strategy and every chain of them, a failed authentication through `AuthGuard` SHALL produce the
same status, body and header names whatever the reason: unknown user or wrong password, expired or forged token,
missing or wrong key, tampered or unknown session. The reason is available only in `Request::auth_info()`.

#### Scenario: Reasons are indistinguishable
- **WHEN** a login route receives a wrong password for an existing user and then any password for a user that does
  not exist
- **THEN** both responses have the same status, body and header names, and neither has a `Set-Cookie` header

#### Scenario: Chain position is not revealed
- **WHEN** a route guarded by `AuthGuard::chain(["jwt", "api-key"])` receives an expired token, then a wrong key,
  then nothing
- **THEN** the three responses are byte-identical apart from the `Date` header

### Requirement: No authority-carrying cookie on a failed request
A request whose authentication failed SHALL NOT receive a `Set-Cookie` that creates or renews a session, carries a
new session identifier, or stores a new OAuth2 state. Deleting cookies, re-signing after a secret rotation and the
`rolling` renewal of an existing session are allowed.

#### Scenario: Failed login with an anonymous session
- **WHEN** a login guarded by `AuthGuard::new("local").log_in()` fails for a browser that already has a session, with
  the default session options
- **THEN** the session identifier is unchanged in the store and the response sets no session cookie

#### Scenario: Failed OAuth2 callback
- **WHEN** an OAuth2 callback fails its state check
- **THEN** the only `Set-Cookie` of the response deletes the state cookie

### Requirement: Public routes and guard order
A guard built with `.skip_public()` SHALL run no strategy on a route whose handler or controller is marked
`#[public]`, and SHALL authenticate every other route. A guard built without it authenticates public routes too. An
optional guard never counts as access control for route posture.

#### Scenario: Invalid credentials on a public route
- **WHEN** a request with an expired token reaches a `#[public]` route under a global
  `AuthGuard::new("jwt").skip_public()`
- **THEN** the handler runs and no strategy was called

#### Scenario: Controller marked public, handler not marked
- **WHEN** a controller is marked `#[public]` and one of its handlers has its own route-level `AuthGuard::new("jwt")`
- **THEN** that handler answers `401` without a valid token: the route-level guard was not built with
  `.skip_public()`

#### Scenario: Posture with only an optional guard
- **WHEN** route posture is enforced and a route has only `AuthGuard::new("jwt").optional()`
- **THEN** the application does not start

### Requirement: Credentials are evaluated only by the strategy configured for their place
A credential SHALL authenticate only through a strategy configured to read the place it was sent in. Strategies that
read the same place MUST extract the same value from it.

#### Scenario: Bearer value shared by two strategies
- **WHEN** a route guarded by `AuthGuard::chain(["jwt", "api-key"])`, with the API key `bearer()` source enabled,
  receives any `Authorization` header
- **THEN** both strategies evaluate the same extracted value, or both find none

#### Scenario: Token in a place that is not configured
- **WHEN** a valid token is sent as the value of the session cookie, or a valid session cookie value is sent as a
  bearer token
- **THEN** the response is the plain `401`

### Requirement: OAuth2 state and session regeneration
When the OAuth2 state is stored in the session and the callback logs in to the session, the state record SHALL be
removed before the provider is contacted, the session identifier SHALL change at login, the old record SHALL be
destroyed, and no `oauth2:` key may exist in the new session, with or without `keep_session_info_on_login`.

#### Scenario: Callback that logs in
- **WHEN** a login started with `state_in_session()` completes on a callback guarded by
  `AuthGuard::new("github").log_in()`
- **THEN** the old session identifier is absent from the store, the new session holds the user and no state record

#### Scenario: Callback replayed after logout
- **WHEN** the user logs out and the same callback URL is requested again with the cookies of the logged-out browser
- **THEN** the response is the plain `401` and the provider received no request

#### Scenario: Forged callback on an authenticated session
- **WHEN** a logged-in browser is sent to a callback URL carrying an attacker's `code` and `state`
- **THEN** the response is the plain `401`, the provider received no request and the session user is unchanged

### Requirement: HTTP authentication state does not reach sockets
Strategies SHALL never run for a WebSocket message, and the handshake request of a gateway MUST NOT carry a session
or a user set by HTTP middleware or guards. The threat model lists every handshake surface with how it is
authenticated.

#### Scenario: Session cookie on an upgrade request
- **WHEN** a logged-in browser opens a WebSocket to a gateway
- **THEN** `client.handshake().user()` is none in `handle_connection`

#### Scenario: Global AuthGuard and a gateway
- **WHEN** a global `AuthGuard` reaches a gateway message handler
- **THEN** the message is denied and no strategy is called

### Requirement: Defaults are the same rule in every strategy
The threat model SHALL contain a defaults table, and a test MUST compare each row with the code: secret minimum
lengths, cookie attributes, credentials read from a URL, token and key size limits, timeouts and lifetimes. A default
that changes without the table fails the test.

#### Scenario: Secret minimum
- **WHEN** the test builds a JWT strategy HMAC secret, cookie signing keys and an expected API key of 31 bytes
- **THEN** each one is refused, and each is accepted with 32 bytes

#### Scenario: Cookies set by the module
- **WHEN** the test reads the default attributes of the session cookie and of the OAuth2 state cookie
- **THEN** both are `HttpOnly`, `Secure` and `SameSite=Lax`

#### Scenario: Credentials in a URL
- **WHEN** the test builds each strategy with default options
- **THEN** none reads a credential from the query string, and enabling the JWT query extractor or the API key query
  source logs a startup warning

### Requirement: Secrets never appear in output
No password, token, key, session identifier, signing secret, authorization code, state value, PKCE verifier or client
secret given to the module SHALL appear in a log line, an error body, a response header other than the cookie that
carries it, or the `Debug` output of a framework type.

#### Scenario: Canary secrets
- **WHEN** the canary fuzz target sends requests whose credentials are unique marker strings to every route of the
  test application, valid and invalid
- **THEN** no marker appears in the captured logs or in any response, except a session cookie in its own `Set-Cookie`

#### Scenario: Debug of every public type
- **WHEN** the test formats every public type of `corvis::auth` that can hold a secret with `{:?}`
- **THEN** the output contains none of the secrets it was built with

### Requirement: Invariants hold under random input
Fuzz targets SHALL exercise these invariants with random requests: a gate property compared with a reference model,
a session state machine, the OAuth2 callback, and redirect targets. They MUST run in `cargo test` with the default
case count, in the CI fuzz job and in the nightly long run.

#### Scenario: Gate property
- **WHEN** random combinations of valid, expired, forged and malformed credentials are sent to routes with every
  guard shape (single, chain, optional, default, `skip_public`)
- **THEN** a handler runs with a user exactly when the model says so, the user is the model's, and no response is a
  `5xx`

#### Scenario: Session state machine
- **WHEN** random sequences of login, logout, requests with old cookies, secret rotation and clock steps run against
  the test application
- **THEN** an identifier that was destroyed or replaced never authenticates again

#### Scenario: OAuth2 callback
- **WHEN** random queries and cookies are sent to a callback route in front of a counting mock provider
- **THEN** the provider is contacted only after a valid state, and no response is a `5xx` caused by request data

#### Scenario: Redirect targets
- **WHEN** random `return_to` values and failure paths are used
- **THEN** every `Location` is a configured URL or a relative path accepted by the documented rule

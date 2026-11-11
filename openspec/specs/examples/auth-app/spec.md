# examples/auth-app Specification

## Purpose
`auth-app` is a small application that is only about authentication: every built-in strategy of `corvis::auth` in
one place, combined the way the guide describes, runnable with one command and no external account. The guide's
samples are taken from it.

## Requirements

### Requirement: The example runs with one command on the loopback interface
`cargo run -p auth-app -- --dev` SHALL start the application and its mock OAuth2 provider on `127.0.0.1`, with
secrets generated for that run, and print the addresses and the demo credentials. Nothing may listen on another
interface, and no network access is needed.

#### Scenario: Development start
- **WHEN** a user runs `cargo run -p auth-app -- --dev` with no environment variables
- **THEN** the application answers on `127.0.0.1`, the log says the secrets were generated for this run and that
  cookies are not marked `Secure`, and the walkthrough of the README works as written

#### Scenario: Restart
- **WHEN** the development application is restarted
- **THEN** tokens and session cookies of the previous run are refused

### Requirement: No default secrets
Without `--dev`, loading the settings (`Settings::from_env()`, which `main` calls before `create_app`) SHALL fail unless
`JWT_SECRET`, `SESSION_SECRET`, `OAUTH_STATE_SECRET` and `SERVICE_API_KEY` are set with at least 32 bytes each and the OAuth2
client settings are present. The error MUST name the variable and MUST NOT
print its value. `.env.example` MUST NOT contain a value that works.

#### Scenario: Missing secret
- **WHEN** the settings are loaded without `SESSION_SECRET`
- **THEN** loading returns an error whose text contains `SESSION_SECRET`

#### Scenario: Short secret
- **WHEN** `JWT_SECRET` is 16 bytes long
- **THEN** loading the settings fails, the error names `JWT_SECRET` and the minimum, and does not contain the value

#### Scenario: Copying the example environment file
- **WHEN** a user copies `.env.example` to `.env` without editing it
- **THEN** the application refuses to start

### Requirement: Local login with hashed passwords
The example SHALL log users in with the local strategy against stored password hashes, never plain text, verify
through `verify_or_dummy`, and answer a wrong password and an unknown user identically. One demo user MUST have a
bcrypt hash that is replaced by an argon2id hash on its first successful login.

#### Scenario: Login for a token
- **WHEN** `POST /auth/login` receives the demo credentials
- **THEN** the response is `200` with an access token

#### Scenario: Wrong password and unknown user
- **WHEN** the same route receives a wrong password, then a user that does not exist
- **THEN** both responses are the same `401`

#### Scenario: Legacy hash upgraded
- **WHEN** the demo user with the bcrypt hash logs in successfully
- **THEN** the stored hash of that user is an argon2id hash afterwards and the next login still works

### Requirement: Token route
The example SHALL protect a route with `AuthGuard::new("jwt")` and read the user with `User<T>`. Tokens it issues
carry an expiry.

#### Scenario: Valid token
- **WHEN** `GET /me` is requested with the token of the login
- **THEN** the response is `200` with that user

#### Scenario: Missing or expired token
- **WHEN** `GET /me` is requested without a token or with an expired one
- **THEN** the response is `401 {"message":"Unauthorized","statusCode":401}`

### Requirement: API key route for machine clients
The example SHALL have a route that accepts either a user token or the service API key through
`AuthGuard::chain(["jwt", "api-key"])`, with the key compared through `ExpectedApiKey`.

#### Scenario: Service key
- **WHEN** `GET /reports` is requested with the configured key in `X-Api-Key`
- **THEN** the response is `200` and the handler sees the service identity

#### Scenario: Wrong key
- **WHEN** the same route is requested with another key
- **THEN** the response is the plain `401`

### Requirement: Session login and logout
The example SHALL log in to a session with `AuthGuard::new("local").log_in()`, protect a route with
`AuthGuard::new("session")`, and log out through `Authenticator::log_out`. The session identifier MUST change at
login and the old cookie MUST be anonymous after logout.

#### Scenario: Session flow
- **WHEN** a client logs in at `POST /session/login`, requests `GET /session/me` with the cookie, and logs out
- **THEN** the login sets a session cookie, the protected route answers `200` with the user, and after the logout
  the same cookie gets `401`

#### Scenario: Identifier changes at login
- **WHEN** a client that already has an anonymous session logs in
- **THEN** the session cookie it receives has a different identifier

### Requirement: OAuth2 login against the bundled mock provider
The example SHALL log in through a generic OAuth2 strategy whose endpoints come from configuration, with the state
in a signed cookie and PKCE, against a mock provider that is part of the example and listens on `127.0.0.1`. The
callback logs in to the session.

#### Scenario: Full flow
- **WHEN** a test follows `GET /auth/provider`, the mock provider's authorization redirect and the callback
- **THEN** the callback sets a session cookie and `GET /session/me` answers `200` with the provider's user

#### Scenario: Forged callback
- **WHEN** the callback is requested with a `state` that was not issued to this client
- **THEN** the response is `401` and the mock provider received no token request

#### Scenario: Real provider by configuration
- **WHEN** a user sets the OAuth2 variables to a real provider's endpoints and client
- **THEN** the same code logs in against it, as the README describes

### Requirement: The example shows how strategies combine
The example SHALL have a route with an optional guard that personalizes its answer, a route that requires a role
through a guard listed after `AuthGuard`, and public routes marked `#[public]`. It MUST enforce route posture, so
it does not start with an unguarded route that is not marked.

#### Scenario: Optional guard
- **WHEN** `GET /feed` is requested without credentials, then with a valid token
- **THEN** both answer `200`, and only the second names the user

#### Scenario: Role required
- **WHEN** `GET /admin/users` is requested with the token of a user without the admin role
- **THEN** the response is `403`

#### Scenario: Posture report
- **WHEN** a test reads `app.route_posture()` after `init()`
- **THEN** no route is unprotected, and `GET /feed` is listed as public: it is marked `#[public]`, since its
  optional guard does not count as access control

### Requirement: Login routes are rate limited
The example SHALL register `ThrottlerGuard` as a global guard, so it runs before authentication, and limit its login
routes per client and per account with `login_tracker`. Startup MUST log no guard-order warning.

#### Scenario: Attempts over the limit
- **WHEN** one account receives more login attempts than its limit, from one client
- **THEN** the next attempt is answered `429` with the retry header of the throttler that blocked it (`Retry-After-login` for the
  per-account limit, `Retry-After` for the per-client one)

#### Scenario: Other routes keep the global limit
- **WHEN** `GET /me` is requested after the login limit was reached
- **THEN** it is not answered `429`

### Requirement: The example is tested and documented
Every flow above SHALL have a test in `examples/auth-app/tests/` that needs neither the network nor Node. The
example MUST have a README in English with a `curl` walkthrough of each flow, and a section in the examples page of
the documentation in both languages.

#### Scenario: Workspace tests
- **WHEN** `cargo test --workspace` runs
- **THEN** the tests of `auth-app` run and pass

#### Scenario: Reader of the examples page
- **WHEN** a reader opens `/faq/examples` or `/es/faq/examples`
- **THEN** a section describes `auth-app`, how to start it and what each route demonstrates

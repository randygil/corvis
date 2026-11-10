# Spec Delta

## Purpose

Strategies written by the application: a type that implements the `Strategy` trait of `auth/strategies` directly, the
equivalent of a `passport-custom` 1.2.1 callback or a `passport-strategy` 1.0.0 subclass. This capability fixes the
documented pattern, its mapping from the original and its verification; the trait, the registry and the outcomes are
specified in `auth/strategies`.

## ADDED Requirements

### Requirement: A user type can be a strategy
A user type that implements `Strategy` SHALL be registrable with `Provider::strategy::<S>()` and usable by name in
`AuthGuard`, alone or in a chain, without any cargo feature. It MUST receive its dependencies through dependency
injection like any other provider.

#### Scenario: Strategy with an injected service
- **WHEN** `DeviceStrategy` (name `"device"`) injects a `DevicesService`, is registered with
  `Provider::strategy::<DeviceStrategy>()`, and a route uses `AuthGuard::new("device")`
- **THEN** a request with a known device header runs the handler with the device as the user, found through the
  injected service

#### Scenario: Built without auth features
- **WHEN** the crate is built with `--no-default-features` and an application registers a custom strategy
- **THEN** it compiles and the strategy authenticates requests

### Requirement: Custom strategy outcomes match passport-custom
For a custom strategy written as the guide documents, each outcome MUST produce the same HTTP response as a
`passport-custom` 1.2.1 strategy behind the original `AuthGuard` whose callback ends the same way: a user (with
optional info) is a success, no user is the 401, an HTTP exception is that exception's response, and any other error
is the 500. The redirect and pass outcomes, which `passport-custom` cannot reach, are covered by `auth/strategies`.

#### Scenario: Success with info
- **WHEN** the strategy returns `Success` with a user and info
- **THEN** the handler receives the user, `Request::auth_info()` returns the info, and the response equals the
  recorded original

#### Scenario: No user
- **WHEN** the strategy returns `Fail` with no challenge
- **THEN** the response is the recorded original 401 `{"message":"Unauthorized","statusCode":401}`

#### Scenario: Error
- **WHEN** the strategy returns `Error(ForbiddenException::new("Device blocked"))`, and in another case an internal
  error
- **THEN** the responses are the recorded original 403 and 500

#### Scenario: Second in a chain
- **WHEN** a route uses `AuthGuard::chain(["jwt", "device"])` and only the device header is valid
- **THEN** the handler runs with the device, as in the recorded original

### Requirement: A strategy can be replaced in tests
A testing module SHALL be able to replace a registered strategy with another implementation under the same name
(`override_strategy`), so that an application's routes can be tested without the real credentials check.

#### Scenario: Override in a testing module
- **WHEN** a test calls `override_strategy::<DeviceStrategy>(..)` with a stub strategy named `"device"` that always
  succeeds
- **THEN** a request to a route with `AuthGuard::new("device")` runs the handler with the stub's user

### Requirement: A strategy never lets a request through by failing
An error returned by a strategy SHALL end the request with that error, also behind an optional guard. A panic inside
a strategy SHALL be contained and answered as the plain 500 `{"statusCode":500,"message":"Internal server error"}`,
also behind an optional guard, without running later strategies of a chain and without reading the panic payload.
The log line about a panic SHALL be rate limited per strategy, and the default exception filter SHALL NOT add one per
request.

#### Scenario: Error behind an optional guard
- **WHEN** a strategy returns an error and the route uses `AuthGuard::new(..).optional()`
- **THEN** the response is the error's response and the handler does not run

#### Scenario: Panic in a chain
- **WHEN** the first strategy of `AuthGuard::chain(["panicking", "succeeding"])` panics with a credential in its message
- **THEN** the response is the plain 500 without the credential and the second strategy does not run

#### Scenario: Panic log lines are limited
- **WHEN** a strategy panics on forty requests
- **THEN** one warning for that strategy is logged in the minute, with no panic payload, and no `ExceptionsHandler` line

### Requirement: An empty user never authenticates
A `Success` whose user serializes to JSON `null`, `false`, the number `0` (integer, float or `-0`) or the empty string
SHALL be treated by `Authenticator::authenticate` as a failure of that strategy, for every strategy kind: the chain
goes on, the auth info says the strategy returned an empty user, and a warning (at most one per strategy per minute,
with a suppressed count) tells the author to return `None`. No path stores such a user. `"0"`, `{}` and `[]` are users.

#### Scenario: Boolean user
- **WHEN** an API key validator with `type User = bool` returns `Ok(Some(false))`
- **THEN** the response is the 401 and the handler does not run

#### Scenario: Empty value of each kind
- **WHEN** the user is `null`, `false`, `0`, `0.0`, `-0.0` or `""` in a JWT, local, API key or custom strategy
- **THEN** the response is the 401, and behind an optional guard the request runs with no user

#### Scenario: Chain continues
- **WHEN** the first strategy of a chain succeeds with an empty user and the second succeeds with a real one
- **THEN** the second strategy's user is stored

#### Scenario: Matches the original
- **WHEN** a `passport-custom` strategy calls `done(null, false)`, `done(null, 0)`, `done(null, '')` or `done(null, null)`
- **THEN** the recorded response is the 401, and so is Corvis's, alone and in a chain

#### Scenario: Truthy values
- **WHEN** the user is `"0"`, `"false"`, `{}`, `[]` or `true`
- **THEN** the request is authenticated

### Requirement: Strategy names are checked at startup
A strategy with an empty name, a name over 128 bytes or a name with a control character MUST stop the application at
startup, and a guard that names a strategy nobody registered MUST stop it too, also when the guard is listed in
`CanActivate::inner_guards` of another guard.

#### Scenario: Empty name
- **WHEN** a registered strategy reports the name `""`
- **THEN** `init()` fails with an error that says the name is invalid

#### Scenario: Name checked through a wrapper guard
- **WHEN** a guard lists `AuthGuard::new("devise")` in `CanActivate::inner_guards` and no strategy has that name
- **THEN** `init()` fails with an error that names the strategy

### Requirement: The guide states the security rules for strategies
The documentation SHALL include a guide to writing a strategy with the mapping from the original's outcomes and a
checklist: fail (never pass); the user must be a real value; compare secrets in constant time and refuse an empty
expected secret; keep request data out of errors, logs and panic messages (the default panic hook prints them);
bound what is read; register URL credentials; never build a redirect target from request data.

#### Scenario: Guide content
- **WHEN** the "Writing a strategy" section of `docs/recipes/passport.md` and its Spanish copy are read
- **THEN** both contain the outcome mapping table, the checklist, and an example whose code is the strategy compiled
  and exercised by `crates/corvis/tests/auth_custom.rs`

### Requirement: Custom strategies are verified against the original
The documented pattern SHALL be covered by a differential test: a generator in the fixture package of
`auth/strategies` records the responses of a `passport-custom` 1.2.1 strategy with an injected service behind the
original `AuthGuard`, and a Rust test replays every case without Node.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis --test auth_custom` runs on a machine without Node
- **THEN** every recorded case is compared (status, listed headers, body)

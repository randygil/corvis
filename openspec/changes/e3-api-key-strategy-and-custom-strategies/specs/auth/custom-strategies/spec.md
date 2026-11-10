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
A testing module SHALL be able to replace a registered strategy with another implementation under the same name, so
that an application's routes can be tested without the real credentials check.

#### Scenario: Override in a testing module
- **WHEN** a test overrides the provider of `DeviceStrategy` with a stub strategy named `"device"` that always
  succeeds
- **THEN** a request to a route with `AuthGuard::new("device")` runs the handler with the stub's user

### Requirement: The guide states the security rules for strategies
The documentation SHALL include a guide to writing a strategy with the mapping from the original's outcomes and a
checklist: fail (never pass) when credentials are missing or wrong; compare secrets in constant time; keep request
data out of error messages and logs; bound what is read from the request; mark credentials read from the URL with
`Request::redact_query_param`; never build a redirect target from request data.

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

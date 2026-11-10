# Spec Delta

## Purpose

Rate limiting of login and callback routes with the existing `ThrottlerGuard` (`security/rate-limiting`): where it
must sit relative to authentication, how attempts are counted per account, and what a throttled request must not
cost. No new throttler is added.

## ADDED Requirements

### Requirement: A throttled request runs no authentication
When a `ThrottlerGuard` precedes an `AuthGuard` in a route's guards and rejects the request, the response SHALL be
the throttler's `429` and no strategy may run: no password key derivation, no session store write, no request to an
OAuth2 provider, no validator call.

#### Scenario: Password guessing over the limit
- **WHEN** a login route limited to 5 attempts per minute receives a sixth attempt with a wrong password
- **THEN** the response is `429` with `Retry-After`, and the key derivation counter of the hasher did not increase

#### Scenario: OAuth2 login start over the limit
- **WHEN** an OAuth2 login route with `state_in_session()` is over its limit
- **THEN** the response is `429`, no redirect is sent and the session store has no new record

#### Scenario: Callback over the limit
- **WHEN** an OAuth2 callback route is over its limit
- **THEN** the response is `429` and the mock provider received no request

### Requirement: An AuthGuard that runs before the throttler is reported at startup
When a route's guards contain an `AuthGuard` followed by a `ThrottlerGuard`, initialization SHALL log one warning
that names the route and says the limit is checked after authentication. The warning MUST NOT stop the application.

#### Scenario: Global AuthGuard, route-level throttler
- **WHEN** an application registers a global `AuthGuard` and a controller adds `ThrottlerGuard` with
  `#[use_guards]`
- **THEN** startup logs the warning with the method and path of each affected route

#### Scenario: Throttler first
- **WHEN** `ThrottlerGuard` is a global guard registered through `Provider::app_guard` and `AuthGuard` is global or
  route-level
- **THEN** no warning is logged

### Requirement: Login attempts can be counted per account
`corvis::auth::local::login_tracker(options)` SHALL return a throttler tracker that identifies the account by the
username the local strategy will read from the same request with the same options. Registered as a named tracker, it
lets a throttler count attempts per account next to the per-client throttler.

#### Scenario: Same account from many addresses
- **WHEN** a login route with a per-account limit of 5 receives six attempts for one username from six client
  addresses
- **THEN** the sixth is answered `429`

#### Scenario: Username moved to the query string
- **WHEN** attempts for one username alternate between the body field and the query parameter
- **THEN** they are counted on the same counter

#### Scenario: Nested field name
- **WHEN** the strategy is configured with `username_field("user[email]")` and the tracker with the same options
- **THEN** the tracker reads the same nested value

### Requirement: The account tracker is bounded and reveals nothing
The tracker value SHALL be a digest of the username bytes, never the username. A request without a readable
username, or with one over the strategy's length limit, MUST be tracked by the normalized client address. Counting
MUST NOT depend on whether the account exists.

#### Scenario: Missing username
- **WHEN** a login request has no username
- **THEN** it is counted for the client address and answered as the strategy answers missing credentials

#### Scenario: Existing and unknown accounts
- **WHEN** five attempts are sent for an existing account and five for one that does not exist
- **THEN** the rate limit headers of both series are the same

#### Scenario: Storage keys
- **WHEN** the throttler storage is inspected after login attempts
- **THEN** no key contains a submitted username

### Requirement: Login rate limiting is documented with its limits
The rate limiting page SHALL document the login recipe (per-client and per-account throttlers, their order, the
tracker), that a per-account limit lets anyone lock an account out for the block duration, that successful logins
are counted too, and that lockout policies are the application's.

#### Scenario: Reader protecting a login route
- **WHEN** a reader opens the login section of `/security/rate-limiting` or `/es/security/rate-limiting`
- **THEN** it shows the two throttlers on the login route with `login_tracker`, and the warning about lockout

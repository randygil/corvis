## ADDED Requirements

### Requirement: The typed user is readable from guards and handlers
The typed user slot SHALL hold what an `AuthGuard` stored, or what the session middleware restored or `log_in` stored.
`User<T>` and `Request::typed_user::<T>()` MUST read that slot only, never another request extension of the same type.
On a route without an `AuthGuard` they return the session user when the session restored one.

#### Scenario: A guard reads the typed user
- **WHEN** a roles guard that runs after `AuthGuard::new("jwt")` calls `Request::typed_user::<Account>()`
- **THEN** it receives the `Account` the validator returned, without deserializing the JSON user

#### Scenario: Route without an AuthGuard
- **WHEN** a request whose session holds a user reaches a route with no `AuthGuard` whose handler takes `User<Account>`
- **THEN** the handler receives the session's user

#### Scenario: Foreign extension
- **WHEN** a guard that is not an `AuthGuard` inserts an `Account` into the request
- **THEN** `Request::typed_user::<Account>()` returns `None`

### Requirement: One body for every 503 of the auth module
Every `503` the authentication module builds itself (a callback past its deadline, a busy password hasher through
`PasswordError::into_http_exception`, an OAuth2 strategy at its concurrency limit) SHALL have the body
`{"statusCode":503,"message":"Service Unavailable"}`, byte for byte.

#### Scenario: Busy hasher and slow validator on one route
- **WHEN** a login route answers once because the hasher is busy and once because the validator missed its deadline
- **THEN** both bodies are the same bytes

#### Scenario: OAuth2 at its limit
- **WHEN** an OAuth2 callback finds every request slot to the provider taken
- **THEN** the body is `{"statusCode":503,"message":"Service Unavailable"}`

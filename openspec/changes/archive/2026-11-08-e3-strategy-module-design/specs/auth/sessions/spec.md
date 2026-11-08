# Spec Delta

## ADDED Requirements

### Requirement: Session cookies compatible with express-session
The session middleware SHALL use the cookie name `connect.sid` by default (configurable) and the value format of
`express-session` 1.19.0: `s:` + session id + `.` + the HMAC-SHA256 of the id, base64 with the standard alphabet and
no `=` padding, URI-encoded (`s:` is sent as `s%3A`). New cookies MUST be signed with the first configured secret; a
cookie MUST be accepted when it verifies with any of them. Signature checks MUST be constant-time.

#### Scenario: Cookie created by the original
- **WHEN** the oracle creates a session with secret `"s1"` and Corvis, configured with the same secret and store,
  receives its `connect.sid` cookie
- **THEN** Corvis loads the same session data

#### Scenario: Secret rotation
- **WHEN** the secrets are `["new", "old"]` and a cookie signed with `"old"` arrives
- **THEN** the session is loaded, and a cookie sent back in the response is signed with `"new"`

### Requirement: Invalid session cookies are ignored
A session cookie that is unsigned, badly signed or malformed MUST be ignored and a new session started, as in
`express-session`; it MUST never cause an error response.

#### Scenario: Tampered cookie
- **WHEN** one character of the signature is changed
- **THEN** the request runs with a new, empty session and no error

### Requirement: Saving rules match express-session
The default cookie attributes SHALL be those of `express-session` (`Path=/`, `HttpOnly`, no `Max-Age`, `Secure` and
`SameSite` only when configured). With the same explicit `resave`, `saveUninitialized` and `rolling` options, whether
a `Set-Cookie` header is sent and whether the store is written MUST match `express-session` for new and existing,
modified and unmodified sessions.

#### Scenario: Explicit options
- **WHEN** both apps use `resave: false, saveUninitialized: false, rolling: true` and the same requests are replayed
- **THEN** each response has a `Set-Cookie` header exactly when the recorded original response has one, with the
  same attributes

### Requirement: Sessions are not saved by default until modified
`resave` and `save_uninitialized` SHALL default to `false`, so an anonymous visit creates no session and no cookie.
Documented divergence: the original defaults both to `true` and warns that the options must be given.

#### Scenario: Anonymous request
- **WHEN** a request without a cookie reaches a handler that does not touch the session
- **THEN** the response has no `Set-Cookie` header and the store is unchanged

### Requirement: Session stores
The framework SHALL provide a memory store, a Redis store and an ORM-backed store behind a `SessionStore` trait. The
Redis store MUST read and write the key format, JSON value format and TTL rule of `connect-redis` 10.0.0 (default
prefix `sess:`, TTL from the cookie's expiry, or one day when the cookie has none), so a Node app and a Corvis app can
share sessions during a migration.

#### Scenario: Shared Redis
- **WHEN** the Node oracle logs in a user with `connect-redis` and the Corvis app receives the same cookie, with the
  same secret and Redis database
- **THEN** Corvis restores the same user

### Requirement: Log in and log out regenerate the session
Logging in SHALL regenerate the session id, store the serialized user under `passport.user`, and send the new cookie.
Logging out SHALL remove the user and regenerate the session id. This MUST match `passport` 0.7.0
(`sessionmanager.js`) and prevents session fixation.

#### Scenario: Session fixation attempt
- **WHEN** a client logs in while presenting a session cookie it obtained before logging in
- **THEN** the response sets a cookie with a different session id, and the old id no longer has a user in the store

#### Scenario: Logout
- **WHEN** a logged-in client calls the logout route
- **THEN** the old session id has no user, a new cookie is set, and later requests with the old cookie are
  anonymous

### Requirement: The session user is restored on later requests
When a session has `passport.user`, the session support SHALL deserialize it through the application's
`SessionSerializer` before the guards run and expose it as the request's user, like `passport.session()`. When
deserialization finds no user, the stored user MUST be removed and the request MUST continue as anonymous. The
built-in `"session"` strategy SHALL succeed when the request has a session user and fail with the 401 otherwise.

#### Scenario: User deleted after login
- **WHEN** the serializer reports that the stored user no longer exists
- **THEN** the request is anonymous, `passport.user` is removed from the session, and a route with
  `AuthGuard::new("session")` answers the recorded original 401

### Requirement: Local login through a session
`AuthGuard::new("local").log_in()` SHALL authenticate with the local strategy and then log in to the session, the
`LocalAuthGuard` pattern of the original (`super.canActivate(context)` followed by `super.logIn(request)`).

#### Scenario: Login form
- **WHEN** valid credentials are posted to a route with `AuthGuard::new("local").log_in()`
- **THEN** the response sets a new session cookie and a later request with that cookie has the user, matching the
  recorded original

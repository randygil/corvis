# Spec Delta

## Purpose

Server-side sessions: a middleware that identifies a browser by a signed cookie, keeps its data in a session store
and restores the logged-in user before the guards, with the cookie format, record format and saving rules of
`express-session` 1.19.0 and the login semantics of `passport` 0.7.0, except where this specification is stricter on
purpose (each divergence is named in its requirement). Available under the `auth-session` cargo feature. Every limit
has a default and a startup check.

## ADDED Requirements

### Requirement: Session cookies compatible with express-session
The session middleware SHALL use the cookie name `connect.sid` by default (configurable) and the value format of
`express-session` 1.19.0: `s:` + session identifier + `.` + signature, as specified in `http/signed-cookies`,
URI-encoded when written (`s:` is sent as `s%3A`). New cookies MUST be signed with the first configured secret; a
cookie MUST be accepted when it verifies with any of them.

#### Scenario: Cookie created by the original
- **WHEN** Corvis, configured with the same secret and a store holding the recorded record, receives the recorded
  `connect.sid` cookie the original created
- **THEN** the handler reads the same session data

#### Scenario: Cookie created by Corvis
- **WHEN** a handler writes to a new session
- **THEN** the `Set-Cookie` value has the recorded shape (`s%3A`, a 32-character identifier, `.`, a 43-character
  signature, encoded) and verifies with `cookie-signature` rules under the configured secret

#### Scenario: Secret rotation
- **WHEN** the secrets are `[new, old]` and a cookie signed with `old` arrives for an existing session
- **THEN** the session is loaded and the response sets the cookie again, signed with `new` (documented addition: the
  original re-sends the cookie only under its own saving rules)

### Requirement: Invalid session cookies are ignored
A session cookie that is unsigned, badly signed, malformed, or whose identifier is longer than 128 bytes or has
bytes outside visible ASCII MUST be ignored without any store access and without an error response: the request runs
with a new, empty session.

#### Scenario: Tampered cookie
- **WHEN** one character of the signature is changed
- **THEN** the request runs with a new, empty session, the store is not read, and the response is not an error

#### Scenario: Unsigned identifier
- **WHEN** the cookie is `connect.sid=<identifier of an existing session>` without the `s:` prefix and signature
- **THEN** the existing session is not loaded

### Requirement: Session identifiers are random and never chosen by the client
New session identifiers SHALL be 24 bytes from the operating system's random generator, encoded as 32 base64url
characters. A failure of the generator MUST produce an error response, never a weaker identifier. A correctly signed
identifier that the store does not have MUST NOT become the identifier of a session: a new identifier is generated.
The framework MUST NOT log session identifiers, and the `Debug` output of a session MUST NOT contain its identifier
or its data.

#### Scenario: Identifier shape
- **WHEN** 1000 sessions are created
- **THEN** every identifier has 32 characters of the base64url alphabet and no two are equal

#### Scenario: Signed but unknown identifier
- **WHEN** a request carries a validly signed cookie for an identifier that is not in the store, and the handler
  writes to the session
- **THEN** the saved session has a different identifier and the response sets a cookie for it

#### Scenario: Captured logs
- **WHEN** the session test suite runs with the framework logger captured
- **THEN** no captured line contains a session identifier, a secret or a cookie value

### Requirement: Session data is available on the request
With the session middleware installed, `Request::session()` SHALL return the session from the first guard to the
last interceptor, with operations to read, write and remove keys, clear the data, regenerate the identifier and
destroy the session. Without the middleware it SHALL return nothing. The keys `cookie` and `__corvis` are reserved:
writing them MUST fail and reading MUST NOT expose them.

#### Scenario: Value kept between requests
- **WHEN** a handler sets `views` to `1` and a later request presents the cookie it received
- **THEN** the later handler reads `1`

#### Scenario: Reserved key
- **WHEN** a handler calls `session.set("cookie", ..)` or `session.set("__corvis", ..)`
- **THEN** the call returns an error and the session is unchanged

#### Scenario: Middleware not installed
- **WHEN** the application has no `SessionModule`
- **THEN** `Request::session()` returns nothing

### Requirement: Saving rules match express-session
With the same explicit `resave`, `save_uninitialized` and `rolling` options, whether the store is written (create,
update or touch) and whether a `Set-Cookie` header is sent MUST match `express-session` 1.19.0 for new and existing,
modified and unmodified sessions, where "modified" means that the session data differs from the data loaded. The
store write MUST complete before the response is returned.

#### Scenario: Explicit options
- **WHEN** the recorded sequences for `resave: false, saveUninitialized: false`, for `rolling: true` and for
  `saveUninitialized: true` are replayed
- **THEN** each response has a `Set-Cookie` header exactly when the recorded original response has one, and the store
  operations are the recorded ones

#### Scenario: Value set to itself
- **WHEN** a handler sets a key to the value it already has
- **THEN** the session counts as unmodified, as in the recorded original

### Requirement: Sessions are not saved by default until modified
`resave` and `save_uninitialized` SHALL default to `false`, so an anonymous visit creates no session and no cookie.
Documented divergence: the original defaults both to `true` and warns that the options must be given.

#### Scenario: Anonymous request
- **WHEN** a request without a cookie reaches a handler that does not touch the session
- **THEN** the response has no `Set-Cookie` header and the store is unchanged

### Requirement: Cookie attributes have secure defaults
The session cookie SHALL be sent with `Path=/`, `HttpOnly`, `SameSite=Lax` and `Secure` by default, and without
`Domain`, `Max-Age` or `Expires`. Each attribute SHALL be configurable. Documented divergence: the original sends
neither `SameSite` nor `Secure` by default. With explicit attributes equal to the original's, the attributes sent
MUST equal the recorded original's.

#### Scenario: Defaults
- **WHEN** a session is created with the default options
- **THEN** the `Set-Cookie` header has `Path=/; HttpOnly; Secure; SameSite=Lax` and no expiry attribute

#### Scenario: Attributes of the original
- **WHEN** the cookie is configured with `secure(false)`, no `SameSite` and a `max_age`, as the recorded original app
- **THEN** the attributes of the `Set-Cookie` header equal the recorded ones

### Requirement: The session cookie does not depend on the transport
The session cookie SHALL be sent with its configured attributes on every connection. Documented divergence: the
original withholds a `Secure` cookie on a connection it does not consider secure; Corvis sends it and the browser
decides.

#### Scenario: Secure cookie over plain HTTP
- **WHEN** a session is created over a plain HTTP connection with the default options
- **THEN** the response has the `Set-Cookie` header with `Secure` (the fixture marks the divergence)

### Requirement: Unsafe cookie configurations are caught at startup
The application MUST NOT start when the session cookie has `SameSite=None` without `Secure`; when its name starts
with `__Host-` (in any letter case) and it lacks `Secure`, has a `Domain` or a `Path` other than `/`; when its name
starts with `__Secure-` (any case) and it lacks `Secure`; when the name, path or domain is not valid for a cookie; or
when `max_age` is longer than the absolute timeout. Turning `Secure` or `HttpOnly` off SHALL log a startup warning.

#### Scenario: SameSite=None without Secure
- **WHEN** the options set `same_site(SameSite::None)` and `secure(false)`
- **THEN** `init()` fails with an error that names both options

#### Scenario: Host prefix
- **WHEN** the cookie name is `__Host-sid` and a `Domain` is configured
- **THEN** `init()` fails with an error that names the prefix rule

#### Scenario: Prefix in another letter case
- **WHEN** the cookie name is `__host-sid` with a `Domain`, or `__SECURE-sid` with `secure(false)`
- **THEN** `init()` fails with an error that names the prefix rule, as it does for `__Host-sid` and `__Secure-sid`

#### Scenario: Secure turned off
- **WHEN** the options set `secure(false)`
- **THEN** the application starts and logs one warning that the session cookie is sent over plain HTTP

#### Scenario: Invalid name cannot panic later
- **WHEN** the cookie name contains `;`
- **THEN** `init()` fails, and no request can reach a panic in cookie serialization

### Requirement: Sessions expire by idle time and by age
A session MUST NOT be loaded when more than the idle timeout (default 24 hours) has passed since its last access, or
more than the absolute timeout (default 7 days) since its identifier was generated, or when its cookie expiry has
passed: the record is destroyed and the request runs with a new, empty session. Documented divergence: the original
has no idle or absolute timeout of its own.

#### Scenario: Idle session
- **WHEN** a session was last used 24 hours and one second ago (test clock) and its cookie arrives
- **THEN** the request runs with a new, empty session and the old record is no longer in the store

#### Scenario: Active session reaches the absolute timeout
- **WHEN** a session is used every hour for 7 days and one hour (test clock)
- **THEN** the last request runs with a new, empty session

#### Scenario: Activity extends the idle time only
- **WHEN** a session is used after 23 hours and again 23 hours later
- **THEN** both requests see the session data

### Requirement: Expiry is enforced by the middleware
The expiry checks MUST be made by the middleware from timestamps kept in the session record, whatever the store does
with the time to live it is given. A record without those timestamps SHALL be accepted and given them at its first
load.

#### Scenario: Store that ignores the time to live
- **WHEN** a custom store keeps records forever and an idle session's cookie arrives
- **THEN** the session is still refused and destroyed

#### Scenario: Record written by the original
- **WHEN** a record without timestamps (created by the original) is loaded
- **THEN** it is accepted, and its timestamps start at that moment

### Requirement: Timeouts are bounded
The idle timeout MUST be between 60 seconds and the absolute timeout, and the absolute timeout at most 400 days;
other values MUST stop the application at startup. Disabling the absolute timeout SHALL require an explicit option
and log a startup warning.

#### Scenario: Absolute timeout disabled
- **WHEN** the options call `no_absolute_timeout()`
- **THEN** the application starts and logs one warning that sessions have no maximum age

#### Scenario: Timeout out of range
- **WHEN** the idle timeout is 10 seconds, or longer than the absolute timeout
- **THEN** `init()` fails with an error that names the option

### Requirement: Session records are compatible with express-session
The record given to the store SHALL be the session data as a JSON object with the `cookie` object of
`express-session` 1.19.0 (same keys and value formats as the recorded original for the same cookie settings) and the
serialized user under `passport.user`. Corvis's own timestamps SHALL live under the single reserved key `__corvis`.

#### Scenario: Record shape
- **WHEN** a session with one value and a logged-in user is saved
- **THEN** the record without `__corvis` equals the recorded original record for the same steps, apart from the
  cookie attributes that differ by configuration

### Requirement: Session size is bounded
A session whose serialized record exceeds `max_session_bytes` (default 16 KiB; allowed range 1 KiB to 1 MiB, other
values stop the application at startup) MUST NOT be written to the store: the response SHALL be a 500 and no session
cookie SHALL be sent.

#### Scenario: Oversized session
- **WHEN** a handler stores a 20 KiB string in the session with the default limit
- **THEN** the response is a 500, the store has no such record, and there is no `Set-Cookie` for the session

### Requirement: Session nesting depth is bounded when written
A record nested deeper than 100 levels, the object that holds the session counting as level 1, MUST NOT be written: the
response SHALL be the 500 of an oversized session and no cookie SHALL be sent. A stored record deeper than that is no
session when read. No write may fall back to a null or empty record when a stored one cannot be read. The limit leaves
room under the 128 levels a JSON parser reads.

#### Scenario: Deeply nested value
- **WHEN** a handler stores a value nested 127 levels, which the body parser accepts
- **THEN** the response is a 500 and nothing is written, and a logged-in session stays logged in

### Requirement: A stored record over the size limit is not a session
A record that the store returns larger than `max_session_bytes` MUST be treated as no session: it is not loaded, its
identifier is not adopted and the record is not deleted. A warning, at most one a minute, SHALL report it without
naming an identifier. Otherwise every write of that session would fail with a 500.

#### Scenario: Record of 200 KB in a shared store
- **WHEN** a valid cookie names a record of 200 KB and the limit is the default
- **THEN** the request is anonymous, a write answers 200 with a new identifier, and the old record is still in the store

### Requirement: Session store contract
The framework SHALL define a `SessionStore` trait with `load`, `create`, `update`, `touch` and `destroy`, and the
middleware SHALL use the store given in the options for every operation. `create` MUST fail when the identifier
already exists. `touch` has no default: every store implements it and it MUST change only the expiry and the
timestamps, never the data. `destroy` MUST report whether it removed a record.

#### Scenario: Custom store
- **WHEN** an application provides its own `SessionStore` implementation in the options
- **THEN** the middleware uses it for every operation

#### Scenario: Identifier collision
- **WHEN** `create` is called with an identifier the store already has
- **THEN** it fails and the existing record is unchanged

### Requirement: A destroyed session cannot be re-created
`update` and `touch` MUST NOT create a record: when the identifier no longer exists they report it. A session that a
request loaded from the store and whose record is gone when the request ends SHALL be treated as destroyed: nothing is
created or updated, whatever the request did (`regenerate_keeping_data`, `regenerate`, a data change, a touch), and the
cookie is cleared. Documented divergence: the original's `store.set` creates or overwrites.

#### Scenario: Destroyed session does not come back
- **WHEN** request A loads a logged-in session, request B logs out (destroying it) and then request A modifies the
  session and finishes
- **THEN** the store has no record under the old identifier, and a later request with the old cookie is anonymous

#### Scenario: Logout racing a regeneration that keeps the data
- **WHEN** request A (logged in) is held in its handler, request B logs out, and A then calls
  `regenerate_keeping_data()` and finishes
- **THEN** A creates no record, its response only clears the cookie, and no cookie of A is logged in

#### Scenario: The same race for destroy, regenerate, a data change and a touch
- **WHEN** request B ends the session with a logout or with `destroy()` while request A, held in its handler, goes on
  to call `regenerate()`, change data, or change nothing
- **THEN** in every case A writes nothing and sends only a clearing cookie

### Requirement: Store errors fail closed
A store error while loading or saving SHALL produce a 500. Store errors, the response and the log line MUST NOT
contain session identifiers or data.

#### Scenario: Store failure on load
- **WHEN** the store returns an error while loading
- **THEN** the response is a 500 whose body and log line contain no identifier

### Requirement: The memory store is bounded
`MemorySessionStore` SHALL hold at most `max_sessions` records (default 100 000) and `max_total_bytes` (default
256 MiB), drop expired records without a background task, and, when a new record does not fit, evict records until it
does, logging a warning at most once a minute that contains no identifier. Using the memory store SHALL log at startup
that sessions are lost on restart and not shared between instances.

#### Scenario: Count bound
- **WHEN** a store with `max_sessions(100)` receives 10 000 new sessions
- **THEN** it never holds more than 100, and the 100 most recently used are the ones kept

#### Scenario: Byte bound
- **WHEN** a store with `max_total_bytes` of 1 MiB receives sessions of 10 KiB
- **THEN** its total stays at or under 1 MiB

#### Scenario: Expired records leave
- **WHEN** records pass their time to live (test clock) and other sessions are written
- **THEN** the expired records are gone without any of them being loaded

### Requirement: The memory store accounts for the memory it uses
`MemorySessionStore` MUST keep each record serialized and count against `max_total_bytes` the allocation of the
record, its identifier, its user key and a fixed overhead per entry, so that the live heap of the store never exceeds
the bytes it accounts for, whatever the shape of the data. Its tables MUST give back their room when they hold less
than a quarter of it.

#### Scenario: Adversarial shapes
- **WHEN** records made of arrays of `0`, arrays of `{}`, objects with many short keys, or long strings fill a store
  with a counting allocator
- **THEN** the live heap of the store is at most the bytes it accounts for, and those are at most `max_total_bytes`

#### Scenario: Create, destroy, refill
- **WHEN** 100 000 minimal sessions are created and destroyed, and the store is then refilled with large records
- **THEN** almost nothing stays live after the destruction, and the live heap of the refill is at most the bound

### Requirement: The memory store does no work proportional to a record under its lock
`MemorySessionStore` MUST NOT parse, serialize or copy a record while holding its lock: a load clones a reference to
the record and parses it afterwards, a create or update serializes before the lock, and a touch replaces only a small
fragment (`cookie`, `__corvis`) kept apart from the record text. A record that does not parse MUST be destroyed on
load and be no session, with a warning at most once a minute.

#### Scenario: Large sessions beside small ones
- **WHEN** 16 tasks keep loading and touching one 15 KiB session while another user makes requests
- **THEN** the other user's p99 latency and throughput stay within a small factor of a run where the 16 tasks use a
  tiny session

#### Scenario: Unreadable record
- **WHEN** the store holds text that is not JSON under an identifier and it is loaded
- **THEN** the load finds no session and the record is gone

### Requirement: The memory store caps the sessions of one user
`MemorySessionStore` SHALL hold at most `max_sessions_per_user` sessions of one user (default 16, at least 1, checked
at startup), keyed by the serialized stored user. A new session of a user at the cap MUST evict that user's oldest
session and never another user's. Other stores need their own policy, which the docs SHALL say.

#### Scenario: One account logs in over and over
- **WHEN** one account logs in 40 times without cookies while another user has a session, with a cap of 4
- **THEN** the other user's session is still valid, and only the account's 4 newest sessions are

### Requirement: Log in and log out regenerate the session
`Authenticator::log_in` SHALL generate a new session identifier, store the serialized user under `passport.user`,
set the request's user, and destroy the old identifier's record in the store. `Authenticator::log_out` SHALL remove
the user, generate a new identifier and destroy the old record. This MUST match `passport` 0.7.0
(`sessionmanager.js`) and prevents session fixation.

#### Scenario: Session fixation attempt
- **WHEN** a client logs in while presenting a session cookie obtained before logging in
- **THEN** the response sets a cookie with a different identifier, the store has no record under the old identifier,
  and a request with the old cookie is anonymous

#### Scenario: Login without a previous session
- **WHEN** a client without a cookie logs in
- **THEN** the response sets a session cookie and a later request with it has the user, matching the recorded
  original

#### Scenario: Logout
- **WHEN** a logged-in client calls a route that logs out
- **THEN** the store has no record under the old identifier, and later requests with the old cookie are anonymous

### Requirement: Session data from before a login is dropped by default
Logging in and logging out SHALL discard the session data that existed before, unless `keep_session_info_on_login`
is set, as `passport` 0.7.0 does with and without `keepSessionInfo`.

#### Scenario: Data from before the login
- **WHEN** an anonymous session holds `cart` and the client logs in, with the default options and with
  `keep_session_info_on_login(true)`
- **THEN** `cart` is gone by default and kept with the option, as the recorded original with and without
  `keepSessionInfo`

### Requirement: Login needs session support
`Authenticator::log_in` and `log_out` MUST fail with a 500 and a fixed message, storing nothing, when the request has
no session or no session strategy is registered.

#### Scenario: Login without session support
- **WHEN** `Authenticator::log_in` is called on a request that has no session
- **THEN** it returns a 500 with a fixed message and stores nothing

### Requirement: The session user is restored on later requests
When a session has `passport.user` and a session strategy is registered, the middleware SHALL deserialize the user
through the application's `SessionSerializer` before the guards run and expose it as the request's user, like
`passport.session()`. When deserialization finds no user, the stored user MUST be removed from the session and the
request MUST continue as anonymous. A deserialization error SHALL be the response.

#### Scenario: Logged-in request
- **WHEN** a request presents the cookie of a logged-in session
- **THEN** a guard, an interceptor and the handler all see the user the serializer returned

#### Scenario: User deleted after login
- **WHEN** the serializer reports that the stored user no longer exists
- **THEN** the request is anonymous, `passport.user` is removed from the stored session, and a route with
  `AuthGuard::new("session")` answers the 401

#### Scenario: Deserialization error
- **WHEN** the serializer returns an error for a logged-in session
- **THEN** that error is the response and the handler does not run

### Requirement: Only one session strategy
Only one session strategy may be registered; a second one MUST stop the application at startup.

#### Scenario: Two session strategies
- **WHEN** two `SessionStrategy` providers are registered
- **THEN** `init()` fails

### Requirement: Session strategy protects routes
The strategy named `"session"` SHALL succeed with the request's session user and fail otherwise, so that
`AuthGuard::new("session")` answers `401 {"message":"Unauthorized","statusCode":401}` to anonymous requests, the
response of the original's usual `isAuthenticated()` guard written with `UnauthorizedException`.

#### Scenario: Anonymous request to a protected route
- **WHEN** a request without a session cookie reaches a route with `AuthGuard::new("session")`
- **THEN** the response is the 401 and the handler does not run

#### Scenario: Chain with a token
- **WHEN** a route uses `AuthGuard::chain(["session", "jwt"])` and the request has only a valid bearer token
- **THEN** the handler runs with the token's user

### Requirement: AuthGuard logs in to the session only when asked
`AuthGuard` SHALL NOT write to the session by itself. `AuthGuard::new(name).log_in()` SHALL, after a successful
authentication, log the user in to the session before the handler runs, the login guard pattern of the original
(`super.canActivate(context)` followed by `super.logIn(request)`). A guard with `.log_in()` in an application without
a registered session strategy MUST stop the application at startup.

#### Scenario: Login route
- **WHEN** valid credentials are posted to a route with `AuthGuard::new("local").log_in()`
- **THEN** the response sets a new session cookie and a later request with that cookie has the user, matching the
  recorded original

#### Scenario: Guard without log_in
- **WHEN** a request with a valid bearer token reaches a route with `AuthGuard::new("jwt")` in an application with
  sessions
- **THEN** the handler runs with the user, no session is created and no session cookie is sent

#### Scenario: Failed login
- **WHEN** wrong credentials are posted to the login route while presenting an anonymous session cookie
- **THEN** the response is the 401, the session identifier is unchanged and the session has no user

#### Scenario: log_in without a session strategy
- **WHEN** a route uses `AuthGuard::new("local").log_in()` and no `SessionStrategy` is registered
- **THEN** `init()` fails with an error that names the route and the missing session strategy

### Requirement: A login made by the guard is discarded when the request is rejected
A login made by `AuthGuard::log_in()` MUST be discarded when the pipeline rejects the request: a later guard denies or
ends it, a pipe or validation fails, the handler returns an error or panics, an interceptor fails. The router marks the
rejection when it happens, before any exception filter or interceptor can rewrite the response, so the final status
never decides. A handler that returns normally keeps the login. Documented divergence: the original keeps it.

#### Scenario: Second guard refuses
- **WHEN** a route has `AuthGuard::new("header").log_in()` followed by a guard that refuses
- **THEN** the response is 403 with no `Set-Cookie`, and a presented anonymous session is unchanged

#### Scenario: A filter rewrites the failure
- **WHEN** a global or route filter turns the denial, the handler error, a panic or a failing pipe into a redirect or a
  200 page, or an interceptor turns the handler error into a normal response
- **THEN** the response has no logged-in cookie and a presented anonymous session is unchanged

#### Scenario: Handler decides
- **WHEN** a handler logs in with `Authenticator::log_in` and answers 403, or returns normally with a redirect after
  the login guard
- **THEN** the response carries the logged-in session cookie

### Requirement: A cancelled request keeps no guard login
When the response is never produced (the client disconnects, a timeout) a login made by the guard MUST NOT be stored.
The identifier the login replaced is destroyed, not restored.

#### Scenario: Disconnect during a guard login
- **WHEN** the request future is dropped after the login guard ran
- **THEN** no record holds the user, and the old cookie is anonymous

### Requirement: A login that keeps nothing cannot be made to fail
A login that carries nothing of the previous session (`keep_session_info_on_login` off) MUST create the new session even
when the previous record is gone, still destroying the old identifier. The rule of `A destroyed session cannot be
re-created` stays for logins that keep data, `regenerate_keeping_data()`, saves and touches.

#### Scenario: Previous session evicted during login
- **WHEN** the anonymous session a login arrived with is evicted while the login runs
- **THEN** the response sets a logged-in cookie and the next request is logged in

#### Scenario: Logout racing a login that keeps data
- **WHEN** request A logs out a session while request B logs in on it with `keep_session_info_on_login(true)`
- **THEN** B writes nothing and its cookie is only a clearing one

### Requirement: Invalid session options stop the application
`SessionModule` MUST stop the application at startup when its options are invalid: cookie keys that `http/signed-cookies`
refuses, limits or timeouts out of range, memory store bounds of zero. The error MUST name the option and MUST NOT
contain a secret.

#### Scenario: Short secret
- **WHEN** the session secret from configuration has 20 bytes
- **THEN** `init()` fails with an error that names the session secret and the 32-byte minimum, without the secret

### Requirement: Sessions are verified against the original
Sessions SHALL be covered by a differential test: a generator in the fixture package of `auth/strategies` records,
for `express-session` 1.19.0 with `passport` 0.7.0, the cookies, responses and store operations of fixed request
sequences, and a Rust test replays every sequence without Node. A step where Corvis diverges on purpose MUST be
marked in the fixture with its reason and asserted from this specification.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis --features auth-session --test auth_session` runs on a machine without Node
- **THEN** every recorded step is compared (status, body, presence and attributes of `Set-Cookie`, store operations,
  final record) and every marked divergence asserts the behavior this specification states

#### Scenario: Arbitrary cookie headers
- **WHEN** a property test sends random `Cookie` headers, including mutations of valid session cookies
- **THEN** the middleware never panics, never answers with an error because of the cookie, and loads a session only
  for a correctly signed identifier that is in the store

### Requirement: Repeated session cookies are not believed
A request with more than one cookie of the session name, in one `Cookie` header or several, MUST be treated as having
no session cookie, so a cookie planted from a sibling subdomain cannot pick the session. Documented divergence:
`Request::cookie` and the original take the first one.

#### Scenario: Two valid cookies
- **WHEN** a request carries two validly signed session cookies, of two different sessions or of the same one
- **THEN** no session is loaded and the store is not read

### Requirement: Ended sessions clear the cookie
When the session that a verified cookie named is destroyed with `Session::destroy` or found expired, and no new session
is saved in the same request, the response SHALL clear the cookie. Documented divergence: the original sends nothing
after `destroy()`.

#### Scenario: Idle session in a store that keeps records
- **WHEN** an idle session's cookie arrives and the store still holds the record
- **THEN** the record is destroyed and the response expires the cookie

### Requirement: Destruction survives a cancelled request
Identifiers queued for destruction by a login, a logout, `regenerate` or `destroy` MUST still be destroyed in the store
when the request is cancelled (the client went away) before the session is committed.

#### Scenario: Client disconnects after destroy
- **WHEN** a handler destroys the session and the request future is dropped before it returns
- **THEN** the record is gone from the store

### Requirement: Store calls are bounded in time
Every call to the store MUST be bounded by `store_timeout` (default 5 seconds, allowed 100 milliseconds to 60 seconds,
other values stop the application at startup). A call that takes longer fails like a store error, as a 500.

#### Scenario: Slow store
- **WHEN** a store takes longer than `store_timeout` to load a session
- **THEN** the response is the 500 of any store failure

### Requirement: Anonymous sessions are evicted first
When the memory store is full it MUST evict the least recently used session without a logged-in user
(`passport.user`) first, then the oldest session of the user with the most sessions, so a flood of anonymous sessions
cannot evict logged-in users.

#### Scenario: Flood of anonymous sessions
- **WHEN** 10 000 anonymous sessions are created in a store with room for 10 that holds 5 logged-in sessions
- **THEN** the 5 logged-in sessions are still there

#### Scenario: Only users left
- **WHEN** a full store holds only logged-in sessions, one user with three and others with two or one
- **THEN** the oldest session of the user with three is evicted

### Requirement: An empty user is never restored or stored
A stored user, or a user the serializer returns, that is JSON `null`, `false`, `0` or `""` MUST NOT authenticate a
request: it is removed from the session. `Authenticator::log_in` MUST refuse such a user, or a serialized value of
that kind, with a 500 and store nothing.

#### Scenario: Stored empty user
- **WHEN** a session record holds `passport.user` equal to `null`, `false`, `0` or `""`
- **THEN** the request is anonymous and `passport.user` is removed from the session

### Requirement: Regenerate matches the original, with a variant that keeps data
`Session::regenerate()` SHALL start a new empty session with a new identifier, as `req.session.regenerate()` does.
`Session::regenerate_keeping_data()` SHALL replace the identifier and keep the data. Both destroy the old record when
the request ends.

#### Scenario: Regenerate drops the data
- **WHEN** a handler regenerates a session that holds a value
- **THEN** the new session is empty, matching the recorded original, and the old record is gone

### Requirement: The session applies to its cookie path
A request whose path does not start with the cookie `Path` MUST run without a session, as in the original.

#### Scenario: Request outside the path
- **WHEN** the cookie path is `/app` and the request path is `/other`
- **THEN** `Request::session()` returns nothing

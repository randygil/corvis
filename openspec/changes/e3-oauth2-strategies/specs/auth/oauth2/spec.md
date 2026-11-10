# Spec Delta

## Purpose

OAuth2 login strategies: the authorization code flow of `passport-oauth2` 1.8.0 with the Google and GitHub profiles
of `passport-google-oauth20` 2.0.0 and `passport-github2` 0.1.12 and the refresh request of `passport-oauth2-refresh`
2.2.0, available under the `auth-oauth2` cargo feature. Where this specification is stricter than the original, the
requirement names the divergence.

## ADDED Requirements

### Requirement: OAuth2 strategies are registered through a validator
An `OAuth2Strategy<V>` registered with `Provider::strategy` SHALL be the strategy named `V::NAME`, built from
`V::options()` once at startup.

#### Scenario: Strategy name
- **WHEN** a validator with `const NAME = "google"` is registered with `Provider::strategy::<OAuth2Strategy<V>>()`
- **THEN** `AuthGuard::new("google")` runs it, and `options()` was called once during `init()`

### Requirement: The validator receives the grant
On a successful callback the strategy SHALL call `V::validate` with the request and an `OAuth2Grant`. `Ok(Some(user))`
SHALL be a success with that user, `Ok(None)` a failure, and `Err(e)` the error `e`, as defined by `auth/strategies`.

#### Scenario: Validator receives the grant
- **WHEN** the callback of the strategy `"google"` completes against the mock provider
- **THEN** `validate` receives the access token and refresh token the mock issued, the whole token response and the
  Google profile, and the handler reads the returned user with `User<T>`

#### Scenario: Provider sends no refresh token
- **WHEN** the token response has no `refresh_token`
- **THEN** the grant's refresh token is absent

#### Scenario: No profile source
- **WHEN** a generic strategy has no userinfo URL configured
- **THEN** the grant's profile is absent and no profile request is sent

#### Scenario: Validator rejects the user
- **WHEN** `validate` returns `Ok(None)`
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}`

#### Scenario: Validator fails
- **WHEN** `validate` returns `Err(ForbiddenException)`
- **THEN** the response is that 403

### Requirement: Authorization redirect
On a `GET` request with neither `code` nor `error` in the query, the strategy MUST answer a 302 whose `Location` is
the configured authorization URL with `response_type=code`, `client_id`, `redirect_uri`, `state`, `code_challenge` and
`code_challenge_method=S256`. No other `response_type` SHALL be expressible.

#### Scenario: Login starts
- **WHEN** a route guarded by the Google strategy with scopes `["email", "profile"]` receives `GET /auth/google`
- **THEN** the response is a 302 whose `Location` has the parameters of the recorded original (ignoring the values of
  `state` and `code_challenge`) plus `state`, `code_challenge` and `code_challenge_method=S256`

#### Scenario: Redirect outside the guard phase
- **WHEN** the strategy is run from an interceptor through the `Authenticator`
- **THEN** the result is the 500 `Internal server error`, no state cookie is sent and the handler does not run

### Requirement: Authorization parameters match the original
The authorization URL SHALL keep the parameters already present in the configured URL and add `scope` and the
configured extra parameters as the recorded original writes them.

#### Scenario: Scope joined by a space
- **WHEN** the scope is `["email", "profile"]`
- **THEN** the `scope` parameter is `email profile`

#### Scenario: Empty scope
- **WHEN** a generic or GitHub strategy has no scope
- **THEN** the redirect has no `scope` parameter

#### Scenario: Parameters already in the URL
- **WHEN** the configured authorization URL is `https://provider.example/authorize?tenant=a`
- **THEN** the redirect keeps `tenant=a` next to the added parameters

#### Scenario: Extra parameters
- **WHEN** the options add the authorization parameter `access_type=offline`
- **THEN** the redirect carries `access_type=offline`, as the recorded original does

#### Scenario: Scope from the guard
- **WHEN** the guard is `AuthGuard::new("github").with_options(json!({ "scope": ["read:org"] }))`
- **THEN** the `scope` parameter of the redirect is `read:org`

### Requirement: Flow responses are not cached
The authorization redirect and the callback response MUST carry `Cache-Control: no-store` and
`Referrer-Policy: no-referrer`.

#### Scenario: Headers of the redirect
- **WHEN** the login starts
- **THEN** the 302 has `Cache-Control: no-store` and `Referrer-Policy: no-referrer`

#### Scenario: Headers of the callback
- **WHEN** a callback is answered
- **THEN** the response has both headers unless the handler set its own values for them

### Requirement: State and PKCE values are random per login
Each authorization request MUST carry a new `state` and a new PKCE verifier, each 32 bytes from the operating
system's random generator encoded as 43 base64url characters, with `code_challenge` equal to the base64url SHA-256 of
the verifier.

#### Scenario: Two logins
- **WHEN** two authorization redirects are produced
- **THEN** their `state` values differ, their `code_challenge` values differ, and each is 43 base64url characters

#### Scenario: Challenge matches the verifier
- **WHEN** the callback completes
- **THEN** the token request carries a `code_verifier` whose base64url SHA-256 equals the `code_challenge` of the
  redirect of the same login

### Requirement: State and PKCE cannot be weakened silently
The `state` check MUST NOT be possible to disable, and the `plain` challenge method MUST NOT be expressible. Disabling
PKCE SHALL be an explicit option that logs a warning at startup. Documented divergence: the original sends neither
`state` nor a challenge unless `state: true` and `pkce: true` are configured with a session.

#### Scenario: PKCE turned off
- **WHEN** the options call `.pkce(false)`
- **THEN** a warning naming the strategy is logged at startup, the redirect has no `code_challenge`, and `state` is
  still sent and checked

#### Scenario: Default options
- **WHEN** a strategy is built with default options
- **THEN** its redirect carries `state` and `code_challenge_method=S256`, where the recorded original carries neither

### Requirement: The state record is bound to the browser
The strategy MUST keep the state record either in a cookie signed with `CookieKeys` or in the session, as configured.
A strategy with neither configured MUST stop application startup.

#### Scenario: Record contents
- **WHEN** a login starts with a `return_to` that was accepted
- **THEN** the stored record holds the strategy name, the `state`, the PKCE verifier, the issue time and the
  `return_to`

#### Scenario: No storage chosen
- **WHEN** a strategy's options choose neither the state cookie nor the session
- **THEN** `init()` fails with an error that names the strategy and the two options

### Requirement: The state cookie has secure attributes
The state cookie MUST be `HttpOnly`, `SameSite=Lax`, `Path=/`, without `Domain`, with `Max-Age` equal to the state
lifetime, and by default `Secure` with the name `__Host-oauth2.<strategy name>`. Turning `Secure` off SHALL be an
explicit option that removes the `__Host-` prefix and logs a warning at startup.

#### Scenario: Cookie attributes
- **WHEN** the login starts with cookie storage and default options
- **THEN** the response has one `Set-Cookie` named `__Host-oauth2.google` with `Path=/`, `HttpOnly`, `Secure`,
  `SameSite=Lax` and `Max-Age=600`, whose value verifies with the configured keys

#### Scenario: Plain-HTTP development
- **WHEN** the options turn `Secure` off for the state cookie
- **THEN** the cookie is named `oauth2.google`, has no `Secure` attribute, and a warning is logged at startup

#### Scenario: Prefixed name without Secure
- **WHEN** the options set a cookie name starting with `__Host-` or `__Secure-` and turn `Secure` off
- **THEN** `init()` fails

### Requirement: Cookies without the host prefix warn at startup
A strategy whose state cookie, or whose session cookie when the state lives in the session, has a name without the
`__Host-` prefix MUST log one startup warning naming the strategy and the cookie, saying that a sibling subdomain can
plant a cookie and cause login CSRF, and how to fix it. It is a warning, not an error.

#### Scenario: Custom state cookie name
- **WHEN** the options set `state_cookie_name("oauth_state")`
- **THEN** one warning names the strategy and `oauth_state`, and the application starts

#### Scenario: Session cookie named connect.sid
- **WHEN** a strategy keeps the state in a session whose cookie is `connect.sid`
- **THEN** one warning names the strategy and `connect.sid`; a session cookie named `__Host-sid` logs nothing

### Requirement: Session storage of the state needs a usable session
A strategy that keeps the state record in the session MUST stop startup when sessions are not installed or when the
session cookie is `SameSite=Strict`.

#### Scenario: Session storage without sessions
- **WHEN** a strategy chooses session storage and `SessionModule` is not imported
- **THEN** `init()` fails with an error that names the strategy

#### Scenario: Strict session cookie
- **WHEN** a strategy chooses session storage and the session cookie is `SameSite=Strict`
- **THEN** `init()` fails with an error that says the callback would arrive without the session

#### Scenario: Request without a session
- **WHEN** a login starts on a request the session middleware did not reach
- **THEN** the response is the 500 `Internal server error`

### Requirement: The callback verifies the state before contacting the provider
On a callback (a `GET` request with `code` or `error`), the strategy MUST fail, without sending any request to the
provider, unless a state record exists for this strategy, is within the state lifetime, and holds the `state` of the
query. The comparison MUST be constant-time.

#### Scenario: Forged callback
- **WHEN** the callback receives `?code=abc&state=forged` with a valid state cookie
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}` and the mock provider receives no
  request

#### Scenario: No state record
- **WHEN** the callback receives a `code` and a `state` and no state cookie
- **THEN** the response is the 401 and the mock provider receives no request

#### Scenario: Expired state
- **WHEN** the callback arrives 601 seconds after the login started with the default lifetime
- **THEN** the response is the 401 and the mock provider receives no request

#### Scenario: Record of another strategy
- **WHEN** the callback of `"github"` receives the state cookie value issued by `"google"` with the same keys
- **THEN** the response is the 401

#### Scenario: Tampered state cookie
- **WHEN** one character of the state cookie's value or signature is changed
- **THEN** the response is the 401 and the mock provider receives no request

### Requirement: Callback parameters are well-formed
`code`, `state` and `error` MUST each be absent or a single string within the documented length limits, `code` and
`error` MUST NOT both be present, and the method MUST be `GET`; otherwise the callback MUST fail without contacting
the provider.

#### Scenario: Repeated parameter
- **WHEN** the callback receives `?code=a&code=b&state=...`
- **THEN** the response is the 401 and the mock provider receives no request

#### Scenario: Code and error together
- **WHEN** the callback receives `?code=a&error=access_denied&state=<valid>`
- **THEN** the response is the 401 and the mock provider receives no request

#### Scenario: Oversized code
- **WHEN** the callback receives a `code` longer than the configured limit
- **THEN** the response is the 401 and the mock provider receives no request

#### Scenario: Other method
- **WHEN** a guarded route receives `POST` with `code` and `state` in the query or the body
- **THEN** the response is the 401 and the mock provider receives no request

### Requirement: The state is used once
Every callback, successful or not, MUST remove the state record: with cookie storage by appending a `Set-Cookie`
that deletes the state cookie, with session storage by removing the record from the session before any request to
the provider.

#### Scenario: State cookie deleted
- **WHEN** a callback is answered (success, 401 or 500)
- **THEN** the response has a `Set-Cookie` that expires the state cookie

#### Scenario: Replay with session storage
- **WHEN** a successful callback is sent a second time with the same session cookie
- **THEN** the second response is the 401 and the mock provider receives no second token request

#### Scenario: Replay with cookie storage
- **WHEN** a client keeps the state cookie and repeats a successful callback
- **THEN** the second response is the 401 and the mock provider receives no second token request (see "A state is
  redeemed once per instance")

### Requirement: A state is redeemed once per instance
Each strategy instance MUST remember, for the state lifetime and in a bounded set (default 100000 entries, an option;
the oldest are dropped when full), the verified states it has presented to the token endpoint, and MUST refuse a
callback with a remembered state, including one being redeemed now, before any token request. A state that does not
verify MUST NOT enter the set. This is a cost control per instance, not a guarantee across instances.

#### Scenario: Concurrent replays of one flow
- **WHEN** one flow's callback is waiting on the token endpoint and sixty more callbacks with the same state arrive
- **THEN** all sixty get the 401 and the mock receives one token request

#### Scenario: Forged states do not enter the set
- **WHEN** two hundred callbacks with states that do not verify arrive before the real one
- **THEN** they send no token request and the real callback still succeeds

#### Scenario: The set is bounded
- **WHEN** the set holds 16 entries and 20 flows have completed
- **THEN** the newest state is still refused locally and the oldest is forgotten

### Requirement: One client address cannot hold every in-flight slot
A strategy MUST bound the callbacks one client address (the address the application resolves, honouring trust proxy,
IPv6 by its prefix) can have waiting on the provider to a share of the in-flight limit, a quarter by default and at
least 1, an option. A client over its share MUST be answered 503 without contacting the provider and without
consuming its flow.

#### Scenario: Share reached
- **WHEN** one address holds its share of the slots and sends another callback, while another address sends one
- **THEN** the first is answered 503 with no token request and the second succeeds

#### Scenario: Slots are given back
- **WHEN** the held callbacks finish
- **THEN** the refused flow's callback succeeds

### Requirement: The redirect URI is configured and exact
The redirect URI MUST be a configured absolute URL, and the same string MUST be sent in the authorization request and
in the token request; no part of it SHALL come from the request. Documented divergence: the original resolves a
relative callback URL against the request's host.

#### Scenario: Host header ignored
- **WHEN** the login starts with `Host: evil.example` (and `X-Forwarded-Host: evil.example`)
- **THEN** the `redirect_uri` parameter is the configured URL

#### Scenario: Relative redirect URI
- **WHEN** a strategy is configured with the redirect URI `/auth/google/callback`
- **THEN** `init()` fails with an error that names the strategy and the option

#### Scenario: Unsafe redirect URI
- **WHEN** the redirect URI has a fragment, has user information, or uses `http` with a host that is not loopback
- **THEN** `init()` fails with an error that names the strategy and the option

#### Scenario: Loopback redirect URI
- **WHEN** the redirect URI is `http://127.0.0.1:3000/auth/google/callback`
- **THEN** the application starts

### Requirement: Token exchange request
After the state check, a callback with `code` MUST send one `POST` to the token URL with the form fields
`grant_type=authorization_code`, `code`, `redirect_uri`, `code_verifier` and the client credentials, equal to the
recorded original's request apart from `code_verifier`.

#### Scenario: Same request as the original
- **WHEN** the callback completes with default options
- **THEN** the mock provider received the method, path, content type and form fields recorded for the original, plus
  `code_verifier`

### Requirement: Client authentication
Client credentials SHALL be sent in the form body by default. When HTTP Basic is selected they SHALL be sent in the
`Authorization` header, form-encoded before base64, and MUST NOT appear in the body.

#### Scenario: Credentials in the body
- **WHEN** the callback completes with default options
- **THEN** the token request has `client_id` and `client_secret` in the body and no `Authorization` header

#### Scenario: Basic authentication
- **WHEN** the strategy selects Basic client authentication with a secret containing `:` and `%`
- **THEN** the token request has `Authorization: Basic` of the form-encoded id and secret, and no `client_secret` in
  the body

### Requirement: Token response formats
The token response SHALL be accepted as JSON or as a form-encoded body, and MUST contain a non-empty `access_token`.

#### Scenario: JSON token response
- **WHEN** the mock answers `{"access_token":"t","token_type":"Bearer"}` as `application/json`
- **THEN** the validator receives the access token `t`

#### Scenario: Form-encoded token response
- **WHEN** the mock answers `access_token=t&token_type=bearer` as `application/x-www-form-urlencoded`
- **THEN** the validator receives the access token `t`

#### Scenario: Token response without access token
- **WHEN** the mock token endpoint answers 200 `{}`
- **THEN** the response is the 500 `Internal server error`

#### Scenario: Unknown token type
- **WHEN** the mock answers a `token_type` other than `bearer` (ignoring case)
- **THEN** the response is the 500 and no profile request is sent

### Requirement: Provider requests use verified TLS
Token, refresh and profile requests MUST use `https`, verifying the certificate chain and host name against the
bundled web PKI roots and the configured extra roots, with no option to skip verification. `http` SHALL be accepted
only when the host is a loopback name or address and the connected address is loopback.

#### Scenario: Plain HTTP to a remote host
- **WHEN** a strategy is configured with the token URL `http://provider.example/token`
- **THEN** `init()` fails with an error that names the strategy and the option

#### Scenario: Loopback name resolving elsewhere
- **WHEN** an `http` URL's host resolves to an address that is not loopback
- **THEN** the request is refused without sending anything

#### Scenario: Untrusted certificate
- **WHEN** the mock provider serves TLS with a certificate that is not under a configured root
- **THEN** the callback answers the 500; with that certificate added as an extra root, the callback succeeds

#### Scenario: Wrong host name
- **WHEN** the mock's certificate is under a configured root but names another host
- **THEN** the callback answers the 500

### Requirement: Provider requests go only to the configured host
The client MUST NOT follow a redirect and MUST ignore proxy settings from the environment.

#### Scenario: Token endpoint redirects
- **WHEN** the mock token endpoint answers 302 to another URL
- **THEN** no request reaches that URL and the callback answers the 500 `Internal server error`

#### Scenario: Proxy variables
- **WHEN** `HTTP_PROXY`, `HTTPS_PROXY` and `ALL_PROXY` point at a listener
- **THEN** that listener receives nothing and the request goes to the configured host

### Requirement: Provider requests are bounded
The client MUST enforce a connect timeout and a total timeout, MUST stop reading and fail when a response body passes
the configured size, and MUST bound the number of requests in flight per strategy.

#### Scenario: Provider does not answer
- **WHEN** the mock token endpoint accepts the connection and never answers
- **THEN** the callback answers the 500 after the total timeout and the connection is closed

#### Scenario: Oversized response
- **WHEN** the mock token endpoint streams a body larger than the token response limit, with or without
  `Content-Length`
- **THEN** the callback answers the 500 and the client reads no more than the limit plus one frame

#### Scenario: Oversized headers
- **WHEN** the mock answers a header block larger than 16 KiB
- **THEN** the callback answers the 500

#### Scenario: Too many requests in flight
- **WHEN** the configured number of token requests are in flight and another callback passes the state check
- **THEN** that callback answers 503 without contacting the provider

### Requirement: Request-caused callback failures are answered 401
On a callback with a valid state, an `error` parameter (any value) and a token endpoint answer of 400 or 401 MUST
each make the strategy fail (401). Documented divergence: the original answers 500 for an `error` other than
`access_denied` and for a rejected code.

#### Scenario: User denies consent
- **WHEN** the callback receives `?error=access_denied&error_description=denied&state=<valid>`
- **THEN** the response is the recorded original 401

#### Scenario: Other provider error
- **WHEN** the callback receives `?error=server_error&error_description=<script>&state=<valid>`
- **THEN** the response is the 401, where the recorded original answers 500

#### Scenario: Code rejected
- **WHEN** the mock token endpoint answers 400 `{"error":"invalid_grant"}`
- **THEN** the response is the 401

### Requirement: Provider failures are the fixed 500
A failure of the token or profile request that the callback request did not cause MUST be answered with the 500
`{"statusCode":500,"message":"Internal server error"}`.

#### Scenario: Token endpoint down
- **WHEN** the mock token endpoint answers 500 with a body
- **THEN** the response is the recorded original 500

#### Scenario: Transport failures
- **WHEN** the token request times out, cannot connect, fails the TLS handshake or is answered with a redirect
- **THEN** the response is the 500

#### Scenario: Unusable answers
- **WHEN** the token endpoint answers a status other than 2xx, 400 or 401, an oversized body or a body that cannot be
  parsed
- **THEN** the response is the 500

#### Scenario: Profile request fails
- **WHEN** the token exchange succeeds and the profile endpoint answers 500
- **THEN** the response is the 500 and the validator is not called

### Requirement: Provider text never reaches a response
No response body, header or redirect produced by the strategy SHALL contain text received from the provider or from
the callback query. A failure's challenge SHALL hold a fixed reason and, for a provider error, the error code only
when it is 1 to 64 characters of `[A-Za-z0-9_.-]`.

#### Scenario: Description not echoed
- **WHEN** the callback receives `?error=server_error&error_description=<script>&state=<valid>`
- **THEN** the response body and headers do not contain the description

#### Scenario: Challenge of a provider error
- **WHEN** that callback runs behind an optional guard
- **THEN** `Request::auth_info()` is `{"message":"provider error","error":"server_error"}`, and
  `AuthGuard::on_failure` receives the same challenge

#### Scenario: Malformed error code
- **WHEN** the `error` parameter is 200 characters long or contains `<`
- **THEN** the challenge is `{"message":"provider error"}` without an `error` key

#### Scenario: Provider body not echoed
- **WHEN** the mock token endpoint answers 500 with a body
- **THEN** the 500 response does not contain the provider's body

### Requirement: Failure redirect
When a failure redirect is configured, every failure of a callback, including the validator's refusal (no user or an
empty one), SHALL be answered with a 302 to that URL, sent as configured with nothing added from the request. The URL MUST be a relative path that satisfies the return-to rule or
an absolute `https` URL; any other value MUST stop startup.

#### Scenario: Denied consent with a failure page
- **WHEN** the strategy has `failure_redirect("/login?error=oauth")` and the callback receives
  `?error=access_denied&state=<valid>`
- **THEN** the response is a 302 with `Location: /login?error=oauth`

#### Scenario: Validator refusal
- **WHEN** a failure redirect is configured and `validate` returns no user or an empty one
- **THEN** the response is the 302, and without a failure redirect it is the 401

#### Scenario: Errors are not redirected
- **WHEN** a failure redirect is configured and the token endpoint is down
- **THEN** the response is the 500, not a redirect

#### Scenario: Login start is not affected
- **WHEN** a failure redirect is configured and the login starts
- **THEN** the response is the redirect to the provider

#### Scenario: Unsafe failure page
- **WHEN** a strategy is configured with `failure_redirect("//evil.example")`
- **THEN** `init()` fails

### Requirement: Return-to accepts only a safe relative path
When a return-to parameter name is configured, the login start SHALL read that query parameter and keep it only if it
is a single string of at most 1024 bytes that starts with `/`, whose second character is neither `/` nor `\`, and that
contains no backslash, control character or non-ASCII byte. Any other value MUST be dropped and the login MUST
continue.

#### Scenario: Unsafe values dropped
- **WHEN** the login starts with `return_to` set to `https://evil.example`, `//evil.example`, `/\evil.example`,
  `/a\b`, a path with a control character, or a 2000-byte path
- **THEN** the login redirects to the provider as usual and `oauth2::return_to(&req)` after the callback is `None`

#### Scenario: Repeated parameter
- **WHEN** the login starts with `?return_to=/a&return_to=/b`
- **THEN** the value is dropped and the login continues

#### Scenario: Option not set
- **WHEN** no return-to parameter name is configured and the login starts with `?return_to=/cats`
- **THEN** the parameter is ignored

### Requirement: Return-to travels in the state record
A kept return-to value SHALL be stored in the state record, MUST NOT appear in the authorization URL, and SHALL be
returned by `oauth2::return_to(&request)` only after a successful callback of that login.

#### Scenario: Relative path kept
- **WHEN** the login starts with `?return_to=/cats/7?tab=toys` and the callback succeeds
- **THEN** `oauth2::return_to(&req)` in the callback handler is `/cats/7?tab=toys`

#### Scenario: Not sent to the provider
- **WHEN** the login starts with an accepted `return_to`
- **THEN** the authorization URL contains it neither as a parameter nor inside `state`

#### Scenario: Callback query is not consulted
- **WHEN** the callback URL carries `&return_to=/admin` that was not part of the login start
- **THEN** `oauth2::return_to(&req)` is `None`

#### Scenario: Failed callback
- **WHEN** the callback of a login with an accepted `return_to` fails
- **THEN** `oauth2::return_to(&req)` is `None`

### Requirement: Provider presets use the original's endpoints
The Google and GitHub presets SHALL use the default authorization, token and profile URLs of
`passport-google-oauth20` 2.0.0 and `passport-github2` 0.1.12.

#### Scenario: Google endpoints
- **WHEN** `OAuth2Options::google(..)` is built
- **THEN** its authorization, token and userinfo URLs equal the ones recorded from the original

#### Scenario: GitHub endpoints
- **WHEN** `OAuth2Options::github(..)` is built
- **THEN** its authorization, token and user URLs equal the ones recorded from the original

### Requirement: Normalized Google and GitHub profiles
From the same provider responses, the presets SHALL build a profile whose JSON form equals field by field the profile
the original passes to `validate()`, apart from `_raw` and the added e-mail flags.

#### Scenario: Google profile
- **WHEN** the mock userinfo endpoint returns the recorded Google document
- **THEN** the profile passed to the validator equals the recorded original profile in `provider`, `id`,
  `displayName`, `name`, `emails`, `photos` and `_json`

#### Scenario: GitHub profile
- **WHEN** the mock returns the recorded `/user` document
- **THEN** the profile equals the recorded original profile in `provider`, `id`, `displayName`, `username`,
  `profileUrl`, `emails`, `photos` and `_json`

#### Scenario: Generic userinfo
- **WHEN** a generic strategy has a userinfo URL and the mock returns an OpenID userinfo document
- **THEN** the profile uses the Google mapping with `provider` set to the strategy name

### Requirement: A profile's provider namespaces its id
The profile's `provider` MUST be the preset's name only when the strategy uses that preset's own endpoints
unchanged; when any endpoint was replaced, and for a generic strategy, it MUST be the strategy's name, or `name@issuer` when that name is a preset's. The profile
SHALL also carry `issuer`, the host of the authorization endpoint. Strategy names are unique in the application.

#### Scenario: Three namespaces for one subject
- **WHEN** the same subject id is returned through the GitHub preset with its own endpoints, the GitHub preset with
  replaced endpoints in a strategy named `corp`, and a generic strategy named `sso`
- **THEN** `provider` is `github`, `corp` and `sso`

#### Scenario: Issuer
- **WHEN** a profile is built
- **THEN** `issuer` is the host of the authorization endpoint, with its port when the URL has one

### Requirement: A record with an impossible timestamp is invalid
A state record whose issue time cannot be compared with the clock without overflow MUST be treated as an invalid
flow, never accepted and never a panic.

#### Scenario: Extreme timestamps
- **WHEN** a record signed with the server's key has an issue time of the smallest or largest integer
- **THEN** the callback answers the 401 and sends no token request

### Requirement: Profile requests carry the token in a header
The access token MUST be sent in the `Authorization: Bearer` header of profile requests and never in a URL.

#### Scenario: Bearer header
- **WHEN** the Google or GitHub profile is requested
- **THEN** the mock received a bearer `Authorization` header and no token in the query

### Requirement: GitHub e-mails are requested by scope
The GitHub strategy SHALL request `/user/emails` when the scope contains `user:email` or `user`, and a failure of
that request MUST fail the login.

#### Scenario: GitHub profile with e-mails
- **WHEN** the scope is `user:email` and the mock returns the recorded `/user` and `/user/emails` documents
- **THEN** the profile equals the recorded original profile, and the mock received both requests

#### Scenario: Scope without e-mail access
- **WHEN** the scope is `read:org`
- **THEN** `/user/emails` is not requested and `emails` holds the public address of `/user`, if any, as unverified

#### Scenario: GitHub e-mail request fails
- **WHEN** the mock answers `/user` with 200 and `/user/emails` with 500
- **THEN** the callback answers the 500 and the validator is not called

### Requirement: Verified e-mail
Each e-mail of a profile SHALL say whether the provider reports it as verified, and
`OAuth2Profile::verified_email()` SHALL return the primary verified address: for Google and the generic userinfo
mapping, the address when `email_verified` is `true`; for GitHub, the `/user/emails` entry that is both primary and
verified. It MUST return nothing otherwise.

#### Scenario: Unverified Google address
- **WHEN** the userinfo document has `email` and `email_verified: false`
- **THEN** `verified_email()` returns nothing and the e-mail entry has `verified: false`

#### Scenario: GitHub primary verified address
- **WHEN** `/user/emails` lists an unverified primary address and a verified secondary address
- **THEN** `verified_email()` returns nothing

### Requirement: Secrets are never printed or logged
The `Debug` output and the log lines of the OAuth2 module MUST NOT contain a client secret, cookie key, authorization
code, `state`, PKCE verifier, access, refresh or ID token, or profile data. Log lines MUST NOT contain a request or
response body, a query string, or the provider's `error_description`. The callback MUST mark `code` and `state` as
redacted query parameters of the request.

#### Scenario: Debug output
- **WHEN** the options, the strategy, the client, a grant, a token, a token response, a profile and a state record
  built with recognizable fake values are formatted with `{:?}`
- **THEN** none of the fake secret, code, token or e-mail values appears

#### Scenario: Logs of a failed exchange
- **WHEN** the mock token endpoint answers 500 with a body that echoes the code and the client secret
- **THEN** the captured log lines contain neither, and name the strategy, the provider host and the status

#### Scenario: Logged callback URL
- **WHEN** a callback request is logged by `LoggingInterceptor`
- **THEN** the logged URL hides the values of `code` and `state`

### Requirement: Invalid OAuth2 options stop the application
Startup MUST fail, with an error that names the strategy and the option and never contains the client secret, when an
OAuth2 option is missing, malformed or outside its documented bounds.

#### Scenario: Empty credentials
- **WHEN** the client id or the client secret is empty
- **THEN** `init()` fails

#### Scenario: Endpoint URL breaks its rule
- **WHEN** an authorization, token or userinfo URL is not `https` (or loopback `http`), or has a fragment or user
  information
- **THEN** `init()` fails

#### Scenario: Invalid scope
- **WHEN** a scope value is empty or contains a space, `"` or `\`
- **THEN** `init()` fails

#### Scenario: Google without scope
- **WHEN** the Google preset has an empty scope
- **THEN** `init()` fails

#### Scenario: Reserved parameter
- **WHEN** the options add an authorization parameter named `response_type`, `client_id`, `redirect_uri`, `scope`,
  `state`, `code_challenge` or `code_challenge_method` (for example `state=fixed`)
- **THEN** `init()` fails with an error that names the parameter

#### Scenario: Limit out of bounds
- **WHEN** the state lifetime, a timeout, a response size, the concurrency limit or the code length is outside its
  documented bounds (for example a total timeout of 10 minutes)
- **THEN** `init()` fails with an error that names the option and its bounds

#### Scenario: Secret not in the error
- **WHEN** any of these errors is produced for options holding a recognizable fake client secret
- **THEN** the error text does not contain it

### Requirement: Refresh tokens
`OAuth2Client::refresh` SHALL send `grant_type=refresh_token` and `refresh_token` with the client credentials to the
token URL, as `passport-oauth2-refresh` 2.2.0 does, through the same client and limits, and return the new tokens.
The framework MUST NOT store any access or refresh token.

#### Scenario: Refresh
- **WHEN** the application refreshes with the Google options and a refresh token
- **THEN** the mock token endpoint receives the form body recorded for the original and the new tokens are returned

#### Scenario: Refresh rejected
- **WHEN** the mock answers 400 `{"error":"invalid_grant"}`
- **THEN** `refresh` returns an error whose text holds the code `invalid_grant` and nothing else from the response

### Requirement: OAuth2 strategies are verified against the original
The redirect parameters, token request, profile requests, profiles, callback outcomes and refresh request MUST be
checked against fixtures recorded ahead of time from the original packages with a fake provider. The tests MUST run
without Node and without network access beyond `127.0.0.1`, and every case where Corvis differs on purpose MUST be
marked as a divergence in the fixture.

#### Scenario: Replay without Node
- **WHEN** `cargo test -p corvis --features auth-oauth2 --test auth_oauth2` runs on a machine without Node and
  without internet access
- **THEN** every recorded case is replayed against the local mock provider and passes

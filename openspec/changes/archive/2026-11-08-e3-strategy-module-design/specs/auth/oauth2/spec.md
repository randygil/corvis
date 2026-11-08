# Spec Delta

## ADDED Requirements

### Requirement: Authorization redirect
On a request without `code` and without `error`, the OAuth2 strategy MUST redirect (302, `Location`,
`Content-Length: 0`, empty body) to the authorization URL with `response_type=code`, `client_id`, `redirect_uri`
(when configured), `scope` (joined with the configured separator, a space by default) and extra parameters, keeping
parameters already in the URL, as `passport-oauth2` 1.8.0 does, plus `state`, `code_challenge` and
`code_challenge_method=S256`.

#### Scenario: Login starts
- **WHEN** a route guarded by the OAuth2 strategy with scopes `["email", "profile"]` receives `GET /auth/google`
- **THEN** the response is a 302 whose `Location` has the same parameters as the recorded original (ignoring the
  values of `state` and `code_challenge`), plus `state`, `code_challenge` and `code_challenge_method=S256`

### Requirement: State and PKCE are always on
The strategy MUST generate an unpredictable, single-use `state` and a PKCE verifier per authorization request and keep
them in the session when sessions are installed, or otherwise in a signed, `HttpOnly`, short-lived cookie. A missing,
unknown, expired or mismatched `state` on the callback MUST make the strategy fail (401) before any token exchange.
PKCE `plain` MUST be rejected at startup. Documented divergence: the original's default is no `state` and no PKCE.

#### Scenario: Forged callback
- **WHEN** the callback receives `?code=abc&state=forged`
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}` and no request reaches the token
  endpoint

#### Scenario: State reused
- **WHEN** a callback with a valid `state` is replayed after a successful login
- **THEN** the second callback fails with the 401

### Requirement: Callback outcomes match passport-oauth2
On the callback, `error=access_denied` MUST make the strategy fail (401); any other `error` value MUST be an error
answered with 500 `Internal server error`; a failed token exchange or profile request, or a token response without
an access token, MUST be an error answered with 500. On success the validator SHALL receive the access token, the
refresh token (if any), the full token response parameters and the profile, and its result SHALL map to the outcomes
of `auth/strategies`.

#### Scenario: User denies consent
- **WHEN** the callback receives `?error=access_denied&error_description=denied`
- **THEN** the response is the recorded original 401

#### Scenario: Provider error
- **WHEN** the callback receives `?error=server_error`
- **THEN** the response is the recorded original 500 `{"statusCode":500,"message":"Internal server error"}`

#### Scenario: Token endpoint down
- **WHEN** the fake token endpoint answers 500
- **THEN** the response is the recorded original 500

### Requirement: Safe token requests
The HTTP client used for the token, refresh and profile requests MUST NOT follow redirects, MUST use TLS for
non-loopback URLs and MUST have a request timeout, so that a misbehaving provider cannot redirect the server to an
internal address.

#### Scenario: Token endpoint redirects
- **WHEN** the fake token endpoint answers 302 to another URL
- **THEN** the redirect is not followed and the callback answers 500

### Requirement: Normalized Google and GitHub profiles
The Google and GitHub strategies SHALL build the same normalized profile as `passport-google-oauth20` 2.0.0 and
`passport-github2` 0.1.12 from the same provider responses: `provider`, `id`, `displayName`, `name`, `username` where
the original sets it, `emails`, `photos`, and the raw JSON. The default authorization, token and profile URLs SHALL be
the originals' defaults.

#### Scenario: GitHub profile
- **WHEN** the fake GitHub user endpoint returns a recorded user JSON
- **THEN** the profile passed to the validator equals, field by field, the profile the original passes to `validate()`

### Requirement: Refresh tokens
The framework SHALL offer a way to exchange a refresh token for new tokens with a registered OAuth2 strategy's
configuration, with the request parameters `grant_type=refresh_token` and `refresh_token` of `passport-oauth2-refresh`
2.2.0, and the same safe HTTP client.

#### Scenario: Refresh
- **WHEN** the application asks to refresh with the strategy named `"google"` and a refresh token
- **THEN** the fake token endpoint receives the same form body as from the original and the new tokens are returned

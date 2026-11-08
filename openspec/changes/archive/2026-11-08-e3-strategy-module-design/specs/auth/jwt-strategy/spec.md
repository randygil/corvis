# Spec Delta

## ADDED Requirements

### Requirement: Authorization header extractors match passport-jwt
The JWT strategy SHALL offer `from_auth_header_as_bearer_token()` and `from_auth_header_with_scheme(s)` with the
semantics of `passport-jwt` 4.0.1: parse the `Authorization` header with the pattern `(\S+)\s+(\S+)` (first match
anywhere in the value) and return the second group when the first equals the scheme ignoring ASCII case. The existing
`extract_bearer_token` function MUST keep its current behavior.

#### Scenario: Tab between scheme and token
- **WHEN** the header is `Authorization: Bearer\t<valid token>`
- **THEN** the bearer extractor returns the token and the request is authenticated, as in passport-jwt

#### Scenario: Extra words after the token
- **WHEN** the header is `Authorization: Bearer <valid token> extra`
- **THEN** the extractor returns only `<valid token>`, as in passport-jwt

#### Scenario: Lowercase scheme
- **WHEN** the header is `Authorization: bearer <valid token>`
- **THEN** the request is authenticated

### Requirement: Other extractors match passport-jwt
The strategy SHALL offer `from_header(name)` (name compared case-insensitively), `from_url_query_parameter(name)`,
`from_body_field(name)` (string values only) and `from_extractors([...])` (first non-empty token) with the semantics
of `passport-jwt`'s `ExtractJwt`, plus `from_cookie(name)`, which matches the usual hand-written extractor
`req => req.cookies?.[name]` with cookie-parser.

#### Scenario: First non-empty extractor
- **WHEN** the extractors are `[from_cookie("jwt"), from_auth_header_as_bearer_token()]` and the request has only the
  bearer header
- **THEN** the token from the header is used

#### Scenario: Non-string body field
- **WHEN** `from_body_field("token")` is used and the body is `{"token": 5}`
- **THEN** no token is found and the response is the 401

### Requirement: Missing or invalid tokens fail
When no extractor finds a token, or the token does not verify (bad signature, algorithm not allowed, `exp` reached,
`nbf` not reached, `iss` or `aud` not matching, malformed token), the strategy MUST fail, so that `AuthGuard` answers
the 401 `{"message":"Unauthorized","statusCode":401}`. The validator MUST NOT be called for a token that fails
verification.

#### Scenario: No token
- **WHEN** a route guarded by the JWT strategy receives a request without an `Authorization` header
- **THEN** the response is the recorded original 401 and the validator is not called

#### Scenario: Expired token
- **WHEN** the token's `exp` is in the past and `ignore_expiration` is off
- **THEN** the response is the recorded original 401

### Requirement: Verification options match passport-jwt
The strategy SHALL support `issuer` (one or several), `audience` (one or several), `algorithms` and
`ignore_expiration` with the semantics `passport-jwt` passes to `jsonwebtoken.verify`. Without `algorithms`, the
accepted algorithms MUST be derived from the key type as `jsonwebtoken` 9 does (HMAC secret: HS256/384/512; RSA
public key: RS* and PS*; EC key: ES*), and `none` MUST never be accepted.

#### Scenario: ignore_expiration
- **WHEN** `ignore_expiration` is on and a token signed with the right key has `exp` in the past
- **THEN** the validator is called with the payload and the request is authenticated

#### Scenario: Audience list
- **WHEN** `audience` is `["web", "mobile"]` and the token's `aud` is `"mobile"`
- **THEN** the request is authenticated; with `aud` `"admin"` it fails with the 401

### Requirement: Static key or key provider
The strategy SHALL take either a static key (secret or public key) or a key provider called per request with the
request and the raw token, which returns the key to verify with. Configuring both or neither MUST be a startup
error, as `passport-jwt` throws when the strategy is created. A key provider error MUST make the strategy fail
(401), as in `passport-jwt`.

#### Scenario: Key chosen by the token's issuer
- **WHEN** the key provider returns a different secret depending on the unverified `iss` claim, and a token is signed
  with the secret of its issuer
- **THEN** the request is authenticated

#### Scenario: Key provider error
- **WHEN** the key provider returns an error
- **THEN** the response is the 401, not a 500

### Requirement: The validator receives the verified payload
The validator SHALL receive the request and the verified payload as JSON, and its result SHALL map to the outcomes of
`auth/strategies`.

#### Scenario: Payload passed through
- **WHEN** a token with payload `{ "sub": "7", "role": "admin" }` is verified and the validator returns a user built
  from it
- **THEN** the handler receives that user and the response matches the recorded original

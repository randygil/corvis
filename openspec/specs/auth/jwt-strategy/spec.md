# auth/jwt-strategy Specification

## Purpose
The JWT strategy: a named strategy of `auth/strategies` that reads a token from the places the app configures,
verifies it, and passes the payload to the app's validator, with the behavior of `passport-jwt` 4.0.1 over
`jsonwebtoken` 9 except where this specification is stricter on purpose (each divergence is named in its
requirement). Every limit has a default and a startup check.

## Requirements

### Requirement: JWT strategy registration and validator
The framework SHALL provide a `JwtStrategy<V>` strategy, registered with `Provider::strategy::<JwtStrategy<V>>()`,
whose validator `V` receives its dependencies through dependency injection, provides the strategy options, and
implements `validate(request, payload)`. The strategy name SHALL be `"jwt"` unless the validator sets another one.

#### Scenario: Two JWT strategies
- **WHEN** one validator keeps the default name and another sets the name `"jwt-refresh"` with a different secret
- **THEN** `AuthGuard::new("jwt")` and `AuthGuard::new("jwt-refresh")` each accept only tokens signed with their own
  secret

### Requirement: The validator receives only verified payloads
The validator SHALL receive the request and the verified payload as a JSON object, and its result SHALL map to the
outcomes of `auth/strategies`: a user is a success, no user is a failure, an exception is an error. The validator
MUST NOT be called for a token that fails extraction or verification.

#### Scenario: Payload passed through
- **WHEN** a token with payload `{ "sub": "7", "role": "admin", "exp": <future> }` is verified and the validator
  returns a user built from it
- **THEN** the handler receives that user and the response matches the recorded original

#### Scenario: Validator returns no user
- **WHEN** the token verifies and the validator returns no user
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}`, as the recorded original when
  `validate()` returns `null`

### Requirement: Extractor and key options are checked at startup
The application MUST NOT start, with an error naming the strategy and the problem, when a JWT strategy has no
extractor, both a static key and a key provider, or neither, an HMAC secret that is empty or shorter than 32 bytes (RFC 7518 section
3.2; `JwtService` is unchanged), or a key that does not parse. A key provider that returns such a secret MUST make that
request fail.

#### Scenario: No extractor
- **WHEN** a JWT strategy is configured with a secret and no extractor
- **THEN** the application does not start and the error names the strategy and the missing extractor, as
  `passport-jwt` throws `JwtStrategy requires a function to retrieve jwt from requests`

#### Scenario: Short secret
- **WHEN** a JWT strategy uses the secret `"changeme"`
- **THEN** the application does not start and the error names the strategy and the secret length, not the secret

#### Scenario: Both a key and a key provider
- **WHEN** a JWT strategy is configured with a secret and a key provider
- **THEN** the application does not start, as `passport-jwt` throws when given both

### Requirement: Verification options are checked at startup
The application MUST NOT start, with an error naming the strategy and the problem, when a JWT strategy has an empty
`algorithms` list, a static key with an `algorithms` list that mixes families or does not match the key's family, an
Ed25519 key without explicit `algorithms`, an empty issuer or audience value or list, `clock_tolerance` above 300
seconds, or `max_token_length` of 0.

#### Scenario: HMAC algorithm with a public key
- **WHEN** a JWT strategy is configured with an RSA public key and `algorithms([HS256])`
- **THEN** the application does not start

#### Scenario: Empty audience
- **WHEN** a JWT strategy is configured with `audience("")`
- **THEN** the application does not start

### Requirement: Tokens are read only from configured places
The strategy SHALL extract the token only with the extractors configured in `jwt_from_request`; there is no default
place. `ExtractJwt::from_extractors([..])` SHALL try its extractors in order and use the first non-empty token. When
no extractor finds a token the strategy MUST fail.

#### Scenario: No token
- **WHEN** a route guarded by the JWT strategy, configured with the bearer extractor only, receives a request without
  an `Authorization` header
- **THEN** the response is the recorded original 401 and the validator is not called

#### Scenario: Token in a place that is not configured
- **WHEN** the strategy is configured with the bearer extractor only and a valid token is sent as `?token=<token>` and
  as cookie `jwt=<token>`
- **THEN** the response is the 401

#### Scenario: First non-empty extractor
- **WHEN** the extractors are `from_extractors([from_cookie("jwt"), from_auth_header_as_bearer_token()])` and the
  request has only the bearer header
- **THEN** the token from the header is used

### Requirement: Authorization header extractors match passport-jwt
`ExtractJwt::from_auth_header_as_bearer_token()` and `from_auth_header_with_scheme(scheme)` SHALL parse the
`Authorization` header as `passport-jwt` 4.0.1 does: the first `Authorization` value (Node discards duplicates),
decoded as latin1, is matched with `(\S+)\s+(\S+)` using JavaScript's whitespace class, at its first match, and the
second group is the token when the first equals the scheme ignoring ASCII case. `extract_bearer_token` MUST keep its
current behavior.

#### Scenario: Tab between scheme and token
- **WHEN** the header is `Authorization: Bearer\t<valid token>`
- **THEN** the request is authenticated, as in passport-jwt

#### Scenario: Extra words after the token
- **WHEN** the header is `Authorization: Bearer <valid token> extra`
- **THEN** only `<valid token>` is used and the request is authenticated, as in passport-jwt

#### Scenario: Lowercase scheme
- **WHEN** the header is `Authorization: bearer <valid token>`
- **THEN** the request is authenticated

#### Scenario: Other scheme
- **WHEN** the strategy uses `from_auth_header_with_scheme("JWT")` and the header is `Authorization: Bearer <valid
  token>`
- **THEN** the response is the 401

### Requirement: Other extractors match passport-jwt
`ExtractJwt::from_url_query_parameter(name)` SHALL read the query parameter, `from_body_field(name)` the parsed body
field, and `from_cookie(name)` the cookie parsed like cookie-parser (an addition: the reference is the hand-written
`req => req.cookies?.[name]`). A value that is not a string, or an empty string, MUST count as no token.

#### Scenario: Non-string body field
- **WHEN** `from_body_field("token")` is used and the body is `{"token": 5}`
- **THEN** no token is found and the response is the 401

#### Scenario: Token in a cookie
- **WHEN** `from_cookie("jwt")` is used and the request has `Cookie: jwt=<valid token>`
- **THEN** the request is authenticated

### Requirement: Custom header extractor
`ExtractJwt::from_header(name)` SHALL read the request header `name`, compared case-insensitively, with repeated
headers joined with `", "` as Node joins them. The case-insensitive name is a documented divergence: the original only
finds names written in lowercase.

#### Scenario: Uppercase header name
- **WHEN** `from_header("X-Token")` is used and the request has `x-token: <valid token>`
- **THEN** the request is authenticated; the fixture case is marked as a divergence

#### Scenario: Repeated custom header
- **WHEN** `from_header("x-token")` is used and the request has two `x-token` headers with the same valid token
- **THEN** the token is the joined value, it fails verification, and the response is the 401, as in the original

### Requirement: Token length is bounded
The strategy SHALL refuse a token longer than `max_token_length` bytes, 8192 by default, before decoding it: the
strategy fails and the validator is not called. This is an addition: the original has no limit.

#### Scenario: Oversized token
- **WHEN** a token of 8193 bytes is sent with the default limit
- **THEN** the response is the 401 and no signature verification runs

#### Scenario: Raised limit
- **WHEN** `max_token_length(16384)` is configured and a validly signed token of 10000 bytes is sent
- **THEN** the request is authenticated

### Requirement: Only allowed algorithms are accepted
The strategy MUST accept a token only when its header `alg` is in the allow-list: the configured `algorithms`, or, by
default, the algorithms of the key's type as `jsonwebtoken` 9 derives them (HMAC secret: HS256, HS384, HS512; RSA
public key: RS256, RS384, RS512, PS256, PS384, PS512; EC public key: ES256, ES384). `none`, in any letter case, MUST
never be accepted.

#### Scenario: Algorithm none
- **WHEN** a token with header `{"alg":"none"}` or `{"alg":"NONE"}` and an empty signature is sent
- **THEN** the response is the 401, as in the original

#### Scenario: Explicit allow-list
- **WHEN** `algorithms([HS256])` is configured and a token signed with the same secret uses `HS512`
- **THEN** the response is the 401

### Requirement: No algorithm confusion between key families
The allow-list SHALL be restricted, for each request, to the family of the key used for it. A token whose algorithm
belongs to a different family than the key MUST fail, so a public key can never be used as an HMAC secret.

#### Scenario: HMAC token against an RSA key
- **WHEN** the strategy has an RSA public key and receives a token with `alg` `HS256` signed with that public key's
  PEM text as the HMAC secret
- **THEN** the response is the 401, as in the original

### Requirement: Expiration is required by default
A token without an `exp` claim MUST fail unless `allow_missing_exp()` is configured; this is a documented divergence
(the original accepts tokens without `exp` forever). `allow_missing_exp()` SHALL log a startup warning naming the
strategy.

#### Scenario: Token without exp
- **WHEN** a correctly signed token has no `exp` and the options are the defaults
- **THEN** the response is the 401; the fixture case is marked as a divergence (the original authenticates it)

#### Scenario: Opt-out
- **WHEN** `allow_missing_exp()` is configured and a correctly signed token has no `exp`
- **THEN** the request is authenticated and a warning was logged at startup

### Requirement: Time claims with bounded clock tolerance
The strategy SHALL check `exp` and `nbf` with the rules of `jsonwebtoken` 9 (`verify.js`): with `now` the current
time in whole seconds and `tolerance` the configured `clock_tolerance` (default 0 seconds, at most 300), the token
fails when `now >= exp + tolerance` or `nbf > now + tolerance`, and when `exp` or `nbf` is present and not a number.
`ignore_expiration` SHALL skip the `exp` check and the `exp` requirement, and SHALL log a startup warning.

#### Scenario: Expired token
- **WHEN** the token's `exp` is 10 seconds in the past and the tolerance is 0
- **THEN** the response is the recorded original 401

#### Scenario: Expired within the tolerance
- **WHEN** `clock_tolerance(30)` is configured and the token's `exp` is 10 seconds in the past
- **THEN** the request is authenticated, as the original with `jsonWebTokenOptions: { clockTolerance: 30 }`

#### Scenario: Not yet valid
- **WHEN** the token's `nbf` is one hour in the future
- **THEN** the response is the 401

#### Scenario: Non-numeric exp
- **WHEN** a correctly signed token has `"exp": "2099-01-01"`
- **THEN** the response is the 401, as in the original (`invalid exp value`)

### Requirement: Non-numeric iat fails
A present `iat` that is not a number MUST make the token fail, as `JwtService::verify` already does. This is a
documented divergence: `jsonwebtoken` 9 checks `iat` only with `maxAge`.

#### Scenario: String iat
- **WHEN** a correctly signed token with a future `exp` has `"iat": "yesterday"`
- **THEN** the response is the 401; the fixture case is marked as a divergence

#### Scenario: Tolerance above the limit
- **WHEN** `clock_tolerance(301)` is configured
- **THEN** the application does not start

#### Scenario: ignore_expiration
- **WHEN** `ignore_expiration` is on and a token signed with the right key has `exp` in the past
- **THEN** the validator is called with the payload and the request is authenticated

### Requirement: Issuer and audience checks
When `issuer` is configured (one value or a list), the token's `iss` MUST equal one of the values; when `audience` is
configured (one value or a list), the token's `aud` (a string or a list) MUST contain at least one of the values, with
the semantics of `jsonwebtoken` 9. A missing or non-string claim MUST NOT match.

#### Scenario: Audience list
- **WHEN** `audience` is `["web", "mobile"]` and the token's `aud` is `"mobile"`
- **THEN** the request is authenticated; with `aud` `"admin"`, or without `aud`, the response is the 401

#### Scenario: Audience array in the token
- **WHEN** `audience` is `"web"` and the token's `aud` is `["admin", "web"]`
- **THEN** the request is authenticated

#### Scenario: Wrong issuer
- **WHEN** `issuer` is `"https://auth.example"` and the token's `iss` is `"https://evil.example"`
- **THEN** the response is the 401

### Requirement: Static key or key provider
The strategy SHALL take either a static key (HMAC secret, RSA, EC or Ed25519 public key) or a key provider called per
request with the request and the raw, unverified token, which returns the key to verify with. A key provider error
MUST make the strategy fail (401), as in `passport-jwt`. The returned key's type SHALL narrow the allow-list to its
family for that request.

#### Scenario: Key chosen by the token's issuer
- **WHEN** the key provider returns a different secret depending on the unverified `iss` claim, and a token is signed
  with the secret of its issuer
- **THEN** the request is authenticated

#### Scenario: Key provider error
- **WHEN** the key provider returns an error
- **THEN** the response is the 401, not a 500

#### Scenario: Provider returns an RSA key for an HMAC token
- **WHEN** the key provider returns an RSA public key and the token uses `HS256`
- **THEN** the response is the 401

### Requirement: The payload is a JSON object
A token whose payload is not a JSON object MUST fail. This is a documented divergence: the original passes a string
payload to `validate()`.

#### Scenario: String payload
- **WHEN** a correctly signed token's payload segment encodes the JSON string `"hello"`
- **THEN** the response is the 401

### Requirement: Risky configurations are reported at startup
The framework SHALL log one startup warning per risky option of a strategy, naming it, when `ignore_expiration` or
`allow_missing_exp()` is configured and when a query parameter extractor is configured.

#### Scenario: Query token extractor
- **WHEN** a JWT strategy reads its token with `from_url_query_parameter`
- **THEN** the application starts and the log has a warning naming the strategy and the URL query

### Requirement: Malformed input never crashes the strategy
Extracting and verifying any header, cookie, query or body value MUST end in a success or a failure and MUST NOT
panic.

#### Scenario: Random Authorization headers
- **WHEN** random byte strings, including non-UTF-8 bytes, dots and whitespace, are sent as the `Authorization` header
- **THEN** every request answers 401 or is authenticated, without a panic

### Requirement: The JWT strategy is verified against passport-jwt
The JWT strategy MUST be covered by the differential fixture of `auth/strategies` with `passport-jwt` 4.0.1: every
recorded case is replayed by `cargo test`, and the cases where this specification diverges are marked with their
reason.

#### Scenario: Fixture replay
- **WHEN** `cargo test --workspace` runs
- **THEN** every case of `jwt.json` passes without Node

### Requirement: Critical headers and malformed tokens are refused early
After the length limit and before the key provider or any verification, the strategy MUST fail a token whose header has `crit` (RFC 7515 section 4.1.11), a header that is not a JSON object that the strict JSON text parser accepts, and a token that is not exactly three non-empty base64url segments. The key provider is not called.

#### Scenario: Critical header
- **WHEN** a validly signed token has the header member `"crit": ["urn:x"]`
- **THEN** the response is the 401 and the key provider is not called

#### Scenario: Header the decoder accepts but a strict parser refuses
- **WHEN** a validly signed header has `crit` and a member such as `1e400`, a lone surrogate, 200 nested arrays or raw bytes 0xFF 0xFE
- **THEN** the response is the 401 and the key provider is not called

#### Scenario: Wrong shape
- **WHEN** the token is `a.b`, `a.b.`, `.b.c`, `a.b.c.d` or has padding
- **THEN** the response is the 401 and the key provider is not called

### Requirement: Key and token work is done once
The strategy SHALL build its verification settings once per key family when it is built, and check a token's header
once per request, before the key provider. A `JwtKey` SHALL parse its key material at most once, shared by its clones,
so a key provider that returns a clone of a cached key costs no parse. The length check of an HMAC secret a provider
returns MUST still run on every request.

#### Scenario: Cached provider key
- **WHEN** a key provider returns clones of one RSA `JwtKey` for 100 requests with valid tokens
- **THEN** the key is parsed once and every request is authenticated

#### Scenario: Header checked once
- **WHEN** a well-formed token goes through the strategy
- **THEN** its header is checked exactly once, and a token whose header has `crit` never reaches the key provider

#### Scenario: Short secret from a cache
- **WHEN** a key provider returns clones of one cached 31-byte secret
- **THEN** every request fails with the reason `key provider error`

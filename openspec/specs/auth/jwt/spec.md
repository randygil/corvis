# auth/jwt Specification

## Purpose
Guarantees that JWT verification only accepts tokens signed with the configured key and algorithm, and handles any
malformed token without crashing.

## Requirements

### Requirement: Forged tokens are rejected
Token verification MUST reject any token not signed with the configured key, including tokens whose header declares
`alg` values such as `none` (in any letter case), a different HMAC size, an asymmetric algorithm or an empty string.

#### Scenario: Algorithm none
- **WHEN** a token with header `{"alg":"none"}` and any payload and signature is verified
- **THEN** verification fails

#### Scenario: Random signature
- **WHEN** a token has a valid header and payload and a random signature
- **THEN** verification fails

### Requirement: Tampered tokens are rejected
Changing any character of the header or payload segment of a validly signed token MUST make verification fail.

#### Scenario: Payload edited after signing
- **WHEN** one character of the payload of a valid token is replaced by another base64url character
- **THEN** verification fails

### Requirement: Malformed tokens are handled
Verifying or decoding any string, and parsing any `expiresIn` duration string, SHALL return a result or an error and
MUST NOT crash.

#### Scenario: Random strings as tokens
- **WHEN** strings of random characters, dots and padding are verified and decoded
- **THEN** each call returns an error or a value, without a panic

### Requirement: Issuer, audience and critical headers in JwtService
When an issuer or audience is configured, `JwtService::verify` SHALL refuse a token whose `iss` is missing, not a string or not equal to it, or whose `aud` is missing, neither a string nor a list of strings, or does not contain it. It MUST refuse any token whose header has `crit` or is not a JSON object that the strict JSON text parser accepts. With none configured, claims are not checked.

#### Scenario: Claim missing or of another type
- **WHEN** an issuer and an audience are configured and the token has no `iss`, a numeric `iss`, a list `iss`, no `aud` or a numeric `aud`
- **THEN** `verify` fails

#### Scenario: Matching claims
- **WHEN** the token's `iss` equals the issuer and its `aud` is the audience or a list containing it
- **THEN** `verify` succeeds

#### Scenario: Critical header
- **WHEN** the token's header contains `crit`, even with a member such as `1e400` that a strict parser refuses
- **THEN** `verify` fails

### Requirement: JwtService signs by the original's rules
`JwtService::sign` and `sign_with` SHALL return the token the original `JwtService.sign` returns for the same payload,
options and clock, or fail with its error text: `exp` and `nbf` counted from a payload `iat`, a payload claim together
with its option (`exp`, `nbf`, `aud`, `iss`, `sub`, `jti`) refused, claims added in the original's order. Any exception
MUST be a recorded divergence with its reason.

#### Scenario: Expiry counted from the payload's iat
- **WHEN** a module with `expires_in("1h")` signs `{"iat":1700000000,"sub":"7"}`
- **THEN** the token's `exp` is `1700003600` and its `iat` is `1700000000`

#### Scenario: A claim and its option together
- **WHEN** the module has an `issuer` and the payload has `iss`, or a per-call `subject` meets a payload `sub`, or an
  `expires_in` meets a payload `exp`
- **THEN** `sign` returns the original's error for that pair and no token

#### Scenario: Order of the added claims
- **WHEN** a module with `expires_in`, `issuer` and `audience` signs a payload that has `iat` with a per-call `subject`,
  `jwt_id` and `not_before`
- **THEN** after the payload's own keys the token lists `nbf`, `exp`, `aud`, `iss`, `sub`, `jti` in that order, and the
  token equals the recorded one byte for byte

#### Scenario: Recorded fixture
- **WHEN** the replay test signs every case of `crates/corvis/tests/fixtures/auth/jwt_sign.json` at the recorded clock
- **THEN** each token equals the recorded token byte for byte, or the error text equals the recorded one, or the case
  carries a `divergence` whose result Corvis returns

### Requirement: JwtService prepares its keys once
`JwtService` SHALL parse its module keys and build its verification settings when it is created and MUST NOT parse a
module key during `sign` or `verify`. A module key that does not parse SHALL make each call fail with the same error, of
the same type, as before, and the application still starts. A key given per call is parsed for that call.

#### Scenario: Many calls, no parse
- **WHEN** a service with RS256 module keys signs and verifies 100 tokens
- **THEN** no module key is parsed after the service was created, and every token verifies

#### Scenario: A key that does not parse
- **WHEN** a service is created with a module private key that is not PEM
- **THEN** it is created, and each `sign` fails with an error that downcasts to `jsonwebtoken::errors::Error`

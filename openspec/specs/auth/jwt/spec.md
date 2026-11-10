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

# Spec Delta

## Purpose

Guarantees that JWT verification only accepts tokens signed with the configured key and algorithm, and handles any
malformed token without crashing.

## ADDED Requirements

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

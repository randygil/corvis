# Spec Delta

## ADDED Requirements

### Requirement: Credentials are read like passport-local
The local strategy SHALL read the username and password from the fields `username` and `password` by default, with
configurable field names that accept `a[b]` paths, looking first in the parsed body and then in the query string, as
`passport-local` 1.0.0 does.

#### Scenario: Credentials in a JSON body
- **WHEN** `POST /auth/login` receives `{"username":"ana","password":"secret"}` and the validator accepts them
- **THEN** the handler runs with the validator's user and the response matches the recorded original

#### Scenario: Custom nested field
- **WHEN** the username field is configured as `user[email]` and the body is
  `{"user":{"email":"ana@x.io"},"password":"secret"}`
- **THEN** the validator receives `ana@x.io`

#### Scenario: Credentials in the query string
- **WHEN** the body is empty and the request is `POST /auth/login?username=ana&password=secret`
- **THEN** the validator receives `ana` and `secret`

### Requirement: Missing credentials fail
A field that is absent, empty, or not a string MUST be treated as missing. When either credential is missing the
strategy MUST fail without calling the validator, and through `AuthGuard` the response MUST be the 401
`{"message":"Unauthorized","statusCode":401}` (the original's 400 status is ignored by `AuthGuard`).

#### Scenario: Missing password
- **WHEN** the body is `{"username":"ana"}`
- **THEN** the validator is not called and the response is the recorded original 401

#### Scenario: Non-string username
- **WHEN** the body is `{"username":123,"password":"secret"}`
- **THEN** the strategy fails with the 401 without calling the validator; this case is marked as a divergence in the
  fixture (the original passes the number to `validate()`), to avoid type confusion in user lookups

### Requirement: Password hashes compatible with bcryptjs
The framework SHALL provide a password hasher that verifies the `$2a$`, `$2b$` and `$2y$` hashes of `bcryptjs` 3.0.3
and creates bcrypt hashes that `bcryptjs.compare` accepts, with cost 10 by default. Verifying any password against a
`bcryptjs` hash, including non-ASCII passwords and passwords over 72 bytes, MUST give the result of
`bcryptjs.compareSync`.

#### Scenario: Hash from the original app
- **WHEN** a hash created by `bcryptjs.hashSync("pässword", 10)` is verified with `"pässword"` and with `"password"`
- **THEN** the results are `true` and `false`

#### Scenario: Hash read back by the original
- **WHEN** Corvis hashes `"secret"` with bcrypt and the oracle runs `bcryptjs.compareSync("secret", hash)`
- **THEN** the oracle returns `true`

#### Scenario: Long password against an original hash
- **WHEN** a `bcryptjs` hash of a 73-byte password is verified with the same password and with its first 72 bytes
- **THEN** both results equal the recorded results of `bcryptjs.compareSync`

### Requirement: Long passwords are not truncated silently when hashing
Creating a bcrypt hash of a password longer than 72 bytes MUST return an error instead of truncating it (documented
divergence; argon2 hashes have no such limit).

#### Scenario: 73-byte password
- **WHEN** the hasher is configured for bcrypt and asked to hash a 73-byte password
- **THEN** it returns an error

### Requirement: Argon2 hashes and transparent migration
The password hasher SHALL also create and verify argon2id hashes in PHC string format. Verification MUST choose the
algorithm from the hash prefix, so a database with bcrypt hashes keeps working while new hashes use the configured
algorithm, and the hasher SHALL report when a hash should be rehashed (different algorithm or weaker parameters than
the configured ones).

#### Scenario: Mixed hashes
- **WHEN** the hasher is configured for argon2id and verifies a correct password against a `$2b$` hash
- **THEN** verification succeeds and `needs_rehash` reports `true`

### Requirement: Verification never panics and compares in constant time
Verifying a password against any string, including malformed, truncated or unknown-format hashes, MUST return
`false` or an error and MUST NOT panic. Comparisons of derived hashes MUST be constant-time. The hasher SHALL offer a
dummy verification for unknown users, so the time of a login does not reveal whether a username is registered.

#### Scenario: Malformed hash
- **WHEN** a password is verified against `"$2b$10$short"` or random text
- **THEN** the result is `false` or an error, without a panic

#### Scenario: Unknown user
- **WHEN** a validator calls the hasher's dummy verification for an unknown username
- **THEN** the call takes the same order of time as a real verification with the configured cost

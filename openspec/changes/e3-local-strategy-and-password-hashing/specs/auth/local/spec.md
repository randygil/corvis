# Spec Delta

## Purpose

Username and password authentication: the local strategy of `auth/strategies`, which reads credentials like
`passport-local` 1.0.0, and the password hasher, which creates argon2id hashes, verifies the bcrypt hashes of
`bcryptjs` 3.0.3, and resists user enumeration and resource exhaustion. Where this specification is stricter than the
original, the requirement names the divergence.

## ADDED Requirements

### Requirement: Local strategy registration and validator
The framework SHALL provide `LocalStrategy<V>`, registered with `Provider::strategy::<LocalStrategy<V>>()` under the
`auth-local` cargo feature, whose validator `V` receives its dependencies through dependency injection and implements
`validate(request, username, password)`. The strategy name SHALL be `"local"` unless the validator sets another one.
The validator's result SHALL map to the outcomes of `auth/strategies`: a user is a success, no user is a failure, an
exception is an error.

#### Scenario: Credentials in a JSON body
- **WHEN** `POST /auth/login` with `AuthGuard::new("local")` receives `{"username":"ana","password":"secret"}` and the
  validator accepts them
- **THEN** the handler runs with the validator's user and the response matches the recorded original

#### Scenario: Wrong password
- **WHEN** the validator returns no user
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}`, as the recorded original

#### Scenario: Validator throws
- **WHEN** the validator returns `Err(ForbiddenException::new("Account locked"))`
- **THEN** the response is the recorded original 403

### Requirement: Credentials are read like passport-local
The local strategy SHALL read the username and the password from the fields `username` and `password` by default,
with configurable field names, in the parsed body (JSON or `application/x-www-form-urlencoded`) and then, when the
body gives no credential, in the query string, as `passport-local` 1.0.0 does. A field name that is empty MUST stop
the application at startup.

#### Scenario: Credentials in the query string
- **WHEN** the body is empty and the request is `POST /auth/login?username=ana&password=secret`
- **THEN** the validator receives `ana` and `secret`, as in the original

#### Scenario: Form body
- **WHEN** the body is `username=ana&password=secret` with `Content-Type: application/x-www-form-urlencoded`
- **THEN** the validator receives `ana` and `secret`

#### Scenario: Body value wins over the query
- **WHEN** the body is `{"username":"ana","password":"secret"}` and the query is `?username=eve`
- **THEN** the validator receives `ana`

### Requirement: Field paths follow passport-local lookup
A field name SHALL be turned into a path by removing every `]` and splitting on `[`, and the path SHALL be followed as
`passport-local` 1.0.0 `utils.lookup` does: an absent segment gives no value, a value that is not an object or an
array is returned at once, even before the end of the path, and an object or array at the end of the path gives no
value. A segment applied to an array is an index.

#### Scenario: Custom nested field
- **WHEN** the username field is configured as `user[email]` and the body is
  `{"user":{"email":"ana@x.io"},"password":"secret"}`
- **THEN** the validator receives `ana@x.io`

#### Scenario: String before the end of the path
- **WHEN** the username field is `user[email]` and the body is `{"user":"ana","password":"secret"}`
- **THEN** the validator receives `ana`, as in the recorded original

#### Scenario: Object at the end of the path
- **WHEN** the username field is `user` and the body is `{"user":{"email":"ana@x.io"},"password":"secret"}`
- **THEN** the username is missing and the response is the recorded original 401

### Requirement: Missing credentials fail
A credential that is absent, empty, or not a string in a source MUST be treated as missing in that source, and the
next source is tried. When either credential is missing in both sources the strategy MUST fail, with challenge
`{"message":"Missing credentials"}` and status 400, without calling the validator, so that `AuthGuard` answers its
401. Treating a non-string value as missing is a documented divergence: the original passes it to `validate()`.

#### Scenario: Missing password
- **WHEN** the body is `{"username":"ana"}` and there is no query string
- **THEN** the validator is not called and the response is the recorded original 401

#### Scenario: Empty password
- **WHEN** the body is `{"username":"ana","password":""}`
- **THEN** the validator is not called and the response is the recorded original 401

#### Scenario: Non-string username
- **WHEN** the body is `{"username":123,"password":"secret"}`
- **THEN** the strategy fails with the 401 without calling the validator; the fixture case is marked as a divergence
  (the original passes the number to `validate()`), to avoid type confusion in user lookups

### Requirement: Credential length is bounded
The local strategy SHALL fail without calling the validator when the username or the password is longer than its
limit: `max_username_bytes` and `max_password_bytes`, 1024 bytes each by default. This is an addition: the original
has no limit besides the body size.

#### Scenario: Oversized password
- **WHEN** the body has a 1025-byte password with the default limits
- **THEN** the response is the 401 and the validator is not called

#### Scenario: Raised limit
- **WHEN** `max_password_bytes(4096)` is configured and the body has a 2000-byte password
- **THEN** the validator is called with it

### Requirement: New hashes use argon2id with documented parameters
The password hasher SHALL create argon2id hashes (version 19) in PHC string format with, by default, 19456 KiB of
memory, 2 iterations, parallelism 1, a 16-byte random salt from the operating system's generator and a 32-byte
output. The parameters SHALL be configurable, but values below the floor (argon2id 19456 KiB with 2 iterations, or
47104 KiB with 1; bcrypt cost 10) MUST fail creation unless weak parameters are explicitly allowed, which logs a
warning.

#### Scenario: Default hash format
- **WHEN** a hasher with default options hashes `"secret"`
- **THEN** the result starts with `$argon2id$v=19$m=19456,t=2,p=1$` and verifies with `"secret"`

#### Scenario: Parameters below the floor
- **WHEN** a hasher is created with argon2id at 8 KiB and 1 iteration, or with bcrypt cost 4
- **THEN** creation fails, unless the options explicitly allow weak parameters, and then it succeeds and logs a warning

#### Scenario: Two hashes of the same password
- **WHEN** the same password is hashed twice
- **THEN** the two hashes differ (different salts) and both verify

### Requirement: Password length is bounded before hashing
The hasher SHALL refuse to hash a password longer than `max_password_bytes`, 1024 bytes by default, with an error,
and verification of such a password MUST return `false` without running the key derivation.

#### Scenario: Hashing an oversized password
- **WHEN** a 1025-byte password is hashed with the default options
- **THEN** the hasher returns a password-too-long error

#### Scenario: Verifying an oversized password
- **WHEN** a 1025-byte password is verified against a valid argon2id hash
- **THEN** the result is `false` and no key derivation ran

### Requirement: Password hashes compatible with bcryptjs
The hasher SHALL verify the `$2a$`, `$2b$` and `$2y$` hashes of `bcryptjs` 3.0.3 with the result of
`bcryptjs.compareSync` for every password, including non-ASCII passwords, passwords with a NUL character and
passwords over 72 bytes. When configured for bcrypt it SHALL create `$2b$` hashes, cost 10 by default, that
`bcryptjs.compareSync` accepts.

#### Scenario: Hash from the original app
- **WHEN** a hash created by `bcryptjs.hashSync("pässword", 10)` is verified with `"pässword"` and with `"password"`
- **THEN** the results are `true` and `false`

#### Scenario: Hash read back by the original
- **WHEN** Corvis hashes `"secret"` with bcrypt and the recorded result of `bcryptjs.compareSync("secret", hash)` is
  read
- **THEN** it is `true`

#### Scenario: Long password against an original hash
- **WHEN** a `bcryptjs` hash of a 73-byte password is verified with the same password and with its first 72 bytes
- **THEN** both results equal the recorded results of `bcryptjs.compareSync`

### Requirement: Bcrypt hashing never truncates silently
Creating a bcrypt hash of a password longer than 72 bytes MUST return an error instead of truncating it (documented
divergence: `bcryptjs` truncates). Argon2id hashes have no such limit below `max_password_bytes`.

#### Scenario: 73-byte password
- **WHEN** the hasher is configured for bcrypt and asked to hash a 73-byte password
- **THEN** it returns an error

### Requirement: Transparent migration between algorithms
Verification MUST choose the algorithm from the hash prefix, so a database with bcrypt hashes keeps working while new
hashes use the configured algorithm. `needs_rehash(hash)` SHALL report `true` when the hash uses another algorithm or
argon2 variant than the configured one, or weaker parameters (lower memory, iterations, output or salt length, or a
lower bcrypt cost), or cannot be parsed.

#### Scenario: Mixed hashes
- **WHEN** the hasher is configured for argon2id and verifies a correct password against a `$2b$` hash
- **THEN** verification succeeds and `needs_rehash` reports `true`

#### Scenario: Older parameters
- **WHEN** the hasher is configured with 2 iterations and checks an argon2id hash made with 1 iteration
- **THEN** `needs_rehash` reports `true`; for a hash made with the configured parameters it reports `false`

### Requirement: Stored hashes cannot exhaust resources
Before running any key derivation, verification MUST refuse with an error a hash longer than 512 bytes, a bcrypt hash
with a cost above `max_bcrypt_cost` (14 by default), and an argon2 hash with memory above 262144 KiB, more than 16
iterations or parallelism above 16 (each configurable). A hasher configured with parameters above these verification
limits MUST fail to be created, since it could not verify its own hashes.

#### Scenario: Planted high-cost bcrypt hash
- **WHEN** a password is verified against a well-formed `$2b$31$...` hash
- **THEN** an error is returned and no key derivation ran

#### Scenario: Planted high-memory argon2 hash
- **WHEN** a password is verified against `$argon2id$v=19$m=4194304,t=1,p=1$...`
- **THEN** an error is returned and no key derivation ran

### Requirement: Hash computations are bounded and off the async workers
The hasher SHALL run key derivations on blocking threads, never on the async workers, and SHALL run at most
`max_concurrent` of them at a time, `std::thread::available_parallelism()` by default; further calls wait for a slot.
Blocking variants of `hash` and `verify` SHALL exist for code outside an async context.

#### Scenario: Concurrency limit
- **WHEN** the hasher has `max_concurrent(2)` and five verifications start together
- **THEN** at no point more than two key derivations run, and all five complete with correct results

### Requirement: Unknown users cost a real verification
The hasher SHALL provide `verify_dummy(password)`, which runs one verification with the configured algorithm and
parameters against a hash created when the hasher was built and always returns `false`, and
`verify_or_dummy(password, stored_hash)`, which verifies against the stored hash when there is one and runs
`verify_dummy` otherwise. The dummy algorithm SHALL be configurable (for example bcrypt with the cost of most stored
hashes during a migration).

#### Scenario: Unknown user
- **WHEN** a validator calls `verify_or_dummy(password, None)` for an unknown username
- **THEN** the result is `false` and exactly one key derivation ran with the configured algorithm and parameters, the
  same count as for a known user with a hash made by this hasher

#### Scenario: Dummy never matches
- **WHEN** `verify_dummy` is called with any password, including an empty one
- **THEN** the result is `false`

### Requirement: A refused stored hash costs a real verification
When a stored hash is refused (malformed, unsupported or over a limit), `verify_or_dummy` MUST run the dummy
verification before returning the error, so a broken stored hash does not answer faster than an unknown user.

#### Scenario: Malformed stored hash
- **WHEN** `verify_or_dummy(password, Some("not-a-hash"))` is called
- **THEN** an error is returned and exactly one key derivation ran

### Requirement: Verification never panics and compares in constant time
Verifying any password against any string, including malformed, truncated or unknown-format hashes, MUST return
`false` or an error and MUST NOT panic. Comparisons of derived hashes MUST be constant-time.

#### Scenario: Malformed hash
- **WHEN** a password is verified against `"$2b$10$short"`, `"$argon2id$"`, `"$scrypt$..."` or random text
- **THEN** the result is `false` or an error, without a panic

#### Scenario: Random inputs
- **WHEN** a property test verifies random passwords against random strings and mutated valid hashes
- **THEN** no call panics

### Requirement: The local strategy and the hasher are verified against the originals
The local strategy MUST be covered by a differential fixture recorded with `passport-local` 1.0.0, and bcrypt
compatibility by a fixture recorded with `bcryptjs` 3.0.3 in both directions (hashes made by `bcryptjs` verified by
Corvis, hashes made by Corvis verified by `bcryptjs`), replayed by `cargo test` without Node.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis --features auth-local` runs
- **THEN** every case of `local.json` and `bcrypt.json` passes, and divergent cases are asserted against this
  specification

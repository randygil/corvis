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
with configurable field names, in the parsed body (JSON or `application/x-www-form-urlencoded`) as `passport-local`
1.0.0 does. The query string SHALL be read only when `allow_query_credentials()` is set (off by default, a startup
warning when on). An empty field name MUST stop the application at startup.

#### Scenario: Credentials in the query string
- **WHEN** the request is `POST /auth/login?username=ana&password=secret` with no body and the option is not set
- **THEN** the credentials are missing and the response is the 401, with a divergence recorded in the fixture

#### Scenario: Credentials in the query string, opted in
- **WHEN** the same request reaches a strategy with `allow_query_credentials()`
- **THEN** the validator receives `ana` and `secret`, as in the original

#### Scenario: Form body
- **WHEN** the body is `username=ana&password=secret` with `Content-Type: application/x-www-form-urlencoded`
- **THEN** the validator receives `ana` and `secret`

#### Scenario: Body value wins over the query
- **WHEN** `allow_query_credentials()` is set and the body is `{"username":"ana","password":"secret"}` and the query is `?username=eve`
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
Before any slot is taken or any key derivation, verification MUST refuse a hash longer than 512 bytes, a bcrypt hash
with cost above `max_bcrypt_cost` (14), an argon2 hash with memory above 65536 KiB, more than 8 iterations or
parallelism above 16, and an argon2 hash with a salt under 8 bytes. Each limit is overridable with `VerifyLimits` and raised to the hasher's own hashing parameters when those are
higher.

#### Scenario: Planted high-cost bcrypt hash
- **WHEN** a password is verified against a well-formed `$2b$31$...` hash
- **THEN** an error is returned and no key derivation ran

#### Scenario: Planted high-memory argon2 hash
- **WHEN** a password is verified against `$argon2id$v=19$m=4194304,t=1,p=1$...`
- **THEN** an error is returned and no key derivation ran

#### Scenario: Short argon2 salt
- **WHEN** a password is verified against an argon2 hash whose salt is 7 bytes
- **THEN** the hash is malformed, and neither a slot nor a key derivation was used

#### Scenario: Own parameters above the default limits
- **WHEN** a hasher is configured with 100000 KiB and 9 iterations and verifies its own hash
- **THEN** the result is `true`, while a default hasher refuses that hash without any derivation

### Requirement: Verification limits have upper bounds
A `VerifyLimits` above 1048576 KiB of argon2 memory, 64 argon2 iterations or bcrypt cost 20 MUST fail construction,
as MUST hashing parameters above them. A startup warning SHALL be logged when `max_concurrent` times the argon2
memory limit exceeds 1 GiB.

#### Scenario: Limits above the upper bounds
- **WHEN** `VerifyLimits` has `max_bcrypt_cost(21)`, 1048577 KiB of memory or 65 iterations
- **THEN** creating the hasher fails

### Requirement: Non-canonical bcrypt hashes are refused
A bcrypt hash whose last salt character or last digest character has non-zero unused trailing bits MUST be refused as
malformed before any key derivation: no bcrypt implementation writes such a hash.

#### Scenario: Altered trailing bits
- **WHEN** the last salt character or the last digest character of a valid `$2b$` hash is changed to a character with
  non-zero trailing bits
- **THEN** the hash is malformed and no key derivation ran

### Requirement: Hash computations are bounded and off the async workers
The hasher SHALL run key derivations on blocking threads, never on the async workers, and at most `max_concurrent`
at a time, `std::thread::available_parallelism()` capped at 16 by default (an explicit value is honoured up to
1024, so the defaults never reach the memory warning). Further calls wait for a slot.

#### Scenario: Concurrency limit
- **WHEN** the hasher has `max_concurrent(2)` and five verifications start together
- **THEN** at no point more than two key derivations run, and all five complete with correct results

### Requirement: The wait for a slot is bounded
At most `max_waiting` calls (default 32 times `max_concurrent`, at least 1) SHALL wait for a slot, each for at most
`acquire_timeout` (default 10 s). A call that finds the queue full or waits too long MUST fail with
`PasswordError::Busy` without any key derivation; a cancelled waiter frees its place. A login validator answers it
`503 Service Unavailable`, the same for every user.

#### Scenario: Full queue
- **WHEN** `max_concurrent(1)` and `max_waiting(2)` are configured, the slot is held and two calls wait
- **THEN** the next call fails at once with `Busy` and no key derivation ran for it

#### Scenario: Cancelled waiter
- **WHEN** one of the two waiting calls is cancelled
- **THEN** the next call is accepted into the queue

#### Scenario: Busy over HTTP
- **WHEN** the only slot is held and a known and an unknown user log in through the documented validator
- **THEN** both get the same 503 response

### Requirement: Blocking variants are for code outside a runtime
`hash_blocking`, `verify_blocking`, `verify_or_dummy_blocking` and `verify_dummy_blocking` MUST return
`PasswordError::InsideRuntime` at once, without waiting or deriving, inside any Tokio runtime context. Outside one they
wait for a slot without a Tokio timer, honouring `acquire_timeout` and `max_waiting` (`Busy`), and derive on the
calling thread.

#### Scenario: Inside a runtime
- **WHEN** a blocking variant is called on a multi-thread or current-thread runtime, a `spawn_blocking` thread or a
  `LocalSet`
- **THEN** it returns `InsideRuntime` at once, nothing panics and no key derivation ran

#### Scenario: Contention outside a runtime
- **WHEN** the only slot is held and a blocking variant is called on a plain thread
- **THEN** it returns `Busy` after `acquire_timeout`, and succeeds when the slot frees earlier

#### Scenario: Mixed async load
- **WHEN** hundreds of async `verify` and `hash` calls run at once and the queue overflows
- **THEN** every call ends with `Ok` or `Busy`, and a timer on the same runtime keeps firing

### Requirement: Unknown users cost a real verification
The hasher SHALL provide `verify_dummy(password)`, one verification against a hash created at construction with the
configured algorithm (or `dummy_algorithm`), always `Ok(false)`, and `verify_or_dummy(password, stored)`, which
verifies against the stored hash when there is one and runs `verify_dummy` otherwise.

#### Scenario: Unknown user
- **WHEN** a validator calls `verify_or_dummy(password, None)` for an unknown username
- **THEN** the result is `Ok(false)` and exactly one key derivation ran, as for a known user with a hash made by this
  hasher

#### Scenario: Dummy never matches
- **WHEN** `verify_dummy` is called with any password, including an empty one
- **THEN** the result is `Ok(false)`

### Requirement: A refused stored hash answers like an unknown user
When a stored hash is malformed, unsupported or over a limit, `verify_or_dummy` MUST run the dummy verification and
return `Ok(false)`. The warning naming the reason (never the hash or the user input) MUST be logged only after the
dummy ran (not on `Busy`), at most once a minute per reason with the count of those suppressed. A refusal found only
inside the task is answered with the dummy in the same slot. `verify` keeps returning the error.

#### Scenario: Malformed stored hash
- **WHEN** `verify_or_dummy(password, Some("not-a-hash"))` is called
- **THEN** `Ok(false)` is returned and exactly one key derivation ran

#### Scenario: Saturated hasher
- **WHEN** the only slot is held and `verify_or_dummy` gets a refused hash
- **THEN** `Busy` is returned and no warning was logged

#### Scenario: Warning rate limit
- **WHEN** the same refusal reason happens repeatedly within a minute
- **THEN** one warning is logged, and the next one after the minute states how many were not logged

#### Scenario: Refused hashes over HTTP
- **WHEN** users with an empty hash, a `$2b$15$` hash, a `$2x$` hash and a `$scrypt$` hash log in through the documented
  validator
- **THEN** the status and body equal those of an unknown user and of a wrong password

### Requirement: A dummy hash that differs from the stored hashes is reported
When a stored hash that parses differs in algorithm or cost parameters from the dummy hash, the hasher SHALL log one
warning, once, pointing to `dummy_algorithm`, since the response time then tells known users from unknown ones.

#### Scenario: Migration without a matching dummy
- **WHEN** a hasher with an argon2id dummy verifies a bcrypt hash
- **THEN** one warning is logged, and none when the dummy is bcrypt of the same cost

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

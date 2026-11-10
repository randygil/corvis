# Spec Delta

## Purpose

API key authentication: a named strategy of `auth/strategies` that reads a key from the places the app configures and
passes it to the app's validator, with the header behavior of `passport-headerapikey` 1.2.2 except where this
specification is stricter on purpose (each divergence is named in its requirement), plus the helpers that compare
keys in constant time and keep key material out of logs and errors. Every limit has a default and a startup check.

## ADDED Requirements

### Requirement: API key strategy registration and validator
The framework SHALL provide `ApiKeyStrategy<V>`, registered with `Provider::strategy::<ApiKeyStrategy<V>>()` under the
`auth-api-key` cargo feature, whose validator `V` receives its dependencies through dependency injection and
implements `validate(request, key)`. The strategy name SHALL be `"api-key"` unless the validator sets another
one. The validator's result SHALL map to the outcomes of `auth/strategies`: a user is a success, no user is a
failure, an exception is an error.

#### Scenario: Valid key
- **WHEN** a route uses `AuthGuard::new("api-key")`, the request has `X-Api-Key: <key>` and the validator
  accepts that key
- **THEN** the handler runs with the validator's user and the response matches the recorded original

#### Scenario: Wrong key
- **WHEN** the validator returns no user
- **THEN** the response is the 401 `{"message":"Unauthorized","statusCode":401}`, as the recorded original

#### Scenario: Validator throws
- **WHEN** the validator returns `Err(ForbiddenException::new("Key suspended"))`
- **THEN** the response is the recorded original 403

#### Scenario: Chained after another strategy
- **WHEN** a route uses `AuthGuard::chain(["jwt", "api-key"])`, the request has no bearer token and a valid API
  key
- **THEN** the handler runs with the API key validator's user, as in the recorded original

### Requirement: The key is read from a header like passport-headerapikey
With the default options the strategy SHALL read the key from the `X-Api-Key` header. The header name SHALL be
configurable and compared case-insensitively. The header value SHALL be read as Node exposes it: bytes decoded as
latin1, repeated values joined with `", "`. A missing or empty header MUST make the strategy fail without calling the
validator.

#### Scenario: Header name case
- **WHEN** the request has `x-api-key: <key>` or `X-API-KEY: <key>`
- **THEN** the validator receives `<key>` in both cases

#### Scenario: Missing header
- **WHEN** the request has no API key header
- **THEN** the validator is not called and the response is the recorded original 401

#### Scenario: Two header values
- **WHEN** the request has two `X-Api-Key` headers with the values `a` and `b`
- **THEN** the validator receives the single string `a, b`, as in the recorded original

### Requirement: Header prefix is compared literally
A header source MAY have a prefix. The value MUST start with the prefix, compared byte by byte and case-sensitively,
and the prefix SHALL be removed before the key reaches the validator; a value without the prefix MUST make the
strategy fail without calling the validator. Documented divergence: `passport-headerapikey` 1.2.2 builds an unescaped
regular expression from the prefix.

#### Scenario: Prefix removed
- **WHEN** the source is the `Authorization` header with the prefix `"Api-Key "` and the header is
  `Authorization: Api-Key <key>`
- **THEN** the validator receives `<key>`

#### Scenario: Prefix absent
- **WHEN** the same source receives `Authorization: <key>`
- **THEN** the validator is not called and the response is the 401

#### Scenario: Prefix with a regular expression character
- **WHEN** the prefix is `"Key+"` and the header value is `Key+abc`, and in another request `Keyyabc`
- **THEN** the validator receives `abc` for the first (the original's `verify` receives `+abc`; the fixture marks the
  divergence) and the second fails without calling the validator, as in the original

### Requirement: Bearer source
When configured with the bearer source, the strategy SHALL read the key from the first `Authorization` header value
with the scheme `bearer`, using the same parsing rule as the bearer extractor of `auth/jwt-strategy`. The behavior
MUST match a `passport-custom` 1.2.1 strategy that extracts the key with that rule.

#### Scenario: Bearer key
- **WHEN** the source is `ApiKeySource::bearer()` and the request has `Authorization: Bearer <key>`
- **THEN** the validator receives `<key>`

#### Scenario: Another scheme
- **WHEN** the request has `Authorization: Basic <value>`
- **THEN** the source yields no key and, with no other source, the response is the 401

### Requirement: Query source only when configured
The strategy SHALL read a key from the query string only when a query source is configured, and SHALL log a startup
warning naming the strategy and the parameter when one is. The value MUST be a single string: a repeated parameter (or
a bracketed one, with the extended query parser) fails the strategy. A single string MUST behave as in a
`passport-custom` 1.2.1 strategy reading `req.query[name]`. The documentation MUST warn that keys in URLs end up in
proxy and access logs and browser history.

#### Scenario: Query key
- **WHEN** the strategy is configured with the query parameter `api_key` and the request is
  `GET /data?api_key=<key>`
- **THEN** the validator receives `<key>`

#### Scenario: Query key without the option
- **WHEN** the strategy has the default options and the request is `GET /data?api_key=<key>` with no header
- **THEN** the validator is not called and the response is the 401

#### Scenario: Repeated parameter
- **WHEN** the request is `GET /data?api_key=a&api_key=b`, with or without a valid key in a header source
- **THEN** the validator is not called and the response is the 401

#### Scenario: Startup warning
- **WHEN** an application with a query source is initialized
- **THEN** one warning is logged that names the strategy and the parameter and contains no key

### Requirement: Keys are read only from configured sources
The strategy MUST NOT read a key from any place that is not a configured source. The request body and cookies MUST
never be sources. Options without any source MUST stop the application at startup.

#### Scenario: Key in the body
- **WHEN** a `POST` request has `{"api_key":"<key>"}` in its body and no key in a configured source
- **THEN** the validator is not called and the response is the 401

#### Scenario: No source
- **WHEN** the validator's options are `ApiKeyStrategyOptions::new()` with no source added
- **THEN** `init()` fails with an error that names the strategy

### Requirement: Conflicting keys fail
When more than one configured source yields a key in the same request, the keys MUST be equal, compared in constant
time; otherwise the strategy MUST fail without calling the validator. A source whose value is present but unusable
(too long, a control character, a repeated query parameter) MUST fail it too, so no other source decides. Documented
divergence: the original has a single source.

#### Scenario: Different keys in the header and the query
- **WHEN** header and query sources are configured and the request has `X-Api-Key: <key A>` and `?api_key=<key B>`
- **THEN** the validator is not called and the response is the 401

#### Scenario: The same key twice
- **WHEN** both places carry `<key A>`
- **THEN** the validator is called once with `<key A>`

#### Scenario: One place unusable
- **WHEN** the header carries a key of 600 bytes and the query a valid key
- **THEN** the validator is not called and the response is the 401

### Requirement: Constant-time comparison helpers
The framework SHALL provide `constant_time_eq`, `ApiKey::ct_eq`, `ExpectedApiKey::matches` and `ApiKeySet::lookup`.
Each MUST compare fixed-size digests of its inputs with a constant-time primitive, so that the time taken depends
neither on the position of the first different byte nor on the length of the stored key. `ApiKeySet::lookup` MUST
compare the presented key with every entry. `ApiKey` MUST NOT implement `PartialEq`. The documented examples SHALL use
these helpers.

#### Scenario: Comparison results
- **WHEN** the helper compares a key with itself, with a key that differs in the first byte, with a key that differs
  in the last byte, with a prefix of itself and with the empty string
- **THEN** only the first comparison returns `true`

#### Scenario: Every entry is compared
- **WHEN** `ApiKeySet::lookup` runs on a set of five keys with a presented key equal to the first entry, and again
  with a key equal to none
- **THEN** a test counter of digest comparisons reads five in both cases

#### Scenario: Digest lookup
- **WHEN** an app stores `sha256_hex` of a key and later receives that key
- **THEN** `ApiKey::sha256_hex()` of the received key equals the stored value, which is the lowercase hexadecimal
  SHA-256 of the key bytes

### Requirement: Configured keys have a minimum length
`ExpectedApiKey::new` MUST refuse a secret shorter than 32 bytes, and `ExpectedApiKey::from_sha256_hex` MUST refuse
anything that is not 64 hexadecimal characters. The error MUST NOT contain the secret. An application whose provider
fails to build an `ExpectedApiKey` MUST NOT start.

#### Scenario: Short configured key
- **WHEN** a provider builds `ExpectedApiKey::new("short")`
- **THEN** it gets an error that states the minimum and the length received, without the secret, and an application
  that propagates it from a provider factory fails in `init()`

#### Scenario: Key of 32 bytes
- **WHEN** the secret has exactly 32 bytes
- **THEN** the `ExpectedApiKey` is built and matches that key only

### Requirement: Key length and characters are bounded
The strategy SHALL refuse a key longer than `max_key_bytes` (default 512) and a key that contains a control character
(bytes `0x00` to `0x1F` and `0x7F`), failing without calling the validator. `max_key_bytes` outside 1 to 4096 MUST
stop the application at startup. Documented divergence: the original has no limit of its own.

#### Scenario: Oversized key
- **WHEN** the header carries a key of 513 bytes with the default options
- **THEN** the validator is not called and the response is the 401

#### Scenario: Control character in a query key
- **WHEN** the query source receives `?api_key=abc%00def`
- **THEN** the validator is not called and the response is the 401

#### Scenario: Limit out of range
- **WHEN** the options set `max_key_bytes(0)` or `max_key_bytes(5000)`
- **THEN** `init()` fails with an error that names the strategy and the option

### Requirement: Invalid options stop the application
An empty header name, a header name that is not a valid HTTP header name, an empty query parameter name, an empty
prefix, more than 8 sources, a `Cookie` header source and a `validator_timeout` outside 1 ms to 300 s MUST each stop
the application at startup with an error that names the strategy and the option.

#### Scenario: Empty header name
- **WHEN** the options contain `ApiKeySource::header("")`
- **THEN** `init()` fails with an error that names the strategy and the header source

### Requirement: Validator failures are uniform and bounded
An exception of the validator with a 4xx status SHALL be passed on unchanged. One with a status of 500 or above, or
below 400, SHALL be replaced by the plain exception of that status, whose body no key can influence. A validator that
runs longer than `validator_timeout` (default 5 s) SHALL be answered with a 503, and a panic in it with a 500, for
every key and also behind an optional guard. Warnings about these are rate limited and never contain the message.

#### Scenario: Infrastructure failure
- **WHEN** the validator fails with a 503 whose message contains the key, for two different keys
- **THEN** both responses are the same `{"statusCode":503,"message":"Service Unavailable"}` without the key

#### Scenario: Slow or panicking validator behind an optional guard
- **WHEN** the validator never answers within `validator_timeout`, or panics
- **THEN** the response is a 503, or a 500, and the route handler does not run

### Requirement: Lookups by id cost the same for unknown keys
`ApiKey::id(delimiter)` SHALL return the public id of a key written `<id><delimiter><secret>` only when it is 1 to 64
characters of `A-Z a-z 0-9 _ -`. `ExpectedApiKey::matches_or_dummy` SHALL compare the key with a fixed digest and return
`false` when there is no stored record, so an unknown id costs the same comparison as a wrong key.

#### Scenario: No record
- **WHEN** `matches_or_dummy` receives `None` and any key
- **THEN** it returns `false`, and it returns `true` only for the key of a given record

### Requirement: No key material in logs, errors or debug output
The framework MUST NOT write an API key, or any part of one, to a log line, an exception message, a response body or
the auth info. Failure reasons SHALL be fixed strings. `ApiKey`, `ExpectedApiKey` and `ApiKeySet` SHALL have a `Debug`
output that does not contain key material, and MUST NOT implement `Display` or `Serialize`.

#### Scenario: Debug output
- **WHEN** an `ApiKey` holding `<key>` is formatted with `{:?}`, alone and inside a struct that derives `Debug`
- **THEN** the output contains `<redacted>` and no substring of `<key>` longer than three characters

#### Scenario: Failure reasons
- **WHEN** the strategy fails for a missing key, a wrong prefix, conflicting keys, an oversized key, a refused
  character and a repeated query parameter, behind an optional `AuthGuard` so the auth info can be read
- **THEN** each auth info message is one of the fixed strings of the design and contains no request data

#### Scenario: Captured logs
- **WHEN** the test suite of the strategy runs every case of the fixture with the framework logger captured
- **THEN** no captured line contains any of the fixture's keys

### Requirement: Query keys are redacted in request logs
`Request::redact_query_param(name)` SHALL mark a query parameter of the current request as sensitive (at most 16 names
per request), and `Request::redacted_url()` SHALL return the request URL with the value of every occurrence of a
marked parameter replaced by `REDACTED`, and the unchanged URL when nothing is marked. The API key strategy MUST mark
every configured query parameter before it reads the request, whatever the outcome. `LoggingInterceptor` SHALL log
`redacted_url()`.

#### Scenario: Logged URL
- **WHEN** a route with a query-source API key strategy and `LoggingInterceptor` receives
  `GET /data?page=2&api_key=<key>` with a valid key
- **THEN** the logged line contains `/data?page=2&api_key=REDACTED` and not `<key>`

#### Scenario: Encoded name and repeated parameter
- **WHEN** the marked parameter is `api_key` and the URL is `/data?api%5Fkey=a&api_key=b`
- **THEN** `redacted_url()` hides both values

#### Scenario: More names than the limit
- **WHEN** a seventeenth name is marked
- **THEN** `redacted_url()` hides the value of every parameter rather than leave a marked one visible

#### Scenario: Nothing marked
- **WHEN** no parameter was marked
- **THEN** `redacted_url()` equals `url()` and the line `LoggingInterceptor` logs is the same as before this change

### Requirement: The API key strategy is verified against the original
The API key strategy SHALL be covered by a differential test: a generator in the fixture package of `auth/strategies`
records the responses of `passport-headerapikey` 1.2.2 (header cases) and of `passport-custom` 1.2.1 strategies
(query and bearer cases) behind the original `AuthGuard`, and a Rust test replays every case without Node. A case
where Corvis diverges on purpose MUST be marked in the fixture with its reason and asserted from this specification.

#### Scenario: Fixture replay
- **WHEN** `cargo test -p corvis --features auth-api-key --test auth_api_key` runs on a machine without Node
- **THEN** every recorded case is compared (status, listed headers, body) and every marked divergence asserts the
  behavior this specification states

#### Scenario: Arbitrary header bytes
- **WHEN** a property test sends random bytes as the API key header and as the `Authorization` header
- **THEN** the strategy never panics and calls the validator only with a key that passes the length and character
  limits

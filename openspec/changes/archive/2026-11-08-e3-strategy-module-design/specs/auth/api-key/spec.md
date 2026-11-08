# Spec Delta

## ADDED Requirements

### Requirement: API key in a header
The API key strategy SHALL read the key from a header, `X-Api-Key` by default (configurable, compared
case-insensitively), and strip an optional prefix compared literally (documented divergence: `passport-headerapikey`
1.2.2 builds an unescaped regular expression from it). A missing header or a missing prefix MUST make the strategy
fail without calling the validator (401). The default strategy name SHALL be `"headerapikey"`.

#### Scenario: Valid key
- **WHEN** a request has `X-API-KEY: k-123` and the validator accepts `k-123`
- **THEN** the handler runs with the validator's user and the response matches the recorded original

#### Scenario: Prefix
- **WHEN** the prefix is `"Api-Key "` and the header is `x-api-key: Api-Key k-123`
- **THEN** the validator receives `k-123`

#### Scenario: Missing header
- **WHEN** the request has no API key header
- **THEN** the validator is not called and the response is the recorded original 401
  `{"message":"Unauthorized","statusCode":401}`

### Requirement: API key in a query parameter
The strategy SHALL also accept the key from a configurable query parameter, when configured. The behavior MUST match a
`passport-custom` 1.2.1 strategy that reads `req.query[name]` and fails when it is absent. The documentation MUST warn
that keys in URLs end up in logs and browser history.

#### Scenario: Query key
- **WHEN** the strategy is configured with the query parameter `api_key` and the request is `GET /data?api_key=k-123`
- **THEN** the validator receives `k-123`

### Requirement: Constant-time key comparison
The framework SHALL provide a constant-time comparison helper for API keys and SHALL use it in the documented
examples, so a validator that compares a received key with a stored key does not reveal the matching prefix through
timing.

#### Scenario: Comparison helper
- **WHEN** the helper compares two keys of the same length that differ only in the last byte, and two that differ in
  the first byte
- **THEN** both return `false` and the comparison does not stop at the first different byte

### Requirement: Custom strategies
A user type that implements the `Strategy` trait directly SHALL be registrable with `Provider::strategy::<S>()` and
usable by name in `AuthGuard`, with dependency injection. Each of its outcomes (success, fail, redirect, pass, error)
MUST produce the same HTTP response as a `passport-strategy` 1.0.0 subclass whose `authenticate()` calls the same
outcome (`passport-custom` can only reach success, fail and error).

#### Scenario: Custom strategy outcomes
- **WHEN** a custom strategy is driven to each of its five outcomes by a request header
- **THEN** each response equals the recorded original response of the `passport-strategy` subclass driven the same
  way

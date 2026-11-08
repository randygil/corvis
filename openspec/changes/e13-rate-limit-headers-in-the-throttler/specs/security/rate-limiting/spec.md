# Spec Delta

## Purpose

The informational headers `ThrottlerGuard` sends, compatible with `@nestjs/throttler`: names, values, suffixes for
named throttlers, the 429 case, and how to turn them off.

## ADDED Requirements

### Requirement: ThrottlerGuard sends rate limit headers on counted requests
For each throttler that counts an HTTP request and does not block it, `ThrottlerGuard` SHALL set, through the
enhancer response-header mechanism, `X-RateLimit-Limit<suffix>` (the effective limit, after `THROTTLE_LIMIT`
metadata), `X-RateLimit-Remaining<suffix>` (`limit - hits`, never below 0) and `X-RateLimit-Reset<suffix>` (seconds
until the throttler's current window ends, rounded up).

#### Scenario: First request with the default throttler
- **WHEN** the only throttler is `ThrottlerOptions::new(60_000, 10)` and a client sends its first request to a route
- **THEN** the response carries `X-RateLimit-Limit: 10`, `X-RateLimit-Remaining: 9` and `X-RateLimit-Reset: 60`

#### Scenario: Limit overridden by metadata
- **WHEN** a route declares `#[set_metadata(THROTTLE_LIMIT, 3)]` and the throttler's limit is 10
- **THEN** the response carries `X-RateLimit-Limit: 3`

### Requirement: Header names carry the throttler name except for the default throttler
The `<suffix>` of every throttler header SHALL be empty for a throttler named `default` and `-<name>` otherwise.

#### Scenario: Named throttlers
- **WHEN** the throttlers are `short` (`ttl: 1_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 100`) and a
  client sends one request
- **THEN** the response carries `X-RateLimit-Limit-short: 3`, `X-RateLimit-Remaining-short: 2`,
  `X-RateLimit-Limit-long: 100` and `X-RateLimit-Remaining-long: 99`, each with its `X-RateLimit-Reset-<name>`

### Requirement: Skipped requests carry no throttler headers
Requests skipped with `SKIP_THROTTLE` metadata SHALL get no `X-RateLimit-*` or `Retry-After` header.

#### Scenario: Skipped route
- **WHEN** a route declares `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** the response carries no `X-RateLimit-*` or `Retry-After` header

### Requirement: A throttled request carries Retry-After
When a throttler blocks a request, `ThrottlerGuard` SHALL set `Retry-After<suffix>` to the seconds until that
throttler's current window ends (rounded up, at least 1), set no `X-RateLimit-*` header for the blocking throttler,
and fail with the existing `ThrottlerException: Too Many Requests` 429 exception. Headers already set by throttlers
checked earlier for the same request MUST stay on the 429 response.

#### Scenario: Limit exceeded
- **WHEN** the only throttler allows 2 requests per 60 seconds and a client sends a third request 1 second after the
  first
- **THEN** the response is 429 with body `{ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }`
  and carries `Retry-After: 59` and no `X-RateLimit-*` header

#### Scenario: Second throttler blocks
- **WHEN** throttler `short` still allows the request and throttler `long` blocks it
- **THEN** the 429 response carries the `X-RateLimit-*-short` headers and `Retry-After-long`

### Requirement: Rate limit headers can be turned off
`ThrottlerStorage` SHALL provide `with_headers(bool)`, `true` by default. When `false`, `ThrottlerGuard` MUST set no
`X-RateLimit-*` or `Retry-After` header and MUST keep counting and blocking as before.

#### Scenario: Headers disabled
- **WHEN** the module is `ThrottlerModule::for_root_storage(ThrottlerStorage::new(vec![..]).with_headers(false))` and
  a client exceeds the limit
- **THEN** responses carry no throttler headers and the request over the limit still answers 429

### Requirement: No rate limit headers in a WebSocket context
When `ThrottlerGuard` runs for a gateway message (`ctx.get_type()` is `ContextType::Ws`), it SHALL count and block as
for HTTP and MUST NOT set any header.

#### Scenario: Gateway message
- **WHEN** a global `ThrottlerGuard` runs on a gateway message
- **THEN** the message is counted, no header is recorded, and a message over the limit receives an `exception` frame

### Requirement: Header values match @nestjs/throttler
The header names, the requests that carry them, and the values of `X-RateLimit-Limit`, `X-RateLimit-Remaining` and
`X-RateLimit-Reset` SHALL match those `@nestjs/throttler` produces for the same throttlers and request sequence,
verified by a differential test against a recorded fixture (time-based values compared within one second).

#### Scenario: Replay of the recorded sequence
- **WHEN** the Rust test replays the requests recorded from `@nestjs/throttler` (two named throttlers, a skipped
  route, a route with a limit override, requests up to and past the limit)
- **THEN** every response has the same throttler header names and the same `Limit` and `Remaining` values, and `Reset`
  within one second of the recorded value

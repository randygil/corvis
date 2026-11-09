# Spec Delta

## Purpose

How `ThrottlerGuard` counts and blocks, compatible with `@nestjs/throttler`: hits counted in a sliding window, a
block that lasts a configurable duration from the moment the limit is exceeded, the order of the throttlers, the
storage memory bound, and the header values that follow from them. Requires the requirements added by
`e13-rate-limit-headers-in-the-throttler` (#120).

## MODIFIED Requirements

### Requirement: ThrottlerGuard sends rate limit headers on counted requests
For each throttler that counts an HTTP request and does not block it, `ThrottlerGuard` SHALL set, through the
enhancer response-header mechanism, `X-RateLimit-Limit<suffix>` (the effective limit, after `THROTTLE_LIMIT`
metadata), `X-RateLimit-Remaining<suffix>` (the limit minus the hits counted for the key in the last `ttl`, including
this one, never below 0) and `X-RateLimit-Reset<suffix>` (seconds until the key's reporting window ends, rounded up;
the window starts with the key's first counted request and restarts with the first request after it ends).

#### Scenario: First request with the default throttler
- **WHEN** the only throttler is `ThrottlerOptions::new(60_000, 10)` and a client sends its first request to a route
- **THEN** the response carries `X-RateLimit-Limit: 10`, `X-RateLimit-Remaining: 9` and `X-RateLimit-Reset: 60`

#### Scenario: Limit overridden by metadata
- **WHEN** a route declares `#[set_metadata(THROTTLE_LIMIT, 3)]` and the throttler's limit is 10
- **THEN** the response carries `X-RateLimit-Limit: 3`

#### Scenario: Hits expire one ttl after they were counted
- **WHEN** the only throttler is `ThrottlerOptions::new(10_000, 3)` and a client sends requests at 0 s, 9 s and 11 s
- **THEN** the third response carries `X-RateLimit-Remaining: 1` (the hits of 9 s and 11 s) and
  `X-RateLimit-Reset: 10` (the window restarted at 11 s)

### Requirement: A throttled request carries Retry-After
When a throttler rejects a request, `ThrottlerGuard` SHALL set `Retry-After<suffix>` to the seconds until that
throttler lets the key in again (the end of the block, or with a block duration of 0 the expiry of the oldest counted
hit), rounded up and at least 1, set no `X-RateLimit-*` header for the rejecting throttler, and fail with the existing
`ThrottlerException: Too Many Requests` 429 exception. Headers already set by throttlers checked earlier for the same
request MUST stay on the 429 response.

#### Scenario: Limit exceeded
- **WHEN** the only throttler allows 2 requests per 60 seconds and a client sends a third request 1 second after the
  first
- **THEN** the response is 429 with body `{ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }`
  and carries `Retry-After: 60` and no `X-RateLimit-*` header

#### Scenario: Retry during the block
- **WHEN** after that 429 the client sends another request 30 seconds later
- **THEN** the response is 429 with `Retry-After: 30`

#### Scenario: Second throttler blocks
- **WHEN** throttler `short` still allows the request and throttler `long` blocks it
- **THEN** the 429 response carries the `X-RateLimit-*-short` headers and `Retry-After-long`

### Requirement: Header values match @nestjs/throttler
The header names, the requests that carry them, which requests are rejected, and the values of
`X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset` and `Retry-After` SHALL match those
`@nestjs/throttler` produces for the same throttlers and request sequence, verified by a differential test against a
recorded fixture (time-based values compared within one second).

#### Scenario: Replay of the recorded sequence
- **WHEN** the Rust test replays the requests recorded from `@nestjs/throttler` (two named throttlers, a skipped
  route, a route with a limit override, requests up to and past the limit, a retry during a block and a request after
  it ends)
- **THEN** every response has the same status, the same throttler header names, the same `Limit` and `Remaining`
  values, and `Reset` and `Retry-After` within one second of the recorded values

## ADDED Requirements

### Requirement: Hits are counted in a sliding window per key
`ThrottlerStorage` SHALL keep, per key (throttler name, client IP, controller and handler), the expiry of each counted
hit, `ttl` after the hit was counted, in at most 64 buckets (hits whose expiries are less than `ttl / 64` apart share a
bucket that expires with its newest hit, never earlier), and the count for a request MUST be the number of hits not yet
expired. A request rejected while its key is blocked MUST NOT be counted.

#### Scenario: No double burst around a window boundary
- **WHEN** the only throttler is `ThrottlerOptions::new(10_000, 3)` and a client sends requests at 0 s, 9.5 s, 9.6 s,
  10.1 s and 10.2 s
- **THEN** the first four requests succeed and the fifth answers 429

#### Scenario: Rejected requests are not counted
- **WHEN** a key is blocked and the client keeps sending requests until the block ends
- **THEN** the first request after the block succeeds with `X-RateLimit-Remaining` equal to the limit minus 1

### Requirement: A key over the limit is blocked for the block duration
When a counted hit takes a key's count over the limit, the storage SHALL block that key and throttler for the
effective block duration from that moment. While blocked, every request for the key MUST be rejected without being
counted and without extending the block. The first request after the block ends MUST clear the key's hits and be
counted as its first hit.

#### Scenario: Block ends after its duration
- **WHEN** the only throttler allows 2 requests per 60 seconds, a client's third request at 2 s is rejected, and the
  client sends another request at 63 s
- **THEN** the request at 63 s succeeds with `X-RateLimit-Remaining: 1` and `X-RateLimit-Reset: 60`

### Requirement: The block duration defaults to the ttl and can be configured
The effective block duration of a throttler SHALL be the route's or controller's `THROTTLE_BLOCK_DURATION` metadata
(milliseconds, applied to every throttler) when present, else the value set with
`ThrottlerStorage::with_block_duration(name, ms)` for that throttler, else the effective `ttl` (after
`THROTTLE_TTL`). `with_block_duration` with a name that matches no configured throttler MUST panic with a message
naming it.

#### Scenario: Default follows the ttl override
- **WHEN** a route declares `#[set_metadata(THROTTLE_TTL, 5_000)]` and `#[set_metadata(THROTTLE_LIMIT, 1)]` and a
  client sends two requests in a row
- **THEN** the second response is 429 with `Retry-After: 5`

#### Scenario: Block longer than the ttl
- **WHEN** the storage is `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("default",
  300_000)`, a client's third request at 2 s is rejected, and the client sends another request at 63 s
- **THEN** the request at 63 s answers 429 with `Retry-After: 239`

#### Scenario: Metadata overrides the storage value
- **WHEN** the storage sets a block duration of 300,000 ms for `default` and a route declares
  `#[set_metadata(THROTTLE_BLOCK_DURATION, 10_000)]`
- **THEN** the first rejected request on that route carries `Retry-After: 10`

#### Scenario: Unknown throttler name
- **WHEN** `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("login", 1_000)` runs
- **THEN** it panics with a message containing `login`

### Requirement: A block duration of zero disables the block
With an effective block duration of 0, the storage SHALL reject a request when the key already has `limit` unexpired
hits, MUST NOT count the rejected request, and `Retry-After` MUST be the seconds until the oldest of those hits
expires.

#### Scenario: Sliding limit without block
- **WHEN** a route declares `#[set_metadata(THROTTLE_BLOCK_DURATION, 0)]`, the throttler is
  `ThrottlerOptions::new(10_000, 2)`, and a client sends requests at 0 s, 1 s, 2 s and 10.5 s
- **THEN** the request at 2 s answers 429 with `Retry-After: 8`, and the request at 10.5 s succeeds with
  `X-RateLimit-Remaining: 0`

### Requirement: A limit of zero rejects every request
When the effective limit is 0, `ThrottlerGuard` SHALL reject every request, including the first request after a block
ends. This deliberately differs from `@nestjs/throttler` 6.7.1, which lets one request through each time a block
ends.

#### Scenario: Request after the block on a closed route
- **WHEN** a route declares `#[set_metadata(THROTTLE_LIMIT, 0)]` and `#[set_metadata(THROTTLE_BLOCK_DURATION, 1_000)]`
  and a client sends a request, waits 1.5 seconds and sends another
- **THEN** both requests answer 429

### Requirement: Throttlers are checked in ascending ttl order
`ThrottlerGuard` SHALL check the configured throttlers in ascending `ttl` order, keeping the configured order between
throttlers with the same `ttl`, and MUST stop at the first throttler that rejects the request, so later throttlers do
not count it.

#### Scenario: Configured from longest to shortest
- **WHEN** the throttlers are configured as `long` (`ttl: 60_000`, `limit: 100`) then `short` (`ttl: 1_000`,
  `limit: 1`), and a client sends a request, a second one 100 ms later and a third one 1.2 seconds after the first
- **THEN** the second response is 429 with `Retry-After-short` and no `X-RateLimit-*-long` header, and the third
  response carries `X-RateLimit-Remaining-long: 98`

### Requirement: Storage memory stays bounded
`ThrottlerStorage` SHALL keep at most `max_entries` keys (`with_max_entries`, default 100,000), and each key MUST use
memory that does not depend on its limit or on the number of requests it receives (at most 64 hit buckets). When the
table is full the keys that would become idle soonest MUST be evicted, a blocked key only when no unblocked key is
left, so flooding other keys never unblocks a blocked key. A key whose hits have expired, whose reporting window has
ended and that is not blocked MUST be removed. A block duration or `ttl` too large to represent MUST saturate to a far
instant, never to the present.

#### Scenario: Round-robin keys cannot exceed the limit
- **WHEN** 2,000 distinct keys each send requests in turn with a limit of 100 per hour
- **THEN** every key gets exactly 100 requests through and the rest are rejected

#### Scenario: Blocked key survives a flood
- **WHEN** a key is blocked and 5,000 other keys then record hits on a storage built with `with_max_entries(100)`
- **THEN** the storage holds at most 100 keys and the first key is still blocked

#### Scenario: Memory per key does not grow with hits
- **WHEN** one key records 20,000 hits
- **THEN** the storage uses less than 8 KiB and the next request reports 20,001 hits

#### Scenario: Idle keys are removed
- **WHEN** a key's hits have expired, its window has ended, it is not blocked, and another key records a hit
- **THEN** the storage holds only the second key

### Requirement: Storage counting matches @nestjs/throttler
For any sequence of hits with given times, keys, `ttl`, `limit` and block duration (with `limit` of at least 1),
`ThrottlerStorage::record_hit` SHALL return the same hit count, blocked state, seconds to the window end and seconds to
the block end (rounded up; 0 when not blocked) as `ThrottlerStorageService.increment` of `@nestjs/throttler` 6.7.1
while the key's hits are more than `ttl / 64` apart, verified by a differential test that replays recorded calls with
a controlled clock and compares exactly. Hits closer together than `ttl / 64` share a bucket and expire together, so
`X-RateLimit-Remaining` and the times MAY differ from the original by up to `ttl / 64`; the blocked state and the
requests that are rejected as the limit is exceeded stay the same.

#### Scenario: Replay of recorded storage calls
- **WHEN** the unit test replays the calls recorded from `ThrottlerStorageService.increment` (a block with retries and
  the request after it, hits expiring across a window boundary, block durations longer and shorter than the ttl, a
  block duration of 0, interleaved keys), all with hits more than `ttl / 64` apart
- **THEN** every returned record equals the recorded one

#### Scenario: Replay of closely spaced hits
- **WHEN** the unit test replays recorded calls whose hits are closer together than `ttl / 64`
- **THEN** the blocked state equals the recorded one and the times are within `ttl / 64` (rounded up to seconds, plus
  one second)

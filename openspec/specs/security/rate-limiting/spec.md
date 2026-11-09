# security/rate-limiting Specification

## Purpose
How `ThrottlerGuard` limits requests, compatible with the original throttler package: the informational headers
(names, values, suffixes for named throttlers, the 429 case, turning them off), block duration, and the bounded
storage of counters and blocked keys.

## Requirements

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

### Requirement: Header names carry the throttler name except for the default throttler
The `<suffix>` of every throttler header SHALL be empty for a throttler named `default` and `-<name>` otherwise.

#### Scenario: Named throttlers
- **WHEN** the throttlers are `short` (`ttl: 1_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 100`) and a
  client sends one request
- **THEN** the response carries `X-RateLimit-Limit-short: 3`, `X-RateLimit-Remaining-short: 2`,
  `X-RateLimit-Limit-long: 100` and `X-RateLimit-Remaining-long: 99`, each with its `X-RateLimit-Reset-<name>`

### Requirement: Skipped requests carry no throttler headers
A throttler skipped for a request (see "Throttlers are skipped one by one") SHALL NOT count the request and MUST NOT
set its `X-RateLimit-*<suffix>` or `Retry-After<suffix>` header. Throttlers that are not skipped SHALL count the
request and set their headers as for any other request. A request skipped for every configured throttler carries no
`X-RateLimit-*` or `Retry-After` header.

#### Scenario: Skipped route
- **WHEN** the only throttler is `ThrottlerOptions::new(60_000, 10)` and a route declares
  `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** the response carries no `X-RateLimit-*` or `Retry-After` header

#### Scenario: Bare skip with a named throttler next to the default one
- **WHEN** the throttlers are `default` (`ttl: 60_000`, `limit: 10`) and `short` (`ttl: 1_000`, `limit: 3`) and a
  route declares `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** the response carries `X-RateLimit-Limit-short: 3` and `X-RateLimit-Remaining-short: 2` and no
  `X-RateLimit-Limit` header without a suffix

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
memory that does not depend on its limit or on the number of requests it receives (at most 64 hit buckets). A key that
goes over its limit MUST move to a separate compact table of blocked keys (its hits are dropped; only the end of the
block and of the reporting window are kept) with its own cap (`with_max_blocked`, default the value of `max_entries`),
so blocked keys never displace counters. When the counters table is full the counters that would become idle soonest
MUST be evicted; when the blocked table is full the block that started first MUST be dropped (first in, first out, whatever the block lengths). A key whose hits have
expired, whose reporting window has ended and that is not blocked, and a block that has ended with its window, MUST be
removed. A block duration or `ttl` too large to represent MUST saturate to a far instant, never to the present.

#### Scenario: Round-robin keys cannot exceed the limit
- **WHEN** 2,000 distinct keys each send requests in turn with a limit of 100 per hour
- **THEN** every key gets exactly 100 requests through and the rest are rejected

#### Scenario: Blocked key survives a flood
- **WHEN** a key is blocked and 5,000 other keys then record hits on a storage built with `with_max_entries(100)`
- **THEN** the storage holds at most 100 keys and the first key is still blocked

#### Scenario: Pinning the blocked table does not disable other counters
- **WHEN** 1,000 keys are blocked on a storage built with `with_max_entries(100)`, and then two other keys alternate
  requests with a limit of 10 and a victim key has a limit of 3
- **THEN** each of the two keys gets exactly 10 requests through in the window and the victim gets exactly 3

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

### Requirement: The guard tracks clients by their normalized IP
`ThrottlerGuard` SHALL build the client part of every counter key, for HTTP requests and gateway messages alike, as
`normalize_ip(&req.ip(), prefix)`, where `prefix` is the storage's IPv6 subnet prefix. The rest of the key (throttler
name, controller and handler) SHALL stay as it is.

#### Scenario: Two addresses of one /64
- **WHEN** a route is limited to 1 request per minute and requests come from `2001:db8:1:2::a` and then from
  `2001:db8:1:2:ffff:ffff:ffff:ffff`
- **THEN** the second request is rejected with status 429

#### Scenario: Addresses of different /64 prefixes
- **WHEN** a route is limited to 1 request per minute and requests come from `2001:db8:1:2::a` and then from
  `2001:db8:1:3::a`
- **THEN** both requests are accepted

#### Scenario: IPv4-mapped address shares the IPv4 counter
- **WHEN** a route is limited to 1 request per minute and requests come from `1.2.3.4` and then from
  `::ffff:1.2.3.4`
- **THEN** the second request is rejected with status 429

#### Scenario: IPv4 clients are unchanged
- **WHEN** a route is limited to 1 request per minute and requests come from `1.2.3.4` and then from `1.2.3.5`
- **THEN** both requests are accepted

### Requirement: normalize_ip keeps IPv4 addresses and values that are not addresses
`normalize_ip` SHALL return the input unchanged when it is empty. It SHALL cut the input at its first `%` (zone
identifier) and, when the rest is a valid IPv4 address, return that rest. When the rest is not a valid IP address, it
SHALL return the input unchanged, zone included.

#### Scenario: Zone identifier
- **WHEN** `normalize_ip` receives `fe80::1%eth0` with prefix 64, and `1.2.3.4%x`
- **THEN** it returns `fe80::/64` and `1.2.3.4`

#### Scenario: Value that is not an IP address
- **WHEN** `normalize_ip` receives `unknown`, `1.2.3.4:80` or `[::1]`
- **THEN** it returns the input unchanged

### Requirement: normalize_ip reduces IPv6 addresses to their prefix
When the input without its zone is a valid IPv6 address, `normalize_ip` SHALL return `::1` for the loopback address
whatever the prefix, the dotted IPv4 form for an IPv4-mapped address (`::ffff:0:0/96`), and for any other address the
address with every bit after the prefix cleared, in lowercase compressed form (the first longest run of two or more
zero groups replaced by `::`), followed by `/` and the prefix.

#### Scenario: Default prefix
- **WHEN** `normalize_ip` receives `2001:db8:1:2:ffff:ffff:ffff:ffff` with prefix 64
- **THEN** it returns `2001:db8:1:2::/64`

#### Scenario: Loopback and mapped addresses
- **WHEN** `normalize_ip` receives `::1` with prefix 0, and `::ffff:1.2.3.4` with prefix 64
- **THEN** it returns `::1` and `1.2.3.4`

#### Scenario: Full prefix
- **WHEN** `normalize_ip` receives `2001:db8:1:2::a` with prefix 128
- **THEN** it returns `2001:db8:1:2::a/128`

### Requirement: The IPv6 subnet prefix is configurable
`ThrottlerStorage` SHALL use `DEFAULT_IPV6_SUBNET_PREFIX` (64) as the IPv6 subnet prefix unless
`ThrottlerStorage::with_ipv6_subnet_prefix(prefix)` sets another one. Values above 128 SHALL be treated as 128. The
prefix SHALL apply to every configured throttler. A prefix of 128 SHALL keep each IPv6 address in its own counter.

#### Scenario: Opting out of grouping
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(128)`, a route is limited to 1 request per minute, and
  requests come from `2001:db8:1:2::a` and then from `2001:db8:1:2::b`
- **THEN** both requests are accepted

#### Scenario: Larger allocation
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(48)`, a route is limited to 1 request per minute, and
  requests come from `2001:db8:1:2::a` and then from `2001:db8:1:3::a`
- **THEN** the second request is rejected with status 429

#### Scenario: Out-of-range prefix
- **WHEN** the storage is built with `with_ipv6_subnet_prefix(200)`
- **THEN** it behaves as with `with_ipv6_subnet_prefix(128)`

#### Scenario: Factory-built storage
- **WHEN** `ThrottlerModule::for_root_async` returns a `ThrottlerStorage` built with `with_ipv6_subnet_prefix(56)`
- **THEN** the guard groups IPv6 clients by /56

### Requirement: The tracker follows the trust proxy setting
The client IP that `ThrottlerGuard` normalizes SHALL be `req.ip()`: the socket address by default, and the client
entry of `X-Forwarded-For` resolved by the `trust proxy` setting when one is configured. Normalization SHALL NOT
change `req.ip()`, `req.ips()` or the value of `#[ip]` parameters.

#### Scenario: IPv6 clients behind a trusted proxy
- **WHEN** `trust proxy` trusts the loopback range, a route is limited to 1 request per minute, and two requests from
  `127.0.0.1` carry `X-Forwarded-For: 2001:db8:1:2::a` and `X-Forwarded-For: 2001:db8:1:2::b`
- **THEN** the second request is rejected with status 429

#### Scenario: IPv4-mapped entry behind a trusted proxy
- **WHEN** `trust proxy` trusts the loopback range, a route is limited to 1 request per minute, and two requests from
  `127.0.0.1` carry `X-Forwarded-For: 1.2.3.4` and `X-Forwarded-For: ::ffff:1.2.3.4`
- **THEN** the second request is rejected with status 429

#### Scenario: Forwarded header without trust proxy
- **WHEN** no `trust proxy` setting is configured and requests from `127.0.0.1` carry different IPv6
  `X-Forwarded-For` values
- **THEN** they all share the counter of `127.0.0.1`

#### Scenario: `#[ip]` keeps the full address
- **WHEN** `trust proxy` trusts the loopback range and a request from `127.0.0.1` carries
  `X-Forwarded-For: 2001:db8:1:2::a` to a route that returns its `#[ip]` parameter
- **THEN** the response contains `2001:db8:1:2::a`

### Requirement: normalize_ip is public
`normalize_ip(ip: &str, ipv6_subnet_prefix: u8) -> String` and `DEFAULT_IPV6_SUBNET_PREFIX` SHALL be public under
`corvis::modules::throttler` and re-exported at the crate root next to `ThrottlerStorage`, so that code calling
`ThrottlerStorage::record_hit` directly can build the same tracker as the guard.

#### Scenario: Custom guard
- **WHEN** application code calls `corvis::normalize_ip("2001:db8:1:2::a", corvis::DEFAULT_IPV6_SUBNET_PREFIX)`
- **THEN** it receives `2001:db8:1:2::/64`

### Requirement: IP normalization matches @nestjs/throttler
For every address and prefix recorded in the differential fixture, `normalize_ip` SHALL return exactly the string that
`normalizeIp` of `@nestjs/throttler` 6.7.1 returns; for prefixes above 128 it SHALL return the recorded output for
128. The fixture SHALL cover IPv4, IPv6 in several spellings, IPv4-mapped addresses, loopback, zone identifiers and
values that are not addresses, at prefixes from 0 to 128.

#### Scenario: Replay of the recorded normalizations
- **WHEN** the Rust test replays `normalize-ip.json`, produced by running the original `normalizeIp` over the corpus
- **THEN** every output string is equal

### Requirement: Throttlers are skipped one by one
For each configured throttler, `ThrottlerGuard` SHALL decide whether to skip it from the boolean metadata stored
under `skip_throttle_key(name)`, read from the handler and then from the controller, the first value found winning.
A throttler with no value, or with `false`, MUST NOT be skipped. Skipping one throttler MUST NOT stop the guard from
checking the others.

#### Scenario: Skip by name
- **WHEN** the throttlers are `short` and `long` and a route declares
  `#[apply_decorators(skip_throttle(["short"]))]`
- **THEN** the response carries the `X-RateLimit-*-long` headers and no `X-RateLimit-*-short` header, and only `long`
  counts the request

#### Scenario: Gateway handler skips by name
- **WHEN** a global `ThrottlerGuard` runs on a gateway handler that declares
  `#[set_metadata(skip_throttle_key("short"), true)]` and the throttlers are `short` and `long`
- **THEN** the message is counted by `long` only

### Requirement: A bare skip is the default throttler's entry
For the throttler named `default`, a `SKIP_THROTTLE` value on a target (handler or controller) SHALL count as that
target's `skip_throttle_key("default")` value when the target has no such entry. `SKIP_THROTTLE` MUST NOT affect
throttlers with other names.

#### Scenario: Bare skip with named throttlers only
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 5`), a route
  declares `#[set_metadata(SKIP_THROTTLE, true)]`, and a client sends four requests to it
- **THEN** the first three answer `200` with `X-RateLimit-*-short` and `X-RateLimit-*-long` headers and the fourth
  answers `429` with `Retry-After-short`

#### Scenario: Handler undoes the controller's skip
- **WHEN** a controller declares `#[set_metadata(SKIP_THROTTLE, true)]`, one of its handlers declares
  `#[set_metadata(SKIP_THROTTLE, false)]`, and the only throttler is `default`
- **THEN** that handler's responses carry `X-RateLimit-Limit` and the controller's other handlers carry no throttler
  header

#### Scenario: Controller and handler skip different throttlers
- **WHEN** a controller declares `#[apply_decorators(skip_throttle(["short"]))]`, a handler of it declares
  `#[set_metadata(SKIP_THROTTLE, true)]`, and the throttlers are `default` and `short`
- **THEN** that handler's responses carry no throttler header (each name is resolved on its own)

### Requirement: Per-name skip helpers
The crate SHALL export, from `corvis::modules::throttler` and the prelude, `skip_throttle_key(name) -> String`
returning `"THROTTLER:SKIP"` followed by the name, and `skip_throttle(names) -> CustomDecorator` that sets
`skip_throttle_key(name)` to `true` for each name. `#[set_metadata(key, value)]` MUST accept a `String` key on routes,
controllers and gateway handlers.

#### Scenario: Key spelling
- **WHEN** `skip_throttle_key("short")` is called
- **THEN** it returns `THROTTLER:SKIPshort`

#### Scenario: String key on a route
- **WHEN** a route declares `#[set_metadata(skip_throttle_key("short"), true)]`
- **THEN** the application compiles and `short` is skipped for that route

### Requirement: A bare skip that skips nothing is reported
When `ThrottlerGuard` sees `SKIP_THROTTLE` set to `true` for a route and no configured throttler is named `default`,
it SHALL log one warning for that route (identified by controller and handler), naming the route, explaining that a
bare skip only affects the `default` throttler, and pointing to `skip_throttle([...])`. It MUST NOT log it again for
the same route, and it MUST still apply the other throttlers.

#### Scenario: Warning once
- **WHEN** the throttlers are `short` and `long` and a client sends three requests to a route with
  `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** one warning naming the route is logged and the three requests are counted by both throttlers

### Requirement: Skips match @nestjs/throttler
For the same throttlers, skip metadata and request sequence, the requests Corvis counts, rejects and answers with
throttler headers SHALL be the same as with `@nestjs/throttler`, where `#[set_metadata(SKIP_THROTTLE, true)]`
corresponds to `@SkipThrottle()`, `skip_throttle([names])` to `@SkipThrottle({ name: true, ... })`, and a `false`
value to `@SkipThrottle({ name: false })`. This is verified by the differential fixture of #120 and #135.

#### Scenario: Replay of the recorded skip cases
- **WHEN** the Rust test replays the skip cases recorded from `@nestjs/throttler` (a bare skip with named throttlers
  only, a bare skip next to `default`, a skip by name, a controller skip undone by its handler, controller and handler
  skipping different names)
- **THEN** every response has the same status and the same throttler header names and values as recorded (time-based
  values within one second)

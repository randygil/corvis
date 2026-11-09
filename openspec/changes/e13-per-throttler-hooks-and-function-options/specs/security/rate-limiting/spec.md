# Spec Delta

## Purpose

Per-throttler tracker and key generator hooks and function-valued `limit`, `ttl` and `blockDuration`, compatible with
the entries of `@nestjs/throttler`'s throttler list.

## ADDED Requirements

### Requirement: Tracker and key generator hooks per throttler
`ThrottlerStorage` SHALL offer `with_tracker_for(name, f)` and `with_key_generator_for(name, f)`, with the signatures
of `with_tracker` and `with_key_generator`, that apply to the throttler called `name` only. Calling one with a name
that matches no configured throttler MUST panic with a message naming it; calling one twice for the same name SHALL
keep the last function.

#### Scenario: One throttler counts per account
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 1`) and `long` (`ttl: 60_000`, `limit: 3`), the storage
  has `with_tracker_for("short", f)` with `f` returning the `x-account` header, and one IP sends two requests with
  `x-account: 1`
- **THEN** the second answers 429 with `Retry-After-short`, and the first carried `X-RateLimit-Remaining-long: 2`

#### Scenario: Other throttlers keep the built-in tracker
- **WHEN** a fresh application with the same storage receives one request with `x-account: 2` and one with
  `x-account: 3` from one IP
- **THEN** both answer 200 and the second carries `X-RateLimit-Remaining-long: 1`

#### Scenario: Unknown throttler name
- **WHEN** `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_tracker_for("login", f)` runs
- **THEN** it panics with a message containing `login`

### Requirement: Function-valued limit, ttl and block duration
`ThrottlerStorage` SHALL offer `with_limit_fn(name, f)`, `with_ttl_fn(name, f)` and `with_block_duration_fn(name, f)`,
where `f` is an async function of the `ExecutionContext` returning `Result<u32, HttpException>` (limit) or
`Result<u64, HttpException>` (milliseconds). The guard SHALL call a function only when no route override of that
option applies, and its value SHALL take precedence over the static value. Unknown names MUST panic.

#### Scenario: Limit from the request
- **WHEN** the only throttler is `default` (`ttl: 60_000`) with a limit function returning 3 when the `x-plan` header
  is `pro` and 1 otherwise, and a client sends four requests with `x-plan: pro` to one route
- **THEN** the first three answer 200 with `X-RateLimit-Limit: 3` and the fourth answers 429

#### Scenario: Route override wins over the function
- **WHEN** the same storage serves a route with `#[apply_decorators(throttle("default").limit(5))]`
- **THEN** its responses carry `X-RateLimit-Limit: 5` and the limit function is not called

#### Scenario: Block duration function
- **WHEN** the only throttler is `default` (`ttl: 60_000`, `limit: 1`) with a block duration function returning
  `20_000`, and a client sends two requests
- **THEN** the second answers 429 with `Retry-After: 20`

### Requirement: Errors of value functions are returned unchanged
When a limit, ttl or block duration function returns `Err`, the guard MUST return that error unchanged and MUST NOT
count a hit for that throttler or the throttlers after it.

#### Scenario: Internal error
- **WHEN** the limit function returns `Err(HttpException::from_anyhow(anyhow!("plan lookup failed")))`
- **THEN** the request answers 500 with `{ "statusCode": 500, "message": "Internal server error" }`, and the next
  request for which the function returns `Ok(1)` answers 200

#### Scenario: HTTP error
- **WHEN** the limit function returns `Err(ForbiddenException::new("no plan"))`
- **THEN** the request answers 403

### Requirement: Per-throttler hooks and value functions match @nestjs/throttler
For the same throttlers, per-throttler `getTracker` / `generateKey`, function-valued `limit` / `ttl` /
`blockDuration`, overrides and requests, each response's status and throttler headers SHALL be the same as with
`@nestjs/throttler` 6.7.1, where `with_tracker_for` / `with_key_generator_for` correspond to a throttler entry's
`getTracker` / `generateKey` and the `_fn` builders to its function-valued options. This is verified by the
differential fixture.

#### Scenario: Replay of the recorded cases
- **WHEN** the Rust test replays the cases recorded from `@nestjs/throttler` (a per-throttler tracker under a module
  tracker and under a route tracker, a per-throttler key generator under a module key generator and under a route key
  generator, a limit function with and without a route override, a block duration function, a ttl function checked
  after a static throttler, a limit function that throws an `Error` and one that throws a `ForbiddenException`)
- **THEN** every response has the same status and the same throttler header names and values as recorded
  (time-based values within one second)

## MODIFIED Requirements

### Requirement: The block duration defaults to the ttl and can be configured
The effective block duration of a throttler SHALL be its block duration override for the route (resolved as in
"Overrides are resolved per throttler and per option") when present, else the value of its block duration function
(`with_block_duration_fn`), else the value set with `ThrottlerStorage::with_block_duration(name, ms)` for that
throttler, else the throttler's effective `ttl` (after its ttl override or ttl function). `with_block_duration` with a
name that matches no configured throttler MUST panic with a message naming it.

#### Scenario: Default follows the ttl override
- **WHEN** a route declares `#[set_metadata(THROTTLE_TTL, 5_000)]` and `#[set_metadata(THROTTLE_LIMIT, 1)]`, the only
  throttler is `default`, and a client sends two requests in a row
- **THEN** the second response is 429 with `Retry-After: 5`

#### Scenario: Block longer than the ttl
- **WHEN** the storage is `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("default",
  300_000)`, a client's third request at 2 s is rejected, and the client sends another request at 63 s
- **THEN** the request at 63 s answers 429 with `Retry-After: 239`

#### Scenario: Metadata overrides the storage value
- **WHEN** the storage sets a block duration of 300,000 ms for `default` and a route declares
  `#[set_metadata(THROTTLE_BLOCK_DURATION, 10_000)]`
- **THEN** the first rejected request on that route carries `Retry-After: 10`

#### Scenario: Block duration override for one throttler
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 5`), a route
  declares `#[apply_decorators(throttle("short").limit(1).block_duration(5_000))]`, and a client sends two requests
- **THEN** the second answers 429 with `Retry-After-short: 5`

#### Scenario: The function wins over the static value
- **WHEN** the storage sets `with_block_duration("default", 300_000)` and `with_block_duration_fn("default", f)` with
  `f` returning `20_000`, in either order, and a client goes over the limit
- **THEN** the rejected request carries `Retry-After: 20`

#### Scenario: Default follows the ttl function
- **WHEN** the only throttler is `default` (`limit: 1`) with a ttl function returning `10_000` and no block duration,
  and a client sends two requests in a row
- **THEN** the second response is 429 with `Retry-After: 10`

#### Scenario: Unknown throttler name
- **WHEN** `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("login", 1_000)` runs
- **THEN** it panics with a message containing `login`

### Requirement: Throttlers are checked in ascending ttl order
`ThrottlerGuard` SHALL check the configured throttlers that have a static ttl in ascending `ttl` order, keeping the
configured order between throttlers with the same `ttl`, then the throttlers that have a ttl function
(`with_ttl_fn`), in configured order. It MUST stop at the first throttler that rejects the request, so later
throttlers do not count it.

#### Scenario: Configured from longest to shortest
- **WHEN** the throttlers are configured as `long` (`ttl: 60_000`, `limit: 100`) then `short` (`ttl: 1_000`,
  `limit: 1`), and a client sends a request, a second one 100 ms later and a third one 1.2 seconds after the first
- **THEN** the second response is 429 with `Retry-After-short` and no `X-RateLimit-*-long` header, and the third
  response carries `X-RateLimit-Remaining-long: 98`

#### Scenario: A ttl function is checked last
- **WHEN** the throttlers are `a` (`ttl: 60_000`, `limit: 2`) and `b` (static `ttl: 1_000`, `limit: 1`) with a ttl
  function for `b` returning `10_000`, and a client sends three requests in a row
- **THEN** the second answers 429 with `Retry-After-b: 10`, and the third answers 429 with `Retry-After-a: 60` and no
  `Retry-After-b` header

### Requirement: Tracker and key generator hooks at module level
`ThrottlerStorage` SHALL offer `with_tracker(f)` (an async function of the `ExecutionContext` returning
`Result<String, HttpException>`) and `with_key_generator(f)` (a function of the context, the tracker and the throttler
name returning the key). When set, they SHALL replace the built-in tracker, including IP normalization, and the
built-in key wherever no route override and no per-throttler hook applies. The guard MUST return a tracker's `Err`
unchanged.

#### Scenario: Tracker from a request header
- **WHEN** the only throttler is `default` (`limit: 1`), the storage has a tracker returning the `x-client` request
  header, and one IP sends requests with `x-client` `a`, `a` and `b`
- **THEN** the responses are 200, 429 and 200

#### Scenario: Key shared between routes
- **WHEN** the only throttler is `default` (`limit: 1`) and the storage has a key generator returning the throttler
  name and the tracker, and a client sends one request to each of two routes
- **THEN** the second request answers 429

#### Scenario: Tracker error
- **WHEN** the tracker returns `Err(HttpException::new("no client id", 400))`
- **THEN** the request answers 400 and no hit is counted

#### Scenario: A per-throttler hook wins over the module hook
- **WHEN** the only throttler is `default` (`limit: 1`), the storage has a module tracker returning the `x-client`
  header and `with_tracker_for("default", f)` with `f` returning `"all"`, and requests come with `x-client` `a` and then
  `b`
- **THEN** the second request answers 429

### Requirement: Tracker and key generator overrides by registered name
`ThrottlerStorage` SHALL offer `with_named_tracker(id, f)` and `with_named_key_generator(id, f)`, with the signatures
of the module-level hooks, and `ThrottleOverride::tracker(id)` / `key_generator(id)` SHALL select them for one
throttler on a route or controller. For each throttler the guard SHALL use the route's override, else the throttler's
hook (`with_tracker_for` / `with_key_generator_for`), else the module hook, else the built-in. Registering the same id
twice MUST panic with a message naming it.

#### Scenario: Route tracker over the module tracker
- **WHEN** the storage has a tracker returning the `x-client` header and a named tracker `fixed` returning `"all"`, a
  route declares `#[apply_decorators(throttle("default").tracker("fixed"))]`, the limit is 1, and requests come with
  `x-client` `a` and then `b`
- **THEN** the second request answers 429

#### Scenario: Route tracker over the throttler's tracker
- **WHEN** the throttler `short` (`limit: 1`) has `with_tracker_for("short", f)` with `f` returning the `x-account`
  header, a named tracker `fixed` returns `"all"`, a route declares
  `#[apply_decorators(throttle("short").tracker("fixed"))]`, and requests come with `x-account` `3` and then `4`
- **THEN** the second request answers 429 with `Retry-After-short`

#### Scenario: Route key generator
- **WHEN** the storage has a key generator shared between routes, a named key generator `own` that prefixes the
  tracker with `own:`, a client has used up its limit on one route, and then sends a request to a route with
  `#[apply_decorators(throttle("default").key_generator("own"))]`
- **THEN** that request answers 200

#### Scenario: Unregistered name
- **WHEN** a route declares `#[apply_decorators(throttle("default").tracker("missing"))]` and no tracker `missing`
  is registered
- **THEN** the request answers 500 and the logged error names `missing`

#### Scenario: Duplicate registration
- **WHEN** `with_named_tracker("fixed", ..)` is called twice on one storage
- **THEN** it panics with a message containing `fixed`

### Requirement: Counters stay separate per throttler
Two throttlers MUST NOT share a counter or a block, whatever key the key generator returns for them. Within one
throttler, the built-in key, the module key generator, the throttler's key generator and each registered key generator
MUST NOT share counters either, whatever string a generator returns; only routes that use the same generator share
counters.

#### Scenario: A crafted tracker does not reach another route's built-in counter
- **WHEN** a tracker returns the `x-email` header (else a fixed value), a registered key generator returns the tracker
  as is, a client sends four requests to a route that uses that generator with `x-email` set to the built-in key of
  `/sensitive` for the fixed value, and another client then requests `/sensitive` without the header
- **THEN** that request answers 200

#### Scenario: Key generator that ignores the throttler name
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 100`) and `long` (`ttl: 60_000`, `limit: 2`), the key
  generator returns the tracker alone, and a client sends four requests
- **THEN** every response carries `X-RateLimit-Reset-short: 30`, the third and fourth answer 429 with
  `Retry-After-long` and carry `X-RateLimit-Remaining-short` (96 on the fourth), and no response carries
  `Retry-After-short`

#### Scenario: The throttler's generator and a registered generator do not meet
- **WHEN** the only throttler is `default` (`limit: 1`), a registered key generator `same` and
  `with_key_generator_for("default", ..)` both return the tracker as is, a client uses up its limit on a route with
  `#[apply_decorators(throttle("default").key_generator("same"))]`, and then requests a route without overrides
- **THEN** that request answers 200

### Requirement: The guard tracks clients by their normalized IP
When no tracker hook applies to a throttler (see "Tracker and key generator hooks at module level", "Tracker and key
generator hooks per throttler" and "Tracker and key generator overrides by registered name"), `ThrottlerGuard` SHALL
use `normalize_ip(&req.ip(), prefix)` as the tracker, for HTTP requests and gateway messages alike, where `prefix` is
the storage's IPv6 subnet prefix. When no key generator hook applies, the counter key SHALL combine the throttler name,
the controller, the handler and the tracker.

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

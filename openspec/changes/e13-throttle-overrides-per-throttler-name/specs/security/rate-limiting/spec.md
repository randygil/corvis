# Spec Delta

## Purpose

Per-route and per-controller overrides of one named throttler (limit, ttl, block duration, tracker, key generator),
compatible with `@nestjs/throttler`'s `@Throttle({ name: { ... } })`, plus module-level tracker and key generator
hooks.

## MODIFIED Requirements

### Requirement: The block duration defaults to the ttl and can be configured
The effective block duration of a throttler SHALL be its block duration override for the route (resolved as in
"Overrides are resolved per throttler and per option") when present, else the value set with
`ThrottlerStorage::with_block_duration(name, ms)` for that throttler, else the throttler's effective `ttl` (after its
ttl override). `with_block_duration` with a name that matches no configured throttler MUST panic with a message naming
it.

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

#### Scenario: Unknown throttler name
- **WHEN** `ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 2)]).with_block_duration("login", 1_000)` runs
- **THEN** it panics with a message containing `login`

### Requirement: The guard tracks clients by their normalized IP
When no tracker hook applies to a throttler (see "Tracker and key generator hooks at module level" and "Tracker and
key generator overrides by registered name"), `ThrottlerGuard` SHALL use `normalize_ip(&req.ip(), prefix)` as the
tracker, for HTTP requests and gateway messages alike, where `prefix` is the storage's IPv6 subnet prefix. When no key
generator hook applies, the counter key SHALL combine the throttler name, the controller, the handler and the tracker.

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

## ADDED Requirements

### Requirement: Overrides are resolved per throttler and per option
For each configured throttler that is not skipped, `ThrottlerGuard` SHALL resolve each option (limit, ttl, block
duration, tracker, key generator) on its own from the metadata stored under `throttle_key(OPTION, name)`, reading the
handler and then the controller; the first target with a value that is not JSON `null` MUST win, and an option with no
value on either target MUST fall back to the throttler's configuration. An override for one throttler MUST NOT change
any other throttler.

#### Scenario: Override one throttler
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 5`), a route
  declares `#[apply_decorators(throttle("short").limit(1))]`, and a client sends two requests
- **THEN** the first response carries `X-RateLimit-Limit-short: 1` and `X-RateLimit-Limit-long: 5`, and the second
  answers 429 with `Retry-After-short: 30`

#### Scenario: Two throttlers overridden on one route
- **WHEN** the same throttlers apply and a route declares
  `#[apply_decorators(throttle("short").limit(2).ttl(10_000), throttle("long").limit(1))]`
- **THEN** the first response carries `X-RateLimit-Limit-short: 2`, `X-RateLimit-Reset-short: 10` and
  `X-RateLimit-Limit-long: 1`, and the second answers 429 with `Retry-After-long: 60`

#### Scenario: Partial override on the handler keeps the controller's other options
- **WHEN** a controller declares `#[apply_decorators(throttle("short").limit(2).ttl(20_000))]` and one of its handlers
  declares `#[apply_decorators(throttle("short").limit(1))]`
- **THEN** that handler's first response carries `X-RateLimit-Limit-short: 1` and `X-RateLimit-Reset-short: 20`, and
  the controller's other handlers carry `X-RateLimit-Limit-short: 2` and `X-RateLimit-Reset-short: 20`

#### Scenario: Null counts as absent
- **WHEN** a controller declares `#[apply_decorators(throttle("short").limit(2))]` and a handler declares
  `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), Option::<u32>::None)]`
- **THEN** that handler's responses carry `X-RateLimit-Limit-short: 2`

### Requirement: Bare override keys are the default throttler's entry
For the throttler named `default`, a target's `THROTTLE_LIMIT`, `THROTTLE_TTL` or `THROTTLE_BLOCK_DURATION` value
SHALL count as that target's `throttle_key(OPTION, "default")` value when the target has no per-name entry for that
option. Bare keys MUST NOT affect throttlers with other names.

#### Scenario: Bare limit next to a named throttler
- **WHEN** the throttlers are `default` (`ttl: 60_000`, `limit: 3`) and `short` (`ttl: 30_000`, `limit: 3`) and a
  route declares `#[set_metadata(THROTTLE_LIMIT, 1)]`
- **THEN** the first response carries `X-RateLimit-Limit: 1` and `X-RateLimit-Limit-short: 3`, and the second answers
  429 with `Retry-After: 60`

#### Scenario: Per-name entry wins over the bare key on the same target
- **WHEN** the only throttler is `default` and a route declares `#[set_metadata(THROTTLE_LIMIT, 1)]` and
  `#[set_metadata(throttle_key(THROTTLE_LIMIT, "default"), 2)]`
- **THEN** the response carries `X-RateLimit-Limit: 2`

#### Scenario: Bare key with named throttlers only
- **WHEN** the throttlers are `short` and `long` and a route declares `#[set_metadata(THROTTLE_LIMIT, 1)]`
- **THEN** the route is limited by the configured `short` and `long` limits

### Requirement: Override helpers
The crate SHALL export, from `corvis::modules::throttler` and the prelude, the constants `THROTTLE_TRACKER` and
`THROTTLE_KEY_GENERATOR`, `throttle_key(option, name)` returning `option` followed by `name`, and `throttle(name)`, a
builder (`limit`, `ttl`, `block_duration`, `tracker`, `key_generator`) that converts into a `CustomDecorator` holding
one `throttle_key(OPTION, name)` entry per option that was set.

#### Scenario: Key spelling
- **WHEN** `throttle_key(THROTTLE_LIMIT, "short")` and `throttle_key(THROTTLE_KEY_GENERATOR, "short")` are called
- **THEN** they return `THROTTLER:LIMITshort` and `THROTTLER:KEY_GENERATORshort`

#### Scenario: Only the options set are written
- **WHEN** `throttle("short").limit(1)` is converted into a `CustomDecorator`
- **THEN** its only entry is `("THROTTLER:LIMITshort", 1)`

#### Scenario: Gateway handler override
- **WHEN** a global `ThrottlerGuard` with throttlers `short` (`limit: 3`) and `long` runs on a gateway handler that
  declares `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), 1)]`
- **THEN** the second message from the same client gets an exception frame

### Requirement: Invalid override values fail closed
When the resolved override of an option is present but invalid (a limit outside `u32`, a ttl or block duration
outside `u64`, a tracker or key generator that is not a string naming a registered function), `ThrottlerGuard` SHALL
fail the request with the standard 500 body and log an error naming the option, the throttler and the value. It MUST
NOT fall back to another value.

#### Scenario: Text instead of a number
- **WHEN** the throttlers are `short` and `long` and a route declares
  `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), "abc")]`
- **THEN** the request answers 500 and an error containing `limit`, `short` and `abc` is logged

#### Scenario: Negative limit
- **WHEN** a route declares `#[set_metadata(THROTTLE_LIMIT, -1)]` and the only throttler is `default`
- **THEN** the request answers 500

### Requirement: Overrides for unknown throttlers are reported
When a route's handler or controller has an override key whose throttler name matches no configured throttler, or a
bare override key while no throttler is named `default`, `ThrottlerGuard` SHALL log one warning for that route
(identified by controller and handler) naming the route and the unknown names. It MUST NOT log it again for the same
route and MUST still apply the configured throttlers.

#### Scenario: Misspelled name
- **WHEN** the throttlers are `short` and `long`, a route declares `#[apply_decorators(throttle("shrot").limit(1))]`,
  and a client sends three requests
- **THEN** one warning naming the route and `shrot` is logged, and the three responses carry
  `X-RateLimit-Limit-short: 3`

#### Scenario: Bare key without a default throttler
- **WHEN** the throttlers are `short` and `long` and a client sends two requests to a route with
  `#[set_metadata(THROTTLE_LIMIT, 1)]`
- **THEN** one warning naming the route is logged

### Requirement: Tracker and key generator hooks at module level
`ThrottlerStorage` SHALL offer `with_tracker(f)` (an async function of the `ExecutionContext` returning
`Result<String, HttpException>`) and `with_key_generator(f)` (a function of the context, the tracker and the throttler
name returning the key). When set, they SHALL replace the built-in tracker, including IP normalization, and the
built-in key wherever no route override applies. The guard MUST return a tracker's `Err` unchanged.

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

### Requirement: Tracker and key generator overrides by registered name
`ThrottlerStorage` SHALL offer `with_named_tracker(id, f)` and `with_named_key_generator(id, f)`, with the signatures
of the module-level hooks, and `ThrottleOverride::tracker(id)` / `key_generator(id)` SHALL select them for one
throttler on a route or controller. For each throttler the guard SHALL use the route's override, else the module hook,
else the built-in. Registering the same id twice MUST panic with a message naming it.

#### Scenario: Route tracker over the module tracker
- **WHEN** the storage has a tracker returning the `x-client` header and a named tracker `fixed` returning `"all"`, a
  route declares `#[apply_decorators(throttle("default").tracker("fixed"))]`, the limit is 1, and requests come with
  `x-client` `a` and then `b`
- **THEN** the second request answers 429

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
Two throttlers MUST NOT share a counter or a block, whatever key the key generator returns for them.

#### Scenario: Key generator that ignores the throttler name
- **WHEN** the throttlers are `short` (`ttl: 30_000`, `limit: 100`) and `long` (`ttl: 60_000`, `limit: 2`), the key
  generator returns the tracker alone, and a client sends four requests
- **THEN** every response carries `X-RateLimit-Reset-short: 30`, the third and fourth answer 429 with
  `Retry-After-long` and carry `X-RateLimit-Remaining-short` (96 on the fourth), and no response carries
  `Retry-After-short`

### Requirement: Overrides match @nestjs/throttler
For the same throttlers, options, overrides and requests, each response's status and throttler headers SHALL be the
same as with `@nestjs/throttler` 6.7.1, where `throttle("name")` corresponds to `@Throttle({ name: { ... } })` and the
storage hooks to `getTracker` and `generateKey`, except for a key generator that returns the same key for two
throttlers. This is verified by the differential fixture.

#### Scenario: Replay of the recorded override cases
- **WHEN** the Rust test replays the override cases recorded from `@nestjs/throttler` (one throttler overridden, two
  overridden, a partial handler override under a controller override, a block duration override, an unknown name, a
  `default` override with and without a `default` throttler, an invalid value, module and route trackers, module and
  route key generators)
- **THEN** every response has the same status and the same throttler header names and values as recorded
  (time-based values within one second)

#### Scenario: The documented divergence differs as documented
- **WHEN** the replay reaches the recorded case whose key generator ignores the throttler name
- **THEN** the recorded values differ from Corvis's and Corvis's match "Counters stay separate per throttler"

# Spec Delta

## Purpose

Which throttlers `ThrottlerGuard` skips for a request, compatible with `@nestjs/throttler`: a bare skip affects only
the throttler named `default`, other throttlers are skipped by name, and the handler's value wins over the
controller's for each name. Requires the requirements added by `e13-rate-limit-headers-in-the-throttler` (#120) and
`e13-throttler-block-duration-like-the-original` (#135).

## MODIFIED Requirements

### Requirement: Skipped requests carry no throttler headers
A throttler skipped for a request (see "Throttlers are skipped one by one") SHALL NOT count the request and MUST NOT
set its `X-RateLimit-*<suffix>` or `Retry-After<suffix>` header. Throttlers that are not skipped SHALL count the
request and set their headers as for any other request. A request skipped for every configured throttler carries no
`X-RateLimit-*` or `Retry-After` header.

#### Scenario: Skipped route with the default throttler
- **WHEN** the only throttler is `ThrottlerOptions::new(60_000, 10)` and a route declares
  `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** the response carries no `X-RateLimit-*` or `Retry-After` header

#### Scenario: Bare skip with a named throttler next to the default one
- **WHEN** the throttlers are `default` (`ttl: 60_000`, `limit: 10`) and `short` (`ttl: 1_000`, `limit: 3`) and a
  route declares `#[set_metadata(SKIP_THROTTLE, true)]`
- **THEN** the response carries `X-RateLimit-Limit-short: 3` and `X-RateLimit-Remaining-short: 2` and no
  `X-RateLimit-Limit` header without a suffix

## ADDED Requirements

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

# Proposal

Implements [randygil/corvis#152](https://github.com/randygil/corvis/issues/152) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found while recording the #120 differential fixture. This change
builds on #120 (`e13-rate-limit-headers-in-the-throttler`) and #135
(`e13-throttler-block-duration-like-the-original`) and is implemented and archived **after both**.

## Why

In `@nestjs/throttler`, `@SkipThrottle()` with no argument means `@SkipThrottle({ default: true })`: it skips only the
throttler named `default`, and every other named throttler still counts the request. Skips are stored per throttler
name (`THROTTLER:SKIP<name>`), so `@SkipThrottle({ short: true })` skips `short` alone, and a handler can undo a
controller's skip for one throttler with `@SkipThrottle({ default: false })`.

Corvis has a single `SKIP_THROTTLE` key, and `ThrottlerGuard` returns before looking at any throttler when it is set
(`crates/corvis/src/modules/throttler.rs:191-194`). A migrated application with named throttlers (`short`, `long`)
and a bare skip on a route is limited by the original and not limited at all by Corvis. Corvis's goal is verified
behavioral parity, and the original's behavior here is the stricter one, so matching it costs no security.

## What Changes

- **Behavior change:** `#[set_metadata(SKIP_THROTTLE, true)]` skips only the throttler named `default`, like a bare
  `@SkipThrottle()`. Throttlers with other names keep counting the request and sending their headers.
- Per-name skips with the original's metadata keys: `skip_throttle_key(name)` returns `"THROTTLER:SKIP<name>"`, and
  `skip_throttle(["short", "long"])` returns a `CustomDecorator` that skips those throttlers
  (`#[apply_decorators(skip_throttle(["short"]))]`, like `@SkipThrottle({ short: true })`). A `false` value on the
  handler undoes the controller's skip for that throttler, as in the original.
- The guard resolves the skip **per throttler**, handler first and then controller (`getAllAndOverride` per name).
  A skipped throttler does not count the request and sets no `X-RateLimit-*` or `Retry-After` header; the others run
  as usual (#120 headers, #135 block model and `ttl` order).
- When a route has a bare `SKIP_THROTTLE` and no configured throttler is named `default`, the guard logs one warning
  per route the first time it sees it, naming the route and the per-name API, so an existing Corvis app whose bare skip
  stopped having an effect finds out at once.
- `RouteDef::set_metadata` accepts `impl Into<String>` keys, like `ControllerMetadata::set_metadata` and
  `WsEnhancers::set_metadata` already do, so `#[set_metadata(skip_throttle_key("short"), true)]` compiles on routes,
  controllers and gateway handlers alike.
- The #120 differential fixture gains the skip cases (bare skip with named throttlers, per-name skip, handler
  `false` over a controller skip) recorded from `@nestjs/throttler`, and its `/skip` route is replayed with
  `skip_throttle(["default", "short", "long"])` instead of `SKIP_THROTTLE`.
- Docs (EN + ES): rate limiting (metadata table, a "Skipping throttlers" section, the behavior change, removal of the
  limitation #120 lists).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/rate-limiting` (created by #120, extended by #135): the requirement on skipped requests is modified to
  work per throttler; requirements are added for the bare skip, per-name skips, handler-over-controller resolution,
  the warning, and the differential check of skips.

## Impact

- `crates/corvis/src/modules/throttler.rs`: `skip_throttle_key`, `skip_throttle`, the per-throttler skip in
  `ThrottlerGuard::can_activate`, the warn-once set; rustdoc of `SKIP_THROTTLE`.
- `crates/corvis/src/router/controller.rs:232`: `RouteDef::set_metadata` key type.
- `crates/corvis/src/lib.rs:275-277` and `:297-298`: export `skip_throttle` and `skip_throttle_key` next to
  `SKIP_THROTTLE` (the `modules::throttler` re-export and the prelude).
- Fixture `crates/corvis/tests/fixtures/throttler/` (created by #120, extended by #135): new cases in `generate.mjs`
  and `throttler.json`; the Rust replay test and new integration tests next to #120's throttler tests.
- Docs: `docs/security/rate-limiting.md`, `docs/es/security/rate-limiting.md`.
- Existing users: a route with a bare `SKIP_THROTTLE` in an app that only has named throttlers is now throttled. No
  test, example or doc in the repository relies on that (`SKIP_THROTTLE` appears only in `throttler.rs`, `lib.rs` and
  the two rate-limiting pages); `examples/cats-app` uses the default throttler only.
- Port apps: none uses the throttler (checked by #135), so apidiff should not move; the parity check confirms it.
- **Order:** after #120 and #135. This change modifies a requirement #120 adds to `security/rate-limiting`, edits the
  guard loop #135 rewrites (sorting, `record_hit`, header helper), and extends the fixture both create. Implementing it
  first would mean rewriting the same loop three times and recording the fixture against a guard without headers.

## Out of scope

- Per-throttler overrides with `@Throttle({ short: { limit, ttl } })`: `THROTTLE_LIMIT` / `THROTTLE_TTL` (and #135's
  `THROTTLE_BLOCK_DURATION`) still apply to every throttler; possible new issue with the same per-name key scheme.
- `skipIf`, `ignoreUserAgents` and an overridable `shouldSkip` on the guard (the original's other ways to skip);
  possible new issue together with custom trackers.
- A "skip every throttler" shortcut that the original does not have: `skip_throttle([...])` with every name does it.
- Throttling on WebSocket gateways (decided by #116): per-name skips work there with `#[set_metadata]`, nothing else
  changes.

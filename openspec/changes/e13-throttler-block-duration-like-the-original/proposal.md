# Proposal

Implements [randygil/corvis#135](https://github.com/randygil/corvis/issues/135) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Found while drafting #120 (`e13-rate-limit-headers-in-the-throttler`); this change builds on it and must be
implemented and archived after it.

## Why

`@nestjs/throttler` blocks a client for `blockDuration` (default: the throttler's `ttl`) from the moment it goes over
the limit, and its `Retry-After` counts down from that moment. Corvis counts in fixed windows instead
(`ThrottlerStorage::increment`): the client is let back in when the window ends, so the first 429 of each window
reports a different `Retry-After` and the block ends earlier or later than in the original.

Reading the original's storage shows a second difference the issue's acceptance criterion depends on: it counts hits
in a **sliding** window (each hit expires `ttl` after it was made), so `X-RateLimit-Remaining` and the request that
gets blocked differ from Corvis's fixed-window count as soon as a sequence crosses a window boundary. A fixed window
also lets a client send up to twice the limit around a boundary; the sliding count does not. Equal `Retry-After` and
`X-RateLimit-*` values for the same request sequence need both the block and the sliding count.

## What Changes

- The storage counts like the original (`@nestjs/throttler` 6.7.1): one record per key (throttler name, client IP,
  controller, handler) with the expiry of each counted hit, a reporting window for `X-RateLimit-Reset`, and a block
  state.
- **Block:** the hit that takes the count over the limit blocks the key for the block duration from that moment.
  While blocked, requests answer 429 and are not counted, and `Retry-After` is the time left in the block. When the
  block ends, counting starts again from zero with the next request.
- **Block duration:** defaults to the effective `ttl` (after `THROTTLE_TTL`), as in the original. Configurable per
  throttler with `ThrottlerStorage::with_block_duration(name, ms)` and per route or controller with the new
  `THROTTLE_BLOCK_DURATION` metadata (milliseconds, all throttlers, like `THROTTLE_LIMIT` / `THROTTLE_TTL`). `0` turns
  the block off: a request is rejected while the last `ttl` already holds `limit` hits, is not counted, and
  `Retry-After` is the time until the oldest of those hits expires.
- Throttlers are checked in ascending `ttl` order, as the original sorts them.
- `ThrottlerStorage::record_hit(key, ttl, limit, block_duration) -> ThrottlerRecord` exposes the new model;
  `increment(key, ttl)` keeps its signature and counts with the sliding window.
- Memory stays bounded: besides `with_max_entries`, `with_max_hits(n)` (default 1,000,000) caps the stored hit
  timestamps.
- The headers of #120 take their values from the new record: `Retry-After` from the block, `X-RateLimit-Remaining`
  from the sliding count; `X-RateLimit-Reset` keeps its meaning.
- Differential tests: a storage-level fixture produced by running the original `ThrottlerStorageService.increment`
  with a controlled clock (compared exactly), and #120's HTTP replay extended to compare `Retry-After` by value.
- **Behavior change:** apps using `ThrottlerGuard` block longer (a full block duration from the moment of the block
  instead of the rest of the window) and count hits in a sliding window.
- **Deliberate divergence:** with `limit` 0, 6.7.1 lets one request through each time a block ends; Corvis keeps
  rejecting every request, as the original's current source does (security over parity).
- Docs (EN + ES): the rate limiting page.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/rate-limiting` (created by #120): the counting model, the block, its duration, the throttler order and
  the storage memory bound are added; the header values that depend on them (`X-RateLimit-Remaining`, `Retry-After`)
  and the differential requirement are modified.

## Impact

- `crates/corvis/src/modules/throttler.rs`: storage records, `record_hit`, `ThrottlerRecord`,
  `with_block_duration`, `with_max_hits`, sorting in `ThrottlerStorage::new`, `THROTTLE_BLOCK_DURATION`, and the
  guard.
- `crates/corvis/src/lib.rs`: export `THROTTLE_BLOCK_DURATION` and `ThrottlerRecord` next to `THROTTLE_TTL` (the
  `modules::throttler` re-export and the prelude).
- Fixtures under `crates/corvis/tests/fixtures/throttler/` (the folder #120 creates): a storage-level generator and
  its JSON output; unit tests in `throttler.rs`; integration tests next to #120's throttler tests.
- Docs: `docs/security/rate-limiting.md` and `docs/es/security/rate-limiting.md`.
- Existing tests keep their expectations: `crates/corvis/tests/modules.rs:140-156` (third request 429) and
  `crates/corvis/tests/input_hardening.rs:127-138` (three 200 then two 429) both stay inside one window.
- `examples/cats-app` uses a global `ThrottlerGuard` (100 per minute): it blocks for a minute from the 101st request
  instead of until the window ends; it has no tests that depend on it.
- Port apps: none uses the throttler (no match for `throttl` in the sources of the three ports under
  `corvis-tests/corvis/*/src`, nor in the originals' sources or `package.json`), so apidiff should not move; the
  parity check confirms it.
- **Order:** after #120. This change modifies two requirements #120 adds to `security/rate-limiting` and sets the
  values of the headers #120 introduces, so #120 must be archived first. Independent of #119
  (`ThrottlerModule::for_root_async`): a factory returning a configured `ThrottlerStorage` reaches
  `with_block_duration` and `with_max_hits`, and #119's `From<Vec<ThrottlerOptions>>` delegates to
  `ThrottlerStorage::new`, which sorts.

## Out of scope

- A `block_duration` field or a builder on `ThrottlerOptions`: a new public field breaks the struct literals in the
  docs and in users' code (design.md, Decision 4, Open question 2); possible new issue together with #120's deferred
  per-throttler `setHeaders`.
- Per-throttler skips and overrides (`@SkipThrottle({ short: true })`, `@Throttle({ short: { ... } })`): Corvis
  metadata apply to every throttler; possible new issue.
- The original's IPv6 tracker normalization (`normalizeIp`, a /64 subnet by default): Corvis keys IPv6 clients by full
  address, so a client with a /64 can rotate addresses; possible new security issue.
- `ignoreUserAgents`, `skipIf`, custom trackers and key generators, custom error messages, and function-valued options.
- A storage trait for custom or shared storages (Redis).
- Throttling on WebSocket gateways (decided by #116).

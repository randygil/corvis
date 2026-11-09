# Proposal

Implements [randygil/corvis#162](https://github.com/randygil/corvis/issues/162) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Depends on #156 (`e13-throttle-overrides-per-throttler-name`,
archived on `main`), which added per-name overrides, module-level `getTracker` / `generateKey` hooks and route-level
hooks by registered id, and left these two pieces out (its proposal, Out of scope; review note filing #162). The issue
body suggests the slug `e13-throttler-per-name-hooks-and-dynamic-options`; this change uses
`e13-per-throttler-hooks-and-function-options`, the name it was assigned when drafted.

## Why

The original throttler library (`@nestjs/throttler` 6.7.1) accepts, in each entry of its throttler list:

- `getTracker` and `generateKey`, used for that throttler only. The guard picks
  `routeOrClass || throttler || module` for each (`dist/throttler.guard.js:94-95`), so a `login` throttler can count
  per account while the `default` throttler keeps counting per IP, without touching any route.
- `limit`, `ttl` and `blockDuration` as functions of the execution context (`Resolvable<number>`,
  `dist/throttler-module-options.interface.d.ts:4-9`), called on every request (`resolveValue`,
  `throttler.guard.js:91-93`, `:189-191`), for example a higher limit for paying users.

Corvis has neither. `ThrottlerOptions` is `{ name, ttl, limit }` with plain numbers
(`crates/corvis/src/modules/throttler.rs:182-196`), and the guard resolves a tracker and a key generator as route
override, then module hook, then built-in (`throttler.rs:1038-1056`). A migrated app with a per-throttler `getTracker`
has to repeat `throttle("login").tracker("by-account")` on every route, and one with a function-valued `limit` has no
equivalent at all.

## What Changes

- Per-throttler hooks on `ThrottlerStorage`, next to `with_block_duration(name, ms)`:
  `with_tracker_for(name, f)` and `with_key_generator_for(name, f)`, with the signatures of `with_tracker` and
  `with_key_generator`. The guard resolves each throttler's tracker and key generator as route override (handler,
  then controller), then the throttler's hook, then the module hook, then the built-in, as the original does.
- Function-valued options on `ThrottlerStorage`: `with_limit_fn(name, f)`, `with_ttl_fn(name, f)` and
  `with_block_duration_fn(name, f)`, where `f` is an async function of the `ExecutionContext` returning
  `Result<u32, HttpException>` (limit) or `Result<u64, HttpException>` (milliseconds). For each throttler and option
  the guard uses the route override, else the function, else the static value; the block duration then falls back to
  the effective ttl, as today. A function is not called when a route override applies.
- The function's typed result replaces the original's runtime number check: no out-of-range or non-numeric value can
  reach the storage, so #156's validation has nothing left to reject. An `Err` is returned by the guard unchanged
  (a 500 for an internal error, the exception's own status otherwise), as an error thrown by the original's function.
- All five builders panic when no configured throttler has that name (like `with_block_duration`); calling one twice
  for the same name keeps the last function.
- Throttlers with a ttl function are checked after the throttlers with a static ttl, in configured order, as the
  original's sort does (`throttler.guard.js:39-47`); static throttlers keep the ascending-ttl order.
- Keys produced by a per-throttler key generator get their own source marker in the stored key, so they never meet
  built-in, module or registered-generator keys (#156, Decision 8 as amended by its security review).
- The differential fixture of #120/#135/#152/#156 gains per-throttler hook and function-valued cases recorded from the
  original library 6.7.1. Docs (EN + ES) of rate limiting.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/rate-limiting`: the block duration, check order, module hooks, registered-name hooks, per-throttler
  counters and default tracker requirements take the new level into account; requirements are added for
  per-throttler hooks, function-valued options, their errors, and the differential check.

## Impact

- `crates/corvis/src/modules/throttler.rs`: the five builders and their storage maps, a type alias for value
  functions, the check order with ttl functions, and the resolution in `ThrottlerGuard::can_activate`; module rustdoc.
- `crates/corvis/src/lib.rs:277-281`: export the new type alias `ThrottlerValueFn` next to `ThrottlerTrackerFn` (not
  in the prelude, like the other hook aliases).
- Tests: `crates/corvis/tests/throttler_headers.rs` (scenarios and replay), unit tests in `throttler.rs` (unknown-name
  panics, check order).
- Fixture: `crates/corvis/tests/fixtures/throttler/generate.mjs` and `throttler.json`.
- Docs: `docs/security/rate-limiting.md`, `docs/es/security/rate-limiting.md`.
- Existing users: nothing changes unless they call the new builders. `ThrottlerOptions` keeps its fields, so struct
  literals keep compiling. Port apps do not use the throttler; apidiff should not move.

## Out of scope

- Function-valued `limit` / `ttl` / `blockDuration` in route overrides (`@Throttle({ short: { limit: (ctx) => .. } })`):
  metadata holds JSON, so it would need a registry of value functions like the named trackers; possible new issue.
- `skipIf`, `ignoreUserAgents` and `setHeaders` per throttler, `errorMessage` and an overridable `shouldSkip` (already
  listed by #152 and #156 as a possible new issue).
- A per-throttler IPv6 subnet prefix (the original has one module-wide value, as Corvis).
- A shared storage (Redis) or any change to the storage's counting model.

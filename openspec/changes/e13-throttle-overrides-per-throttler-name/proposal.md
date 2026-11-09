# Proposal

Implements [randygil/corvis#156](https://github.com/randygil/corvis/issues/156) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Builds on #152 (`e13-skip-throttle-matches-the-original`, per-name
skip keys) and #154 (`e13-throttler-groups-ipv6-clients-by-prefix`, whose review note Q5 assigns the module-level
`getTracker` and `generateKey` hooks to this issue). Both are archived on `main`.

## Why

In the original throttler library, `@Throttle({ short: { limit: 1, ttl: 10000 } })` overrides the options of one named
throttler for a route or a controller, and leaves the other throttlers alone. Each option is stored under its own
per-name metadata key (`THROTTLER:LIMITshort`, `THROTTLER:TTLshort`, `THROTTLER:BLOCK_DURATIONshort`,
`THROTTLER:TRACKERshort`, `THROTTLER:KEY_GENERATORshort`) and resolved per option, handler first.

Corvis has only the bare keys `THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION`, and the guard applies
them to **every** throttler (`crates/corvis/src/modules/throttler.rs:727-734`). A migrated app with `short` (1 s) and
`long` (60 s) throttlers cannot tighten its login route on `long` alone; setting `THROTTLE_LIMIT` to 5 there also
raises `short` from 3 to 5, and `THROTTLE_TTL` gives both throttlers the same window. Corvis also has no way to choose
what identifies a client (`getTracker`) or how the counter key is built (`generateKey`), at module or route level, so a
per-account login limit cannot be expressed without writing a guard by hand.

## What Changes

- Per-name overrides with the original's key spelling: `throttle("short")` returns a `ThrottleOverride` builder
  (`.limit(n)`, `.ttl(ms)`, `.block_duration(ms)`, `.tracker(id)`, `.key_generator(id)`) that converts into a
  `CustomDecorator`, for `#[apply_decorators(throttle("short").limit(1), throttle("long").limit(100))]` on controllers
  and routes (`@Throttle({ short: { limit: 1 }, long: { limit: 100 } })`). `throttle_key(option, name)` returns
  `option` followed by `name` (`THROTTLER:LIMITshort`) for `#[set_metadata]` on gateway handlers. New constants
  `THROTTLE_TRACKER` (`"THROTTLER:TRACKER"`) and `THROTTLE_KEY_GENERATOR` (`"THROTTLER:KEY_GENERATOR"`).
- The guard resolves each option of each throttler on its own: handler first, then controller, the first value present
  wins, so a handler that only sets `limit` keeps the controller's `ttl` (a partial override, as in the original).
- **Behavior change:** the bare `THROTTLE_LIMIT`, `THROTTLE_TTL` and `THROTTLE_BLOCK_DURATION` keys become the
  `default` throttler's entry, like the bare `SKIP_THROTTLE` after #152. Throttlers with other names are no longer
  affected by them.
- Override values are validated: a value that is not a non-negative integer in range (or, for a tracker or key
  generator, the id of a registered function) answers 500 and logs the key and value, like the original's
  `Invalid "limit" for the "short" throttler` error. Today such a value is silently ignored.
- Overrides naming a throttler that is not configured (including bare keys when no throttler is named `default`) are
  logged once per route, naming the route and the unknown name.
- Module-level hooks on `ThrottlerStorage` (the module options): `with_tracker(f)` (the original's `getTracker`,
  async, gets the `ExecutionContext`) and `with_key_generator(f)` (`generateKey(context, tracker, name)`).
- Route-level hooks by registered name: `with_named_tracker(id, f)` and `with_named_key_generator(id, f)` register
  functions on the storage; `throttle("short").tracker("by-email")` selects one for a route or controller. Metadata
  holds JSON, so routes reference functions by name instead of holding closures. An unknown name answers 500.
- Counters stay separate per throttler whatever the key generator returns (the throttler name is always part of the
  stored key).
- The differential fixture of #120/#135/#152 gains override and hook cases recorded from the original library 6.7.1.
- Docs (EN + ES) of rate limiting; `examples/cats-app` tightens its login route with a per-name override.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `security/rate-limiting`: the block duration requirement becomes per name and the tracker requirement allows hooks;
  requirements are added for per-name overrides, the bare override keys, the helpers, value validation, unknown names,
  tracker and key generator hooks at module and route level, per-throttler counters, and the differential check.

## Impact

- `crates/corvis/src/modules/throttler.rs`: constants, `throttle_key`, `throttle` / `ThrottleOverride`, hook types and
  the four `ThrottlerStorage` builders, override resolution and validation in `ThrottlerGuard::can_activate`, the
  unknown-name warning, the stored key layout; rustdoc of the bare constants.
- `crates/corvis/src/lib.rs:277-280` and the prelude list (`:296-304`): export the new items next to `skip_throttle`.
- Tests: `crates/corvis/tests/throttler_headers.rs` (routes, scenarios, replay), `crates/corvis/tests/throttler_skip_log_ws.rs`
  (warning and gateway cases), unit tests in `throttler.rs` (the hand-built guards gain the new warn-once field).
- Fixture: `crates/corvis/tests/fixtures/throttler/generate.mjs` and `throttler.json`.
- Docs: `docs/security/rate-limiting.md`, `docs/es/security/rate-limiting.md`. Example: `examples/cats-app/src/auth/auth_controller.rs`.
- Existing users: a route with a bare override in an app without a `default` throttler loses that override (and the
  guard warns once per route); a bare override no longer changes named throttlers. In the repository only
  `tests/throttler_headers.rs` and `tests/modules.rs:77` use bare keys, all with the `default` throttler, so they keep
  their results. A wrongly typed override value now answers 500 instead of being ignored.
- Port apps: none uses the throttler (checked by #135 and #152); apidiff should not move.

## Out of scope

- Per-throttler `getTracker` / `generateKey` in `ThrottlerOptions` (the middle level of the original's chain); possible
  new issue, it would reuse the hook types added here.
- Function-valued `limit`, `ttl` and `blockDuration` (`Resolvable<number>`), at module or route level; possible new
  issue (the named-function registry of this change could carry them).
- Numeric strings as override values (`"5"`, which the original coerces with `Number()`): rejected with a 500 here,
  see design.md, Decision 4.
- `skipIf`, `ignoreUserAgents`, `setHeaders` per throttler, `errorMessage` and an overridable `shouldSkip` (already
  listed by #152 as a possible new issue).
- Reporting unknown names in skip keys (`skip_throttle(["shrot"])`): harmless (it skips nothing) and decided by #152.
- A shared storage (Redis) or any change to the storage's counting model.

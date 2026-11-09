# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code at `af99b71`
(all paths under `crates/corvis/` unless stated):

**Corvis**

- `SKIP_THROTTLE` is `"THROTTLER:SKIP"` (`src/modules/throttler.rs:28-29`), documented as `@SkipThrottle()`; the
  module rustdoc shows `#[set_metadata(SKIP_THROTTLE, true)] // @SkipThrottle()` (`throttler.rs:11`).
- `ThrottlerOptions::new` names the throttler `default` (`throttler.rs:44-48`); `name` is a public field, so apps can
  configure `short` / `long` with struct literals (`docs/security/rate-limiting.md:52-57`).
- `ThrottlerGuard::can_activate` (`throttler.rs:189-206`) reads `SKIP_THROTTLE` once with
  `get_all_and_override::<bool>` over `[handler, class]` and returns `Ok(true)` before the loop when it is `true`
  (lines 191-194). So a bare skip skips **every** throttler, and a handler's `false` over a controller's `true` turns
  every throttler back on. The loop (lines 196-204) applies `THROTTLE_LIMIT` / `THROTTLE_TTL` to every throttler.
- `Reflector::get_all_and_override` (`src/common/reflector.rs:21-23`) returns the first target whose value
  deserializes as `T`; `get` (line 15-17) treats a value of another type as missing.
- Metadata of a target is a `HashMap<String, Value>` (`src/common/context.rs:15-18`) built from the declared entries in
  order (`src/router/mod.rs:377` for controllers, `:417` for routes; `src/websockets.rs:599`, `:616` for gateways), so
  the last `#[set_metadata]` for a key on one target wins.
- Metadata builders: `ControllerMetadata::set_metadata(key: impl Into<String>, ..)` (`src/router/controller.rs:99`),
  `RouteDef::set_metadata(key: &str, ..)` (`src/router/controller.rs:232`), `WsEnhancers::set_metadata(key: impl
  Into<String>, ..)` (`src/discovery.rs:56`), `CustomDecorator::set_metadata(key: impl Into<String>, ..)`
  (`src/common/decorator.rs:44`). The macros pass the attribute's key expression as is
  (`corvis-macros/src/controller.rs:23-30`, `:107-110`, `:274-277`; `gateway.rs:51-57`; `handlers.rs:118-124`), so
  a `String` key fails to compile on routes only. `#[apply_decorators(..)]` works on controllers and routes
  (`decorator.rs:4-19`) but not on gateway handlers (no `decorate` in `discovery.rs` or the gateway macros).
- Exports: `src/lib.rs:275-277` (`modules::throttler` re-export) and the prelude list at `src/lib.rs:297-298`.
- Tests: no test uses `SKIP_THROTTLE` today (grep over `crates/` and `examples/`); the docs use it in
  `docs/security/rate-limiting.md:65` and `:80` (and the Spanish copy).
- Logging: other runtime warnings use `Logger::new("<Context>").warn(..)` (`src/application.rs:207-208`, `:542`).

**Pending changes this one builds on**

- #120 (`e13-rate-limit-headers-in-the-throttler`) adds `X-RateLimit-*` / `Retry-After` headers set from one private
  helper per throttler (its design.md, Decision 4) and the requirement "Skipped requests carry no throttler headers".
  Its draft fixture (`crates/corvis/tests/fixtures/throttler/generate.mjs`) skips `/skip` with
  `SkipThrottle({ default: true, short: true, long: true })` because Corvis's bare skip skips everything, and its docs
  list the difference under Limitations, as #152 says.
- #135 (`e13-throttler-block-duration-like-the-original`) sorts the throttlers by `ttl` in `ThrottlerStorage::new`,
  replaces `increment` with `record_hit` in the loop, stops at the first rejecting throttler, and adds a storage
  fixture next to #120's. Its proposal lists per-throttler skips and overrides as out of scope; this change takes the
  skips.

**Original (`@nestjs/throttler` 6.7.1, the version pinned by the #120 and #135 fixtures; read from the installed
package's `dist/throttler.decorator.js`, `dist/throttler.constants.js` and `dist/throttler.guard.js`)**

- `THROTTLER_SKIP = 'THROTTLER:SKIP'`.
- `SkipThrottle(skip = { default: true })` defines, for each key of `skip`, metadata `THROTTLER_SKIP + key` with the
  given boolean on the handler (method decorator) or the class (class decorator).
- `canActivate`: `shouldSkip(context)` (an overridable hook returning `false`) first; then, for each throttler in
  `ttl` order, `skip = reflector.getAllAndOverride(THROTTLER_SKIP + name, [handler, classRef])`. When `skip` (or the
  throttler's `skipIf`) is truthy the throttler is skipped (`continues.push(true); continue`), so it neither counts
  nor sets headers; otherwise it reads `THROTTLER_LIMIT + name`, `THROTTLER_TTL + name`, `THROTTLER_BLOCK_DURATION +
  name` and calls `handleRequest`, which throws on a block.
- `getAllAndOverride` returns the first value that is not `undefined`, so a handler's `false` overrides the class's
  `true` for that name only, and a class skip of `short` still applies when the handler only declares
  `@SkipThrottle()` (another key).
- Unnamed throttlers are named `default` in `onModuleInit`; a bare `@SkipThrottle()` in an app whose throttlers all
  have other names skips nothing, silently.

## Goals / Non-Goals

**Goals:**
- The same throttlers count, reject and set headers for a request as in the original, for every combination of bare,
  per-name, handler and controller skips; checked by the differential fixture.
- Existing Corvis apps whose bare skip changes meaning find out without reading the changelog.

**Non-Goals:**
- Per-name limit, ttl or block duration overrides, `skipIf`, `ignoreUserAgents`, an overridable `should_skip`
  (proposal, Out of scope).

## Decisions

1. **Match the original: a bare skip only skips `default`.** The issue asks to decide between matching and
   documenting the difference. Matching wins: parity is the project's positioning, a migrated app's named throttlers
   would otherwise silently stop protecting skipped routes (login endpoints often carry a bare skip on one throttler
   and rely on the others), and the change only makes Corvis stricter.
   Alternative considered: keep "skip all" and document it. Rejected: the docs already list it as a limitation, and
   every migrated app with named throttlers would need a manual rewrite to be safe.
2. **Per-name metadata keys with the original's spelling.** The guard reads `skip_throttle_key(name)` =
   `format!("{SKIP_THROTTLE}{name}")` (`THROTTLER:SKIPshort`), one key per throttler, exactly as the original stores
   them. `SKIP_THROTTLE` keeps its value and becomes the bare form: for the throttler named `default`, a target's
   `SKIP_THROTTLE` value counts when that target has no `THROTTLER:SKIPdefault` entry. Resolution per throttler walks
   `[handler, class]` and takes the first target with a boolean for that name, so handler-over-controller works per
   name as in the original.
   Alternatives considered: one key holding a map (`#[set_metadata(SKIP_THROTTLE, json!({"short": true}))]`).
   Rejected: a handler's map would replace the controller's whole map, so a controller skip of `short` plus a handler
   bare skip would lose the `short` skip, unlike the original; and the value type of an existing key would change.
   A new `SKIP_THROTTLE_DEFAULT` constant for the bare form. Rejected: `SKIP_THROTTLE` already is the bare form in
   the docs and the rustdoc.
3. **Helpers instead of a macro.** `skip_throttle(names: impl IntoIterator<Item = impl AsRef<str>>) ->
   CustomDecorator` covers `@SkipThrottle({ a: true, b: true })` on controllers and routes through
   `#[apply_decorators(..)]`. `skip_throttle_key(name) -> String` covers `false` values
   (`#[set_metadata(skip_throttle_key("short"), false)]`) and gateway handlers, which have no `#[apply_decorators]`.
   Both are exported next to `SKIP_THROTTLE`.
   Alternative considered: a `#[skip_throttle(short, long)]` attribute. Rejected: a new macro attribute in three
   macro front-ends for something `#[apply_decorators]` already does.
4. **`RouteDef::set_metadata` takes `impl Into<String>`.** It is the only metadata builder with a `&str` key
   (`controller.rs:232`); aligning it with `ControllerMetadata` and `WsEnhancers` lets the same attribute work
   everywhere. `&'static str` constants keep compiling; the only callers are the macros and users' builder code.
   Alternative considered: document `#[set_metadata(&skip_throttle_key("short"), true)]` (deref coercion). Rejected:
   a trap that only fails on routes.
5. **Skip check inside the loop, before anything else for that throttler.** After #135 the loop iterates the
   `ttl`-sorted throttlers; the per-throttler skip runs first (no limit, ttl, block duration, key, counter or header
   work for a skipped throttler), then `continue`. The early return on `SKIP_THROTTLE` is removed. A request skipped
   for every throttler costs one metadata lookup per throttler, as in the original.
6. **Warn once per route when a bare skip skips nothing.** When the bare skip resolves to `true` for a route and no
   throttler is named `default`, the guard logs, through `Logger::new("ThrottlerGuard").warn(..)`, a message naming the
   controller and handler, saying that a bare `SKIP_THROTTLE` only skips the `default` throttler, and pointing to
   `skip_throttle([...])`. The guard keeps a `Mutex<HashSet<(String, String)>>` of warned routes; its size is bounded
   by the number of routes and gateway handlers, which are fixed at startup. The original is silent; a log line is not
   part of the parity surface (apidiff compares HTTP responses and database state).
   Alternatives considered: a startup check over every route. Rejected: the guard does not see the route table, and
   a guard registered with `#[use_guards]` on some controllers only would need the posture machinery of #26. Silence,
   as in the original. Rejected: Corvis apps written before this change would lose their skip with no signal.
7. **Fixture cases appended to #120's `generate.mjs`.** A new app in the generator with throttlers `short`
   (`ttl: 30_000`, `limit: 3`) and `long` (`ttl: 60_000`, `limit: 5`), and one with `default` and `short`, both with
   long windows so the values do not depend on timing, and routes declared with the original's decorators applied by
   hand (the fixture is plain JavaScript):
   - `/bare`: `@SkipThrottle()`; four requests (both apps).
   - `/by-name`: `@SkipThrottle({ short: true })`.
   - a controller with `@SkipThrottle()` and a handler with `@SkipThrottle({ default: false })`, plus a sibling
     handler with no decorator.
   - a controller with `@SkipThrottle({ short: true })` and a handler with `@SkipThrottle()`.
   The Rust replay declares the same routes with `SKIP_THROTTLE`, `skip_throttle([..])` and `skip_throttle_key(..)` and
   compares status, header names and values as #120 / #135 already do. #120's existing `/skip` route switches to
   `skip_throttle(["default", "short", "long"])` on the Rust side, so its recorded responses stay equal.

## Risks / Trade-offs

- [Existing Corvis apps with only named throttlers start limiting routes they skipped] → stated as a behavior change
  in the docs with the one-line fix (`skip_throttle([...])`), and the warning of Decision 6 names each affected route
  on its first request. The ports and examples are not affected.
- [A typo in a throttler name in `skip_throttle(["shrot"])` skips nothing] → same as the original; the docs say the
  names must match `ThrottlerOptions::name`. Validating names would need the guard to see every route at startup
  (rejected in Decision 6).
- [`RouteDef::set_metadata` signature change] → source compatible for `&str` and `String` arguments; a caller passing
  some other `AsRef<str>` type that is not `Into<String>` would stop compiling. No such caller exists in the
  workspace (the macros pass the attribute expression).
- [Warn-once set grows] → bounded by the number of route handlers; keys are the static controller and handler names.
- [Order dependency on #120 and #135] → stated in proposal.md and at the top of tasks.md; the change does not apply
  cleanly on `main` before them.

## Open questions

1. **Warning.** Keep the warn-once log for a bare skip that skips nothing (proposed), or stay silent like the
   original?
2. **Per-name overrides next.** Should `@Throttle({ short: { limit } })` per-name overrides be filed now as a follow-up
   issue, reusing the key scheme of Decision 2 (`THROTTLER:LIMIT<name>`, `THROTTLER:TTL<name>`,
   `THROTTLER:BLOCK_DURATION<name>`)?

## Review notes (2026-11-09)

- Q1: keep the warn-once log for a bare skip that skips nothing.
- Q2: per-name overrides are filed as a follow-up issue reusing the key scheme of Decision 2.

## Implementation notes

- Reference version re-checked (task 1.1): `@nestjs/throttler` 6.7.1 is installed from the fixture's lockfile; `SkipThrottle`
  and the skip part of `canActivate` match the Context of this design.
- The design was drafted at `af99b71`; the guard now (after #120 and #135) loops over the `ttl`-sorted throttlers and calls
  `record_hit`, so the per-throttler skip is the first statement of that loop and the early return is gone, as Decision 5
  says. Line numbers in the Context are stale; the code sketches still apply.
- `ThrottlerGuard` gained a `#[default]` field `warned: Mutex<HashSet<(String, String)>>` (Decision 6); the unit tests that
  build the guard by hand set it with `Default::default()`.
- The warning is decided on the bare `SKIP_THROTTLE` resolved over `[handler, class]` (as before the change); it is logged
  before the loop, so the other throttlers still run.
- Decision 2 resolution: per target, `skip_throttle_key(name)` first and, for `default` only, `SKIP_THROTTLE` as its
  fallback; the first target with a boolean wins. A handler `SKIP_THROTTLE: false` therefore undoes a controller's bare
  skip, and `skip_throttle_key("default")` on a target takes precedence over its `SKIP_THROTTLE`. The key scheme
  (`THROTTLER:SKIP<name>`) is the one #156 reuses for `THROTTLER:LIMIT<name>` and the other per-name keys.
- Fixture: instead of changing the `named` and `second` apps, two apps were added (`skips-named`, `skips-default`) and the
  `default` app got two `/bare` requests, so the recorded values of the existing cases are unchanged. The Rust replay
  declares `/undo/*` and `/mixed/h` as controllers next to the existing routes; `/skip` now uses
  `skip_throttle(["default", "short", "long"])`.
- The warn-once and gateway tests live in `tests/throttler_skip_log_ws.rs` (own binary: process-wide logger).
- Review note Q2 (per-name overrides as a follow-up) is #156; no code here.
- Task 5.5 (corvis-parity) is left to the maintainer's session.

# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `bcc778d` (`main`); paths
under `crates/corvis/` unless stated.

**Corvis**

- Metadata keys (`src/modules/throttler.rs`): `SKIP_THROTTLE = "THROTTLER:SKIP"` (line 36), `skip_throttle_key(name)`
  (41-43) and `skip_throttle(names)` (48-53) from #152; `THROTTLE_LIMIT = "THROTTLER:LIMIT"` (55-56, rustdoc
  "`@Throttle({ default: { limit } })`"), `THROTTLE_TTL = "THROTTLER:TTL"` (57-58), `THROTTLE_BLOCK_DURATION =
  "THROTTLER:BLOCK_DURATION"` (59-61, "for every throttler").
- `ThrottlerOptions { name, ttl, limit }` with public fields (63-76); apps build named throttlers with struct literals
  (`docs/security/rate-limiting.md:60-65`, `tests/throttler_headers.rs:136-138`).
- `ThrottlerStorage` (388-396) holds the module options: `throttlers` (sorted by `ttl`, 402-421), `block_durations`
  per name (`with_block_duration`, 469-473, panics on an unknown name), `headers`, `ipv6_subnet_prefix`. It is what
  `for_root_storage` and `for_root_async` factories return (611-640).
- `ThrottlerGuard` (650-658) has `storage`, `reflector` and a `warned` set for the #152 warning. `can_activate`
  (718-741): computes `client = normalize_ip(&req.ip(), prefix)` once (722); for each throttler, skips per name
  (724-726, `is_skipped` 664-675), then reads `THROTTLE_LIMIT` and `THROTTLE_TTL` with `get_all_and_override` over
  `[handler, class]` for **every** throttler (727-728), builds the key `"{name}:{client}:{class}:{handler}"` (729),
  resolves the block duration as bare metadata, then `with_block_duration`, then the effective `ttl` (730-734), calls
  `record_hit` (735), sets headers (736) and returns 429 on a block (737-739).
- `record_hit_at` hashes the key with SHA-256 (517-520); the storage never sees the throttler name separately.
- `Reflector::get` (`src/common/reflector.rs:15-17`) deserializes the JSON value and returns `None` when the type does
  not match, so a mistyped override (`"5"`, `-1`, `5.5`) is ignored today; `get_all_and_override` (21-23) returns the
  first target whose value deserializes.
- Metadata values are `serde_json::Value` (`CustomDecorator::set_metadata`, `src/common/decorator.rs:44-47`;
  `Target.metadata: HashMap<String, Value>`, `src/common/context.rs:15-18`), so metadata cannot hold closures.
- `#[apply_decorators(a, b)]` turns each expression into `.decorate(expr)` (`corvis-macros/src/controller.rs:34-38`),
  and `decorate` takes `impl Into<CustomDecorator>` (`src/router/controller.rs:104`, `:237`). Gateway handlers have no
  `#[apply_decorators]`; `WsEnhancers::set_metadata` takes `impl Into<String>` keys (`src/discovery.rs:56`).
- `ExecutionContext` is `Clone` (`src/common/context.rs:58-59`); `request()` (116) gives the HTTP request or the
  gateway handshake request.
- A 500 built with `HttpException::from_anyhow` (`src/http/exception.rs:180-191`) answers
  `{ "statusCode": 500, "message": "Internal server error" }` and is logged with its message by
  `BaseExceptionFilter::handle` (`src/common/filter.rs:51-62`).
- Tests with bare overrides: `tests/throttler_headers.rs:46-92` (routes `/override`, `/ttl5`, `/block1`, `/block10`,
  `/noblock`, `/closed`, `/three`), all exercised with the `default` throttler only; `tests/modules.rs:77` with
  `ThrottlerOptions::new(60_000, 100)` (line 105). No example or port app uses them.
- Fixture: `tests/fixtures/throttler/generate.mjs` pins `@nestjs/throttler` 6.7.1 and `@nestjs/core` 12.4.0
  (`package.json`); `record()` (77-102) passes a throttler array to `forRoot` and sends plain GETs; the only override
  case is `/override` with `Throttle({ default: { limit: 1, ttl: 60000 } })` (57), replayed with a bare
  `THROTTLE_LIMIT` (`tests/throttler_headers.rs:46-50`) in the `default` app only. The replay is
  `replays_the_original_library_fixture` (`tests/throttler_headers.rs:420-460`).

**Original (`@nestjs/throttler` 6.7.1, `dist/` of the npm tarball; `@nestjs/core` 12.4.0)**

- Keys (`throttler.constants.js:4-8`): `THROTTLER:LIMIT`, `THROTTLER:TTL`, `THROTTLER:TRACKER`,
  `THROTTLER:BLOCK_DURATION`, `THROTTLER:KEY_GENERATOR`.
- Storage (`throttler.decorator.js:7-15`): `setThrottlerMetadata` loops over the names of the options object and
  defines **all five** keys `KEY + name` on the target, with `undefined` for options not given. `Throttle` (16-25)
  targets `descriptor.value` (the handler) or the class.
- Types (`throttler.decorator.d.ts:2-8`): every option is optional; `limit`, `ttl`, `blockDuration` are
  `Resolvable<number>` = a number or `(context) => number | Promise<number>` (`throttler-module-options.interface.d.ts:4`);
  `getTracker(req, context) => string | Promise<string>` and `generateKey(context, tracker, throttlerName) => string`
  (same file, 36-37).
- Resolution (`throttler.guard.js:76-105`), per configured throttler in `ttl` order:
  - each option is read with `reflector.getAllAndOverride(KEY + name, [handler, classRef])` (86-90).
    `getAllAndOverride` returns the first value that is not `undefined` (`@nestjs/core` 12.4.0,
    `services/reflector.service.js:91-99`), so the undefined entries written for missing options fall through to the
    class: **partial overrides are per option**, handler before class.
  - `limit = toNumber(await resolveValue(ctx, route ?? throttler.limit))`, same for `ttl`, and
    `blockDuration = toNumber(await resolveValue(ctx, route ?? throttler.blockDuration ?? ttl))` with the effective
    `ttl` (91-93). `resolveValue` calls a function value with the context (189-191). `toNumber` (182-188) converts a
    non-blank string with `Number()` and throws `Invalid "<option>" for the "<name>" throttler: expected a number, got
    <json>.` for anything that is not a number afterwards; the error becomes a 500.
  - `getTracker = route || throttler.getTracker || commonOptions.getTracker` and the same for `generateKey` (94-95);
    the module-level options default to the guard's own methods (65-66).
- Unknown names: only configured throttlers are iterated (76), so `@Throttle({ shrot: ... })` is ignored silently, and
  `@Throttle({ default: ... })` does nothing in an app without a `default` throttler.
- A `null` override on the handler is returned by `getAllAndOverride` (`null !== undefined`) and then replaced by the
  throttler's value through `??`, skipping the class value.
- Per request and throttler (`handleRequest`, 111-149): `tracker = await getTracker(req, context)` (123),
  `key = generateKey(context, tracker, name)` (124), `storage.increment(key, ttl, limit, blockDuration, name)` (125).
  Default tracker: `normalizeIp(req.ip, ipv6SubnetPrefix)` (150-152); default key:
  `sha256("<Class>-<handler>-<name>-<tracker>")` (165-168). A custom tracker skips `normalizeIp`.
- Storage (`throttler.service.js`): hit counts are kept per `(key, throttlerName)` (60-72, 89-93), but the record's
  `expiresAt`, `isBlocked` and `blockExpiresAt` are per **key** (110-150). Two throttlers that get the same key from a
  custom `generateKey` share the reporting window and the block.

## Goals / Non-Goals

**Goals:**
- `@Throttle({ name: { limit, ttl, blockDuration } })` on routes and controllers has a one-line Corvis equivalent with
  the same per-name, per-option, handler-over-controller resolution, checked against the original by the fixture.
- Applications can choose the tracker and the counter key at module level and per route, so per-account limits and
  limits shared between routes work without a hand-written guard.
- Mistakes in override metadata are visible: invalid values fail closed, unknown names are logged.

**Non-Goals:**
- Per-throttler hooks, function-valued numbers, `skipIf`, `ignoreUserAgents` (proposal, Out of scope).
- Changing how hits are counted or blocked (#135) or how the default tracker normalizes IPs (#154).

## Decisions

1. **One change for overrides and hooks.** The issue's first task names `getTracker` and `generateKey` overrides,
   and #154's review note Q5 puts the module-level hooks here. The route-level hooks need the module-level hook types
   and the same registry, and the guard's tracker/key code is touched once. The change stays bounded by leaving out
   the per-throttler level and function-valued numbers (both reuse what this change adds).
   *Alternative:* a separate issue for the hooks. Rejected: two changes rewriting the same lines of `can_activate`
   in sequence, and a route-level `.tracker(..)` with nothing to point at.

2. **Per-name keys, resolved per option.** The guard reads `throttle_key(OPTION, name)` = `OPTION` followed by the
   throttler name, the original's spelling, for `THROTTLE_LIMIT`, `THROTTLE_TTL`, `THROTTLE_BLOCK_DURATION`,
   `THROTTLE_TRACKER` and `THROTTLE_KEY_GENERATOR`. For each throttler and each option it walks `[handler, class]` and
   takes the first target that has a value that is not JSON `null`. A handler that sets only `limit` therefore keeps
   the controller's `ttl`, and `ThrottleOverride` writes only the options that were set (the original writes
   `undefined` for the others, which `getAllAndOverride` skips).
   *Alternative:* one key holding a map per throttler (`THROTTLER:OVERRIDE` → `{"short": {...}}`). Rejected: a handler
   map would hide the controller's whole map, unlike the original, and it would not match the key spelling #152
   started.

3. **Bare keys are the `default` throttler's entry.** For the throttler named `default`, a target's bare
   `THROTTLE_LIMIT` (and `_TTL`, `_BLOCK_DURATION`) counts as that target's `throttle_key(.., "default")` value when the
   target has no per-name entry; other throttlers ignore bare keys. This matches the rustdoc (`@Throttle({ default:
   { limit } })`), the bare `SKIP_THROTTLE` of #152 and the replay of the `/override` fixture case.
   *Alternative:* keep "bare applies to every throttler", with per-name keys taking precedence. Rejected (security):
   a bare limit meant for a 60 s window silently raises a 1 s burst throttler (5 instead of 3), and a bare `ttl` gives
   every throttler the same window, which makes multi-throttler setups meaningless; the failure is silent. With the
   chosen rule, the only lost effect (a bare key in an app with no `default` throttler) is reported by Decision 5.

4. **Values are validated and fail closed.** `limit` must deserialize as `u32`, `ttl` and `block_duration` as `u64`
   milliseconds, tracker and key generator as a string naming a registered function (Decision 7). A present value
   that does not makes the guard return `HttpException::from_anyhow` with the original's message shape
   (`Invalid "limit" for the "short" throttler: expected a number, got "abc".`), so the client gets the standard 500
   body and `BaseExceptionFilter` logs the message. JSON `null` counts as absent (it falls through to the controller,
   then the throttler), which is stricter than the original's "null skips the class" and needs no special case.
   Numeric strings (`"5"`), fractions, negatives and values above the type's range are rejected; the original
   coerces strings with `Number()` and accepts fractions and negatives.
   *Alternatives:* keep ignoring invalid values (today's behavior). Rejected: a mistyped tightening silently falls
   back to the looser throttler value. Coerce numeric strings like `Number()`. Rejected: replicating JavaScript number
   parsing (hex, exponents, `Infinity`, whitespace) for a typed API whose builder only takes integers; the 500 shows
   the problem on the first request.

5. **Overrides for unknown names are reported once per route.** On each request the guard scans the metadata keys of
   the handler and the controller for the five override prefixes; a key whose suffix is not a configured throttler
   name, or a bare key when no throttler is named `default`, makes it log one warning per route (controller and
   handler) through `Logger::new("ThrottlerGuard").warn(..)`, listing the unknown names and the configured ones. The
   scan reads a few map entries and takes the lock of a second warn-once set (same bound as #152's: the number of
   routes and gateway handlers) only when it finds an unknown name. The original is silent; logs are not part of the
   parity surface.
   *Alternatives:* a startup check over all routes. Rejected for the same reason as #152 Decision 6 (the guard does
   not see the route table). Silence. Rejected: `throttle("shrot").limit(1)` on a login route would leave it at the
   global limit with no signal.

6. **Module-level hooks on `ThrottlerStorage`.** `ThrottlerStorage` already carries the module options (`headers`,
   `ipv6_subnet_prefix`, block durations) and is what `for_root_storage` and `for_root_async` receive, so:
   - `with_tracker<F, Fut>(f) where F: Fn(ExecutionContext) -> Fut + Send + Sync + 'static, Fut: Future<Output =
     Result<String, HttpException>> + Send + 'static` (`getTracker(req, context)`: the request is
     `ctx.request()`; async like the original; an `Err` is returned by the guard as is, as a thrown error is in the
     original);
   - `with_key_generator<F>(f) where F: Fn(&ExecutionContext, &str, &str) -> String + Send + Sync + 'static`
     (`generateKey(context, tracker, throttlerName)`, synchronous like the original).
   Stored as `Arc<dyn Fn ..>` aliases `ThrottlerTrackerFn` and `ThrottlerKeyGeneratorFn`. A custom tracker replaces
   `normalize_ip` entirely, as in the original; the docs show calling `normalize_ip(&ctx.request().ip(), 64)` inside a
   tracker that falls back to the IP. Calling `with_tracker` twice keeps the last function.
   *Alternatives:* a `ThrottlerTracker` trait implemented by an injectable. Rejected: heavier than the original's
   function option and not needed to reach providers (a `for_root_async` factory can capture them). Fields on
   `ThrottlerOptions`. Rejected here: that is the per-throttler level (out of scope) and adding fields breaks the
   struct literals apps use.

7. **Route-level hooks by registered name.** Metadata is JSON, so a route cannot hold a closure. The storage gets a
   registry: `with_named_tracker(id, f)` and `with_named_key_generator(id, f)` (same bounds as Decision 6) store
   functions under an id; `ThrottleOverride::tracker(id)` / `::key_generator(id)` write the id under
   `THROTTLE_TRACKER<name>` / `THROTTLE_KEY_GENERATOR<name>`. Registering the same id twice panics with a message
   naming it (like `with_block_duration` with an unknown name). An id that is not registered, or a value that is not a
   string, answers 500 (Decision 4): the original fails the same way when the override is not a function (`TypeError`
   in `handleRequest`, a 500), and falling back to the IP tracker would silently drop an intended per-account limit.
   Resolution per throttler: route override (handler, then controller), then the module hook, then the built-in
   (`normalize_ip` tracker; key of Decision 8).
   *Alternatives:* a process-wide static registry. Rejected: hidden global state shared across test apps. Typed keys
   (`Token`) resolved through DI. Rejected: more machinery than a string id, and ids appear in logs and errors.

8. **Counters stay per throttler.** The guard stores each hit under
   `format!("{}:{}:{}", name.len(), name, generated)`, where `generated` is the key generator's output; the built-in
   generator returns `"{class}:{handler}:{tracker}"`, so the client-chosen part comes last, as in the original's
   `<Class>-<handler>-<name>-<tracker>`. The length prefix keeps names and generated keys from running into each
   other. A custom generator that ignores the name (`|_, tracker, _| tracker.into()`) therefore shares a counter across
   routes (the reason to have one) but never across throttlers. The original shares the reporting window and the block
   between throttlers in that case (Context, Storage), so `X-RateLimit-Reset-long` shows the first throttler's window
   and a block by `long` makes `short` answer `Retry-After-short`; Corvis keeps each throttler's own values. This is a
   documented divergence, recorded in the fixture and asserted as such (Decision 10).
   *Alternative:* pass the generated key through unchanged for parity. Rejected: the original's shared block state
   looks accidental, its headers name the wrong throttler, and Corvis's storage does not keep per-name counts under
   one key.

9. **Trackers are resolved per throttler.** As in the original, the guard calls the resolved tracker for each
   throttler that is not skipped (route overrides can differ per name). The built-in tracker is computed at most once
   per request, as today (`throttler.rs:722`). Hooks are expected to be cheap and side-effect free; the docs say so.

10. **Fixture cases.** `generate.mjs` gains new controllers (so the recorded values of existing cases do not move), a
    `send` field for request headers (`{ path, send: { "x-client": "a" } }`), and `record()` accepts a module options
    object besides a throttler array. Apps (all windows of 10 s or more, so values do not depend on timing):
    - `overrides-named` (`short` 3 per 30 s, `long` 5 per 60 s): `/ovr/short1` with `Throttle({ short: { limit: 1 } })`,
      two requests; `/ovr/both` with `Throttle({ short: { limit: 2, ttl: 10000 }, long: { limit: 1 } })`, two
      requests; a controller `ovr-class` with `Throttle({ short: { limit: 2, ttl: 20000 } })` whose handler `h` has
      `Throttle({ short: { limit: 1 } })` (two requests) and whose handler `sibling` has none (three requests);
      `/ovr/block` with `Throttle({ short: { limit: 1, blockDuration: 5000 } })`, two requests; `/ovr/unknown` with
      `Throttle({ shrot: { limit: 1 } })`, two requests; `/ovr/default` with `Throttle({ default: { limit: 1 } })`,
      two requests; `/ovr/invalid` with `Throttle({ short: { limit: 'abc' } })`, one request (500).
    - `overrides-default` (`default` 3 per 60 s, `short` 3 per 30 s): `/ovr/default`, two requests.
    - `module-tracker` (`default` 1 per 60 s, `getTracker: (req) => req.headers['x-client'] ?? req.ip`): `/plain` with
      `x-client` `a`, `a`, `b`; `/ovr/fixed` (`Throttle({ default: { getTracker: () => 'fixed' } })`) with `a` then `b`.
    - `module-key` (`default` 1 per 60 s, `generateKey: (ctx, tracker, name) => name + ':' + tracker`): `/plain`, then
      `/byname`; `/ovr/own-key` (`Throttle({ default: { generateKey: (ctx, t) => 'own:' + t } })`), one request.
    - `shared-key` (`short` 100 per 30 s, `long` 2 per 60 s, `generateKey: (ctx, t) => t`): `/plain` four times. This
      case is the divergence of Decision 8.
    The Rust replay declares the same routes with `throttle(..)`, the bare `THROTTLE_LIMIT` for `/ovr/default` (so the
    fixture also checks Decision 3), `throttle_key(THROTTLE_LIMIT, "short")` with `"abc"` for `/ovr/invalid`, and builds
    each app's storage from a table keyed by the case name (`with_tracker`, `with_named_tracker("fixed", ..)`,
    `with_key_generator`, `with_named_key_generator("own", ..)`). Every case must match except `shared-key`, which the
    replay lists as a known divergence and checks that the recorded values differ while Corvis's follow Decision 8.

## Risks / Trade-offs

- [Apps with only named throttlers lose their bare overrides] → Decision 5 warns once per route with the fix
  (`throttle("name")`); the docs state the behavior change next to #152's. No test, example or port app is affected.
- [Invalid values now answer 500 instead of being ignored] → the first request shows it; the builder takes integers,
  so only hand-written `#[set_metadata]` values can hit it. The docs list the accepted types.
- [A custom tracker can be chosen by the client] (a header, a body field) → clients rotate values and get fresh
  limits. The docs say to keep an IP-based throttler next to a per-account one (`default` per IP plus a named
  throttler whose route override uses `.tracker(..)`), and warn that a tracker returning one value for every
  anonymous request puts all of them in one counter, so one client can exhaust it for everybody.
- [A custom tracker skips IPv6 grouping] → as in the original; the docs show `normalize_ip` in the example tracker.
- [Hook panics] → a panic in a hook unwinds like a panic in any guard; not handled specially.
- [A throttler literally named `""`] → its per-name key equals the bare key; bare keys then apply to it and to
  `default`. Not worth a rule; `ThrottlerStorage::new` already warns on duplicate names only. Mentioned in rustdoc.
- [Per-request scan cost] → a handful of string prefix checks over the handler's and controller's metadata; no lock
  unless an unknown name is found.
- [Stored key layout changes] → counters are in memory and hashed, so the only effect is that counters reset on the
  deploy that ships this change, as on any restart.

## Open questions

1. Follow-up issues: should the out-of-scope items be filed now (per-throttler `getTracker`/`generateKey` in
   `ThrottlerOptions`; function-valued `limit`/`ttl`/`blockDuration`)? This change only notes them.

## Review notes (2026-11-09)

- Decisions 1-9 are approved as written, including the behavior change of Decision 3 (unnamed keys affect only
  `default`) and the documented divergences of Decisions 4 and 8.
- Open question: per-throttler `getTracker`/`generateKey` in `ThrottlerOptions` and function-valued
  `limit`/`ttl`/`blockDuration` are filed as #162 (phase 0.8).

## Implementation notes

- Task 1.1: re-read in `@nestjs/throttler` 6.7.1 and `@nestjs/core` 12.4.0 (`npm ci` from the committed lockfile, run in a
  scratch folder so no `node_modules` enters the repository). `throttler.decorator.js`, `throttler.guard.js` (per-option
  `getAllAndOverride` over `[handler, class]`, `toNumber`, `routeX || throttler.X || common.X`) and
  `reflector.service.js` (`getAllAndOverride` returns the first value that is not `undefined`) match the Context section.
- Task 1.2: the regenerated `throttler.json` only adds cases (447 added lines, 0 removed): the existing cases are
  byte-identical. `/ovr/invalid` recorded a 500 with no throttler headers. `shared-key` recorded
  `x-ratelimit-reset-long: 30` (the short window) on its first response and `retry-after-short: 60` on the fourth, as
  Decision 8 predicts. Recorded requests carry an optional `send` object (request headers) that the replay sends.
- A per-name entry that is JSON `null` counts as absent even for `default`, so a bare key on the same target then
  applies (the spec only fixes "no per-name entry"; null is absent everywhere, Decision 4).
- The tracker and key generator keys have no bare form (Decision 3 only covers the three numeric options). A bare
  `THROTTLER:TRACKER` or `THROTTLER:KEY_GENERATOR` key overrides nothing, and the unknown-name warning of Decision 5 also
  reports it (the spec lists only bare numeric keys; reporting this one is stricter and harmless).
- Error messages label the options as the original does (`limit`, `ttl`, `blockDuration`) and name the tracker and key
  generator options `tracker` and `keyGenerator`; the text after the colon is Corvis's own (`expected a non-negative
  integer within range, got "abc"`).
- The replay lists `shared-key` in `KNOWN_DIVERGENCES`: each of its responses is compared with the recording, the test
  requires at least one difference, and `counters_stay_separate_per_throttler` asserts Corvis's values (Decision 8).
- The route tests that need the process-wide logger (unknown-name warnings, the invalid-value error log, the gateway
  override) live in `tests/throttler_skip_log_ws.rs`; the other scenarios are in `tests/throttler_headers.rs`.
- Task 7.5 (corvis-parity) is not run here: the brief leaves it to the maintainer's session. No port app uses the
  throttler.

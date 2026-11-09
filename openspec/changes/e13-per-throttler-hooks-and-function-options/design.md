# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `eb8bed1` (`main`, which
holds #156); paths under `crates/corvis/` unless stated. Reference package: `@nestjs/throttler` 6.7.1 (`npm pack` into
the scratchpad; the fixture pins the same version, `tests/fixtures/throttler/package.json`).

**Corvis**

- `ThrottlerOptions { name, ttl: u64, limit: u32 }` derives `Debug, Clone`, has public fields and a `new(ttl, limit)`
  constructor (`src/modules/throttler.rs:182-196`); the docs and tests build named throttlers with struct literals
  (`docs/security/rate-limiting.md:62-63`, `tests/throttler_headers.rs`).
- `ThrottlerStorage` (`throttler.rs:507-519`) holds the module options: `throttlers`, `block_durations` per name,
  `headers`, `ipv6_subnet_prefix`, the module hooks `tracker` and `key_generator`, and the registries
  `named_trackers` / `named_key_generators` from #156. `new` sorts the throttlers by `ttl` with a stable sort
  (`:525-550`, `:526`). Per-name builder precedent: `with_block_duration(name, ms)` asserts that a throttler has that
  name and inserts into a map, so a second call for the same name replaces the first (`:596-600`). Module hooks:
  `with_tracker` (`:609-616`), `with_key_generator` (`:623-629`); registries `with_named_tracker` /
  `with_named_key_generator` panic on a duplicate id (`:636-662`).
- Hook types: `ThrottlerTrackerFn = Arc<dyn Fn(ExecutionContext) -> Pin<Box<dyn Future<Output = Result<String,
  HttpException>> + Send>> + Send + Sync>` and `ThrottlerKeyGeneratorFn = Arc<dyn Fn(&ExecutionContext, &str, &str) ->
  String + Send + Sync>`, built with `boxed_tracker` (`:166-180`); exported from `src/lib.rs:277-281`, not from the
  prelude.
- `ThrottlerGuard::can_activate` (`:1020-1063`): checks unknown overrides, computes the built-in client once
  (`:1026`), then for each throttler not skipped resolves `limit` and `ttl` as route override else the static value
  (`:1032-1033`), the block duration as route override, else `block_durations`, else the effective ttl
  (`:1034-1037`), the tracker as route override else module hook (`:1038-1041`), the key generator as route override
  (`:1042-1043`), calls the tracker (`:1044-1047`), and builds the stored key with a source marker: `n{len(id)}:{id}:`
  for a registered generator, `m:` for the module generator, `b:{class}:{handler}:` for the built-in key, all after
  `{len(name)}:{name}:` (`:1048-1056`). Then `record_hit`, headers, 429 on a block (`:1057-1061`).
- `record_hit_at` (`:706-786`) takes `ttl` and `block_duration` per call; each hit expires at `now + ttl` and the
  reporting window ends at the first hit's `now + ttl`, so a ttl that changes between requests is handled per hit.
- Fixture: `tests/fixtures/throttler/generate.mjs` (321 lines) records apps through `record(throttlers, requests,
  moduleOptions)` (`:143`); requests may carry headers (`send`, `:156-167`); #156 added `module-tracker`,
  `module-key` and `shared-key` (`:288-318`). The replay is `replays_the_original_library_fixture` in
  `tests/throttler_headers.rs`, which builds each case's storage from a table keyed by case name and lists known
  divergences and rejected routes.

**Original (`@nestjs/throttler` 6.7.1, `dist/`)**

- Types (`throttler-module-options.interface.d.ts:4-13`, `:36-37`): `Resolvable<T> = T | ((context) => T |
  Promise<T>)`; a throttler entry has `limit: Resolvable<number>`, `ttl: Resolvable<number>`,
  `blockDuration?: Resolvable<number>`, `getTracker?`, `generateKey?` (plus `skipIf`, `ignoreUserAgents`,
  `setHeaders`, out of scope).
- Sort (`throttler.guard.js:39-47`): `(first, second) => typeof first.ttl === 'function' ? 1 : typeof second.ttl ===
  'function' ? 0 : first.ttl - second.ttl`. Throttlers with a ttl function go after the static ones; between two
  function throttlers the comparator is inconsistent (it answers 1 both ways), so their relative order depends on the
  engine's sort.
- Hook chain (`:94-95`): `getTracker = routeOrClass || namedThrottler.getTracker || commonOptions.getTracker`, same for
  `generateKey`; `commonOptions` defaults to the guard's own methods (`:56-66`).
- Values (`:91-93`): `limit = toNumber(await resolveValue(ctx, routeOrClassLimit ?? namedThrottler.limit))`, same for
  `ttl`; `blockDuration = toNumber(await resolveValue(ctx, (routeOrClassBlockDuration ?? namedThrottler.blockDuration)
  ?? ttl))`. `resolveValue` calls a function with the context (`:189-191`), so a function is not called when a route
  override exists. `toNumber` (`:182-188`) coerces numeric strings and throws `Invalid "<option>" for the "<name>"
  throttler: expected a number, got <json>.` otherwise; the thrown `Error` becomes a 500. An error thrown by the
  function itself propagates the same way (an `HttpException` keeps its status).
- The key layout: `generateKey(context, tracker, name)` is used as is, and the storage keys hits by `(key, name)`
  (see #156's design, Context).

## Goals / Non-Goals

**Goals:**
- Each throttler entry of the original (`getTracker`, `generateKey`, function-valued `limit` / `ttl` / `blockDuration`)
  has a Corvis equivalent with the same precedence, checked against the original by the fixture.
- No change for applications that do not use the new builders; `ThrottlerOptions` struct literals keep compiling.

**Non-Goals:**
- Function values in route overrides, `skipIf`, `ignoreUserAgents`, `setHeaders` per throttler (proposal, Out of
  scope). Changing how hits are counted or blocked.

## Decisions

1. **Per-name builders on `ThrottlerStorage`, not fields on `ThrottlerOptions`.** `with_tracker_for(name, f)`,
   `with_key_generator_for(name, f)`, `with_limit_fn(name, f)`, `with_ttl_fn(name, f)` and
   `with_block_duration_fn(name, f)`, stored in maps keyed by throttler name. They follow `with_block_duration(name,
   ms)`, the existing way to set a per-throttler option that `ThrottlerOptions` does not carry: each asserts that a
   configured throttler has that name (panic message `ThrottlerStorage::<builder>: no throttler is named `<name>``),
   and a second call for the same name replaces the first.
   *Alternatives:* `Option<..>` fields on `ThrottlerOptions`. Rejected: adding fields breaks every struct literal
   (documented usage), and closures are neither `Debug` nor meaningfully `Clone`, which `ThrottlerOptions` derives.
   An enum `Value::Fixed(u64) | Value::Fn(..)` for `ttl` / `limit`. Rejected: same break, for every user. Builder
   methods on `ThrottlerOptions` that keep private fields. Rejected: still needs the new fields.
   Names: `_for(name)` marks the per-throttler level of the existing hooks (`with_tracker`, `with_key_generator`),
   distinct from `with_named_tracker(id)`, whose "named" refers to a registry id; `_fn` marks the function form of an
   option that already has a value form.
2. **Precedence.** Per throttler and request: tracker and key generator are route override (handler, then
   controller), then the throttler's hook, then the module hook, then the built-in, as the original's `||` chain.
   `limit` and `ttl` are route override, then the throttler's function, then the static value; the block duration is
   route override, then the function, then `with_block_duration`, then the effective ttl. A function value takes
   precedence over the static value of the same option whatever the call order of the builders, so the outcome does
   not depend on how a `for_root_async` factory orders its calls. A function is not called when a route override
   applies, nor for a skipped throttler, as in the original.
3. **Typed results replace the runtime number check.** Value functions have the type
   `ThrottlerValueFn<T> = Arc<dyn Fn(ExecutionContext) -> Pin<Box<dyn Future<Output = Result<T, HttpException>> +
   Send>> + Send + Sync>` with `T = u32` for the limit and `u64` (milliseconds) for ttl and block duration; the
   builders take `F: Fn(ExecutionContext) -> Fut, Fut: Future<Output = Result<T, HttpException>> + Send + 'static`,
   like `with_tracker`. The original's `toNumber` errors (a non-number, `NaN`) cannot happen, and #156's override
   validation (non-negative integers in range) is guaranteed by the type, which is what the issue's "same validation
   as #156" asks for. An `Err` is returned by the guard as is, before `record_hit` for that throttler: a
   `from_anyhow` error answers the standard 500 body and is logged by the exception filter, an `HttpException` keeps
   its status, as an error thrown by the original's function. Values that are valid but extreme (`ttl` of
   `u64::MAX`, `limit` 0) behave as the same static values do today (durations saturate, a limit of 0 rejects).
   *Alternatives:* functions returning `serde_json::Value` checked like override metadata. Rejected: the check would
   exist only to reject values a typed API cannot produce. Synchronous functions. Rejected: the original accepts
   promises, and a limit from the user's plan usually needs an async lookup; the cost is one boxed future per call,
   as for trackers.
4. **Check order with ttl functions.** Throttlers with a ttl function are checked after the static ones, in configured
   order; the static ones keep the ascending-ttl stable order. `ThrottlerStorage` keeps each throttler's configured
   index so `with_ttl_fn` can re-sort (static ttl ascending, configured index for ties; function throttlers last, by
   configured index). This matches the original for every case its comparator decides consistently (one function
   throttler, or several whose engine order happens to keep the configured order). The static `ttl` of a throttler
   with a ttl function is not used for anything; the rustdoc says so.
   *Alternative:* keep sorting by the static `ttl`. Rejected: a migrated app gets a different check order, and with
   it different headers and a different throttler counting first (#156's security note: the IP throttler only stops a
   rotating client tracker when it is checked first).
5. **A source marker for per-throttler key generators.** The stored key for a per-throttler generator is
   `{len(name)}:{name}:t:{generated}`, next to `b:`, `m:` and `n{len(id)}:{id}:` from #156. Keys from the four sources
   never meet, so a client-chosen tracker cannot be crafted into another source's counter, which is the property #156's
   security review asked for. The built-in key is used when a throttler has a per-throttler tracker but no key
   generator, as with module trackers.
6. **The built-in tracker is still computed at most once per request**, and each throttler calls its resolved tracker
   (route, throttler, module) as today. Value functions run once per throttler and request; the docs ask for cheap
   functions without side effects, as for trackers.
7. **Fixture cases.** `generate.mjs` gains a `Ptr` controller (routes `/ptr/fixed`, `/ptr/own-key`, `/ptr/limit5`,
   `/ptr/pro`) so the recorded values of existing cases do not move, and these apps (windows of 10 s or more):
   - `throttler-tracker`: module `getTracker: (req) => 'm:' + (req.headers['x-client'] ?? 'none')`; `short` (1 per
     30 s) with `getTracker: (req) => 't:' + (req.headers['x-account'] ?? 'none')`, `long` (3 per 60 s). `/plain` with
     (`x-client` a, `x-account` 1), (b, 1), (a, 2); `/ptr/fixed` (`Throttle({ short: { getTracker: () => 'fixed' } })`)
     with `x-account` 3 then 4.
   - `throttler-key`: module `generateKey: (ctx, t) => ctx.getHandler().name + ':' + t`; `default` (1 per 60 s) with
     `generateKey: (ctx, t) => 'shared:' + t`. `/plain`, then `/byname` (shared, 429), then `/ptr/own-key`
     (`Throttle({ default: { generateKey: (ctx, t) => 'own:' + t } })`, 200).
   - `dynamic-limit`: `default` (`ttl: 60000`, `limit: (ctx) => header x-plan === 'pro' ? 3 : 1`, `blockDuration:
     async () => 20000`). `/plain` twice without the header; `/ptr/pro` four times with `x-plan: pro`; `/ptr/limit5`
     (`Throttle({ default: { limit: 5 } })`) once.
   - `dynamic-ttl-order`: `a` (60 s, limit 2) and `b` (`ttl: () => 10000`, limit 1). `/plain` three times.
   - `function-error`: `default` (`ttl: 60000`, `limit: (ctx) =>` throws `new Error('boom')` with `x-boom`, throws
     `new ForbiddenException()` with `x-forbid`, else 1). `/plain` with `x-boom`, with `x-forbid`, then plain.
   The Rust replay builds each storage from the case table (`with_tracker_for`, `with_key_generator_for`,
   `with_limit_fn`, `with_ttl_fn`, `with_block_duration_fn`) and declares the `Ptr` routes with `throttle(..)` and
   named hooks (`fixed`, `own`). Every case must match; the cases record only statuses and throttler headers, so the
   500 body difference (Corvis's standard body) is not compared. Function-valued route overrides and non-numeric
   function results are not recorded (Out of scope; not representable).

## Risks / Trade-offs

- [A limit or ttl function that reads client-controlled input] (`x-plan: pro`) → the client chooses its own limit. The
  docs say to derive values from server-side state (the authenticated user, looked up by the function) and show that
  example rather than a header.
- [A ttl that changes between requests for one key] → each hit expires after its own ttl and the reporting window
  follows the first hit (Context, `record_hit_at`); the docs ask for a ttl that depends on stable properties of the
  client.
- [Function throttlers checked last] → an IP throttler with a ttl function no longer runs first; the docs repeat
  #156's advice (keep the IP throttler static with the shortest ttl).
- [Slow value functions] → they run on every request for every throttler not skipped; the docs say to cache lookups.
- [Hook panics] → unwind like a panic in any guard, as for #156's hooks.

## Open questions

None. The technical choices, including the public builder names, are recorded above and in the review notes.

## Review notes (2026-11-09)

- Drafted without a maintainer round. Decided here: builders on `ThrottlerStorage` keyed by throttler name instead of
  new `ThrottlerOptions` fields (Decision 1, keeps struct literals compiling); names `with_tracker_for`,
  `with_key_generator_for`, `with_limit_fn`, `with_ttl_fn`, `with_block_duration_fn` and the alias
  `ThrottlerValueFn<T>`; typed async functions instead of a runtime number check (Decision 3); ttl-function throttlers
  checked last like the original (Decision 4); a `t:` source marker for per-throttler generators (Decision 5);
  function values in route overrides left out of scope.

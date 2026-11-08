# Design

## Context

See proposal.md for motivation; the behavior is in the three spec deltas. Current state, verified in the code:

**Enhancer overrides in tests**

- `TestingModuleBuilder::override_guard::<G>` (`crates/corvis/src/testing.rs:124`) and its siblings register a
  provider under `enhancer_override_token::<G>()` (`testing.rs:60`, token `__corvis_override:<type name>`) in
  `Overrides::enhancers`. `Scanner::scan` (`crates/corvis/src/di/container.rs:202`) adds those providers to the
  internal core module and exports them.
- The only consumer of that token is the lazy enhancer factory of `guard_of::<E>()` and siblings
  (`crates/corvis/src/common/enhancer.rs:49`), used for `#[use_guards(E)]` and friends. Nothing else reads it.
- `Provider::app_guard::<T>()` (`crates/corvis/src/di/provider.rs:259`) is a `use_class::<T>` provider with token
  `APP_GUARD:<type name>` and `enhancer: Some(GlobalEnhancer::Guard(cast))`, where `cast` is `downcast::<T>` to
  `Arc<dyn CanActivate>`. `app_interceptor`, `app_pipe` and `app_filter` (`provider.rs:266-284`) are the same.
- `CorvisApplication::init()` (`crates/corvis/src/application.rs:364-397`) walks `container.all_providers()` and, for
  every provider with `enhancer`, takes its instance and casts it; if the instance is missing or the cast fails, the
  enhancer is skipped silently (`let Some(..) else { continue }` and `if let Some(g) = f(&instance)`).
- So `override_guard::<RolesGuard>` does not touch `Provider::app_guard::<RolesGuard>()`; the docs say so in a warning
  (`docs/fundamentals/testing.md:133-135`).
- Overriding the `app_*` provider itself goes through `Scanner::add_provider` (`container.rs:93-109`): it clones the
  override provider and only copies the token back, so `enhancer` becomes `None` and the global enhancer disappears.
  Even if `enhancer` were kept, `cast` would fail for any value of another type and the guard would still be skipped.
- The existing override test is `crates/corvis/tests/app.rs:356-375` (`override_guard::<RolesGuard>` on a guard used
  through `#[use_guards]`).

**Global filter order**

- In `init()`, guards, interceptors and pipes registered as providers are inserted at an increasing index
  (`guard_at`, `interceptor_at`, `pipe_at`), which keeps registration order ahead of `use_global_*`. Filters use
  `global_filters.insert(0, x)` (`application.rs:391-395`), so each new provider filter goes before the previous
  one: registration order is reversed.
- `build_router` (`crates/corvis/src/router/mod.rs:321-322`) builds route filters as method, controller, then
  `cfg.global_filters`; `handle_exception` (`router/mod.rs:745-761`) picks the first filter whose `catches` is true.
- Docs: `docs/exception-filters.md:195-201` and `docs/faq/request-lifecycle.md:75-83` describe provider filters
  first, then `use_global_filters` in call order, without saying the order among provider filters.
- `crates/corvis/tests/enhancer_order.rs` covers interceptor order only.

**Global prefix exclusions**

- `set_global_prefix_with_exclude` (`crates/corvis/src/application.rs:152`) stores the patterns in
  `RouterConfig::prefix_exclude`.
- `build_router` decides the prefix once per controller path (`router/mod.rs:301`):
  `prefix_exclude.iter().any(|e| path_matches(e, c_path))`, so the handler path is never considered.
- `path_matches` (`crates/corvis/src/common/middleware.rs:230`) is the matcher shared with middleware routes.
- `docs/faq/global-prefix.md:28-33` documents the limitation in a warning.
- Uses: `crates/corvis/tests/express_parity.rs:115` and the facten port (`set_global_prefix_with_exclude("api/v1",
  &["health"])`, whose `#[controller("health")]` has a single `#[get]`, so its route `/health` matches both before
  and after this change).

**Middleware routes under URI versioning**

- `build_middleware` (`router/mod.rs:397-437`) compiles each `RouteSpec::Path` text pattern to
  `join_paths(&[global_prefix, pattern])` (line 421); the version segment is never added. `RouteSpec::Controller`
  uses the final paths of the compiled routes (lines 424-428), which already include the version.
- Route paths are built as `join_paths(&[prefix, version_path, c_path, r_path])` (line 348) with
  `version_paths` (lines 248-255) producing `<prefix><version>` for URI versioning.
- `docs/middleware.md:145-147` documents the limitation in a warning.

**Optional parameters**

- `to_matchit` (`router/mod.rs:73-96`) turns `:name?` into `{name}` (`trim_end_matches('?')`, line 80), so the
  route is registered with a required parameter. The unit test at `router/mod.rs:789-791` covers plain params only;
  `fuzz_tests.rs:211-213` only checks `to_matchit` does not panic.
- Missing params already become `Value::Null` in handler arguments (`router/mod.rs:659-662`), so an
  `Option<String>` parameter receives `None` when the segment is absent.
- Route conflicts are reported by `RouterTable::insert` as an error log (`router/mod.rs:186`), not a startup error.
- `corvis-swagger` already strips `?` for documentation (`crates/corvis-swagger/src/explorer.rs:691-709`).

## Goals / Non-Goals

**Goals:**
- No enhancer, middleware or route is dropped silently by the five cases in the issue.
- Each fix has a regression test that fails on the current code.
- The docs warnings that described these limitations are replaced by the new behavior.

**Non-Goals:**
- Method-aware prefix exclusions, version-targeted middleware routes, the `{/:id}` syntax (see proposal, Out of scope).
- Changing how route conflicts are reported.

## Decisions

1. **Global enhancer overrides reuse the existing override token.** `Provider::app_guard::<T>()` and siblings also
   record `enhancer_override_token::<T>()` on the provider (a new `pub(crate)` field next to `enhancer`, or inside
   the `GlobalEnhancer` variant). In `init()`, before casting the provider's own instance, the loop looks up that
   token in the core module (where `Scanner::scan` puts enhancer overrides) and, if present, uses the override value
   (stored as `Arc<dyn CanActivate>` etc. by `override_guard`) at the same position. The original `T` is still
   constructed by DI (its dependencies must resolve) but never called.
   Alternative considered: make `override_guard::<G>` also insert into `Overrides::providers` under the
   `APP_GUARD:<G>` token. Rejected: the value is a `dyn CanActivate` mock, not a `G`, so the existing `downcast::<T>`
   cast would fail; it would also construct the mock under two tokens.
2. **Overriding an `app_*` provider keeps the enhancer and fails loudly when it cannot.** `Scanner::add_provider`
   copies `enhancer` (and the new override token) from the original provider to the override, as it already does
   with the token. The cast stored in `GlobalEnhancer` tries `downcast::<T>` first and then the trait object the
   testing API stores (`Arc<dyn CanActivate>` and siblings, via the existing `downcast` helper on the erased
   instance). If both fail, or the instance is missing, `init()` returns an error such as
   `global guard APP_GUARD:<type> could not be applied: the provider value is not a guard` instead of `continue`.
   Alternative considered: keep skipping and log a warning. Rejected: a missing guard is a security-relevant silent
   failure, and tests are where it would be hidden.
3. **Provider filters in registration order.** Replace `insert(0, x)` with an increasing `filter_at` index, the same
   pattern as guards, interceptors and pipes. Provider filters still come before `use_global_filters` ones, as
   documented. See Open questions about the original framework's order.
4. **Prefix exclusion per route.** Move the decision inside the route loop: for each `(c_path, r_path)` compute
   `relative = join_paths(&[c_path, r_path])` and use `prefix = ""` when any exclusion `path_matches` it. The version
   segment is not part of `relative`, so exclusions keep working when URI versioning is on. The controller-level
   resolver log line (line 302-307) keeps using the controller decision for display only, or is computed from the
   first route; this is cosmetic and must not change the routes. Behavior change: a pattern equal to a controller
   path no longer excludes its sub-routes; `"health/*"` does (whole-segment `*` matches zero or more segments in
   `path_matches`). Documented in the docs page and the change summary.
   Alternative considered: match either the controller path or the full path. Rejected: it keeps the surprise
   (`"health"` excludes `/health/live`) and differs from how `for_routes` patterns work.
5. **Version-aware middleware patterns by expansion.** In `build_middleware`, when `cfg.versioning` is URI, collect
   the set of version path segments actually published (`version_paths` of every compiled route's `versions`, which
   already includes the default version), and compile each text pattern to the unversioned path plus one path per
   version: `join_paths(&[prefix, "<vp><v>", pattern])`. Wildcard-only patterns (`*`) stay as they are. `exclude`
   uses the same compilation, so exclusions cover versions too. `RouteSpec::Controller` is unchanged.
   Alternative considered: teach `path_matches` to skip an optional version segment. Rejected: the matcher is shared
   with prefix exclusions and would need versioning knowledge; expansion keeps it a pure pattern matcher.
6. **Optional parameters by route expansion.** A new helper expands a path with `:name?` segments into every
   combination with and without those segments (2^n paths), each converted with `to_matchit`; `build_router`
   registers the same route indexes under each expanded matchit path. `to_matchit` stops silently dropping `?` for
   paths that reach it unexpanded (it becomes an internal invariant, checked by a unit test). Cap: more than 4
   optional segments in one path is a startup error (16 variants), which no real route needs.
   Alternative considered: reject `:id?` at startup and ask for two routes. Rejected: the docs reviewers expected
   it to work, and code migrated from Express 4 based apps uses it.
   The startup log (`Mapped {...}`) prints the declared path once, with the `?`.

## Risks / Trade-offs

- [Prefix exclusion behavior change breaks an app relying on controller-level exclusion] → documented, with the
  `"name/*"` replacement; the only known users (facten port, `express_parity.rs`) are unaffected; the parity check
  confirms it.
- [Expanding middleware patterns per version adds entries] → one per published version per pattern; negligible.
- [Optional segment expansion creates conflicts with explicit routes] → reported like any other conflicting route
  (`Unable to register route` error log, `router/mod.rs:186`); a test documents it.
- [Failing `init()` on an incompatible override could break an existing test suite] → only tests that override an
  `APP_*` token with a non-enhancer value, which today silently run without the enhancer; that is the bug.
- [Changing filter order changes which provider filter answers] → only apps with two or more `app_filter`s that
  catch the same exception; none of the port apps register `app_filter`.

## Open questions

- **Original order of global filters.** The issue treats reverse order as a bug and this change specifies
  registration order. The original Node framework is believed to try the combined global, class and method filter
  list in reverse (last bound first), which would also put `use_global_filters` ahead of provider filters. Task 1.1
  checks this with a small differential test against the original; if it confirms reverse order, the maintainer must
  decide between parity (reverse, and update `docs/exception-filters.md`) and the issue's expectation before 2.x is
  implemented.
- **Version segment in prefix exclusions.** This change matches exclusions without the version segment. Confirm
  this is the intended rule (the alternative is matching `v1/health`).
- **Optional segment cap.** 4 optional segments per path is an arbitrary limit; confirm or pick another.

## Review notes (2026-11-08)

- Parity first: where the original Node framework's order or matching differs from the issue's wording (global filter
  order, prefix exclusion matching), the spec follows the original, verified in task 1.1 against it, and the result is
  recorded here. Diverge only for security.
- Prefix exclusions compare without the version segment, as proposed.
- The cap of 4 optional segments per route is accepted; exceeding it is a startup error with the route name.

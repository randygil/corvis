# Proposal

Implements [randygil/corvis#17](https://github.com/randygil/corvis/issues/17) (epic [#13](https://github.com/randygil/corvis/issues/13)).

## Why

The docs review found five places where enhancers and routing behave differently from what a user migrating an
existing application expects, and in two of them the docs had to add a warning instead of the framework doing the
right thing:

- Testing overrides (`override_guard` and its siblings) cannot replace a global enhancer registered with
  `Provider::app_guard` (and equivalents), and overriding the `app_*` provider itself silently removes the global
  enhancer, so a test can pass while the guard it meant to mock is simply gone.
- Several `Provider::app_filter` are tried in reverse registration order (the issue treats this as a bug; the original
  framework does the same, see design.md).
- `set_global_prefix_with_exclude` compares the exclusions with the controller path only, so a single route cannot
  be kept outside the prefix.
- With URI versioning, a text pattern in `for_routes("cats")` does not match `/v1/cats`, so module middleware is
  silently skipped on versioned routes.
- An optional route parameter `:id?` loses its `?` and becomes required.

Each one fails silently (a guard dropped, a middleware skipped, a route answering 404), which is exactly the kind of
mistake the framework should not let through.

## What Changes

- `override_guard::<G>`, `override_interceptor::<I>`, `override_pipe::<P>` and `override_filter::<F>` also replace a
  global enhancer registered with `Provider::app_guard::<G>()` (and equivalents) in the testing module.
- Overriding the provider behind an `app_*` registration keeps it registered as a global enhancer; if the override's
  value cannot be used as that enhancer, `init()` fails with a clear error instead of dropping it.
- Global exception filters are tried last registered first, as in the original framework (checked in task 1.1): the
  `use_global_filters` ones from the last call to the first, then the `Provider::app_filter` ones from the last to the
  first. Guards, interceptors and pipes keep registration order. **Behavior change:** a bootstrap filter now comes
  before the provider filters (it came after).
- `set_global_prefix_with_exclude` matches each exclusion against the full route path (controller path plus handler
  path, without the prefix), so a single endpoint can be excluded. **Behavior change:** an exclusion `"health"` no
  longer excludes `/health/live`; write `"health/*"` for that.
- With URI versioning, text patterns given to `for_routes` and `exclude` of module middleware also match the route at
  each URI version it is published under (`for_routes("cats")` covers `/v1/cats` and `/v2/cats`).
- A route parameter marked optional (`:id?`) registers the route both with and without that segment; the handler
  receives no value for the parameter when the segment is absent.
- Docs (EN/ES) drop the warnings that described the old behavior and describe the new one.

## Capabilities

### New Capabilities

- `core/enhancers`: order of global enhancers registered as providers, in particular exception filters.
- `core/testing`: enhancer overrides in testing modules, including global enhancers registered as providers.
- `http/routing`: global prefix exclusions, module middleware route matching under URI versioning, and optional
  route parameters.

### Modified Capabilities

None (no spec covers these areas yet).

## Impact

- `crates/corvis/src/testing.rs`, `crates/corvis/src/di/provider.rs`, `crates/corvis/src/di/container.rs`,
  `crates/corvis/src/application.rs` (enhancer registration in `init()`).
- `crates/corvis/src/router/mod.rs` (`build_router`, `build_middleware`, `to_matchit`).
- `crates/corvis-swagger`: no change expected (it already strips `?`; see Out of scope).
- Docs: `fundamentals/testing.md`, `exception-filters.md`, `faq/request-lifecycle.md`, `faq/global-prefix.md`,
  `middleware.md`, `controllers.md` and their Spanish translations under `docs/es/`.
- Port apps: facten uses `set_global_prefix_with_exclude("api/v1", &["health"])` with a single `#[get]` in
  `#[controller("health")]`, so its routes are unchanged; facten registers two `app_guard`s (guards were already in
  registration order). The parity check confirms it.

## Out of scope

- Method-specific global prefix exclusions (`{ path, method }` items): the signature stays `&[&str]`; possible new issue.
- Targeting one version from middleware (`RouteInfo { version }` / `RouteSpec::path(..).version(..)`): possible new issue.
- Text patterns in `for_routes` for routes excluded from the global prefix (`for_routes("health")` still compiles to
  `/<prefix>/health`): possible new issue.
- Express 5 brace syntax for optional segments (`{/:id}`) and optional segments that are not a whole parameter
  segment: possible new issue.
- OpenAPI output for optional parameters (`corvis-swagger` emits them as required path parameters, `explorer.rs`
  around line 707), which matches what OpenAPI allows for path parameters.
- Overriding enhancers passed as an expression (`#[use_guards(RolesGuard::new("admin"))]`) or registered with
  `use_global_*`: unchanged, still documented as not overridable.

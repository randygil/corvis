# Proposal

Implements [randygil/corvis#119](https://github.com/randygil/corvis/issues/119) (epic [#13](https://github.com/randygil/corvis/issues/13)).
Design notes come from the #23 spike, feature 5
(`openspec/changes/archive/2026-11-08-e13-missing-core-features-users-expect/design.md`).

## Why

`ConfigModule`, `CacheModule` and `ThrottlerModule` only take their options as values, so the options cannot come
from `ConfigService` or any other provider: a cache TTL or a rate limit read from the environment has to be computed
before the module graph is built, outside DI. `JwtModule`, `MulterModule`, `ScheduleModule` and `OrmModule` already
have async variants, and users of the original framework expect `forRootAsync` / `registerAsync` on these three
modules as well. The existing variants also disagree on their signatures, and the docs do not say which shape to
expect.

## What Changes

- `ConfigModule::for_root_async(is_global, factory)`: the factory returns `ConfigModuleOptions`; `ConfigService` is
  built from them.
- `CacheModule::register_async(is_global, factory)`: the factory returns `CacheOptions`; `Arc<CacheManager>` and the
  `CACHE_MANAGER` token resolve to the same instance.
- `ThrottlerModule::for_root_async(factory)`: the factory returns anything `Into<ThrottlerStorage>` (a
  `Vec<ThrottlerOptions>` or a configured `ThrottlerStorage`); the module stays global. `ThrottlerStorage` gains
  `From<Vec<ThrottlerOptions>>`.
- A documented convention for async variants: named after the synchronous method plus `_async`; arguments
  `(is_global, factory)` when the module has an `is_global` option, `(factory)` when it is always global or never
  global; extra imports added with `.import(...)` on the returned `DynamicModule`.
- Tests: each variant reads a value from another provider (`ConfigService` for Cache and Throttler); a factory error
  aborts bootstrap with the usual `Error while instantiating <token>: <cause>` message.
- Docs (EN + ES): configuration, caching, rate limiting, and the dynamic modules page (convention and table).

## Capabilities

### New Capabilities

- `core/modules`: async registration of the built-in dynamic modules `ConfigModule`, `CacheModule` and
  `ThrottlerModule`, and the signature convention for async variants.

### Modified Capabilities

None (no spec covers built-in modules yet).

## Impact

- `crates/corvis/src/config.rs` (`ConfigModule::for_root_async`).
- `crates/corvis/src/modules/cache.rs` (`CacheModule::register_async`).
- `crates/corvis/src/modules/throttler.rs` (`ThrottlerModule::for_root_async`, `From<Vec<ThrottlerOptions>> for
  ThrottlerStorage`).
- New tests in `crates/corvis/tests/modules.rs`.
- Docs: `application/configuration.md`, `data/caching.md`, `security/rate-limiting.md`,
  `fundamentals/dynamic-modules.md` and their Spanish translations under `docs/es/`.
- Purely additive: no existing signature changes, so the port apps (consi only uses `ConfigModule::for_root`) are
  unaffected.

## Out of scope

- Async `load` factories for `ConfigModule` (`register_as` returning a future): excluded in the spike review.
- `use_class` / `use_existing` options factories (`ThrottlerOptionsFactory`, `CacheOptionsFactory` in the original):
  the closure factory covers them; possible new issue.
- Renaming the existing async variants (`JwtModule::register_async(global, ..)` names its flag `global`,
  `ScheduleModule::for_root_async(imports, ..)` takes imports first): they already fit the convention in shape, and
  changing them would break callers; possible new issue for `ScheduleModule` if the maintainer wants one shape.
- The units of `CacheOptions.ttl` (#18), throttler headers (#120) and the throttler block model (#135). Both
  throttler changes configure the storage (`with_headers`, `with_block_duration`), so a factory returning a configured
  `ThrottlerStorage` reaches them with no change here.

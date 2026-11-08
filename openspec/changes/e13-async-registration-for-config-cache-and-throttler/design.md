# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code:

**The three modules**

- `ConfigModule::for_root(options)` (`crates/corvis/src/config.rs:275-284`) captures the options by value and
  registers `ConfigService` with `Provider::use_factory`, whose closure is already async and calls
  `ConfigService::load(&options)` (errors mapped with `anyhow::Error::msg`). Globality comes from
  `options.is_global` (line 276). `ConfigModuleOptions` (`config.rs:42-61`) is `Clone` and has `is_global`,
  `env_file_path`, `ignore_env_file`, `ignore_env_vars`, `load`, `validate`, `expand_variables`.
- `CacheModule::register(options)` (`crates/corvis/src/modules/cache.rs:152-163`) builds one `Arc<CacheManager>`
  eagerly and registers it twice with `Provider::use_value_arc`, under `Token::of::<CacheManager>()` and
  `CACHE_MANAGER` (`cache.rs:31`), so both resolve to the same instance. `CacheOptions` (`cache.rs:37-45`, default at lines 47-51) has `ttl`,
  `max` and `is_global`.
- `ThrottlerModule::for_root(throttlers)` and `for_root_storage(storage)` (`crates/corvis/src/modules/throttler.rs:141-155`)
  register a `ThrottlerStorage` with `Provider::use_value` and always call `.global(true)`. `ThrottlerGuard`
  (`throttler.rs:158-162`) injects `Arc<ThrottlerStorage>`. There is no `From<Vec<ThrottlerOptions>>` for
  `ThrottlerStorage` (only `ThrottlerStorage::new`, `throttler.rs:76-78`).

**Existing async variants**

- `JwtModule::register_async(global, factory)` (`crates/corvis/src/jwt.rs:342-355`): `use_factory` on
  `Token::of::<JwtService>()`.
- `MulterModule::register_async(factory)` (`crates/corvis/src/multer/mod.rs:262-275`): the module has no globality
  option.
- `ScheduleModule::for_root_async(imports, factory)` (`crates/corvis/src/modules/schedule/mod.rs:573`).
- `OrmModule::for_root_async(is_global, factory)` (`crates/corvis-orm/src/module.rs:223-233`).
- All take `F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static`, `Fut: Future<Output = corvis::Result<Options>> + Send +
  'static`.

**DI facts the design relies on**

- `Provider::use_factory` (`crates/corvis/src/di/provider.rs:267-277`) wraps `use_factory_arc` (lines 280-303), which
  turns any non-DI error into `DiError::Factory { token, cause }` (line 298), displayed as
  `Error while instantiating {token}: {cause}` (`crates/corvis/src/di/error.rs:35-36`); `docs/faq/common-errors.md:82-86`
  documents that message for `register_async` / `for_root_async`.
- `Provider::use_existing(token, existing)` (`provider.rs:308-320`, `alias_of` set at line 318) resolves `existing` and returns the same instance.
- `resolve_provider` (`crates/corvis/src/di/container.rs:458`) returns `DiError::Circular` when a provider's
  resolution path already contains its own key (lines 469-473), so a `ConfigService` factory that asks for
  `ConfigService` fails with the circular dependency error instead of hanging.
- Each `ModuleImport::Dynamic` creates a new module node (`container.rs:156-166`); `DynamicModule::import`
  (`crates/corvis/src/di/module.rs:131`) adds imports to it, and the factory's `ResolveCtx` belongs to that node.
- `docs/fundamentals/dynamic-modules.md:107-156` explains the async pattern and lists the built-in modules; the table
  (lines 146-156) shows no async variant for Config, Cache or Throttler.

**Original framework.** `ThrottlerModule.forRootAsync({ imports, useFactory, inject, useClass, useExisting })` lives on
a `@Global()` module (read from the `@nestjs/throttler` repository, master branch, on 2026-11-08). For the other two modules the only point that
matters here is that globality cannot depend on the factory's result in Corvis (Decision 2); no differential test is
needed because the variants have no observable output beyond the options they build (spike, Decision 4).

## Goals / Non-Goals

**Goals:**
- Options of the three modules can come from any provider, with the same factory shape as the existing variants.
- One written convention for async variants, applied to the new ones and documented.

**Non-Goals:**
- Async `load` factories, options-factory classes, or renaming existing async variants (proposal, Out of scope).

## Decisions

1. **Same factory type as the existing variants.** `F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static`,
   `Fut: Future<Output = corvis::Result<Options>> + Send + 'static`, stored in an `Arc` and called inside
   `Provider::use_factory`, as `JwtModule::register_async` does. Errors therefore surface as `DiError::Factory` with the
   token of the provider being built, with no new error type.
   Alternative considered: a builder (`ConfigModule::for_root_async().imports(..).factory(..)`). Rejected: no other
   module uses one, and `.import(...)` on `DynamicModule` already covers imports.
2. **Globality is an argument, not a field of the result.** The module graph (and so which modules are global) is
   fixed in `Container::scan` before any factory runs, so `is_global` must be known when the `DynamicModule` is built.
   The `is_global` field of `ConfigModuleOptions` / `CacheOptions` returned by the factory is ignored, and the rustdoc
   of both `*_async` methods says so.
   Alternative considered: log a warning when the returned field differs from the argument. Left as an Open question:
   `Default::default()` sets it to `false`, so a warning would fire for most `for_root_async(true, ..)` callers.
3. **Cache: one factory provider plus an alias.** `register_async` registers `Provider::use_factory(Token::of::<CacheManager>(),
   ..)` that builds `CacheManager::new(options)`, and `Provider::use_existing(CACHE_MANAGER, Token::of::<CacheManager>())`;
   both are exported. The factory therefore runs once and both tokens give the same `Arc`.
   Alternative considered: two factory providers. Rejected: two managers with separate entries, the bug the
   synchronous version avoids by sharing one `Arc`.
4. **Throttler: generic over `Into<ThrottlerStorage>`.** `for_root_async<F, Fut, S>(factory)` with
   `Fut: Future<Output = corvis::Result<S>>`, `S: Into<ThrottlerStorage> + Send + 'static`, plus
   `impl From<Vec<ThrottlerOptions>> for ThrottlerStorage` (delegating to `ThrottlerStorage::new`). One method covers
   both `for_root` and `for_root_storage` shapes. Tests cover a closure returning `Ok(vec![...])` and one returning
   `Ok(ThrottlerStorage::new(..).with_max_entries(..))`, so type inference is checked for both.
   Alternative considered: `for_root_async` and `for_root_storage_async`. Rejected: two methods for one concept.
5. **Config: the factory replaces only the options.** The provider closure awaits the factory and then calls the same
   `ConfigService::load(&options)` as `for_root`, so `.env` loading, `validate` and `expand_variables` behave
   identically. The factory cannot depend on `ConfigService` (it is the provider being built); the test for this
   variant reads its value from another provider, and a second test pins the circular dependency error for the
   self-reference.
6. **Convention, written down.** `_async` suffix on the synchronous name; `(is_global, factory)` when the module has an
   `is_global` option, `(factory)` otherwise; `.import(...)` for extra imports. `JwtModule` (flag named `global`) and
   `OrmModule` already fit; `MulterModule` fits; `ScheduleModule::for_root_async(imports, factory)` predates the
   convention and is listed as an exception in the docs table (renaming it is out of scope).

## Risks / Trade-offs

- [Generic `S: Into<ThrottlerStorage>` hurts inference in closures with early `return Err(..)`] → the error type is
  fixed by `corvis::Result`, and `S` is inferred from the `Ok(..)` value; the two tests in Decision 4 catch a
  regression. If inference fails in practice, callers can annotate `Ok::<_, anyhow::Error>(vec![..])`.
- [Users expect `is_global` in the result to work, as in the original's options object] → rustdoc and the docs pages
  say the argument decides.
- [#18 changes `CacheOptions.ttl` units] → whichever lands second rebases and adjusts the `CACHE_TTL` value in the
  TTL test; the test only checks that the value read from `ConfigService` reaches the manager.

## Open questions

1. **Warning on a mismatched `is_global`.** Warn when the factory returns `is_global: true` but the argument is
   `false`? Proposed: no warning (Decision 2); confirm.
2. **`ScheduleModule` shape.** Keep `for_root_async(imports, factory)` as a documented exception, or file an issue to
   add the convention's shape and deprecate the current one?

## Review notes (2026-11-08)

4. No warning when the factory's `is_global` differs from the argument; document that the argument wins.
5. `ScheduleModule::for_root_async(imports, f)` stays as a documented exception to the convention.

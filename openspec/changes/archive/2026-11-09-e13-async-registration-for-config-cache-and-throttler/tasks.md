# Tasks

## 1. Failing tests

- [x] 1.1 Add tests to `crates/corvis/tests/modules.rs`, one per spec scenario: Config options from another provider,
  Config self-reference (circular error), Cache TTL from `ConfigService`, Cache tokens are one instance (`Arc::ptr_eq`),
  Throttler limit from `ConfigService` (third request 429), Throttler factory returning a storage, factory error message,
  non-global dependency through `.import(...)`; set environment variables through `ConfigModuleOptions` env files in a
  temp dir rather than the process environment, so tests do not race; verify they fail to compile or fail for the
  expected reason with `cargo test -p corvis --test modules`

## 2. Implementation

- [x] 2.1 Add `ConfigModule::for_root_async(is_global, factory)` in `crates/corvis/src/config.rs`, calling the same
  `ConfigService::load` as `for_root`; rustdoc says the returned `is_global` is ignored; verify the two Config tests
  pass
- [x] 2.2 Add `CacheModule::register_async(is_global, factory)` in `crates/corvis/src/modules/cache.rs` (one
  `use_factory` provider plus `Provider::use_existing(CACHE_MANAGER, ..)`, both exported); verify the two Cache tests
  and the existing `cache_throttle_sse_health_static` test pass
- [x] 2.3 Add `impl From<Vec<ThrottlerOptions>> for ThrottlerStorage` and `ThrottlerModule::for_root_async(factory)`
  (global) in `crates/corvis/src/modules/throttler.rs`; verify the two Throttler tests pass, including the closure that
  returns `Ok(vec![..])` without annotations
- [x] 2.4 Verify the factory error and `.import(...)` tests pass

## 3. Docs

- [x] 3.1 `docs/fundamentals/dynamic-modules.md`: add the convention (name, arguments, `.import(...)`, `is_global` is an
  argument) and the three new methods to the table, with `ScheduleModule::for_root_async(imports, factory)` marked as
  the exception; same in `docs/es/fundamentals/dynamic-modules.md`
- [x] 3.2 `docs/application/configuration.md`, `docs/data/caching.md`, `docs/security/rate-limiting.md`: an async
  example reading from another provider or `ConfigService`; same in `docs/es/`
- [x] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of 1.1
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 4.5 Framework runtime code changed (additive only): run the `corvis-parity` check and verify the three port apps
  keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #119, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

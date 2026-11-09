# core/modules Specification

## Purpose
Async registration of the built-in dynamic modules: options produced by a factory that can resolve other providers,
and the signature convention every async variant follows.

## Requirements

### Requirement: ConfigModule can be registered with an async factory
The framework SHALL provide `ConfigModule::for_root_async(is_global, factory)`, where `factory` is
`Fn(ResolveCtx) -> Future<Output = corvis::Result<ConfigModuleOptions>>`. `ConfigService` MUST be built from the
returned options exactly as `for_root` builds it, and exported. The module MUST be global when `is_global` is `true`;
the `is_global` field of the returned options MUST be ignored, because globality is fixed before any factory runs.

#### Scenario: Options from another provider
- **WHEN** a global module exports a provider holding an env file path and the application imports
  `ConfigModule::for_root_async(true, factory)` whose factory resolves that provider and returns options with
  `env_file_path` set to it
- **THEN** `ConfigService::get` returns the values of that env file in every module

#### Scenario: Factory asks for ConfigService
- **WHEN** the factory of `ConfigModule::for_root_async` resolves `ConfigService` itself
- **THEN** bootstrap fails with the circular dependency error naming `ConfigService`

### Requirement: CacheModule can be registered with an async factory
The framework SHALL provide `CacheModule::register_async(is_global, factory)`, where `factory` returns
`corvis::Result<CacheOptions>`. The `CacheManager` MUST be built once from the returned options and be resolvable
both as `Arc<CacheManager>` and through the `CACHE_MANAGER` token, and both MUST be the same instance. The module MUST
be global when `is_global` is `true`; the `is_global` field of the returned options MUST be ignored.

#### Scenario: TTL read from ConfigService
- **WHEN** `ConfigModule` is global, `CACHE_TTL=1000` is set, and the factory of
  `CacheModule::register_async(true, factory)` resolves `ConfigService` and returns `CacheOptions` whose `ttl` is the
  value of `CACHE_TTL`
- **THEN** a value stored with the default TTL expires after about one second

#### Scenario: Both tokens resolve to one manager
- **WHEN** a provider injects `Arc<CacheManager>` and another injects `#[inject(CACHE_MANAGER)] Arc<CacheManager>`
- **THEN** a value set through the first is read through the second, and `Arc::ptr_eq` holds

### Requirement: ThrottlerModule can be registered with an async factory
The framework SHALL provide `ThrottlerModule::for_root_async(factory)`, where `factory` returns
`corvis::Result<S>` for any `S: Into<ThrottlerStorage>`. `ThrottlerStorage` MUST implement
`From<Vec<ThrottlerOptions>>`, so a factory can return either a list of throttlers or a configured storage. The
module MUST be global, like `for_root`, and `ThrottlerGuard` MUST use the storage the factory produced.

#### Scenario: Limit read from ConfigService
- **WHEN** `ConfigModule` is global, `THROTTLE_LIMIT=2` is set, the factory resolves `ConfigService` and returns
  `vec![ThrottlerOptions::new(60_000, limit)]` with `limit` read from `THROTTLE_LIMIT`, and `ThrottlerGuard` is a
  global guard
- **THEN** the third request to a route within a minute answers 429

#### Scenario: Factory returns a configured storage
- **WHEN** the factory returns `ThrottlerStorage::new(vec![...]).with_max_entries(10)`
- **THEN** the application starts and the guard uses that storage

### Requirement: A failing options factory aborts bootstrap with the factory error
When the factory of any async variant returns an error, application creation SHALL fail with
`Error while instantiating <token>: <cause>`, where `<token>` is the provider the factory builds (`ConfigService`,
`CacheManager`, `ThrottlerStorage`), the same error `Provider::use_factory` reports today.

#### Scenario: Missing variable
- **WHEN** the factory of `CacheModule::register_async` returns `Err(anyhow!("CACHE_TTL is not set"))`
- **THEN** creating the application fails with an error containing `Error while instantiating` and
  `CACHE_TTL is not set`

### Requirement: Async variants follow one signature convention
Every async variant of a built-in module added from now on SHALL be named after the synchronous method plus `_async`
(`for_root` → `for_root_async`, `register` → `register_async`) and take `(is_global, factory)` when the module has an
`is_global` option and `(factory)` otherwise.

#### Scenario: New variants follow the convention
- **WHEN** a user registers `ConfigModule::for_root_async(true, f)`, `CacheModule::register_async(true, f)` and
  `ThrottlerModule::for_root_async(f)`
- **THEN** the three calls compile with those argument lists, and the modules are global

### Requirement: Options factories can resolve providers of imported modules
The factory of an async variant SHALL run in the context of the dynamic module, so it MUST be able to resolve providers
of global modules and of modules added to the returned `DynamicModule` with `.import(...)`.

#### Scenario: Dependency from a non-global module
- **WHEN** `CacheModule::register_async(false, factory).import(SettingsModule::for_root())` is imported and the factory
  resolves a provider exported by `SettingsModule`, which is not global
- **THEN** the application starts and the cache uses the value of that provider

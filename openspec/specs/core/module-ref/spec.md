# core/module-ref Specification

## Purpose
`ModuleRef` gives programmatic access to the container. Its lookups are anchored at one module and each method has a
fixed visibility rule.

## Requirements

### Requirement: A ModuleRef is located in the module that owns the provider
A `ModuleRef` field of an `#[injectable]`, and the `ModuleRef` returned by `ResolveCtx::module_ref()` inside a factory,
SHALL be located in the module that registers that provider or factory, even when the provider is consumed from
another module. `app.module_ref()` SHALL be located in the root module.

#### Scenario: Provider exported to another module
- **WHEN** `CatsModule` registers and exports `CatsService`, which has a `module_ref: ModuleRef` field, and `AppModule`
  imports `CatsModule`
- **THEN** `cats_service.module_ref.module_name()` returns `"CatsModule"`

### Requirement: get and get_token look in the module first, then in the whole application
`ModuleRef::get::<T>()` and `ModuleRef::get_token::<T>(token)` SHALL return an existing instance, looking first with
the module's visibility rules (own providers, exports of imports, global modules) and, only if none is visible, in the
whole application. They MUST NOT create instances.

#### Scenario: Same type registered privately in two modules
- **WHEN** modules `A` and `B` each register their own private `Counter`, and a `ModuleRef` located in `A` calls
  `get::<Counter>()`
- **THEN** it returns `A`'s instance, not `B`'s

#### Scenario: Private provider of an unrelated module
- **WHEN** a `ModuleRef` located in `A` calls `get::<Secret>()` and `Secret` is a private singleton of module `B`,
  which `A` does not import
- **THEN** it returns `B`'s instance

#### Scenario: Unknown token
- **WHEN** `get_token::<String>("NOPE")` is called and no module registers `"NOPE"`
- **THEN** it returns `Err(DiError::NotFound { .. })`

#### Scenario: Wrong requested type
- **WHEN** `"PORT"` is registered with `Provider::use_value("PORT", 8080u32)` and `get_token::<String>("PORT")` is
  called
- **THEN** it returns `Err(DiError::TypeMismatch { .. })`

### Requirement: get_token_in_context sees exactly what field injection sees
`ModuleRef::get_token_in_context::<T>(token)` SHALL find only the module's own providers, the exports of its imports
and the exports of global modules, and MUST return `DiError::NotFound` for anything else.

#### Scenario: Visible and invisible providers
- **WHEN** a `ModuleRef` located in `A` calls `get_token_in_context` for a provider exported by a module `A` imports,
  for an export of a global module, and for a private provider of a module `A` does not import
- **THEN** the first two return the instances and the third returns `Err(DiError::NotFound { .. })`

### Requirement: get_strict only sees the module's own providers
`ModuleRef::get_strict::<T>()` SHALL return only an instance registered by the module itself and MUST return
`DiError::NotFound` for providers obtained through imports or global modules.

#### Scenario: Own and imported providers
- **WHEN** a `ModuleRef` located in `A` calls `get_strict` for a provider `A` registers and for a provider exported by a
  module `A` imports
- **THEN** the first returns the instance and the second returns `Err(DiError::NotFound { .. })`

### Requirement: resolve resolves like injection
`ModuleRef::resolve::<T>().await` SHALL resolve `T` with field-injection visibility: for a singleton it MUST return the
existing instance, and for a transient provider a new one.

#### Scenario: Resolving a singleton
- **WHEN** `resolve::<CatsService>().await` is called for a singleton `CatsService`
- **THEN** it returns the same instance as `get::<CatsService>()`

### Requirement: create builds unregistered classes without storing them
`ModuleRef::create::<T>().await` SHALL construct an `#[injectable]` `T` that is not registered as a provider, injecting
its fields from the `ModuleRef`'s module, and MUST return a new value on every call without storing it in the
container. A dependency that is not visible MUST produce an error, not a panic.

#### Scenario: Two creations
- **WHEN** `create::<ReportBuilder>().await` is called twice and `ReportBuilder` injects a visible `RoomsService`
- **THEN** both values receive the same `RoomsService` instance, they are separate values, and
  `get::<ReportBuilder>()` afterwards returns `Err(DiError::NotFound { .. })`

#### Scenario: Missing dependency
- **WHEN** `create::<T>().await` is called for a `T` whose field type is not visible from the module
- **THEN** it returns `Err(DiError::UnknownDependencies { .. })` naming that module

### Requirement: resolve_for resolves a provider of any scope for a request
`ModuleRef::resolve_for::<T>(&request)` and `ModuleRef::resolve_token_for::<T>(token, &request)` SHALL resolve a
provider with constructor-injection visibility from the `ModuleRef`'s module: the stored instance for a singleton, a
new instance for a transient provider, and the instance of that request for a request-scoped provider (created on
first use). A provider that is not visible MUST return `DiError::NotFound`.

#### Scenario: Singleton through resolve_for
- **WHEN** a handler calls `module_ref.resolve_for::<CatsService>(&req)` and `CatsService` is a singleton
- **THEN** it returns the same instance as `module_ref.get::<CatsService>()`

#### Scenario: Request-scoped provider by token
- **WHEN** a handler calls `module_ref.resolve_token_for::<Tenant>("TENANT", &req)` twice and `"TENANT"` is a
  request-scoped factory provider
- **THEN** both calls return the same instance

#### Scenario: Not visible from the module
- **WHEN** a `ModuleRef` located in `CatsModule` calls `resolve_for::<Secret>(&req)` and `Secret` is a private
  request-scoped provider of another module
- **THEN** it returns `Err(DiError::NotFound { .. })`

### Requirement: resolve_for needs a request being dispatched only for request-scoped providers
`ModuleRef::resolve_for` and `ModuleRef::resolve_token_for` MUST resolve a provider that is not request-scoped
(a singleton or a transient provider that does not reach a request-scoped one) exactly as `ModuleRef::resolve` does,
whatever the request. They MUST return `DiError::OutsideRequest` when the provider is request-scoped, directly or
through a chain of transient providers, and the request is not being dispatched by the application.

#### Scenario: Request not being dispatched
- **WHEN** `resolve_for::<RequestContext>(&req)` is called with a request built with `Request::new(..)` that never went
  through the application, and `RequestContext` is request-scoped
- **THEN** it returns `Err(DiError::OutsideRequest { .. })`

#### Scenario: Request-scoped through a transient provider
- **WHEN** the same call resolves a transient `TransientUser` that injects the request-scoped `RequestContext`
- **THEN** it returns `Err(DiError::OutsideRequest { .. })`

#### Scenario: Singleton or transient without a store
- **WHEN** `resolve_for::<CatsService>(&req)` is called with a request that was never dispatched and `CatsService` is a
  singleton, or an application has no request-scoped provider at all
- **THEN** it returns the same instance as `ModuleRef::get::<CatsService>()`, and a transient provider is created anew
  on each call

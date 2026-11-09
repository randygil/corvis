# Spec Delta

## ADDED Requirements

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

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

### Requirement: resolve_for needs a request being dispatched
`ModuleRef::resolve_for` and `ModuleRef::resolve_token_for` MUST return `DiError::OutsideRequest` when the request is
not being dispatched by the application, whatever the scope of the provider.

#### Scenario: Request not being dispatched
- **WHEN** `resolve_for::<RequestContext>(&req)` is called with a request built with `Request::new(..)` that never went
  through the application
- **THEN** it returns `Err(DiError::OutsideRequest { .. })`

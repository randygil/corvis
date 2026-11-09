# Spec Delta

## ADDED Requirements

### Requirement: Scoped<T> resolves a provider for the current request
`Scoped<T>` SHALL implement `FromContext` and dereference to `T`. As an undecorated handler parameter, or through
`Scoped::<T>::from_context(ctx)` in a guard, an interceptor or a route-level exception filter, it MUST resolve the
provider registered under `Token::of::<T>()` for the request being handled, from the module of the route's
controller, as `ModuleRef::resolve_for::<T>(&request)` would.

#### Scenario: Handler parameter
- **WHEN** a handler declares `ctx: Scoped<RequestContext>` and `RequestContext` is request-scoped
- **THEN** the handler receives the instance of the current request, and a guard of the same route that calls
  `Scoped::<RequestContext>::from_context(ctx)` received the same instance (`Arc::ptr_eq`)

#### Scenario: Different requests
- **WHEN** two requests reach a handler that returns the address of its `Scoped<RequestContext>` instance while both
  are in flight
- **THEN** the two addresses differ

### Requirement: Scoped<T> works for every scope
`Scoped<T>` MUST resolve request-scoped, transient and singleton providers, so that changing a provider's scope does
not change its consumers.

#### Scenario: Transient through Scoped
- **WHEN** a handler declares two parameters `a: Scoped<OperationTimer>` and `b: Scoped<OperationTimer>` and
  `OperationTimer` is transient
- **THEN** `a` and `b` are different instances

#### Scenario: Singleton through Scoped
- **WHEN** a handler declares `cats: Scoped<CatsService>` and `CatsService` is a singleton
- **THEN** it receives the same instance as `app.get::<CatsService>()`

### Requirement: Scoped<T> errors do not expose internals
When resolution fails, `Scoped<T>` MUST return the `HttpException` carried by a factory error unchanged, and for every
other failure MUST return a `500` with the default internal-error body and log the `DiError` message. In a WebSocket
context it MUST fail with a `500` and MUST NOT create any instance.

#### Scenario: Factory rejects the request
- **WHEN** a request-scoped factory provider returns `Err(BadRequestException::new("missing tenant").into())` and a
  handler takes `Scoped` of it
- **THEN** the response is `400` with message `missing tenant`

#### Scenario: Provider not visible
- **WHEN** a handler declares `Scoped<Secret>` and `Secret` is not visible from the controller's module
- **THEN** the response is `500` with the default internal-error body, its body does not contain `Secret`, and the log
  contains the `DiError` message naming `Secret`

### Requirement: The scope error recommends Scoped<T>
The `DiError::RequestScopedDependency` message SHALL name `Scoped<T>` (with the dependency's name) as the first way to
consume the dependency per request, and `ModuleRef::resolve_for` as the second.

#### Scenario: Controller field
- **WHEN** `CatsController` declares `ctx: Arc<RequestContext>` and `RequestContext` is request-scoped
- **THEN** the startup error message contains `Scoped<RequestContext>` and `resolve_for`

# core/injection-scopes Specification

## Purpose
How many instances of a provider the container creates and when, for the two supported scopes: the default
(singleton) and transient.

## Requirements

### Requirement: Singleton providers are shared within their module
A provider with the default scope SHALL be instantiated once per module that registers it, when the application
starts, and every consumer that resolves it from that module MUST receive the same instance.

#### Scenario: Two consumers of a singleton
- **WHEN** two providers of a module each declare an `Arc<Counter>` field and `Counter` has the default scope
- **THEN** both fields point to the same instance (`Arc::ptr_eq`), and `app.get::<Counter>()` returns that instance

### Requirement: Transient providers give each consumer its own instance
A provider with the transient scope SHALL be instantiated every time it is resolved: each consumer that injects it MUST
receive a distinct instance, and each `ModuleRef::resolve` call MUST return a new one. The transient scope SHALL be
honored whether it is declared with `#[injectable(scope = "transient")]` or with `Provider::scope(Scope::Transient)`.

#### Scenario: Two consumers of a transient provider
- **WHEN** two singleton providers each declare an `Arc<OperationTimer>` field and `OperationTimer` is declared with
  `#[injectable(scope = "transient")]`
- **THEN** the two fields hold different instances

#### Scenario: Transient scope set on the provider
- **WHEN** a module registers `Provider::use_class::<OperationTimer>("TIMER").scope(Scope::Transient)` for a class
  without a scope attribute, and two providers inject `#[inject("TIMER")] Arc<OperationTimer>`
- **THEN** the two fields hold different instances

#### Scenario: Two fields of the same transient type
- **WHEN** one singleton declares two `Arc<OperationTimer>` fields and `OperationTimer` is transient
- **THEN** the two fields hold different instances

#### Scenario: Resolving a transient provider on demand
- **WHEN** `module_ref.resolve::<OperationTimer>().await` is called twice
- **THEN** both calls succeed and return different instances

### Requirement: Transient instances are neither created at startup nor stored
The container MUST NOT instantiate a transient provider at startup unless a singleton consumer injects it, and it MUST
NOT store transient instances. Consequently, `ModuleRef::get` and `app.get` MUST return `DiError::NotFound` for a
transient provider, and lifecycle hooks MUST NOT be called on transient instances.

#### Scenario: No consumer at startup
- **WHEN** an application registers a transient provider that no singleton injects and starts
- **THEN** the provider's constructor has not run

#### Scenario: Looking up a transient instance
- **WHEN** `app.get::<OperationTimer>()` is called for a transient `OperationTimer`
- **THEN** it returns `Err(DiError::NotFound { .. })`

#### Scenario: Lifecycle hooks
- **WHEN** a transient provider implements `OnModuleInit`, a singleton injects it, and the application runs `init()`
- **THEN** the transient provider's `on_module_init` is not called

### Requirement: Scope does not propagate to consumers
A singleton that depends on a transient provider SHALL receive one transient instance when the singleton is created
and MUST keep it for its whole life; the singleton MUST NOT become transient.

#### Scenario: Singleton consumer resolved twice
- **WHEN** a singleton `Report` injects a transient `OperationTimer` and `Report` is obtained twice (`get` and
  `resolve`)
- **THEN** both calls return the same `Report`, holding the same `OperationTimer` instance

### Requirement: Transient providers' dependencies are validated at startup
When an application or testing module is created, the container MUST check the declared dependencies of every
transient provider before it instantiates any provider, and MUST NOT instantiate transient providers to do so. A
required dependency that is not visible from the provider's module MUST make creation fail with the same `DiError` its
first resolution would return: `UnknownDependencies` (provider, token, index, module) for an injected field, `NotFound`
for an alias target.

#### Scenario: Missing dependency of an unused transient provider
- **WHEN** a module registers a transient `OperationTimer` with an `Arc<Clock>` field, no module provides `Clock`, no
  provider injects `OperationTimer`, and the testing module is compiled
- **THEN** compilation fails with `DiError::UnknownDependencies` whose message names `OperationTimer`, `Clock`, index
  `[0]` and the module

#### Scenario: Not instantiated by the check
- **WHEN** a transient provider with all its dependencies available counts its constructions and no singleton injects
  it
- **THEN** compilation succeeds and the counter is still zero

#### Scenario: Alias with a missing target
- **WHEN** a module registers `Provider::use_existing("TIMER", "MISSING").scope(Scope::Transient)`
- **THEN** compilation fails with `DiError::NotFound` naming `"MISSING"`

### Requirement: The startup check follows transient dependencies only where it can know them
Dependencies that are transient providers MUST be checked in their own module, and a cycle made only of transient
providers MUST fail with `DiError::Circular`. Absent optional dependencies (`Option<Arc<T>>`) MUST NOT fail the check.
Dependencies that can only be known by running code (factory providers) SHALL NOT be checked.

#### Scenario: Transitive transient dependency
- **WHEN** transient `Report` injects transient `OperationTimer`, and `OperationTimer`'s `Arc<Clock>` dependency is
  missing
- **THEN** compilation fails with `DiError::UnknownDependencies` naming `OperationTimer` and `Clock`

#### Scenario: Optional dependency absent
- **WHEN** a transient provider has an `Option<Arc<Clock>>` field and no module provides `Clock`
- **THEN** compilation succeeds

#### Scenario: Cycle of transient providers
- **WHEN** transient `A` injects transient `B`, `B` injects `A`, and no singleton injects either
- **THEN** compilation fails with `DiError::Circular` whose chain contains `A` and `B`

#### Scenario: Transient factory provider
- **WHEN** a module registers `Provider::use_factory("CLIENT", ..).scope(Scope::Transient)` whose factory asks for a
  missing token
- **THEN** compilation succeeds and the error appears when `"CLIENT"` is first resolved, as before

#### Scenario: Transient provider replaced by a testing override
- **WHEN** a transient `OperationTimer` whose `Clock` dependency is missing is replaced in the testing module with
  `override_provider::<OperationTimer>().use_value(..)`
- **THEN** compilation succeeds, because only the providers actually registered after overrides are checked

### Requirement: Request-scoped providers have one instance per request
A provider with the request scope SHALL be instantiated at most once per HTTP request for each module that registers
it, on its first resolution during that request, and every resolution of it during the same request MUST return that
instance. Different requests MUST receive different instances, also when they are handled concurrently.

#### Scenario: Two resolutions in one request
- **WHEN** a handler calls `module_ref.resolve_for::<RequestContext>(&req)` twice during one request and
  `RequestContext` is declared with `#[injectable(scope = "request")]`
- **THEN** both calls return the same instance (`Arc::ptr_eq`)

#### Scenario: Concurrent requests
- **WHEN** 50 requests are sent concurrently to a route whose handler resolves `RequestContext`, stores a value derived
  from its own request in it, waits, and returns the value it reads back together with the instance address
- **THEN** every response carries the value of its own request, and the 50 instance addresses that were alive at the
  same time are distinct

#### Scenario: Shared within a request across consumers
- **WHEN** request-scoped `AuditLog` and request-scoped `Billing` both inject `Arc<RequestContext>`, and a handler
  resolves both in one request
- **THEN** `audit_log.ctx` and `billing.ctx` are the same instance

### Requirement: The request scope can be declared on the class or on the provider
The request scope SHALL be honored whether it is declared with `#[injectable(scope = "request")]` or with
`Provider::scope(Scope::Request)`, and a `Provider::use_existing` alias of a request-scoped provider SHALL resolve to
the instance of the current request.

#### Scenario: Alias of a request-scoped provider
- **WHEN** a module registers request-scoped `RequestContext` and `Provider::use_existing("CTX", Token::of::<RequestContext>())`,
  and a handler resolves both in one request
- **THEN** both resolutions return the same instance

#### Scenario: Scope set on the provider
- **WHEN** a module registers `Provider::use_class::<RequestContext>("CTX").scope(Scope::Request)` for a class without
  a scope attribute, and a handler resolves `"CTX"` twice in one request and once in another request
- **THEN** the first two resolutions return the same instance and the third returns a different one

### Requirement: Request-scoped instances are never stored in the container
The container MUST NOT instantiate a request-scoped provider at startup and MUST NOT store its instances. The
per-request instances MUST be released when the request's dispatch ends, whether it ends with a response, an error, a
panic or the client disconnecting. `ModuleRef::get`, `ModuleRef::get_token`, `ModuleRef::get_strict` and `app.get`
MUST return `DiError::NotFound` for a request-scoped provider, and lifecycle hooks MUST NOT be called on request-scoped
instances.

#### Scenario: Not created at startup
- **WHEN** an application registers a request-scoped provider that counts its constructions and starts
- **THEN** the counter is zero

#### Scenario: Released after the response
- **WHEN** a request-scoped provider that keeps the `Request` it was given is resolved during a request, the test keeps
  a `Weak` reference to the instance, and the response has been received
- **THEN** the `Weak` reference can no longer be upgraded

#### Scenario: Looking up a stored instance
- **WHEN** `app.get::<RequestContext>()` is called for a request-scoped `RequestContext`
- **THEN** it returns `Err(DiError::NotFound { .. })`

#### Scenario: No lifecycle hooks
- **WHEN** a request-scoped provider implements `OnModuleInit` and is resolved during a request
- **THEN** its `on_module_init` is never called

### Requirement: Request-scoped providers can inject the current request
A field of type `Request` in an `#[injectable]` request-scoped provider SHALL receive the request being handled, sharing
its extension store, and `ResolveCtx::request()` SHALL return it inside the factory of a request-scoped factory
provider. A request-scoped provider MAY depend on singletons, on transient providers and on other request-scoped
providers; a transient provider resolved while a request-scoped provider is built MAY depend on request-scoped
providers and on `Request`.

#### Scenario: Request field
- **WHEN** request-scoped `RequestContext` has a `req: Request` field and a handler for `GET /cats?x=1` resolves it
- **THEN** `ctx.req.url()` is `/cats?x=1`

#### Scenario: Extensions stay shared
- **WHEN** a guard inserts a `User` into the request after `RequestContext` was resolved by a middleware in the same
  request
- **THEN** `ctx.req.get::<User>()` returns that user in the handler

#### Scenario: Factory provider
- **WHEN** a module registers `Provider::use_factory("TENANT", |ctx| async move { Ok(tenant_of(&ctx.request()?)) })
  .scope(Scope::Request)` and two requests with different `X-Tenant` headers resolve `"TENANT"`
- **THEN** each request receives the tenant of its own header

### Requirement: Singletons, controllers and gateways cannot depend on request-scoped providers
When an application or testing module is created, the container MUST fail with `DiError::RequestScopedDependency` if
the declared dependencies of a provider with the default scope, a controller or a gateway reach a request-scoped
provider or a `Request` field, directly, through `use_existing` aliases, or through transient providers. The message
MUST name the consumer, the request-scoped dependency and the chain between them, and say how to resolve it per
request.

#### Scenario: Controller field
- **WHEN** `CatsController` declares `ctx: Arc<RequestContext>` and `RequestContext` is request-scoped
- **THEN** application creation fails with `DiError::RequestScopedDependency` whose message contains `CatsController`,
  `RequestContext` and `resolve_for`

#### Scenario: Through a transient provider
- **WHEN** singleton `Report` injects transient `Formatter`, and `Formatter` injects request-scoped `RequestContext`
- **THEN** application creation fails with `DiError::RequestScopedDependency` whose chain is
  `Report -> Formatter -> RequestContext`

#### Scenario: Request field in a singleton
- **WHEN** a singleton provider declares a `req: Request` field
- **THEN** application creation fails with `DiError::RequestScopedDependency` naming the provider and `Request`

#### Scenario: Request-scoped provider depending on a singleton
- **WHEN** request-scoped `RequestContext` injects singleton `Clock`
- **THEN** the application starts and every `RequestContext` holds the same `Clock` instance

### Requirement: Singleton factories cannot resolve request-scoped providers
A singleton factory provider that resolves a request-scoped provider, or calls `ResolveCtx::request()`, while it is
built MUST make application creation fail with `DiError::OutsideRequest` naming the request-scoped provider.

#### Scenario: Singleton factory
- **WHEN** a module registers `Provider::use_factory("REPORT", |ctx| async move { ctx.get::<RequestContext>().await
  .map(Report::new).map_err(Into::into) })` with the default scope and `RequestContext` is request-scoped
- **THEN** application creation fails with `DiError::OutsideRequest` naming `RequestContext`

### Requirement: Enhancers cannot be request-scoped
Global enhancers registered with `Provider::app_guard`, `Provider::app_interceptor`, `Provider::app_pipe` or
`Provider::app_filter`, and guards, interceptors, pipes, filters and middleware referenced by type, MUST NOT be
request-scoped: application creation MUST fail with an error naming the enhancer and saying it is created once.

#### Scenario: Request-scoped global guard
- **WHEN** a module registers `Provider::app_guard::<TenantGuard>()` and `TenantGuard` is request-scoped
- **THEN** application creation fails with an error naming `TenantGuard` that says global enhancers cannot be
  request-scoped

#### Scenario: Request-scoped guard referenced by type
- **WHEN** a route declares `#[use_guards(TenantGuard)]` and `TenantGuard` is request-scoped
- **THEN** application creation fails with an error naming `TenantGuard`

### Requirement: Request-scoped providers' dependencies are validated at startup
The startup validation of declared dependencies that applies to transient providers SHALL also apply to request-scoped
providers: a missing required dependency MUST fail creation with the error its first resolution would return, and a
cycle made only of transient and request-scoped providers MUST fail with `DiError::Circular`. Request-scoped providers
MUST NOT be instantiated by the check, and factory providers SHALL NOT be checked.

#### Scenario: Missing dependency
- **WHEN** request-scoped `RequestContext` has an `Arc<Clock>` field and no module provides `Clock`
- **THEN** creation fails with `DiError::UnknownDependencies` naming `RequestContext`, `Clock`, index `[0]` and the
  module

#### Scenario: Cycle of request-scoped providers
- **WHEN** request-scoped `A` injects request-scoped `B` and `B` injects `A`
- **THEN** creation fails with `DiError::Circular` whose chain contains `A` and `B`

### Requirement: Request-scoped providers cannot be resolved outside a request
Resolving a request-scoped provider, or building a provider that has a `Request` field, MUST fail with
`DiError::OutsideRequest` naming the provider when no HTTP request is being dispatched for that resolution: in
WebSocket gateway handshakes and message handlers, standalone application contexts, lifecycle hooks, scheduled jobs,
event listeners, and with a request whose dispatch has ended.

#### Scenario: Gateway message handler
- **WHEN** a `#[subscribe_message]` handler calls `module_ref.resolve_for::<RequestContext>(&handshake_request)`
- **THEN** the call returns `Err(DiError::OutsideRequest { .. })` and no instance is created

#### Scenario: Request kept after its response
- **WHEN** a handler spawns a task with a clone of its `Request`, the response is sent, and the task then calls
  `resolve_for::<RequestContext>(&req)`
- **THEN** the call returns `Err(DiError::OutsideRequest { .. })`

#### Scenario: Standalone context
- **WHEN** a standalone application context calls `module_ref.resolve::<RequestContext>()`
- **THEN** the call returns `Err(DiError::OutsideRequest { .. })`

### Requirement: The scope argument of #[injectable] is parsed exactly
`#[injectable]` SHALL accept `scope = "default"`, `scope = "transient"` and `scope = "request"` (and the equivalent
`Scope::Default`, `Scope::Transient` and `Scope::Request` paths), and MUST reject any other argument with a compile
error that lists the accepted scopes.

#### Scenario: Request scope accepted
- **WHEN** a struct is declared with `#[injectable(scope = "request")]`
- **THEN** it compiles and `<T as Injectable>::scope()` returns `Scope::Request`

#### Scenario: Unknown scope
- **WHEN** a struct is declared with `#[injectable(scope = "requested")]`
- **THEN** compilation fails with an error that mentions `"default"`, `"transient"` and `"request"`

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

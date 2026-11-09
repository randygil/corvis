# Spec Delta

## ADDED Requirements

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

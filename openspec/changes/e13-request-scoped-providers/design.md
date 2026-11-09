# Design

## Context

Issue #115, first of two changes (see proposal.md). The #23 spike (archived
`2026-11-08-e13-missing-core-features-users-expect/design.md`, feature 1, and its review notes) chose an explicit
request scope with a startup error instead of the original framework's scope bubbling.

### The original's scope model

- Three scopes: `DEFAULT` (singleton), `REQUEST` (one instance per incoming request, garbage-collected after it) and
  `TRANSIENT` (one instance per consumer).
- **Bubbling:** a provider or controller that depends on a request-scoped provider becomes request-scoped itself, up
  the injection chain. A controller that injects a request-scoped service is rebuilt for every request, with every
  request-scoped provider of its graph.
- `@Inject(REQUEST)` gives a request-scoped provider the current request; `INQUIRER` gives a transient provider the
  class that injects it.
- **Durable providers** (`durable: true` plus a `ContextIdStrategy`) share one request-scoped sub-tree per key (for
  example per tenant) instead of per request.
- Request-scoped providers do not receive lifecycle hooks; WebSocket gateways and scheduled jobs must not depend on
  them (there is no request).
- The original's docs warn that request scope costs latency, because every request builds the request-scoped part of
  the graph.

### Corvis today (verified in the code)

- `Scope` has `Default` and `Transient` (`crates/corvis/src/di/provider.rs:30-39`). Its doc comment already says there
  is no request scope (fixed by #14).
- `Provider::scope(scope)` sets the field (`provider.rs:324-328`); `use_class`/`use_class_as` take `T::scope()`
  (`provider.rs:217`, `236`); `Injectable::scope()` defaults to `Scope::Default`
  (`crates/corvis/src/di/injectable.rs:90-92`).
- The macro parser lowercases each argument and matches substrings: anything containing `transient` sets transient,
  anything containing `request` is rejected with "request-scoped providers are not supported; use transient or default
  scope" (`crates/corvis-macros/src/injectable.rs:191-207`). `#[controller]` and `#[websocket_gateway]` always pass
  `InjectableOptions { transient: false }` (`crates/corvis-macros/src/controller.rs:134`,
  `crates/corvis-macros/src/gateway.rs:116`). The generated `scope()` is emitted only for transient
  (`injectable.rs:119-123`).
- `ResolveCtx` holds the container, the module, the requester name, the dependency names and the resolution path
  (`crates/corvis/src/di/injectable.rs:97-104`). Fields are resolved through `InjectField`
  (`injectable.rs:177-220`): `Arc<T>`, `Option<Arc<T>>` and `ModuleRef`. `Request` does not implement `Default`
  (`crates/corvis/src/http/request.rs:16`), so the macro's `Probe` dispatch would pick an `InjectField` impl for it
  without ambiguity (`crates/corvis/src/__private.rs:27-85`).
- The container caches every non-transient instance by `(ModuleId, Token)` in `instances`
  (`crates/corvis/src/di/container.rs:69`, `458-504`); `is_transient` follows aliases (`container.rs:424-440`);
  `instantiate_all` skips transient providers and instantiates every controller (`container.rs:545-563`);
  `construct_injectable` builds enhancers referenced by type and caches them unless transient (`container.rs:508-542`).
- `Container::scan` ends with `validate_transients` (`container.rs:302`, `309-361`), a depth-first walk of the declared
  dependencies (`DeclaredDeps`, `provider.rs:149-163`) of transient providers that reports missing dependencies and
  cycles without creating anything.
- Routes bind their handlers to the controller instance once, at startup (`(def.routes)(&instance)`,
  `crates/corvis/src/router/mod.rs:378`); enhancers are resolved once per route (`router/mod.rs:372-414`). Each route
  keeps a `ModuleRef` of its controller's module (`router/mod.rs:380`, `461`), given to the `ExecutionContext`
  (`router/mod.rs:695`, `crates/corvis/src/common/context.rs:66-79`).
- Every HTTP request goes through `router::dispatch` (`router/mod.rs:578`): from the server (`server.rs:371`), from
  `app.handle` (`application.rs:585-589`) and from the testing client (`testing.rs:384`). WebSocket upgrades are
  answered before it (`server.rs:346-351`); gateway messages build an `ExecutionContext::for_ws` over the handshake
  request (`context.rs:84-103`).
- `Request` is cheap to clone and its clones share one extension store, `Arc<RwLock<http::Extensions>>`
  (`request.rs:14-33`, `insert`/`get`/`remove` at `request.rs:210-219`). The router already inserts `RouteStatus` in
  every routed request (`router/mod.rs:694`).
- Global enhancers registered as providers are read from the container's stored instances during `init`
  (`application.rs:500-533`); a provider without an instance fails with "the provider has no instance"
  (`application.rs:508`).
- The ports use no request or transient scope (see proposal.md, Impact).

## Goals / Non-Goals

**Goals:**
- `Scope::Request` with a per-request cache that cannot leak one request's instance into another.
- The current request injectable into request-scoped providers.
- Every place where a request-scoped instance would outlive its request is an error, at startup when it can be known
  statically, at resolution time otherwise, and never a silent singleton.
- No cost for singletons and no per-request cost for applications that do not use the scope.
- An explicit way to resolve per request: `ModuleRef::resolve_for` now, `Scoped<T>` in the follow-up change.

**Non-Goals:**
- Scope bubbling, durable providers, `INQUIRER` (proposal.md, Out of scope).
- Request scope outside HTTP requests.

## Decisions

1. **Where the instances live: a per-request store in the request's extensions, installed and removed by `dispatch`.**
   A new crate-private `RequestScope` (`crates/corvis/src/di/request_scope.rs`) holds
   `Mutex<HashMap<(ModuleId, Token), Arc<tokio::sync::OnceCell<Instance>>>>`. `dispatch` inserts
   `RequestScopeSlot(Arc<RequestScope>)` into the request before running the middleware chain and removes it with a
   drop guard when `dispatch` returns, panics or is cancelled (client gone).
   - It satisfies the issue's "cached in the request, never in the container": the container never sees these
     instances.
   - Removal at the end of `dispatch` breaks the cycle `Request → extensions → RequestScope → instance → Request` that
     an injected `Request` field would otherwise create (a leak of every request that uses one).
   - The slot doubles as the "a request is being handled" marker: gateway handshakes (answered before `dispatch`),
     gateway messages, hooks and spawned tasks after the response never see it, so they get `OutsideRequest` instead
     of quietly creating a store that would live as long as a WebSocket connection.
   - Alternatives considered: (a) a `tokio::task_local!` around `dispatch`: no cycle and no allocation, but invisible
     to `resolve_for(&request)` callers on another task and to code that holds the request but runs after `dispatch`;
     it also ties correctness to the executor's task boundaries. (b) A cache in `ExecutionContext`: created per route
     execution (`router/mod.rs:695`), so middleware (which run before it) could not share instances with the handler.
     (c) A new field on `Request`: every `Request` constructor and clone would carry it, and clones taken before the
     field is set would not see it.
2. **No per-request cost unless the scope is used.** `Container::scan` records whether any registered provider has an
   effective request scope (`has_request_scoped`). `init` copies it to a new `AppShared::request_scope: bool`, and
   `dispatch` installs the slot only when it is `true`. When it is `true`, the cost is one `Arc` allocation and one
   insert/remove in the extension map (which `RouteStatus` already allocates). Per resolution: one mutex lock, one map
   lookup and a `OnceCell`. The benchmark (`bench/`) is run before and after to confirm no change for an app without
   request-scoped providers.
3. **Resolution threads an optional request context through `ResolveCtx`.** `ResolveCtx` gains
   `request: Option<RequestCtx>` with `RequestCtx { request: Request, scope: Arc<RequestScope> }` (crate-private).
   `Container::resolve_provider` (`container.rs:458-504`) branches on the provider's effective scope:
   - `Default`: unchanged (cache in `instances`). The `ResolveCtx` given to its factory has `request: None`, so a
     singleton built lazily can never capture a request.
   - `Transient`: unchanged, and the factory's context keeps the caller's `request` (a transient built for a
     request-scoped consumer can use the request; one built for a singleton cannot).
   - `Request`: requires `request`; otherwise `DiError::OutsideRequest`. The cycle check on `path` runs first (so a
     cycle errors instead of waiting on its own cell); then `get_or_try_init` on the key's `OnceCell` builds the
     instance once even if two futures of the same request resolve it concurrently. A failed construction leaves the
     cell empty, so a later resolution in the same request retries, as for singletons.
   The effective scope follows aliases, generalizing `is_transient` into `effective_scope(owner, provider) -> Scope`.
4. **The current request is a `Request` field.** `impl InjectField for Request` returns the request of the context and
   fails with `OutsideRequest` (naming the requester) without one; its `dependency()` reports
   `Token::of::<Request>()` so the startup check sees it. `ResolveCtx::request() -> Result<Request, DiError>` gives
   factory providers the same thing. The injected value is a clone of the request as the first resolver saw it: its
   extensions are shared and stay current, but route params and `route_path` are only set when routing has matched
   (`router/mod.rs:691-694`), so a provider first resolved from a middleware sees no params. Documented.
   Alternative considered: a `REQUEST` string token. Rejected: `Request` is already the type users ask for, and a
   type gives a compile-time check; `Token::of::<Request>()` remains usable with `#[inject]` for symmetry.
5. **Startup errors instead of bubbling (review decision of #23).** After `validate_transients`, `scan` runs a scope
   check over the declared dependencies (`DeclaredDeps::Fields`) of every singleton provider and every controller
   (gateways are singleton providers). The walk follows aliases and transient providers (a transient injected into a
   singleton is created once, so it would freeze a request too) and fails on the first dependency whose effective
   scope is `Request`, or on a `Request` field, with:
   `DiError::RequestScopedDependency { consumer, dependency, chain }`, for example:
   "CatsController depends on RequestContext, which is request-scoped (CatsController -> AuditLog -> RequestContext).
   A singleton, a controller or a gateway is created once and would keep the instance of the first request. Resolve
   RequestContext per request with ModuleRef::resolve_for(&request) (inject a ModuleRef and take the Request as a
   handler parameter), or make AuditLog request-scoped too."
   The follow-up change adds `Scoped<T>` to the hint. Factory providers declare no dependencies; a singleton factory
   that resolves a request-scoped provider is caught at startup anyway, because singletons are built at startup with
   `request: None` and get `OutsideRequest`, whose message names the chain from `path`.
   Alternative considered: full bubbling (size L, per-request controller rebuilds; spike feature 1). Rejected by the
   review.
6. **Request-scoped enhancers and controllers are rejected.** Enhancers and controllers are resolved once at startup
   (decision context): a request-scoped one would silently become a singleton.
   - `Provider::app_guard/app_interceptor/app_pipe/app_filter::<T>()` where `T::scope()` is `Request`: `scan` fails
     with `DiError::Other` naming the enhancer ("global guards, interceptors, pipes and filters are created once and
     cannot be request-scoped; resolve the request-scoped provider inside it with `ModuleRef::resolve_for`"), instead
     of today's "the provider has no instance" from `init`.
   - `construct_injectable::<T>` (enhancers and middleware referenced by type) with `T::scope() == Request`: same
     error, at `build_router` / `build_middleware` time (startup). An enhancer referenced by type that is not
     request-scoped but has a request-scoped field is built at startup with `request: None`, so it fails with
     `OutsideRequest` naming the chain; `Provider::app_*` enhancers with such a field are singleton providers and get
     `RequestScopedDependency` from the scope check (decision 5).
   - `#[controller]` and `#[websocket_gateway]` have no scope argument; a hand-written `Injectable` impl returning
     `Scope::Request` for a controller is rejected by the scope check (controllers are checked as consumers, and their
     own scope must be `Default`).
7. **Request-scoped providers are validated at startup like transients.** `validate_transients` becomes
   `validate_lazy_providers`: it walks providers whose effective scope is `Transient` or `Request`, reporting missing
   dependencies (`UnknownDependencies`, `NotFound` for alias targets) and cycles through lazy providers
   (`Circular`). This also prevents the only deadlock the `OnceCell` could cause (two request-scoped class providers
   depending on each other and resolved concurrently in one request). Factory providers stay unchecked, as for
   transients (documented).
8. **`ModuleRef::resolve_for` / `resolve_token_for`.** `resolve_for::<T>(&request)` reads the request's slot
   (`OutsideRequest` if absent), builds a `ResolveCtx` with `request: Some(..)` located in the `ModuleRef`'s module and
   continuing the current resolution path (as `ModuleRef::ctx` does, `module_ref.rs:69-78`), and resolves with the
   normal visibility rules. It works for any scope, so callers do not need to know the provider's scope. `get`,
   `get_token`, `get_strict` and `app.get` keep returning only stored instances, so they return `NotFound` for a
   request-scoped provider, as for transients.
9. **Macro arguments are parsed exactly.** `parse_injectable_args` accepts `scope = "default" | "transient" |
   "request"` (string literal, also `Scope::Default`/`Scope::Transient`/`Scope::Request` paths if the current
   substring matching accepted them, checked by tests) and fails on anything else with "unknown scope ...; expected
   \"default\", \"transient\" or \"request\"". `InjectableOptions { transient: bool }` becomes
   `InjectableOptions { scope: ScopeArg }`, and `injectable_impl` emits `fn scope()` for transient and request.
   Alternative: keep substring matching. Rejected: `"requested"` or `"not_transient"` would be read as scopes.

## Risks / Trade-offs

- [One request's instance reaching another request] → the store lives in the request and is removed when `dispatch`
  ends; singletons build with `request: None`; the startup scope check rejects singleton consumers; regression tests
  run concurrent requests and assert distinct instances, and assert the store is gone after the response.
- [Leak through `Request` ↔ instance cycles] → the drop guard removes the store on every exit path, including panic and
  cancellation; a test holds a `Weak` to a request-scoped instance that keeps its `Request` and checks it is dropped
  after the response.
- [A clone of the request kept by user code after `dispatch` (SSE stream, spawned task)] → it no longer has the store,
  so resolving fails with `OutsideRequest` instead of creating a second set of instances. Documented.
- [Adding a `Scope` variant breaks exhaustive `match`es in user code] → `Scope` was never `#[non_exhaustive]`; the
  pre-1.0 changelog notes it. Making it `#[non_exhaustive]` now is an option (Open questions).
- [Testing overrides can hide a scope error] → `override_provider::<T>().use_value(..)` replaces a request-scoped
  provider with a singleton value, so a singleton consumer passes in the test and fails in production. The scope
  check runs on the overridden graph, as the transient check does; documented.
- [Factory providers are not statically checked] → they fail at startup when they are singletons (built eagerly) and
  at first resolution when lazy; same trade-off as transient factories.

## Open questions

1. Should `Scope` become `#[non_exhaustive]` in this change, so later scopes (durable) are not breaking?
   Recommended: yes.
2. A testing helper to resolve request-scoped providers without dispatching a request (for unit tests of a
   request-scoped service), for example `TestingModule::resolve_for_request::<T>(request)` that installs a temporary
   store. Not in the issue; proposed as a follow-up if users ask.
3. Is one per-request store per `dispatch` acceptable for `app.handle(req)` called re-entrantly from a handler (a
   handler that dispatches a sub-request with the same `Request` clone)? The inner `dispatch` would see an existing
   slot. Proposed: `dispatch` installs a slot only if none is present and only the installer removes it, so an inner
   dispatch shares the outer request's instances. Confirm.

## Review notes (2026-11-09)

- Q1: `Scope` becomes `#[non_exhaustive]` in this change.
- Q2: no testing helper now; follow-up if users ask.
- Q3: confirmed: `dispatch` installs a store only if none is present, so a nested dispatch shares the outer request's
  instances.
- This change and `e13-request-scoped-providers-scoped-parameter` together close #115.

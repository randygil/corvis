# Proposal

Implements the container part of [randygil/corvis#115](https://github.com/randygil/corvis/issues/115) (epic
[#13](https://github.com/randygil/corvis/issues/13)). The issue is split in two changes, applied in this order:

1. **This change** (`e13-request-scoped-providers`): `Scope::Request` in the container and the macros, the per-request
   instance cache, injecting the current `Request`, the startup errors, and `ModuleRef::resolve_for`.
2. `e13-request-scoped-providers-scoped-parameter`: the `Scoped<T>` parameter for handlers, guards, interceptors and
   filters, the async local storage recipe, the migration guide and an example. It depends on this change.

## Why

`#[injectable(scope = "request")]` is a compile error (`crates/corvis-macros/src/injectable.rs:202-204`) and `Scope`
only has `Default` and `Transient` (`crates/corvis/src/di/provider.rs:30-39`). Code that keeps per-request state in a
provider (current user, tenant, request id, a unit of work) has to be rewritten with request extensions and
`FromContext` before it can be migrated. A request scope with explicit consumption keeps most of that code's shape
without the original framework's implicit scope bubbling, which would rebuild whole controller graphs per request.

## What Changes

- `Scope::Request`: at most one instance per HTTP request and per (module, provider), built on first use and cached in
  a per-request store that the router installs in the request and removes when the request is done. Never stored in
  the container, never created at startup, no lifecycle hooks.
- `#[injectable(scope = "request")]` and `Provider::scope(Scope::Request)` are accepted. The argument parser stops
  matching substrings and accepts exactly `"default"`, `"transient"` and `"request"`.
- A `Request` field in an `#[injectable]` struct receives the request being served (the original's `REQUEST` token).
  `ResolveCtx::request()` gives factory providers the same access.
- A request-scoped provider may depend on singletons, on transients and on other request-scoped providers. A transient
  provider resolved inside a request may depend on request-scoped providers too.
- `ModuleRef::resolve_for::<T>(&request)` and `ModuleRef::resolve_token_for::<T>(token, &request)` resolve a provider
  of any scope for a request that is being handled.
- Startup errors instead of scope bubbling: a singleton provider, a controller or a gateway that depends on a
  request-scoped provider (directly, through an alias or through transient providers), or on `Request`, fails
  `CorvisFactory::create` / `compile()` with a new `DiError::RequestScopedDependency` that names the consumer, the
  dependency and the chain, and says how to consume it per request. Global enhancers (`Provider::app_*`) and enhancers
  referenced by type (`#[use_guards(T)]` and siblings) cannot be request-scoped: also a startup error.
- The dependencies of request-scoped providers are checked at startup like those of transient providers (missing
  dependency, cycles), without creating them.
- Resolving a request-scoped provider (or a `Request` field) outside a request being handled (gateway messages and
  handshakes, standalone contexts, lifecycle hooks, scheduled jobs, event listeners, spawned tasks) fails with a new
  `DiError::OutsideRequest` naming the provider.
- Applications without request-scoped providers do no extra work per request.
- Regression tests, including concurrent requests that must get distinct instances.
- Docs (EN + ES): injection scopes and common errors.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/injection-scopes`: adds the request scope, injection of the current request, the startup scope check, the
  startup validation of request-scoped providers' dependencies and the outside-a-request error.
- `core/module-ref`: adds `resolve_for` and `resolve_token_for`.

## Impact

- Code: `crates/corvis/src/di/provider.rs` (`Scope::Request`), `crates/corvis/src/di/injectable.rs` (request context in
  `ResolveCtx`, `InjectField for Request`, `ResolveCtx::request`), `crates/corvis/src/di/container.rs` (effective scope,
  per-request resolution, startup checks), `crates/corvis/src/di/module_ref.rs` (`resolve_for`,
  `resolve_token_for`), `crates/corvis/src/di/error.rs` (two variants), a new
  `crates/corvis/src/di/request_scope.rs` (the per-request store), `crates/corvis/src/router/mod.rs` (`dispatch`
  installs and removes the store; `AppShared` flag), `crates/corvis/src/application.rs` (sets the flag; global
  enhancer check), `crates/corvis-macros/src/injectable.rs` (argument parsing and generated `scope()`).
- Public API: one new `Scope` variant (a breaking change for code that matches `Scope` exhaustively; `Scope` is not
  `#[non_exhaustive]`), two `DiError` variants, two `ModuleRef` methods, `ResolveCtx::request`, and `Request` as an
  injectable field type. Existing applications compile and behave as before.
- Performance: singleton resolution is unchanged (instances are created at startup and handlers hold `Arc`s). A request
  pays for the per-request store only when the application registers at least one request-scoped provider.
- Ports: the three port apps and their Node originals use no request or transient scope (checked: no `Scope.REQUEST`,
  `Scope.TRANSIENT`, `durable` or `REQUEST` in `corvis-tests/nest/{consi-backend,facten-api,houseplan}/**/src`, and no
  `Scope::` or `scope = "..."` in `corvis-tests/corvis/*/src`). Their behavior and apidiff results must not change.
- Docs (EN + ES): `fundamentals/injection-scopes.md`, `faq/common-errors.md`.

## Out of scope

- `Scoped<T>` as a handler/guard/interceptor parameter, the async local storage recipe, the migration guide and the
  example: the follow-up change `e13-request-scoped-providers-scoped-parameter`.
- Implicit scope bubbling (a consumer becoming request-scoped because a dependency is): rejected by the #23 review; the
  startup error replaces it.
- Durable providers and context identification strategies (`ContextIdStrategy`, `durable: true`, per-tenant
  sub-trees): possible later as a keyed cache next to the per-request one; new issue if a migration needs it.
- The `INQUIRER` token (a transient provider learning which class injects it): new issue if needed.
- Request-scoped controllers, gateways and enhancers: rejected at compile time or at startup.
- Request scope in WebSocket gateways (per message or per connection), microservice transports, scheduled jobs and
  event listeners: there is no HTTP request; the error says so.
- A testing helper to resolve a request-scoped provider without dispatching a request (see design, Open questions).
- Pipes (`PipeTransform::transform` receives no execution context, `crates/corvis/src/common/pipe.rs:73`), so they
  cannot reach the request: unchanged.

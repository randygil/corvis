# Design

## Context

Second change for issue #115. It assumes `e13-request-scoped-providers` is archived: `Scope::Request`, the per-request
store installed by `router::dispatch`, `ModuleRef::resolve_for` / `resolve_token_for`, `DiError::OutsideRequest` and
`DiError::RequestScopedDependency`.

Current code this change builds on (verified):

- Undecorated handler parameters are resolved with
  `<T as ::corvis::FromContext>::from_context(&__ctx).await?` (`crates/corvis-macros/src/controller.rs:411-414`).
- `FromContext` is `async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException>`
  (`crates/corvis/src/common/param.rs:26-29`); `Request`, `Res`, `ExecutionContext`, `ModuleRef` and `Option<T>`
  implement it (`param.rs:31-67`). The `ModuleRef` impl answers `500` "ModuleRef not available" when the context has
  none (`param.rs:55-60`).
- Each route's `ExecutionContext` carries the `ModuleRef` of the controller's module
  (`crates/corvis/src/router/mod.rs:380`, `461`, `695`); the contexts built for errors before routing have none
  (`empty_context`, `router/mod.rs:636-638`). Route-level exception filters receive the route's context
  (`handle_exception(e, &ctx, &route.filters)` in `execute_route`, `router/mod.rs:704`, `725`).
- Guards and interceptors receive `&ExecutionContext` (`crates/corvis/src/common/guard.rs:25`,
  `crates/corvis/src/common/interceptor.rs:27`); exception filters receive it as `ArgumentsHost`
  (`crates/corvis/src/common/filter.rs:37`). Pipes do not (`crates/corvis/src/common/pipe.rs:73`).
- Gateway contexts are `ContextType::Ws` (`crates/corvis/src/common/context.rs:84-103`, `131-133`); gateway handler
  parameters do not go through `FromContext`.
- Factory errors reach the container as `DiError::Factory { token, cause: Arc<anyhow::Error> }`
  (`crates/corvis/src/di/provider.rs:296-301`, `crates/corvis/src/di/error.rs`); an `HttpException` travels in
  `anyhow::Error` as `HttpExceptionError` (`crates/corvis/src/http/exception.rs:315-323`).
- `docs/recipes/async-local-storage.md` (status `unavailable`) and `docs/migration-guide.md:58` say there is no request
  scope.

## Goals / Non-Goals

**Goals:**
- One-parameter access to request-scoped providers in handlers, and a one-liner in guards, interceptors and filters.
- Errors that do not leak internals to clients but keep user-chosen HTTP errors from factories.
- Docs and an example that a migration can follow.

**Non-Goals:**
- Pipes, token-based extraction, implicit (parameter-less) access: proposal.md, Out of scope.

## Decisions

1. **`Scoped<T>(Arc<T>)` with `Deref<Target = T>` and `into_inner()`.** `T: ?Sized + Send + Sync + 'static`, so
   `Scoped<dyn Repo>` works for a provider registered as `Token::of::<dyn Repo>()`. Its `FromContext` impl takes the
   context's `ModuleRef` and request and calls `resolve_for::<T>`. Alternatives considered: (a) a `#[scoped]`
   parameter attribute generating the call; rejected, it adds macro surface for what a type already expresses;
   (b) `Inject<T>` as the name; rejected, it suggests constructor injection and would be confused with `#[inject]`.
2. **Any scope resolves.** `Scoped<CatsService>` for a singleton returns the singleton and for a transient a new
   instance. A provider can change scope without touching its consumers, and the type name says "per request" only in
   the sense of "resolved for this request". Alternative: error when `T` is not request-scoped. Rejected: it makes a
   scope change a breaking change for every handler.
3. **Error mapping.** `DiError::Factory` whose cause downcasts to `HttpExceptionError` returns that `HttpException`
   unchanged. Everything else (`NotFound`, `OutsideRequest`, `UnknownDependencies`, other factory errors) returns
   `InternalServerErrorException` with the default body, after logging the `DiError` text with the `Scoped` logger
   context. A context without a `ModuleRef` (errors raised before routing) is `OutsideRequest`. A `ContextType::Ws`
   context fails before resolving, with a log line that request scope is HTTP only. Alternative: put the `DiError`
   text in the `500` body. Rejected: it names internal providers and modules to any client.
4. **Guards, interceptors and filters use `Scoped::<T>::from_context(ctx)`.** No new method on `ExecutionContext`:
   `FromContext` is public, and one spelling is easier to document. The docs show it in a guard.
5. **Hint text.** The `RequestScopedDependency` message gains "take a `Scoped<T>` handler parameter" as the first
   option; the test of that message is updated.
6. **Example in cats-app.** A `RequestContext { request_id, started }` with a `Request` field, built from
   `X-Request-Id` or a random id; a `RequestIdInterceptor` sets the `X-Request-Id` response header from
   `Scoped<RequestContext>`; one cats handler returns the id in a debug route or logs it. The e2e test checks that two
   requests get different ids and that a given `X-Request-Id` is echoed. Alternative: tertulia. Rejected: tertulia's
   per-request user is already a good request-extension example, and cats-app is the reference example for core
   features.

## Risks / Trade-offs

- [Users read `Scoped<T>` as "T is request-scoped"] → docs state that it resolves any scope; the rustdoc says so first.
- [A `500` hides a misconfiguration during development] → the `DiError` is logged with the provider and module; the
  common-errors page lists the log line.
- [OpenAPI generation treating `Scoped<T>` as a body or query parameter] → it is a custom parameter like the existing
  `FromContext` types; the cats-app OpenAPI test guards it.

## Open questions

1. Name: `Scoped<T>` (issue's suggestion) or `RequestScoped<T>`? Recommended: `Scoped<T>`, since it resolves any scope.
2. Should `Scoped<T>` in a WebSocket handler context become meaningful later (per-message scope)? Not planned; the
   error keeps the door open.

## Review notes (2026-11-09)

- Q1: the parameter is named `Scoped<T>`.
- Q2: no per-message scope for WebSocket handlers is planned; the error stays.

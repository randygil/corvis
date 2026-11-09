# Proposal

Implements the consumption part of [randygil/corvis#115](https://github.com/randygil/corvis/issues/115) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Second of two changes; it starts after
`e13-request-scoped-providers` is archived, which adds `Scope::Request`, the per-request store and
`ModuleRef::resolve_for`.

## Why

After the first change, a handler reaches a request-scoped provider with two parameters and a call
(`module_ref: ModuleRef, req: Request` and `module_ref.resolve_for::<T>(&req).await`). The issue asks for a
`FromContext` parameter, so that the common case reads like constructor injection in the original
(`fn list(&self, ctx: Scoped<RequestContext>)`), and for the docs that still say there is no request scope (async local
storage recipe, migration guide) to describe it. Users migrating also need an example to copy.

## What Changes

- `corvis::Scoped<T>`: a `FromContext` type that resolves `T` (by type) for the current request from the module of the
  route's controller, with constructor-injection visibility. It derefs to `T` and works for any scope (request-scoped,
  transient or singleton), so code does not depend on the provider's scope.
- Usable as an undecorated handler parameter, and from guards, interceptors and route-level exception filters through
  `Scoped::<T>::from_context(ctx).await`.
- Errors: a factory error that carries an `HttpException` (for example a `400` when a tenant header is missing) is
  returned as that exception; every other resolution error becomes a generic `500` and is logged with its `DiError`
  message (provider names are not sent to the client). In a WebSocket context it fails with a `500` and a log line
  saying request scope is HTTP only.
- The `DiError::RequestScopedDependency` hint and the docs recommend `Scoped<T>` first, `ModuleRef::resolve_for` second.
- Docs (EN + ES): injection scopes (`Scoped<T>` section), async local storage recipe (request-scoped provider as the
  closest equivalent), migration guide (the "No request scope" difference becomes "Request scope without bubbling"),
  custom decorators (mention), common errors.
- Example: `examples/cats-app` gets a request-scoped `RequestContext` (request id from `X-Request-Id` or generated)
  used by a handler and an interceptor through `Scoped<T>`, with an e2e test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/injection-scopes`: adds the `Scoped<T>` parameter and its error mapping.
- `examples/cats-app`: adds the request-scoped provider example.

## Impact

- Code: a new `crates/corvis/src/di/scoped.rs` (or `common/param.rs`, next to the other `FromContext` impls at
  `crates/corvis/src/common/param.rs:31-67`), re-exported from `crates/corvis/src/lib.rs`;
  `crates/corvis/src/di/error.rs` (hint text); `examples/cats-app/src/` (new provider, handler, interceptor).
- Public API: one new type. No behavior change for existing applications.
- OpenAPI: `Scoped<T>` is a custom (`FromContext`) parameter and must not appear in the generated document; checked by
  the cats-app OpenAPI test (`examples/cats-app/tests/openapi.rs`).
- Ports: unaffected (they use no request scope).
- Docs (EN + ES): `fundamentals/injection-scopes.md`, `recipes/async-local-storage.md`, `migration-guide.md`,
  `custom-decorators.md`, `faq/common-errors.md`.

## Out of scope

- Everything already listed as out of scope in `e13-request-scoped-providers` (bubbling, durable providers,
  `INQUIRER`, request scope outside HTTP).
- `Scoped<T>` for trait objects registered under a string token (`Scoped` resolves by type); use
  `ModuleRef::resolve_token_for`. A token-based extractor can be a later issue.
- Pipes: `PipeTransform::transform` has no execution context (`crates/corvis/src/common/pipe.rs:73`). Giving pipes the
  context is a separate API change.
- Implicit access without a parameter (an `AsyncLocalStorage` equivalent through a task-local): new issue if needed.

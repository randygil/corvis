# Tasks

Prerequisite: `e13-request-scoped-providers` is implemented and archived.

## 1. Scoped<T>

- [x] 1.1 Add `Scoped<T>` (`Deref`, `into_inner`, `Debug` when `T: Debug`) next to the other `FromContext` impls
  (`crates/corvis/src/common/param.rs`) and re-export it from `crates/corvis/src/lib.rs` (and the prelude if there is
  one); rustdoc with a handler and a guard example; verify `cargo build -p corvis` and `cargo test -p corvis --doc`
- [x] 1.2 Implement `FromContext for Scoped<T>` per design decisions 1-3 (module from `ctx.module_ref()`, request from
  `ctx.request()`, error mapping, WebSocket context check, `Scoped` logger); verify with tasks 3.1-3.3
- [x] 1.3 Update the `DiError::RequestScopedDependency` hint (`crates/corvis/src/di/error.rs`) and its unit test
  (design decision 5); verify `cargo test -p corvis --lib di::error`

## 2. Example

- [x] 2.1 Add `RequestContext` (`#[injectable(scope = "request")]`, `Request` field, request id) and
  `RequestIdInterceptor` to `examples/cats-app/src/common/`, register them, and use `Scoped<RequestContext>` in one
  handler of `examples/cats-app/src/cats/cats_controller.rs`; verify `cargo build -p cats-app`
- [x] 2.2 Add `examples/cats-app/tests/request_scope.rs` for the two scenarios of `specs/examples/cats-app/spec.md`;
  verify `cargo test -p cats-app` (including the existing `openapi.rs` and `posture.rs`)

## 3. Regression tests

In `crates/corvis/tests/di/request_scope.rs` (created by the first change):

- [x] 3.1 Handler parameter and guard share the instance; different requests in flight get different instances;
  transient through `Scoped` (two parameters, two instances); singleton through `Scoped`; verify `cargo test -p corvis --test di request_scope`
- [x] 3.2 Factory returning `BadRequestException` gives `400`; provider not visible gives a `500` whose body does not
  name the provider; verify the same command. The log assertion (the `DiError` naming the provider is logged) goes in
  its own test binary, `crates/corvis/tests/scoped_error_log.rs`, because the logger is process-wide (same pattern as
  `crates/corvis/tests/cors_startup_warning.rs`); verify `cargo test -p corvis --test scoped_error_log`
- [x] 3.3 `Scoped::<T>::from_context` on a WebSocket context (feature `websockets`) fails and creates no instance
  (construction counter stays 0); verify the same command
- [x] 3.4 The scope error message contains `Scoped<RequestContext>`; verify the same command

## 4. Documentation

- [x] 4.1 `docs/fundamentals/injection-scopes.md`: a "`Scoped<T>`" subsection in the request scope section (handler,
  guard, interceptor, filter; any scope resolves; error mapping) and the example from cats-app; same in
  `docs/es/fundamentals/injection-scopes.md`; verify `npm run docs:check` in `docs/`
- [x] 4.2 `docs/recipes/async-local-storage.md`: add "Request-scoped providers" as the closest equivalent (a
  request-scoped `RequestContext` plus `Scoped<T>`), keep the request-extensions alternative, and update the status in
  the front matter (`unavailable` → `partial`: there is still no implicit access without a parameter); same in
  `docs/es/recipes/async-local-storage.md`; verify `npm run docs:check`
- [x] 4.3 `docs/migration-guide.md`: add `Scope.REQUEST` / `@Inject(REQUEST)` rows to the equivalences table (line 23
  area) and replace the "No request scope" item (line 58) with "Request scope without bubbling" (startup error, use
  `Scoped<T>`); same in `docs/es/migration-guide.md`; verify `npm run docs:check`
- [x] 4.4 `docs/custom-decorators.md`: mention `Scoped<T>` among the built-in `FromContext` types; and
  `docs/faq/common-errors.md`: the `Scoped` log line for a `500`; same in `docs/es/`; verify `npm run docs:check`

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features -- -D warnings`
- [x] 5.3 `cargo test --workspace` passes
- [x] 5.4 Regression tests: every scenario of both spec deltas maps to a test (list the mapping in the PR description)
- [x] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 5.6 Framework runtime code changed (`FromContext` impl, error text): run the `corvis-parity` check and verify the
  three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- With both #115 changes archived, close issue #115, tick it in epic #13 and move its card to Done (see the
  `corvis-task` skill).

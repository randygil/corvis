# Tasks

## 1. Scope and macro

- [x] 1.1 Add `Scope::Request` to `crates/corvis/src/di/provider.rs:30-39` with rustdoc (one instance per request, not
  stored, no hooks, how to consume it); decide Open question 1 (`#[non_exhaustive]`); verify `cargo build -p corvis`
- [x] 1.2 Replace `InjectableOptions { transient }` with an exact scope argument in
  `crates/corvis-macros/src/injectable.rs` (design decision 9), update the two callers
  (`crates/corvis-macros/src/controller.rs:134`, `crates/corvis-macros/src/gateway.rs:116`) and emit `fn scope()` for
  transient and request; update the `#[injectable]` doc in `crates/corvis-macros/src/lib.rs:24`; verify unit tests in
  `injectable.rs` for `"default"`, `"transient"`, `"request"`, `Scope::Request`, and the compile error for
  `"requested"` (`cargo test -p corvis-macros`)

## 2. Errors and the per-request store

- [x] 2.1 Add `DiError::OutsideRequest { token, chain }` and `DiError::RequestScopedDependency { consumer, dependency,
  chain }` to `crates/corvis/src/di/error.rs` with the messages of design decision 5 (no mention of other frameworks);
  verify unit tests on both messages in `error.rs`
- [x] 2.2 Create `crates/corvis/src/di/request_scope.rs` with `RequestScope` (keyed `OnceCell`s) and the
  `RequestScopeSlot` extension type (crate-private); verify a unit test that `get_or_try_init` runs the constructor once
  for two concurrent callers of one key and again after a failed attempt

## 3. Resolution

- [x] 3.1 Add `request: Option<RequestCtx>` to `ResolveCtx` (`crates/corvis/src/di/injectable.rs:97-104`), set to
  `None` in `for_module` and in `ModuleRef::ctx`; add `ResolveCtx::request() -> Result<Request, DiError>`; verify
  `cargo build -p corvis` and the existing `cargo test -p corvis --test di`
- [x] 3.2 Generalize `is_transient` into `effective_scope` (`crates/corvis/src/di/container.rs:424-440`) and branch
  `resolve_provider` (`container.rs:458-504`) per design decision 3 (singletons get `request: None`, transients
  inherit, request-scoped use the store after the path check); skip request-scoped providers in `instantiate_all`
  (`container.rs:550`); verify the existing DI tests still pass (`cargo test -p corvis --test di`)
- [x] 3.3 Reject request-scoped classes in `construct_injectable` (`container.rs:508-542`) with the enhancer error of
  design decision 6; verify with task 6.3
- [x] 3.4 `impl InjectField for Request` (`injectable.rs:177-220`) with `dependency()` reporting
  `Token::of::<Request>()`; verify a `#[injectable(scope = "request")]` struct with a `Request` field compiles in a
  test and that the macro-level dependency description lists the field

## 4. Startup checks

- [x] 4.1 Record `has_request_scoped` in `Container::scan` (`container.rs:241-304`); verify a unit test
- [x] 4.2 Extend `validate_transients` to request-scoped providers (`validate_lazy_providers`, design decision 7);
  verify with task 6.4
- [x] 4.3 Add the scope check over singleton providers and controllers (design decision 5), run after the lazy-provider
  validation in `scan`; verify with task 6.3
- [x] 4.4 Reject request-scoped `Provider::app_*` enhancers in `scan` (design decision 6), replacing the generic "the
  provider has no instance" of `application.rs:508` for that case; verify with task 6.3

## 5. Dispatch and ModuleRef

- [x] 5.1 Add `request_scope: bool` to `AppShared` (`crates/corvis/src/router/mod.rs:558-576`), set from the container
  in `init_steps` (`crates/corvis/src/application.rs:565-579`, the only place an `AppShared` is built; the testing
  client reuses it, `crates/corvis/src/testing.rs:384`); verify `cargo build -p corvis`
- [x] 5.2 In `dispatch` (`router/mod.rs:578`), install the slot when the flag is on and none is present, and remove it
  with a drop guard (design decisions 1 and Open question 3); verify with tasks 6.1 and 6.2
- [x] 5.3 Add `ModuleRef::resolve_for` and `resolve_token_for` (`crates/corvis/src/di/module_ref.rs`) per design
  decision 8, with rustdoc examples; verify with task 6.5

## 6. Regression tests

All in a new `crates/corvis/tests/di/request_scope.rs` (registered in `crates/corvis/tests/di/main.rs`), driving
requests through the testing client or `listen_in_background` on `127.0.0.1:0`. One test per scenario of
`specs/core/injection-scopes/spec.md` and `specs/core/module-ref/spec.md`.

- [x] 6.1 One instance per request, shared across consumers, scope set on the provider, alias of a request-scoped
  provider, and the concurrent test (50 requests over a real socket, each handler awaits a short sleep between writing
  and reading its value); verify `cargo test -p corvis --test di request_scope`
- [x] 6.2 Not created at startup, released after the response (`Weak` test, also with a handler that panics and with a
  client that disconnects before the response), `NotFound` from `get`, no lifecycle hooks; verify the same command
- [x] 6.3 Startup errors: controller field, through a transient, `Request` field in a singleton, singleton factory
  resolving a request-scoped provider, request-scoped `app_guard`, request-scoped guard referenced by type with
  `#[use_guards]`, request-scoped provider depending on a singleton (starts); verify the same command
- [x] 6.4 Lazy-provider validation: missing dependency and cycle of request-scoped providers; verify the same command
- [x] 6.5 `resolve_for`/`resolve_token_for`: singleton, request-scoped factory by token with `ResolveCtx::request()`,
  not visible, request never dispatched, gateway message handler (feature `websockets`), spawned task after the
  response, standalone context; verify the same command
- [x] 6.6 Request field and shared extensions (middleware resolves first, guard inserts a value later); verify the same
  command
- [x] 6.7 Check the examples still start: `cargo test -p cats-app -p tertulia -p blog-orm`; verify they pass

## 7. Performance

- [x] 7.1 Run the synthetic benchmark (`bash bench/run.sh`, see `openspec/specs/bench/synthetic/spec.md`) on the commit before and
  after this change for an app without request-scoped providers; verify the throughput difference is within the
  benchmark's noise and record both numbers in the PR description

## 8. Documentation

- [x] 8.1 `docs/fundamentals/injection-scopes.md`: add `Scope::Request` to the table, a "Request scope" section
  (declaration, `Request` field, `ResolveCtx::request`, `ModuleRef::resolve_for`, what may depend on what, the startup
  error and why there is no bubbling, outside-a-request errors, params seen by a provider first resolved in a
  middleware), replace "There is no request scope" (line 14) and update "What is not available" (line 89: durable
  providers, bubbling, `INQUIRER`); change the front matter `status` if the page becomes complete; same in
  `docs/es/fundamentals/injection-scopes.md`; verify `npm run docs:check` in `docs/`
- [x] 8.2 `docs/faq/common-errors.md`: replace the row for "request-scoped providers are not supported" (line 94) with
  rows for the new compile error and the two new `DiError` messages; same in `docs/es/faq/common-errors.md`; verify
  `npm run docs:check`

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features -- -D warnings` (the request store must not depend on `websockets`)
- [x] 9.3 `cargo test --workspace` passes
- [x] 9.4 Regression tests: every scenario of both spec deltas maps to a test in
  `crates/corvis/tests/di/request_scope.rs` (list the mapping in the PR description)
- [x] 9.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 9.6 Framework runtime code changed (DI container, router dispatch, macros): run the `corvis-parity` check and
  verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Issue #115 stays open until `e13-request-scoped-providers-scoped-parameter` is archived too; then close #115, tick it
  in epic #13 and move its card to Done (see the `corvis-task` skill).

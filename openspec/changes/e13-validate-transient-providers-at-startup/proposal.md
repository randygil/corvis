# Proposal

Implements [randygil/corvis#114](https://github.com/randygil/corvis/issues/114) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

A singleton with a missing dependency stops the application at startup with a clear message. A transient provider
with the same mistake starts fine: transient providers are skipped at startup, so the error only appears the first
time something resolves the provider. That may be a request in production, long after deploy. The behavior is
inconsistent and moves a configuration error to request time. Transient providers must not be *created* at startup (the
docs and #15's spec say so), but their dependencies can be *checked* without creating them. Found while drafting the DI
test coverage change (#15).

## What Changes

- `#[injectable]` (and the macros that share its expansion: `#[controller]`, `#[module]`, `#[websocket_gateway]`)
  also generates a static description of the class's injected fields: token, position, and whether the field is
  optional. Fields with `#[init]`/`#[default]`, `ModuleRef` fields and fields filled with `Default` are not
  dependencies. The description comes from a new provided method `Injectable::dependencies()`, which returns `None`
  (not declared) by default for hand-written implementations.
- `Provider` keeps that description for class providers (`class`, `use_class`, `use_class_as`, and the `app_*`
  enhancer providers), an empty list for value providers, the target token for aliases (`use_existing`), and "unknown"
  for factory providers.
- While the application is created (`CorvisFactory::create`, `Test::create_testing_module().compile()`), right after
  the module graph is scanned and before any provider is instantiated, the container walks the dependencies of every
  transient provider. A missing required dependency fails startup with the same `DiError` the first resolution would
  have returned (`UnknownDependencies` for a field, `NotFound` for an alias target), naming the provider, the token
  and the module. Dependencies on other transient providers are followed. A cycle made only of transient providers
  fails with `DiError::Circular`.
- Nothing is instantiated by the check: transient providers are still created only when resolved.
- Regression tests and a docs update in English and Spanish.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/injection-scopes`: adds the requirement that transient providers' dependencies are validated at startup. The
  capability is introduced by #15 (`e13-di-test-coverage`). This delta only adds a requirement, so it applies whichever
  change is archived first.

## Impact

- Code: `crates/corvis-macros/src/injectable.rs` (generate `dependencies()`), `crates/corvis/src/di/injectable.rs`
  (`Injectable::dependencies`, `InjectField::dependency`, the `Dependencies`/`Dependency` types),
  `crates/corvis/src/__private.rs` (dependency description through the existing `Probe` dispatch),
  `crates/corvis/src/di/provider.rs` (store the description), `crates/corvis/src/di/container.rs` (the check),
  `crates/corvis/src/application.rs` (call it in `bootstrap`).
- Public API: two provided trait methods and two small public types. Existing hand-written `Injectable` and
  `InjectField` implementations compile unchanged.
- Behavior: an application with a transient provider whose required dependency is not visible now fails at startup
  instead of at first use. That is the purpose. The three port apps have no transient providers (no
  `scope = "transient"` or `Scope::Transient` in `corvis-tests/corvis/*`), so their startup is unchanged.
- Tests: `crates/corvis/tests/di/` (created with #15's layout if #15 has not landed), a macro-level test of the
  generated description.
- Docs (EN + ES): `fundamentals/injection-scopes.md`, `faq/common-errors.md`.

## Out of scope

- Dependencies requested inside factory providers (`ctx.get` in a closure) cannot be known without running the
  factory, so transient factory providers are not validated. Documented.
- Validating singletons statically before instantiation (they are already validated by being instantiated).
- Transient guards, pipes, interceptors and filters constructed by type (`construct_injectable`): they are built
  during `init()` and already fail there.
- The `#[optional]` attribute and `Option<Arc<T>>` semantics (issue #14).

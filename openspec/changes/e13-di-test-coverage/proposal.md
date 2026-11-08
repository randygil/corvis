# Proposal

Implements [randygil/corvis#15](https://github.com/randygil/corvis/issues/15) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The documentation describes the dependency injection container in detail (injection scopes, `ModuleRef`, custom
providers, circular dependencies, global and dynamic modules), and the ports rely on it, but the docs review found that
most of those promises have no test. The only DI tests today are two unit tests of `Token`
(`crates/corvis/src/di/token.rs:123-139`); `crates/corvis/tests/modules.rs` boots the built-in modules but never checks
a transient instance, a `ModuleRef` lookup, an alias, a cycle, a global module or a user-written dynamic module. A
refactor of `crates/corvis/src/di/container.rs` could silently change any of these behaviors and nothing would fail.

This change states the documented behavior as requirements and checks each one with a regression test through the
public API, so the next DI changes (#14, #23) have a safety net.

## What Changes

- New integration test target `crates/corvis/tests/di/` (one binary, one module per area) using only the public API
  (`corvis::prelude`, `corvis::testing::Test`, `DiError`):
  - **Transient scope**: one instance per consumer, a new one per `ModuleRef::resolve`, not created at startup, not
    stored (`get` does not find it), no lifecycle hooks, no scope propagation to singleton consumers; both the
    `#[injectable(scope = "transient")]` and the `.scope(Scope::Transient)` forms.
  - **`ModuleRef`**: located in the owning module; `get` / `get_token` (module visibility first, then the whole
    application), `get_token_in_context`, `get_strict`, `resolve`, `create`, `module_name`; `NotFound` and
    `TypeMismatch` errors.
  - **`use_existing`**: the alias hands over the same `Arc`, resolves the existing token with the alias module's
    visibility, can be exported, chains, and fails at startup when the target is not visible.
  - **`use_class_as`**: the trait object is injected, the class's own fields are injected from the registering module,
    the scope of the class is inherited, the concrete type is not available under that token, and runtime selection
    by token works.
  - **Cycle detection**: provider cycles (two and three providers, self-dependency, across modules, through factories,
    through aliases, through a transient provider) fail with `DiError::Circular` and the full chain, never with a hang
    or a stack overflow; static module import cycles and re-export cycles are accepted.
  - **Global modules**: the three declaration forms, visibility without imports, the "must be in the graph" and "only
    exports count" rules, lookup precedence, `DynamicModule::global(...)` overriding the attribute, one shared instance.
  - **User-defined dynamic modules**: static and dynamic parts merged, dynamic-only providers private unless exported,
    `.import(...)`, `.controller(...)`, one node per dynamic import, factories resolved in the dynamic module's context,
    invalid exports rejected at startup.
- Small, local fixes for any behavior these tests show to be wrong, each with its regression test; anything larger
  becomes its own issue (see design.md, Decision 6).
- Docs: the DI pages are checked against the tests and corrected in English and Spanish only where a test contradicts
  them.

## Capabilities

### New Capabilities

- `core/injection-scopes`: singleton and transient instance rules.
- `core/module-ref`: programmatic access to the container and its lookup rules.
- `core/custom-providers`: aliases (`use_existing`) and implementations behind a trait (`use_class_as`).
- `core/circular-dependencies`: provider cycles are detected and reported; module import cycles are allowed.
- `core/global-modules`: visibility of global modules' exports.
- `core/dynamic-modules`: how a user-defined `DynamicModule` combines with its static definition.

### Modified Capabilities

None (no `core/*` specs exist yet).

## Impact

- `crates/corvis/tests/di/` (new test target `di`): about seven files, test-only. No runtime dependency or public API
  change, unless Decision 6 applies to a small fix.
- Test time: a few dozen small applications built in memory; expected well under 5 s for `cargo test -p corvis --test
  di`.
- One more test binary to link in the shared target directory (kept to one on purpose, see design.md Decision 1).
- Docs pages possibly touched (EN and ES): `fundamentals/injection-scopes.md`, `fundamentals/module-ref.md`,
  `fundamentals/custom-providers.md`, `fundamentals/circular-dependency.md`, `fundamentals/dynamic-modules.md`,
  `modules.md`.

## Out of scope

- Wording of DI error messages, the `forwardRef()` hint in the circular error (`di/error.rs:15-18`) and lifecycle hooks
  lost by `use_class_as` (`di/provider.rs:154-169`): issue #14. Tests here assert error variants and the cycle chain,
  not the full message, and do not assert hooks for `use_class_as`.
- Request scope: issue #23.
- Dynamic module import recursion: when module A's `#[module]` imports `B::for_root()` and B's imports
  `A::for_root()`, `Scanner::add_import` (`di/container.rs:124-131`) creates a new node on every dynamic import and
  recurses without bound (stack overflow instead of a `DiError`). Candidate new issue.
- Cycles through `ModuleRef::resolve` called inside a factory are not detected, because `ModuleRef` starts with an
  empty resolution path (`di/module_ref.rs:78-86`); a factory that resolves its own token this way recurses without
  bound. Candidate new issue.
- A transient provider with an unresolvable dependency is only reported when it is first resolved, not at startup,
  because `instantiate_all` skips transient providers (`di/container.rs:407-409`). Candidate new issue (startup
  validation).
- Compile-fail tests for the macros (for example `#[global]` above `#[module]`, `scope = "request"`): would need a new
  dev-dependency such as `trybuild`; candidate for a macro test-coverage issue.
- Enhancers (guards, pipes, interceptors, filters) built by `construct_injectable` and global enhancers registered as
  providers: covered by issue #17.
- The testing utilities' overrides (`override_provider`, `override_module`): only used here as the harness, not tested.

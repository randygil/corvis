# Tasks

## 1. Dependency description

- [x] 1.1 Add `Dependencies` and `Dependency` to `crates/corvis/src/di/injectable.rs` (re-export from
  `crates/corvis/src/di/mod.rs`), the provided method `Injectable::dependencies() -> Option<Dependencies>` (default
  `None`) and `InjectField::dependency(token, index) -> Option<Dependency>` (default `None`; required for `Arc<T>`,
  optional for `Option<Arc<T>>`), with rustdoc; verify `cargo build -p corvis` and that `OrmService`
  (`crates/corvis-orm/src/module.rs:169`) compiles unchanged (`cargo build -p corvis-orm`)
- [x] 1.2 Add `dependency::<T>(token, index)` to `InjectTag` (calls `T::dependency`) and `DefaultTag` (returns `None`)
  in `crates/corvis/src/__private.rs`; make `default_injectable!` return an empty list; verify `cargo build -p corvis`
- [x] 1.3 Generate `fn dependencies()` in `injectable_impl` (`crates/corvis-macros/src/injectable.rs`) per design
  decision 1, reusing `name_str` and `deps_lits`; verify with a test in `crates/corvis/tests/di/transient_validation.rs`
  that a class with `Arc<A>`, `#[inject("T")] Arc<B>`, `#[inject("PORT")] u16`, `Option<Arc<C>>`, `ModuleRef`,
  `#[init(..)]` and a `Default` field reports exactly the four expected items with the right indexes and optionality
  (create `crates/corvis/tests/di/main.rs` with #15's layout if needed)

## 2. Providers keep the description

- [x] 2.1 Add `deps: DeclaredDeps` to `Provider` (`crates/corvis/src/di/provider.rs:110-118`) and set it in `new`
  (`Unknown`), `use_class`, `use_class_as`, `app_*` (`T::dependencies()`), `use_value_arc` (empty), `use_existing`
  (`Alias`); verify a unit test in `provider.rs` checks the variant of each constructor

## 3. Startup check

- [x] 3.1 Extract the `UnknownDependencies` construction of `ResolveCtx::resolve_dependency`
  (`crates/corvis/src/di/injectable.rs:116-131`) into a crate-private helper and use it there; verify
  `cargo test -p corvis --test app missing_dependency_message` still passes unchanged
- [x] 3.2 Implement `Container::validate_transients` (design decision 3: depth-first walk, `path` for cycles, `done`
  set, first error wins) and call it at the end of `Container::scan` (`crates/corvis/src/di/container.rs:191-251`);
  verify with task 4.1
- [x] 3.3 Confirm no transient provider is constructed by the check (a static counter in a test fixture stays at 0 after
  `compile()`); verify with task 4.1

## 4. Regression tests

- [x] 4.1 In `crates/corvis/tests/di/transient_validation.rs`, one test per scenario of
  `specs/core/injection-scopes/spec.md`: unused transient with a missing dependency (assert the `UnknownDependencies`
  message names the provider, token, `[0]` and the module in the same wording `missing_dependency_message` checks for
  singletons), not instantiated by the check, transitive transient dependency, optional dependency absent,
  alias with a missing target (`NotFound`), cycle of transient providers (`Circular`), transient factory provider not
  checked, transient provider replaced by `override_provider`; verify
  `cargo test -p corvis --test di transient_validation`
- [x] 4.2 Check that the examples still start: `cargo test -p cats-app -p tertulia -p blog-orm`; verify they pass

## 5. Documentation

- [x] 5.1 `docs/fundamentals/injection-scopes.md`: extend "They are not created at startup" with "but their
  dependencies are checked at startup" (class providers and aliases; factory providers are checked when first
  resolved); same in `docs/es/fundamentals/injection-scopes.md`; verify `npm run docs:check` in `docs/`
- [x] 5.2 `docs/faq/common-errors.md`, entry "Corvis can't resolve dependencies of the ...": add that the error can
  come from a transient provider that nothing injects yet; same in `docs/es/faq/common-errors.md`; verify
  `npm run docs:check`

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes
- [x] 6.4 Regression tests: every scenario of `specs/core/injection-scopes/spec.md` maps to a test in
  `crates/corvis/tests/di/transient_validation.rs` (list the mapping in the PR description)
- [x] 6.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 6.6 Framework runtime code changed (DI container, providers, macros): run the `corvis-parity` check and verify the (not run by the implementer: the maintainer runs it)
  three port apps (consi, facten, houseplan) keep 0 apidiff differences (maintainer batch run on batch/phase03-d 2780936: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #114, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

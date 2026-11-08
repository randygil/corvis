# Tasks

## 1. Messages and rustdoc

- [x] 1.1 Replace the `DiError::Circular` text in `crates/corvis/src/di/error.rs` with the message of design decision 1 (no `forwardRef`, the three remedies, chain format unchanged); add a test in `crates/corvis/tests/app.rs` (next to `missing_dependency_message`) that compiles a module with `AService <-> BService` and asserts the chain, `ModuleRef`, and the absence of `forwardRef`; verify with `cargo test -p corvis --test app circular`
- [x] 1.2 Replace `@Module` with `#[module]` in `DiError::UnknownDependencies`; extend `missing_dependency_message` to assert `#[module(` is present and `@Module` is absent; verify with `cargo test -p corvis --test app missing_dependency_message`
- [x] 1.3 Rewrite the `Scope` rustdoc in `crates/corvis/src/di/provider.rs:30` (design decision 3) and update the module doc comment of `error.rs` if it still claims to mirror another container's wording; verify with `cargo doc -p corvis --no-deps` showing no `Request` scope and no new rustdoc warnings

## 2. `#[optional]` rejected

- [x] 2.1 In `crates/corvis-macros/src/injectable.rs`, make the `"optional"` arm return a `syn::Error` spanned on the attribute with the message of design decision 4, and update the function's doc comment (line 13); verify by reading the generated error once with a scratch struct in a test (see 2.2)
- [x] 2.2 Add a `#[cfg(test)]` module in `corvis-macros/src/injectable.rs` that parses a struct with `#[optional] cats: Arc<CatsService>` via `syn::parse_quote!`, calls `injectable_impl`, and asserts an `Err` whose message mentions `Option<Arc<T>>`; plus one case without the attribute that returns `Ok`; verify with `cargo test -p corvis-macros`
- [x] 2.3 Add a regression test in `crates/corvis/tests/app.rs` for an `Option<Arc<CatsService>>` field with no provider: the app compiles and the field is `None`; verify with `cargo test -p corvis --test app optional`

## 3. `use_class_as` keeps hooks

- [x] 3.1 Introduce the crate-private `Built { instance, hook_target }` factory result in `crates/corvis/src/di/provider.rs` and update every constructor (`use_class`, `use_class_as`, `use_value_arc`, `use_factory_arc`, `use_existing`) and the single call site in `Container::resolve_provider` (`container.rs:357`); no behavior change yet; verify `cargo test -p corvis` still passes
- [x] 3.2 Add `hook_targets` to `Container`, fill it in `resolve_provider` only when the singleton is cached, and add `Container::hook_instance(module, token)`; switch `call_hook` (`container.rs:458`), `discovery::discover` (`discovery.rs:97`) and `websockets::build_gateways` (`websockets.rs:553`) to it; verify `cargo test -p corvis --all-features` still passes
- [x] 3.3 Make `use_class_as` return `hook_target: Some(erase(arc_t.clone()))` and set `p.hooks = ErasedHooks::from_typed(T::hooks())`; update its rustdoc to say hooks are kept; verify with 3.4
- [x] 3.4 Add `crates/corvis/tests/di_use_class_as.rs`: (a) a `MemoryRepo` behind `dyn Repo` with `OnModuleInit` + `OnApplicationShutdown` counters, asserting one call each across `init()`/`close()` and that the init state is visible through the `Arc<dyn Repo>`; (b) an `#[on_event]` handler on a class behind a trait, asserting it runs once; (c) a transient `T` behind a trait gets distinct instances and no hooks; verify with `cargo test -p corvis --test di_use_class_as`

## 4. Documentation (EN + ES)

- [x] 4.1 `docs/fundamentals/circular-dependency.md` and `docs/faq/common-errors.md`: show the new message and remove the sentences explaining that the message mentions `forwardRef()` (keep "Corvis has no `forwardRef`" in `docs/migration-guide.md`); same in `docs/es/`; verify by grepping `forwardRef()` in both trees returns only the migration guide
- [x] 4.2 `docs/providers.md:97-99`: replace the `#[optional]` info box with a short note that `#[optional]` is rejected and `Option<Arc<T>>` is the way; add the compile error to the "Attribute compile errors" section of `docs/faq/common-errors.md`; same in `docs/es/`; verify by reading both pages
- [x] 4.3 `docs/fundamentals/custom-providers.md:137-139`: remove the "Hooks and `use_class_as`" warning and state that hooks, `#[handlers]` and gateways work as with `use_class`; same in `docs/es/`; verify by reading both pages
- [x] 4.4 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes
- [x] 5.4 Regression tests from 1.1, 1.2, 2.2, 2.3 and 3.4 are in place and fail on the code before this change
- [x] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 5.6 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences (`facten-api` uses `use_class_as`)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #14, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

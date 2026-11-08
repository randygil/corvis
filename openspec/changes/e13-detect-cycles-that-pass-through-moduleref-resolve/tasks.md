# Tasks

## 1. Failing tests first

- [ ] 1.1 Create `crates/corvis/tests/di/module_ref_cycles.rs` and declare it in `crates/corvis/tests/di/main.rs`
  (create `main.rs` with only this module, following #15's layout, if #15 has not landed). Write the four
  construction scenarios of the first requirement (factory resolving its consumer, factory resolving itself with a
  call counter, stored `ModuleRef` of another provider, `ModuleRef::create`), each under
  `tokio::time::timeout(Duration::from_secs(5), compile())`; verify they fail today (timeout or abort) and record which
  in the PR description

## 2. Task-local resolution path

- [ ] 2.1 Declare `tokio::task_local! { static RESOLUTION_PATH: Arc<Vec<(ModuleId, Token)>>; }` in
  `crates/corvis/src/di/container.rs` and await the factory in `resolve_provider` (`:357`) inside
  `RESOLUTION_PATH.scope(path, ..)`; verify the existing DI tests still pass (`cargo test -p corvis --test app --test
  modules`)
- [ ] 2.2 Wrap `T::construct(ctx)` in `construct_injectable` (`container.rs:392`) in the same scope with its path;
  verify `cargo test -p corvis --test app` (guards, pipes and interceptors constructed by type)
- [ ] 2.3 Make `ModuleRef::ctx` (`crates/corvis/src/di/module_ref.rs:111-119`) start from
  `RESOLUTION_PATH.try_with(|p| p.clone()).unwrap_or_default()`; add a short comment saying why; verify the tests of
  task 1.1 now pass with `DiError::Circular` and the expected chains

## 3. No false positives

- [ ] 3.1 In `module_ref_cycles.rs`, add the run-time scenarios of the second requirement: transient `Job` resolving a
  fresh copy of itself after construction, singleton `Orders` resolving a transient dependent after `init()`, and ten
  concurrent `resolve::<Job>()` calls returning distinct instances (`Arc::ptr_eq`); verify
  `cargo test -p corvis --test di module_ref`
- [ ] 3.2 Add one test where a factory runs two resolutions with `futures::join!` (one through `ctx.get`, one through
  `ctx.module_ref().resolve`) without a cycle, to check that each branch keeps its own path; verify it passes

## 4. Documentation

- [ ] 4.1 `docs/fundamentals/circular-dependency.md`, section "Resolve one of the sides later with `ModuleRef`":
  replace the closing warning with: calling `resolve`/`create` for the other side inside a constructor or factory
  fails at startup with the circular-dependency error (show the chain), and work spawned with `tokio::spawn` inside a
  constructor is not covered; same in `docs/es/fundamentals/circular-dependency.md`; verify `npm run docs:check` in
  `docs/`
- [ ] 4.2 `docs/fundamentals/module-ref.md`: one sentence next to the `resolve` row or example saying cycles through
  `ModuleRef` during construction are detected; same in `docs/es/`; verify `npm run docs:check`
- [ ] 4.3 `docs/faq/common-errors.md` "Circular dependencies": mention that the chain can pass through a `ModuleRef`
  call in a constructor or factory; same in `docs/es/faq/common-errors.md`; verify `npm run docs:check`

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes
- [ ] 5.4 Regression tests: every scenario of `specs/core/circular-dependencies/spec.md` maps to a test in
  `crates/corvis/tests/di/module_ref_cycles.rs`, all cycle tests run under a timeout (list the mapping in the PR
  description)
- [ ] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [ ] 5.6 Framework runtime code changed (`di/container.rs`, `di/module_ref.rs`): run the `corvis-parity` check and
  verify the three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #113, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

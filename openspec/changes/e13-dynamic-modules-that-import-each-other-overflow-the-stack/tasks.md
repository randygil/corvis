# Tasks

## 1. Error variant

- [ ] 1.1 Add `DiError::CircularImport { chain: String }` to `crates/corvis/src/di/error.rs` with the message of
  design decision 2 (no `forwardRef`, no static-reuse advice) and a rustdoc line; verify a unit test in the same file
  that formats a value and finds the chain and the words "dynamic import"

## 2. Detection in the scanner

- [ ] 2.1 Add the import chain to `Scanner` (`crates/corvis/src/di/container.rs:66-70`): push an entry
  (`type_id`, module name, dynamic or not) in `add_import` before `fill` and pop it afterwards on both success and
  error; verify the existing tests that boot modules still pass (`cargo test -p corvis --test modules --test app`)
- [ ] 2.2 Before filling a `ModuleImport::Dynamic` (`container.rs:124-131`), look for the same (importing class,
  imported class, dynamic) edge on the chain and return `DiError::CircularImport` with the chain from the earlier
  importer to the new import (design decision 1); verify with task 3.1

## 3. Regression tests

- [ ] 3.1 Create `crates/corvis/tests/di/dynamic_import_cycles.rs` and declare it in `crates/corvis/tests/di/main.rs`
  (create `main.rs` with only this module if #15 has not landed yet). Cover: two dynamic modules (chain
  `AModule -> BModule -> AModule`), three modules (`AModule -> BModule -> CModule -> AModule`), root imported as
  `AModule::for_root()`, and the message check. Each `compile()` runs under `tokio::time::timeout(5 s)` and the test
  asserts `err.downcast_ref::<DiError>()` is `CircularImport`; verify `cargo test -p corvis --test di dynamic_import`
- [ ] 3.2 In the same file, cover the graphs that must keep starting: static mutual imports, `A::for_root()` → static
  `B` → `A::for_root()` (two `AModule` nodes, application starts), and the same dynamic module imported from two
  branches; verify `cargo test -p corvis --test di dynamic_import`
- [ ] 3.3 Temporarily remove the check from task 2.2 and run task 3.1 to confirm the tests fail (timeout or abort)
  without it; restore it; record the result in the PR description

## 4. Documentation

- [ ] 4.1 `docs/fundamentals/circular-dependency.md`: in "Cycles between modules: allowed", add that the rule covers
  static modules, and that dynamic modules importing each other fail at startup with the new error (show the message
  and the two remedies); same change in `docs/es/fundamentals/circular-dependency.md`; verify `npm run docs:check` in
  `docs/`
- [ ] 4.2 `docs/fundamentals/dynamic-modules.md` "Each call is a new instance": one paragraph linking to the cycle
  section; same in `docs/es/`; verify `npm run docs:check`
- [ ] 4.3 `docs/faq/common-errors.md`: add an entry headed with the start of the new message, cause and fix, next to
  "Circular dependencies"; same heading level in `docs/es/faq/common-errors.md`; verify `npm run docs:check` (the
  structure check requires the same headings and code fences in both languages)

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes
- [ ] 5.4 Regression tests: every scenario of `specs/core/circular-dependencies/spec.md` maps to a test in
  `crates/corvis/tests/di/dynamic_import_cycles.rs` (list the mapping in the PR description)
- [ ] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [ ] 5.6 Framework runtime code changed (`di/container.rs`, `di/error.rs`): run the `corvis-parity` check and verify
  the three port apps (consi, facten, houseplan) keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #112, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

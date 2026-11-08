# Design

## Context

Current behavior, read in the code of this worktree:

- **Scanner.** `Container::scan` (`crates/corvis/src/di/container.rs:191-251`) creates the internal core module and
  then calls `Scanner::add_import(root)` (`:208`). `Scanner` (`:66-70`) holds the nodes, a `static_index`
  (`TypeId` → node) and the testing overrides; it has no notion of "currently being filled".
- **Static imports terminate.** `add_import` for `ModuleImport::Static` (`:113-123`) returns the existing node when
  the class is already in `static_index` (`:114-116`), and inserts the new node into `static_index` *before* calling
  `fill` (`:119-121`). A static cycle therefore stops at the second visit. The docs promise this
  (`docs/fundamentals/circular-dependency.md:9-21`).
- **Dynamic imports always recurse.** `add_import` for `ModuleImport::Dynamic` (`:124-131`) calls the module's
  `definition()` through `dm.base` (`:125`), always creates a new node (`:126`), registers it in `static_index` only
  with `or_insert` (`:128`, so later *static* imports of the class reuse it), and calls `fill` (`:129`). `fill`
  (`:136-186`) calls `add_import` for every import of the definition and of the dynamic part (`:157-163`). Nothing
  checks the chain, so `A` → `B::for_root()` → `A::for_root()` → `B::for_root()` … recurses until the stack
  overflows. `#[module(imports = [B::for_root()])]` evaluates `B::for_root()` every time `A::definition()` runs, so
  each level produces a fresh `DynamicModule` (`crates/corvis/src/di/module.rs:92-95` for `ModuleImport`,
  `:111-118` for `DynamicModule`, whose `base` is `M::definition`).
- **Errors.** `DiError` (`crates/corvis/src/di/error.rs:5-33`) is a public `thiserror` enum, not
  `#[non_exhaustive]`. `Circular { chain }` (`:15-18`) is the provider-cycle error. Its text is being rewritten by
  #14 (`openspec/changes/e13-di-misleading-messages-and-no-op-attributes/design.md`, decision 1).
- **Startup path.** `CorvisApplication::bootstrap` (`crates/corvis/src/application.rs:95-123`) runs
  `Container::scan` and `instantiate_all` (`:108-112`), logs the `DiError` through `ExceptionHandler` and returns it as
  `anyhow::Error` (`:114-122`; `abort_on_error` exits the process instead). `TestingModuleBuilder::compile` goes through
  the same function, so tests can `downcast_ref::<DiError>()`.
- **Docs.** `docs/fundamentals/dynamic-modules.md:160-166` ("Each call is a new instance") explains that every
  `DynamicModule` in an `imports` list creates a node, and says not to rely on the static-reuses-dynamic detail
  (`:166`). `docs/faq/common-errors.md` lists DI errors by message (`:10-73`).
- **Port apps** (`corvis-tests/corvis/*`, read only): their dynamic imports are `ConfigModule::for_root` and
  `ScheduleModule::for_root` from the root (`consi-backend/src/app.module.rs:29-30`, `houseplan-api/src/app.module.rs:14`)
  and `OrmModule::for_root` / `for_root_async` inside a database module. None of them imports a dynamic module from a
  module that the dynamic module imports back.

## Goals / Non-Goals

**Goals:**
- Every module graph either finishes scanning or fails with a `DiError`. Scanning never aborts the process.
- The error names the modules of the cycle and tells the user how to break it.
- No graph that scans successfully today is rejected.

**Non-Goals:**
- Making mutual dynamic imports work (that needs dynamic-module deduplication, see proposal "Out of scope").
- Changing provider-cycle detection or any DI message other than the new one.

## Decisions

1. **Detect a repeated dynamic import edge on the current chain.** `Scanner` gains a stack
   `chain: Vec<ChainEntry { type_id, name, dynamic }>`. `add_import` pushes the entry before `fill` and pops it after
   (also on error, so the stack stays consistent; with `?` this is a small guard or an explicit pop before returning).
   Before a `Dynamic` import of class `C` is filled from the module at the top of the stack (class `P`), the scanner
   looks for an index `i` with `chain[i].type_id == P`, `chain[i + 1].type_id == C` and `chain[i + 1].dynamic`. If one
   exists, it returns `DiError::CircularImport` with the names `chain[i..]` plus `C`'s name. Termination: with no
   repeated (parent class, child class) dynamic edge, a chain can have at most one dynamic level per distinct pair of
   module classes, and static levels cannot repeat (`static_index`), so every chain is finite.
   - Alternative: fail as soon as a class appears twice on the chain. Rejected: it rejects
     `A::for_root()` → static `B` → `A::for_root()`, which terminates today (the second `A` imports the static `B`,
     which is already indexed) and is covered by a "keeps starting" scenario.
   - Alternative: a fixed depth limit (for example 64 nested modules). Rejected: arbitrary, and the error could not
     name the cycle.
   - Alternative: reuse the earlier node of the same class (as static imports do). Rejected: the earlier node may
     have been configured with different options, so `B` would silently get the wrong `A`. Deduplication by options
     is out of scope.

2. **A new variant, `DiError::CircularImport { chain: String }`.** Message:
   `A circular import between dynamic modules has been detected: {chain}. Each dynamic import (for example
   for_root()) creates a new module instance, so this cycle never ends. Move what both modules need into a separate
   module that both import, or remove one of the two imports.`
   The chain uses module names (`ModuleDefinition::name`) joined with ` -> `, the same format as provider cycles.
   - Alternative: reuse `DiError::Circular`. Rejected: its text talks about providers and the remedies differ;
     callers and tests could not tell the two cases apart.
   - Alternative: `DiError::Other(String)`. Rejected: not matchable in tests and inconsistent with the other DI errors.
   - The message does not suggest importing the static class on one side. That works today only because a static
     import reuses an earlier dynamic node, which the docs tell users not to rely on
     (`docs/fundamentals/dynamic-modules.md:166`).

3. **Only dynamic imports are checked.** Static imports keep their current path (`static_index` lookup first), so
   static cycles keep working and pay no cost. The check is a linear scan of the chain per dynamic import. Chains are
   a handful of modules deep, so the cost is negligible.

4. **Tests live in the `di` integration target.** Add `crates/corvis/tests/di/dynamic_import_cycles.rs` (a module
   of `crates/corvis/tests/di/main.rs`). If #15 has not created that target yet, create `main.rs` with only this
   module, following #15's layout (`openspec/changes/e13-di-test-coverage/design.md`, decision 1), so #15 adds its
   modules next to it later. Each failing case runs `compile()` under `tokio::time::timeout`. A regression then shows
   up as a timeout or as a stack overflow that aborts the binary, which is loud either way.

## Risks / Trade-offs

- [A graph that repeats a dynamic edge but still terminates (for example `T::for_root()` → `X::a()` →
  `T::for_feature()` → `X::b()` with no further imports) is now rejected] → it needs the same importer and imported
  classes to appear twice on one chain with different configurations. No built-in module or port app does this.
  The message explains how to restructure. Recorded as an open question.
- [Adding a variant to a public enum breaks exhaustive matches downstream] → none in the workspace or the ports; the
  project is pre-1.0; noted in the proposal.
- [Partially built nodes are left in the scanner when the error is returned] → the scanner is dropped with the error,
  and no provider has been instantiated at that point (`instantiate_all` runs after `scan`).

## Open questions

1. Is the rare false positive of decision 1 (same dynamic edge twice on one chain, terminating) acceptable, or should
   detection wait for a second full repetition of the cycle before failing?
2. Should `DiError` become `#[non_exhaustive]` now (in this change) since a variant is being added anyway? Proposed:
   no, decide it for the whole public API in E0.

## Review notes (2026-11-08)

- The rare false positive (the same dynamic edge twice in a chain that terminates) is acceptable; document it in the
  error's doc comment.
- Whether `DiError` becomes `#[non_exhaustive]` is decided with the public API review of #10, not here.

## Implementation notes

- **Chain content**: the error chain is the slice of the import chain from the earlier importer up to the module that
  repeats the import (`AModule -> BModule -> AModule`), without appending the imported class again. Decision 1 said
  "plus C", which would give `AModule -> BModule -> AModule -> BModule` and contradict the spec scenarios.
- **#15 landed first**: the `di` target already existed; the tests are in `dynamic_import_cycles.rs`, and the stale
  "tracked by #112" comments in `cycles.rs` and `dynamic_modules.rs` were updated.
- **Extra imports of `for_root`**: a cycle written as `DynamicModule::new::<A>().import(B::for_root())` with `B::for_root()`
  doing the same recurses while user code builds the `DynamicModule`, before the scanner runs. The container cannot
  report it, so it has no test.
- Without the check the tests abort the binary with a stack overflow (verified).
- The corvis-parity task 5.6 is left for the maintainer.

# Design

## Context

See proposal.md for motivation; the behavior under test is stated in the six spec deltas. Current state, verified in
the code:

- **Container** (`crates/corvis/src/di/container.rs`):
  - `Scanner::add_import` (lines 111-133): a static import is looked up in `static_index` before its definition is
    filled (lines 114-120), which is why static module import cycles terminate; a dynamic import always creates a new
    node (lines 124-131) and also registers itself in `static_index` with `or_insert` (line 128).
  - `Scanner::fill` (lines 136-186): `global` is `DynamicModule::global` if set, otherwise the attribute's value
    (line 146); static providers are added before dynamic ones and `add_provider` replaces a provider with the same
    token (lines 93-109); module re-exports must name a module in the same `imports` list (lines 170-180).
  - `Container::scan` (lines 191-251): an internal global core module is created first (lines 194-206); exported
    tokens are validated after the whole graph is built and an unknown one yields `DiError::InvalidExport`
    (lines 230-241).
  - `find_exported_in` (lines 257-288) carries a `visited` set, so re-export cycles terminate; `lookup`
    (lines 291-311) searches own providers, then exports of imports, then exports of global modules.
  - `resolve_provider` (lines 329-370): transient providers skip the instance cache (lines 337-342) and are not stored
    (lines 358-360); the cycle check compares `(module, token)` against the resolution `path` and builds the chain from
    token display names (lines 343-347).
  - `instantiate_all` (lines 402-420) skips transient providers (lines 407-409); `call_hook` (lines 458-513) only calls
    hooks on stored instances (line 482); `find_instance` (lines 427-442) is the application-wide fallback of
    `ModuleRef::get`.
- **Providers** (`crates/corvis/src/di/provider.rs`): `use_class` (lines 137-150) copies `T::scope()` and `T::hooks()`;
  `use_class_as` (lines 154-169) copies the scope but not the hooks; `use_existing` (lines 237-246) is a default-scope
  provider whose factory resolves the target through `ResolveCtx::resolve_instance`, which forwards the current path
  (`crates/corvis/src/di/injectable.rs:105-107`), so alias cycles are detectable; `scope` (lines 249-252) overrides the
  scope; `use_factory_arc` (lines 209-234) passes a `DiError` returned by the factory through unchanged and wraps any
  other error in `DiError::Factory`.
- **ModuleRef** (`crates/corvis/src/di/module_ref.rs`): `get`/`get_token` (lines 20-37) use `lookup` then
  `find_instance`; `get_token_in_context` (lines 41-52) uses `lookup` only; `get_strict` (lines 55-65) only reads the
  instance stored under its own module; `resolve` (lines 68-70) and `create` (lines 74-76) go through a `ResolveCtx`
  whose path starts empty (lines 78-86). A `ModuleRef` field is filled with `ctx.module_ref()`
  (`injectable.rs:183-187`), and `ctx.module` is the owning module (`container.rs:350-356`).
- **Field injection** (`crates/corvis/src/di/injectable.rs`): `resolve_dependency` (lines 111-133) returns
  `DiError::UnknownDependencies` when the token is not visible; `ResolveCtx::get`/`get_token` (lines 79-91) return
  `DiError::NotFound` for the same situation.
- **Errors** (`crates/corvis/src/di/error.rs`): `Circular { chain }` (lines 15-18), `NotFound` (12-13),
  `TypeMismatch` (20-21), `InvalidExport` (23-26), `Factory` (28-29). The circular message still suggests
  `forwardRef()` (issue #14).
- **Macros** (`crates/corvis-macros/src`): `#[injectable(scope = "transient")]` is detected by
  `parse_injectable_args` (`injectable.rs:158-173`), which also rejects `request`; the three global forms are parsed in
  `module.rs:39-46` and `module.rs:59-67`.
- **Testing harness** (`crates/corvis/src/testing.rs`): `TestingModuleBuilder::compile` (lines 158-174) wraps the
  imports and providers in a dynamic `RootTestModule` and calls `CorvisApplication::bootstrap`
  (`crates/corvis/src/application.rs:95-123`), which returns the scan/instantiation `DiError` as an `anyhow::Error`
  (`abort_on_error` defaults to `false`, `application.rs:44`), so tests can `downcast_ref::<DiError>()`.
  `TestingModule::resolve` (line 228) uses the root module's `ModuleRef`.
- **Existing tests**: only `di/token.rs:123-139` (two unit tests) and `crates/corvis/tests/modules.rs`, which boots
  built-in dynamic modules. No test covers any of the features listed in issue #15.

## Goals / Non-Goals

**Goals:**
- Every requirement and scenario in the six spec deltas is checked by at least one test.
- Tests use only the public API, the way an application or a port would, so they keep passing across internal
  refactors of the container.
- Tests are deterministic under the parallel test runner.

**Non-Goals:**
- Changing DI behavior or messages (issue #14), adding request scope (issue #23), or fixing the unbounded recursions
  listed under "Out of scope" in proposal.md.
- Differential tests against the Node container. The behavior here is the documented Corvis behavior; where it
  intentionally differs (module cycles allowed without a deferred reference, no request scope), the docs already say so.
- Pinning the documented-but-discouraged implementation detail that a static import reuses an earlier dynamic node of
  the same class (`container.rs:128`; `docs/fundamentals/dynamic-modules.md` says not to rely on it).

## Decisions

1. **One integration test target, `crates/corvis/tests/di/main.rs`, with one module per area.** Files:
   `main.rs` (shared helpers and `mod` declarations), `transient.rs`, `module_ref.rs`, `custom_providers.rs`,
   `cycles.rs`, `global_modules.rs`, `dynamic_modules.rs`. Cargo discovers `tests/<name>/main.rs` as the test target
   `di`; its `mod x;` declarations resolve to `tests/di/x.rs`. Alternatives considered: one file per area directly in
   `tests/` (six more binaries to link in the shared target directory, which is disk-constrained), or unit tests
   inside `crates/corvis/src/di/` with access to `Container` (would test internals instead of the documented API and
   break on refactors). A single file was rejected for size (several hundred lines of fixtures).
2. **Assert error variants and chain contents, not full messages.** Tests match on `DiError` variants
   (`err.downcast_ref::<DiError>()` after `compile()`), and for `Circular` they check that the chain names every
   provider of the cycle and starts and ends with the same name. The surrounding text belongs to issue #14 and will
   change (the `forwardRef()` hint). For the two-provider cycle the spec accepts either rotation, because which side
   is resolved first depends on registration order, which is an implementation detail.
3. **Instance identity, not global counts.** Distinctness is checked with `Arc::ptr_eq`, or with an `id: usize` field
   initialized by `#[init(NEXT_ID.fetch_add(1, Ordering::SeqCst))]` when the value is behind a trait object. A test
   that needs a count ("the constructor has not run", "the hook was not called") uses a static owned by that test only,
   because the test runner runs tests of the binary in parallel threads. Each test module declares its own fixture
   types; names may repeat across modules because error chains use short type names.
4. **Applications through the testing harness.** Every test builds its graph with
   `Test::create_testing_module().import::<M>()` (or `.import_dynamic(..)` / `.providers(..)`) and `compile()`;
   lifecycle tests call `init()`/`close()`; the dynamic-controller scenario uses `create_corvis_application()`,
   `init()` and `corvis::testing::request`. No sockets are opened. `ModuleRef`s located in a specific module are
   obtained from a provider of that module that has a `module_ref: ModuleRef` field, because `TestingModule` only
   exposes the root's.
5. **Startup failures are asserted on `compile()`.** Cycles, invisible dependencies, invalid exports and alias targets
   that are not visible all fail while the container is scanned or instantiated, so the tests assert on the `Err` of
   `compile()`, never on a panic. A test that would hang or overflow the stack if the behavior regressed (the cycle
   tests) runs its `compile()` under `tokio::time::timeout` so a regression fails fast instead of stalling the suite;
   a stack overflow would still abort the binary, which is acceptable as a loud failure.
6. **Findings.** If a test shows that the code contradicts a spec scenario that matches the docs, the implementer
   fixes it in this change only when the fix is local to `crates/corvis/src/di/` and small (a few lines), with the test
   as its regression test, and then runs the corvis-parity check. Otherwise the test is kept with
   `#[ignore = "see #<issue>"]`, a new issue is opened in epic #13, and the spec scenario stays as the target behavior.
   Already known gaps (proposal.md, "Out of scope") get no test here.
7. **Capabilities follow the docs pages.** Six `core/*` capabilities map one-to-one to the pages under
   `docs/fundamentals/` (plus the global section of `docs/modules.md`), so a later change to a page knows which spec
   to update. Alternative considered: a single `core/di` capability (one large spec that every DI change would touch).

## Risks / Trade-offs

- [Tests pin behavior someone may want to change, such as the alias of a transient provider keeping one instance] →
  each such case is listed in the open questions; the maintainer confirms or removes the scenario before
  implementation.
- [Flaky tests from shared statics under the parallel runner] → Decision 3: identities instead of global counts,
  per-test statics for counts.
- [Overlap with issue #14, which edits the same files] → tests assert variants and chains only (Decision 2), so #14 can
  change messages without touching these tests; if #14 lands first and restores `use_class_as` hooks, its own change
  adds that test.
- [A regression to unbounded recursion aborts the whole `di` binary] → acceptable: it is loud, and it is the behavior
  the cycle tests exist to catch.
- [More link time and disk use] → one extra test binary only (Decision 1).

## Open questions

1. **Alias of a transient provider** (`core/custom-providers`, "Alias of a transient provider"): the alias is a
   singleton that keeps the first transient instance, consistent with "the scope doesn't propagate". Confirm this is
   the intended contract rather than an accident; otherwise drop the scenario and open an issue.
2. **Two fields of the same transient type in one consumer** currently receive two different instances (each field is
   resolved separately). The docs say "a new one for each consumer". Should the spec say per field, per consumer, or
   stay silent (the current draft is silent)?
3. **Dynamic provider replacing a static provider with the same token** (`container.rs:93-109`, dynamic providers
   added after static ones): the dynamic one wins, which is useful for `for_root` defaults but is undocumented. Should
   this change add it to `core/dynamic-modules` and the docs, or leave it for a later change?
4. **New issues for the recursions found** (dynamic import recursion, cycles through `ModuleRef::resolve` inside a
   factory, transient dependencies not validated at startup): open them in epic #13 now, or fold any of them into this
   change?
5. **Capability naming**: this change introduces six `core/*` capabilities; the parallel draft of #14 may choose
   different names for DI specs. Align the names when both are reviewed.

## Review notes (2026-11-08)

- Questions 1-3 follow the original model, which the docs describe: an injection point of a transient provider gets
  its own instance (so two fields get two instances, and an alias to a transient provider is transient too), and a
  provider from a dynamic module overrides a static one with the same token. Where the code differs, the test is
  written for the original behavior; fix it here if the fix is small and local, otherwise mark it `#[ignore]` with
  the new issue number.
- The three defects found (dynamic-module import recursion, cycles through `ModuleRef::resolve`, transient providers
  not validated at startup) are filed as separate issues in epic #13 and added to phase 0.3 of #111.
- Capability names: `core/di` (#14, messages and attributes) and the per-page `core/*` capabilities here do not
  overlap; keep both.

## Implementation notes

- **Test count and layout**: 65 tests in `crates/corvis/tests/di/` (`transient` 9, `module_ref` 13, `custom_providers` 11,
  `cycles` 9, `global_modules` 11, `dynamic_modules` 11, one smoke test). Transient implementations behind
  `use_class_as` and the hooks of `use_class_as` are already covered by `crates/corvis/tests/di_use_class_as.rs` (#14)
  and are not repeated.
- **Fixed here (Decision 6)**: an alias (`Provider::use_existing`) of a transient provider was stored as a singleton and
  kept the first transient instance, contradicting the review notes (an alias takes the scope of its target). `Provider`
  now remembers `alias_of`, and `Container::is_transient` follows the alias chain (with a guard against alias cycles) in
  `resolve_provider` and `instantiate_all`. Local to `crates/corvis/src/di/`, so the corvis-parity check is needed.
  Regression tests: `custom_providers::an_alias_of_a_transient_provider_is_transient_too` and
  `a_chain_of_aliases_ending_in_a_transient_provider_gives_new_instances`.
- **Chain of a cycle entered from outside**: when the singleton that starts the resolution is not part of the cycle, the
  `Circular` chain starts at that singleton (`NodeHolder -> Node -> Node`), so it does not start and end with the same
  provider. The cycle is still named and the last link is the repeated provider. The spec scenario for the transient
  self-dependency was written to match; a chain with only the cycle is a message change for a later issue.
- **Not covered by tests, tracked elsewhere**: #112 (dynamic modules importing each other), #113 (cycles through
  `ModuleRef::resolve` in a factory) and #114 (transient dependencies validated at startup). No test in this change
  exercises them, so none is marked `#[ignore]`.
- **Documentation**: custom providers (alias scope, target visibility, chained and exported aliases), injection scopes
  (one instance per injection point) and dynamic modules (a dynamic provider overrides a static one with the same token),
  in English and Spanish. The other pages already matched the tests.

# Proposal

Implements [randygil/corvis#112](https://github.com/randygil/corvis/issues/112) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

When module A's `#[module]` imports `B::for_root()` and B's imports `A::for_root()`, building the container never
ends: every dynamic import creates a new module node and fills it right away, so the scanner recurses until the
process aborts with a stack overflow. The user gets a crash with no message instead of a `DiError` that says which
modules form the cycle. Static modules that import each other already work (the docs promise it), so the dynamic case
is the only module cycle that crashes. The defect was found while drafting the DI test coverage change (#15), which
listed it as out of scope.

## What Changes

- The module scanner keeps the chain of modules it is currently filling. Before it fills a new dynamic import, it
  checks whether the same import edge (importing module class → imported module class, through a dynamic import) is
  already on that chain. If it is, the graph can only repeat itself, and scanning stops with an error.
- A new error variant `DiError::CircularImport { chain }` names the modules in the cycle (`AModule -> BModule ->
  AModule`) and says how to break it. The startup path already logs and returns `DiError`s, so the error reaches
  `CorvisFactory::create` / `Test::create_testing_module().compile()` like any other DI error.
- Graphs that terminate today keep working. That includes static import cycles and a dynamic module whose class shows
  up a second time through a different edge (for example `A::for_root()` → static `B` → `A::for_root()`, which stops
  because `B` is reused).
- Regression tests: cycles of two and three dynamic modules, a mixed static/dynamic cycle, and the graphs that must
  still start.
- Docs: the circular dependencies page, the dynamic modules page and the common errors FAQ explain the new error, in
  English and Spanish.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/circular-dependencies`: adds the requirement that import cycles between dynamic modules are reported. The
  capability is introduced by the DI test coverage change (#15, `e13-di-test-coverage`). This delta only adds a
  requirement, so it applies whichever change is archived first.

## Impact

- Code: `crates/corvis/src/di/container.rs` (`Scanner`, `add_import`), `crates/corvis/src/di/error.rs` (new variant).
- Public API: one new variant on `DiError`. The enum is not `#[non_exhaustive]`, so an exhaustive `match` on
  `DiError` in user code stops compiling. No such match exists in the workspace, the examples or the three port apps.
- Tests: `crates/corvis/tests/di/` (the target introduced by #15; created here with the same layout if #15 has not
  landed).
- Docs (EN + ES): `fundamentals/circular-dependency.md`, `fundamentals/dynamic-modules.md`, `faq/common-errors.md`.
- Port apps: none of them has a dynamic import cycle (their only dynamic imports are `ConfigModule::for_root`,
  `ScheduleModule::for_root` and `OrmModule::for_root[_async]` from the root or a database module), so startup is
  unchanged. The parity check still runs because runtime code changes.

## Out of scope

- Deduplicating dynamic modules with equal options (the source ecosystem hashes the dynamic metadata; Corvis cannot
  compare providers that hold closures). Possible future issue if users need mutual `for_root` imports to work.
- Cycles between providers (`DiError::Circular`) and cycles through `ModuleRef::resolve` (issue #113).
- Rewording `DiError::Circular` (issue #14).
- Making `DiError` `#[non_exhaustive]` (an API policy decision for the 1.0 surface; candidate issue in E0).

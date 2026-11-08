# Proposal

Implements [randygil/corvis#113](https://github.com/randygil/corvis/issues/113) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The container detects provider cycles by carrying the resolution path through every `ResolveCtx`. `ModuleRef` does
not take part in that: `ModuleRef::resolve` and `ModuleRef::create` start from an empty path. When a factory (or a
hand-written constructor) resolves, through a `ModuleRef`, a provider that depends back on the provider being built,
the container does not see the cycle. It calls the factory again and again until the process hangs or overflows the
stack, instead of returning the circular-dependency error with the chain. The docs recommend `ModuleRef` as a way to
break cycles, and the new circular message proposed in #14 says "resolve one side lazily with ModuleRef (not in its
constructor)". Doing it in the constructor by mistake should produce that message, not a crash. Found while drafting
the DI test coverage change (#15).

## What Changes

- While a provider's factory runs, the container makes the current resolution path available to the task that runs
  it (a task-local value set around the factory future). `ModuleRef::resolve` and `ModuleRef::create` start from that
  path when they run inside a construction, and from an empty path otherwise.
- A cycle that passes through `ModuleRef` during construction now fails with `DiError::Circular` and the full chain,
  like any other provider cycle.
- A `ModuleRef` used after construction (in a handler, a hook, a later call) behaves exactly as today. It keeps no
  path from the moment it was created, so resolving a transient provider from its own methods, or resolving a provider
  that depends on the caller once the caller exists, keeps working.
- Regression tests, each under a timeout so a regression fails fast instead of stalling the suite.
- Docs: the circular dependencies page, the module reference page and the common errors FAQ say that resolving the
  other side of a cycle with `ModuleRef` inside a constructor or factory fails with the circular-dependency error.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/circular-dependencies`: adds requirements for cycles that pass through `ModuleRef` during construction and
  for `ModuleRef` use after construction. The capability is introduced by #15 (`e13-di-test-coverage`). This delta only
  adds requirements, so it applies whichever change is archived first.

## Impact

- Code: `crates/corvis/src/di/container.rs` (`resolve_provider`, `construct_injectable`: run the factory inside the
  task-local scope), `crates/corvis/src/di/module_ref.rs` (`ModuleRef::ctx` reads it). No public API change.
- Dependencies: none new (`tokio::task_local!` is in the workspace `tokio`, which enables the `full` feature set).
- Tests: `crates/corvis/tests/di/` (created with #15's layout if #15 has not landed).
- Docs (EN + ES): `fundamentals/circular-dependency.md`, `fundamentals/module-ref.md`, `faq/common-errors.md`.
- Port apps: houseplan injects `ModuleRef` in three services but only calls `get` at run time
  (`ledger/bags.service.rs:30`, `bot/nudges.service.rs:121`, `bot/bot.service.rs:198`); no port calls `resolve` or
  `create`. Startup and request handling are unchanged. The parity check still runs because runtime code changes.

## Out of scope

- `ResolveCtx::get_or_construct` → `Container::construct_injectable` also restarts the path, at
  `vec![key]` (`container.rs:390`), so a cycle through an enhancer constructed by type is reported with a shorter
  chain or not at all. Possible new issue. This change only makes `construct_injectable` publish its path for
  `ModuleRef` calls inside it.
- Work spawned onto another task from inside a factory (`tokio::spawn`) does not inherit the task-local path, so a
  cycle closed from a spawned task is still not detected. Documented as a limitation.
- A general recursion depth limit as a backstop for undetected cycles (open question).
- Rewording `DiError::Circular` (issue #14) and import cycles between dynamic modules (issue #112).

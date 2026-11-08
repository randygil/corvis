# Design

## Context

Current behavior, read in the code of this worktree (the issue cites older line numbers for `module_ref.rs`):

- **Path-based cycle detection.** `Container::resolve_provider` (`crates/corvis/src/di/container.rs:329-370`) returns
  a cached instance for non-transient providers first (`:337-342`), then fails with `DiError::Circular` when
  `(owner, token)` is already on `path`, building the chain from the token display names (`:343-347`). It then
  extends the path (`:348-349`), builds a `ResolveCtx` with it (`:350-356`) and awaits the factory (`:357`).
  `ResolveCtx::resolve_instance` forwards `self.path` (`crates/corvis/src/di/injectable.rs:105-107`), so field
  injection, `ctx.get`/`get_token` and aliases (`Provider::use_existing`, `crates/corvis/src/di/provider.rs:237-246`)
  all see the path.
- **ModuleRef drops the path.** `ResolveCtx::module_ref` builds `ModuleRef { container, module }`
  (`injectable.rs:147-149`); the `ModuleRef` field of an `#[injectable]` is filled the same way (`injectable.rs:183-187`).
  `ModuleRef::resolve` (`crates/corvis/src/di/module_ref.rs:100-103`) and `ModuleRef::create` (`:105-109`) use
  `ModuleRef::ctx` (`:111-119`), which always starts with `path: Arc::new(Vec::new())` (`:117`). A factory that calls
  `ctx.module_ref().resolve::<X>()` where `X` needs the provider under construction therefore re-enters
  `resolve_provider` with an empty path. The provider is not cached yet, so the factory runs again, recursively, until
  the boxed futures exhaust the stack or the test hangs.
- **Enhancers constructed by type** restart the path too: `Container::construct_injectable` uses
  `path: Arc::new(vec![key])` (`container.rs:385-391`) and calls `T::construct(ctx)` (`:392`).
- **Startup.** `instantiate_all` (`container.rs:402-420`) calls `resolve_provider` with an empty path for every
  non-transient provider and controller. After startup, `ModuleRef::resolve` is the documented way to get transient
  providers on demand (`docs/fundamentals/module-ref.md:35`, `:74`) and to break cycles at run time
  (`docs/fundamentals/circular-dependency.md:41-69`, which warns "Don't call `get` inside the constructor", `:69`).
- **Runtime.** The workspace `tokio` dependency enables `full` (`Cargo.toml:28`; `crates/corvis/Cargo.toml:17`), so
  `tokio::task_local!` is available. Factories are awaited inline by `resolve_provider` on the caller's task; the
  container never spawns them.
- **Port apps**: houseplan has `module_ref: ModuleRef` fields and calls only `get` at run time
  (`houseplan-api/src/ledger/bags.service.rs:25-30`, `bot/nudges.service.rs:113-121`, `bot/bot.service.rs:161-198`).

## Goals / Non-Goals

**Goals:**
- A cycle closed through `ModuleRef::resolve` / `create` during construction fails with `DiError::Circular` and the
  full chain.
- `ModuleRef` behaves exactly as today outside construction: no false cycles, no state kept from the injection moment.
- No change to the public API.

**Non-Goals:**
- Cycles closed from a task spawned inside a factory (see Risks).
- Fixing the shortened path of `construct_injectable` for its own resolution (proposal "Out of scope").
- Changing the circular message (issue #14).

## Decisions

1. **Publish the resolution path in a task-local while a factory runs.** Add to `di/container.rs`:
   ```rust
   tokio::task_local! {
       static RESOLUTION_PATH: Arc<Vec<(ModuleId, Token)>>;
   }
   ```
   `resolve_provider` awaits `RESOLUTION_PATH.scope(new_path.clone(), (provider.factory)(ctx))` instead of the bare
   factory future, and `construct_injectable` wraps `T::construct(ctx)` the same way with its path. `ModuleRef::ctx`
   uses `RESOLUTION_PATH.try_with(|p| p.clone()).unwrap_or_default()` instead of an empty path. Nested resolutions
   set nested scopes, so the value seen inside any factory is that factory's full path. When the factory future
   completes, the scope ends, so a `ModuleRef` used later sees no path.
   - Alternative: store the path in the `ModuleRef` returned by `ResolveCtx::module_ref` and injected into fields.
     Rejected: the `ModuleRef` outlives the construction. A transient provider calling
     `self.module_ref.resolve::<Self>()` later would get a false `Circular`, and the stored path would be stale. A
     `ModuleRef` stored by another, already built provider and called from inside a factory would carry no path at all.
   - Alternative: a per-key "under construction" set in the `Container`. Rejected: two tasks resolving the same
     transient provider (or the same singleton during a concurrent first resolution) would report false cycles, and
     the set cannot rebuild the chain.
   - Alternative: a shared "construction active" flag in the `ModuleRef`, cleared when the factory returns. Rejected:
     it covers only the `ModuleRef` handed to that factory, not a stored `ModuleRef` of another provider called on the
     same task (spec scenario "Stored ModuleRef of another provider").
   - `tokio::task_local!` works on any executor: the scope is applied on each poll, so `futures::join!` of two
     resolutions inside one factory keeps each branch's own path.

2. **`ResolveCtx` keeps its explicit path.** The task-local is only the fallback for `ModuleRef`. Field injection and
   `ctx.get*` keep forwarding `ResolveCtx::path` as today, so this change does not touch the main resolution code
   path beyond wrapping the factory future.

3. **Same error, same chain format.** The cycle is reported by the existing check at `container.rs:343-347`, so the
   error is `DiError::Circular { chain }` with the usual `A -> B -> A` format, and any text change from #14 applies
   automatically. The requester shown for `ModuleRef` calls stays `"ModuleRef"` (`module_ref.rs:115`).

4. **Tests with a timeout.** Add `crates/corvis/tests/di/module_ref_cycles.rs` (module of
   `crates/corvis/tests/di/main.rs`; create `main.rs` with #15's layout if needed). Every test that expects a cycle
   wraps `compile()` in `tokio::time::timeout(Duration::from_secs(5), ..)` and fails with a clear message on timeout.
   A factory call counter (a static owned by that test) proves the factory ran once. A regression that overflows the
   stack aborts the test binary, which is loud. The run-time scenarios use `init()` and the `ModuleRef` of a provider
   obtained from the testing module, as in #15 (`openspec/changes/e13-di-test-coverage/design.md`, decision 4).

## Risks / Trade-offs

- [A factory that spawns a task which resolves the provider under construction through `ModuleRef` and awaits it
  still hangs] → task-locals are not inherited by `tokio::spawn`. This needs an explicit spawn inside a constructor.
  Documented on the circular dependencies page. A depth backstop is an open question.
- [Extra cost per factory call] → one task-local scope per constructed instance (a pointer swap per poll). Instances
  are created at startup and for transient resolutions only.
- [User code that relied on the hang/overflow] → none can, since the process never finished starting.

## Open questions

1. Should the container also have a maximum resolution depth (for example 512) that returns a `DiError` naming the
   path, as a backstop for cycles this change cannot see (spawned tasks, custom executors)? Proposed: not now, open an
   issue if a real case appears.
2. Should `ResolveCtx::get_or_construct` / `construct_injectable` start from the caller's path instead of
   `vec![key]` (proposal "Out of scope")? Proposed: separate issue, since it changes which errors enhancer
   construction reports.

## Review notes (2026-11-08)

- Add a generous depth limit (256) as a safety net that returns the circular-dependency error.
- Fix the trimmed resolution path of `construct_injectable` here if it is the same mechanism; otherwise file it.

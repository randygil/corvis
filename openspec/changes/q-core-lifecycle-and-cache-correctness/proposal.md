# Proposal

Implements the issue filed from `quality/q-core-lifecycle-and-cache-correctness.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: an engineering-quality review (not a
security review) of the core of the framework: dependency injection, router, application lifecycle, built-in modules,
the plain WebSocket gateway layer and the procedural macros, read against `dev` at `d46758a`. This is the first of three
changes cut from that review, in dependency order:

1. **this change**: correctness of the lifecycle, the cache, the cron jobs and the test client;
2. `q-core-request-hot-path`: allocations and copies per request and per broadcast, measured;
3. `q-core-macros-diagnostics-and-api-hygiene`: macro diagnostics, return-type detection and small API hygiene.

It lands after the gateway adapter (#35, `openspec/changes/e2-socket-io-gateway-adapter/`), which is being finished on a
branch and edits `application.rs` (`init_steps`, `AppShared`) and `websockets.rs`, and after
`openspec/changes/q-http-server-robustness/` (same epic), which makes `serve` end the connection tasks it started before
it returns (the server half of Decision 4) and routes `TestRequest::send` through one `server::respond` (Decision 11 edits
that new `send`). Task 1.1 re-checks every line cited here once both are merged (design.md "Order with the other changes").

## Why

- **Shutdown stops at the first failing hook.** `close()` marks the application closed, then a single `on_module_destroy`
  that returns `Err` skips the destroy hooks of every later provider, leaves the cron jobs, intervals and timeouts
  running and skips `before_application_shutdown` and `on_application_shutdown`. Because the application is already
  marked closed, a second `close()` does nothing. The original framework also aborts its close sequence, but its process
  then exits; a Rust process (a test binary, an embedded server, `listen_in_background`) keeps running with the leftovers.
- **Shutdown runs in the wrong order.** Corvis runs shutdown hooks deepest module first, like the init hooks, and the
  docs say so; the original framework (11.2.1) reverses the order on shutdown, so a database module is closed after the
  services that use it. Corvis closes it first.
- **Background work outlives the application.** Nothing stops the scheduler when an application is dropped without
  `close()`, when `listen` fails to bind after `init()`, when `on_application_bootstrap` fails after the scheduler was
  mounted, or when `create_application_context` fails. `listen_in_background` drops the server task's handle, so
  `close()` never stops that server: it keeps serving requests with providers whose destroy hooks already ran. Plain
  WebSocket gateway connections are not closed either.
- **The cache's order queue leaks and evicts fresh entries.** An entry that expires and is read is removed from the map
  but not from the order queue; storing the key again pushes a duplicate. The queue grows without bound while the map
  stays under `max`, and when eviction runs, the stale duplicate makes it remove the live entry, sometimes the one just
  stored.
- **Cron jobs with `wait_for_completion` can run twice at once**, can stop forever after a panic in a hand-built tick
  function, write warnings with `eprintln!` past the application's logger, and `stop_and_wait` polls every 100 ms.
- **Smaller defects in the same paths:** `listen(70000)` listens on port 4464; the test client turns an invalid path into
  `/`, drops invalid headers and sends an empty body when `json()` cannot serialize, so tests pass when they should not;
  `WsServer::for_path` keeps the servers of applications that are gone; `ConfigModule::for_feature` runs its factory
  twice.

## What Changes

- **Lifecycle fixture.** `crates/corvis/tests/fixtures/lifecycle/` records, from the original framework, the order of
  every lifecycle hook across modules and within a module, what runs when a shutdown hook fails, and what a connected
  WebSocket gateway client sees when the application closes. A Corvis test replays it.
- **Shutdown completes.** Every shutdown hook runs even when an earlier one failed; the scheduler stops in every case;
  `close()` returns a new public `ShutdownError` that lists every failure (each is also logged). Deliberate difference
  from the original, stated in the docs.
- **Shutdown order.** Shutdown hooks walk the modules from the root to the deepest, the reverse of the init hooks, as the
  original does.
- **Background work belongs to the application.** `close()` stops and drains the servers started with
  `listen_in_background` before the hooks run, and closes plain gateway connections with close code 1001. Dropping an
  application that was not closed stops its scheduler and its background servers. A failed `init()` stops the scheduled
  jobs it started; a failed `create_application_context` too. `listen` closes the application whatever `serve` returned.
- **Cache bookkeeping.** The order queue holds each live key once, with a generation number; expiry, deletion and
  re-insertion keep the queue and the map in step; the queue never holds more than twice `max` entries. `set` of a value
  that fails to serialize stores nothing instead of `null`.
- **Cron jobs.** The running flag is claimed with `compare_exchange` and released by a guard, so two ticks cannot both run
  and a panic cannot leave the flag set; warnings go through `Logger::new("Cron")`; `stop_and_wait` waits on a `Notify`.
- **Small fixes.** `ListenAddr for i32` refuses values outside `0..=65535` with `InvalidInput`; the test client records
  invalid input and `send()` panics with the reason (new `try_send()` returns it); `close()` and `Drop` unregister the
  application's gateway servers from `WsServer::for_path`; `for_feature` runs its factory once; dead code in the touched
  functions goes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `core/lifecycle`: added requirements for completing the shutdown, its order, background servers, dropped applications,
  scheduled jobs after a failed start and listen ports out of range.
- `core/modules`: added requirements for cache eviction bookkeeping, values the cache cannot serialize, cron jobs that
  wait for completion and the factory of `ConfigModule::for_feature`.
- `core/testing`: added requirement for test requests that cannot be built.
- `ws/gateways`: added requirements for gateway connections when the application closes and for `WsServer::for_path`.

## Impact

- New: `crates/corvis/tests/fixtures/lifecycle/{package.json,package-lock.json,generate.mjs,lifecycle.json}`,
  `crates/corvis/tests/lifecycle_fixture.rs`, `crates/corvis/tests/shutdown.rs`.
- Edited: `crates/corvis/src/application.rs`, `crates/corvis/src/di/container.rs`, `crates/corvis/src/lib.rs` (re-export
  of `ShutdownError`), `crates/corvis/src/modules/{mod,cache}.rs`, `crates/corvis/src/modules/schedule/{mod,cron_job}.rs`,
  `crates/corvis/src/websockets.rs`, `crates/corvis/src/testing.rs`, `crates/corvis/src/config.rs`,
  `crates/corvis/src/di/module_ref.rs` (rustdoc), `crates/corvis/tests/{schedule,app,application_context}.rs`,
  `docs/fundamentals/lifecycle-events.md`, `docs/es/fundamentals/lifecycle-events.md`, the testing and caching pages
  named in tasks.md group 11 and their Spanish versions.
- Public API: new `ShutdownError` (`close()` returns it inside `anyhow::Error` when a hook failed; code that downcast the
  hook's own error from `close()` now finds it in `ShutdownError::failures()`); new `TestRequest::try_send` and
  `TestRequestError`; `impl Drop for CorvisApplication`; `ListenAddr for i32` returns an error for values out of range.
  Nothing else changes signature.
- Behavior visible to applications: listed in design.md "Behavior visible to applications". The largest: shutdown hooks
  run root module first; a failing shutdown hook no longer stops the rest; `close()` stops background servers and closes
  gateway connections; a dropped application stops its timers.
- Dependencies: none. The fixture generator pins the original framework's packages in its own `package.json`.
- Framework runtime code changes, so the corvis-parity check is run. The port apps do not exercise shutdown through
  apidiff; their own test suites run `close()`.

## Out of scope

- An injectable clock for cron jobs (they read `Utc::now()`): the cron tests that wait for wall-clock seconds keep doing
  so; the new tests use the paused clock for intervals and timeouts and test hooks for the cron race. Possible later
  issue.
- The cache's eviction policy (insertion order) and a comparison with the original cache store: this change fixes the
  bookkeeping, not the policy (design.md Review note 4).
- Breaking the `ModuleRef` reference cycle that keeps a container alive (design.md Decision 12): documented only.
- The per-request cost of the router, the throttler and broadcasts (`q-core-request-hot-path`), and the macros
  (`q-core-macros-diagnostics-and-api-hygiene`).

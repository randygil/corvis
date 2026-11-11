# Design

## Context

Source: the engineering-quality review of the core recorded in the issue (`quality/q-core-lifecycle-and-cache-correctness.issue.md`).
Code read on `dev` at `d46758a` (2026-11-11). The review was done by reading only; every finding below was read again in
the code, and the claims about the original framework were checked in its source (`@nestjs/core` 11.2.1 and
`@nestjs/testing` 11.2.1 from the facten port's `node_modules`, the version the three ports use and the one the
`http_filter_order` fixture pins). Task 2.1 records the lifecycle behavior from the original into a fixture so the
comparison does not depend on that reading.

The gateway adapter (#35) edits `application.rs` (`init_steps`, `AppShared`, a new `use_websocket_adapter`) and rewrites
much of `websockets.rs` on its branch (`wip-35`). Neither touches `close_with_signal`, `call_hook`, the scheduler or the
cache. **Task 1.1 re-checks every line cited below against `dev` with #35 merged** and records moved lines in
"Implementation notes"; the decisions depend on behavior, not on line numbers.

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| M1 cache order queue | `modules/cache.rs:73-84` (expired entry removed from the map only), `:92-109` (`push_back` when the map lacks the key, `pop_front` evicts whatever the front names) | Confirmed. Sequence that evicts the key just stored, with `max = 2`: set `a`; set `x`; `a` expires and is read (map `{x}`, queue `[a, x]`); set `y` (queue `[a, x, y]`); set `a` (queue `[a, x, y, a]`, map has 3) → `pop_front` removes the live `a`, `x` (the oldest) survives. Repeating "expire, read, set" on one key grows the queue by one each time while the map stays under `max`. Sibling found: `del` is O(queue length) (`retain`, `:112-116`) on that unbounded queue |
| Sibling: `CacheManager::set` of a value that fails to serialize | `modules/cache.rs:87-90` (`to_value(value).unwrap_or(Value::Null)`) | Found while confirming (also listed as a follow-up by `q-http-response-consistency`): a `null` is stored, and a later `get_value` returns it as a hit. Decision 8 |
| M2 shutdown stops at the first error | `application.rs:749-761` (`closed = true`, then `call_hook(..)?` three times with `stop_scheduler` between), `di/container.rs:776-801` (`h(inst).await?` in every arm) | Confirmed: one failing hook skips the rest of its module, every later module, the scheduler stop and both later phases, and `closed` is already set. The original aborts too (see "The original framework" below): deliberate difference, Decision 2 |
| M2 sibling in `listen_with` | `application.rs:648-655` (`serve(..).await?` before `close_with_signal`) vs `:685-690` (`listen_until_in_background` closes first, then propagates) | Latent: `server::serve` (`server.rs:2280-2354`) has no path that returns `Err` today. Restructured anyway (Decision 4) so a future error cannot skip the close |
| New: shutdown order | `di/container.rs:752-756` (the comment says shutdown uses the init order and the code reassigns the same order), `docs/fundamentals/lifecycle-events.md` ("Shutdown follows the same order as startup") | Differs from the original: `callDestroyHook`, `callBeforeShutdownHook` and `callShutdownHook` iterate `[...getModulesToTriggerHooksOn()].reverse()` (`nest-application-context.js:252-292`), so shutdown runs root first. Found while confirming M2; the reviewer's N2 note on `:753-756` is the trace of it. Decision 3 |
| M3 background work survives | no `Drop` for `CorvisApplication` (`application.rs:118-135`) or `SchedulerRegistry` (`schedule/mod.rs:308-314`); `set_interval` / `set_timeout` (`:251-280`) and `CronJob::start` (`cron_job.rs:332-340`) spawn tasks whose handles only `shutdown()` aborts | Confirmed for the four paths: `bind` failure after `init()` (`application.rs:706-721`, `listen(self)` drops the app), `on_application_bootstrap` failing after `mount_scheduler` (`:466-468`), an app dropped without `close()`, and `create_application_context` (`:65-69`) dropping the app when a hook fails. Sibling found: `SchedulerRegistry::mount` (`schedule/mod.rs:437-478`) returns `Err` half way (a cron expression that `CronJob::from` refuses) with the earlier timers already running |
| M3 background server | `application.rs:695-704` (`tokio::spawn(serve(.., pending()))`, the handle dropped) | Confirmed: `close()` never stops it; it serves with providers whose destroy hooks ran |
| N13 plain gateway connections after `close()` | `close_with_signal` closes only the Engine.IO sessions (`application.rs:754-755`, `http_adapter.rs:166-171`); upgraded connections run on their own task (`websockets.rs:1030-1100`), outside `finish_connection`'s stop signal (`server.rs:1431-1460`) | Confirmed by reading; task 5.1 writes the failing test. What the original sends is recorded by the fixture |
| S7 cron | `cron_job.rs:301-304` (load, then store), `:321` (reset only on the normal path), `:327,351,370,375,448` (`eprintln!`), `:441-443` (`sleep(100ms)` loop) | Confirmed. The panic case needs a hand-built `TickFn`: `tick_fn` (`:39-56`) and the scheduler's `wrap_task` (`schedule/mod.rs:504-521`) catch panics. Concurrent ticks exist: every tick, `run_on_init` and the threshold path spawn `fire_on_tick`, and `fire_on_tick` is public |
| S8 `ListenAddr for i32` | `application.rs:796-800` (`self as u16`) | Confirmed: `70000` binds 4464, `-1` binds 65535 |
| S9 test client swallows input | `testing.rs:389` (bad path becomes `/`), `:350-355` (invalid header dropped), `:363-366` (`unwrap_or_default` on serialization) | Confirmed |
| S10 process-global state | logger: `application.rs:143-148`, `testing.rs:159-166`; gateway servers: `websockets.rs:454,464-466,637` | Logger part **dropped**: the original overrides its static logger the same way at creation (`nest-factory.js:161-167`) and in `TestingModuleBuilder.compile()` (`testing-module.builder.js:127-128`); the rustdoc of `create_with_options` and `compile` will say it is process-wide (task 10.2). `SERVERS` part confirmed: entries of closed or dropped applications stay reachable through `for_path` (Decision 7) |
| N6 `for_feature` runs the factory twice | `config.rs:309-322` (`add_factory` calls `merge_factory`, which calls the factory, `:189-190`, then `(factory.factory)()` again) | Confirmed |
| N10 `std::mem::forget(tx)` | `application.rs:644` | Confirmed (a future that never resolves is what is meant) |
| N2 in these paths | `di/container.rs:753-756` (same value reassigned), `application.rs:730` (`let _ = self.started;`, field never read) | Confirmed; both go with Decisions 3 and 12 |
| N11 reference cycle | a provider holding a `ModuleRef` (`di/module_ref.rs`, `container: Arc<Container>`) keeps the container alive | Confirmed. Not fixed: cleanup must not depend on dropping the container, which is what Decisions 2 and 5 do. Documented (Decision 12) |
| N12 deadlock through a spawned task | a request-scoped factory that spawns a task which resolves the same provider with `resolve_for`: the task-local resolution path is lost and the task waits on the factory's own `OnceCell` | Plausible by reading (`di/container.rs:614-619`, `di/request_scope.rs:30-37`), not reproduced. Detecting it would need the task tree. Documented on `ModuleRef::resolve_for` (Decision 12) |

No finding of this group is wrong. One is dropped (the global logger matches the original), one is latent (`listen_with`),
two are documented only (N11, N12), and three were found while confirming: the shutdown order, `mount` failing half
way, and `del` walking the leaked queue.

### The original framework

From `@nestjs/core` 11.2.1 (read; task 2.1 records it):

- `close()` (`nest-application-context.js:119-126`) awaits `callDestroyHook`, `callBeforeShutdownHook`, `dispose` (closes
  the HTTP adapter and the WebSocket module, `nest-application.js:48-56`) and `callShutdownHook`, with no `try`: the first
  rejection ends `close()`.
- Each phase walks the modules root first (`.reverse()` of the deepest-first list, `:252-292`); within a module
  `callModuleDestroyHook` (`hooks/on-module-destroy.hook.js`) calls every non-transient instance's hook, then every
  transient one, each group with `Promise.all`, then the module class. So when one hook of a module rejects, the other
  hooks of that module have already started; the later modules, the later phases and `dispose` do not run.
- On a signal with `enableShutdownHooks`, the same sequence runs inside a `try`; a failure logs `Error happened during
  shutdown` and calls `process.exit(1)` (`:189-218`). The process ends, which is why the skipped cleanup does not matter
  there.

## Goals / Non-Goals

**Goals:**

- `close()` releases everything the application started, whatever a hook returns, and reports every failure.
- Nothing the application spawned keeps running once the application is closed, dropped or failed to start.
- The cache and the cron jobs keep their documented behavior with bounded bookkeeping and no races.
- The test client fails loudly on input it cannot send.
- Every bug gets a test that fails before the fix.

**Non-Goals:**

- New lifecycle hooks or options.
- An injectable clock for cron jobs; a different cache eviction policy.
- Running shutdown hooks from `Drop` (they are async; Decision 5 says why `Drop` does not try).

## Decisions

### 1. A lifecycle fixture recorded from the original

`crates/corvis/tests/fixtures/lifecycle/` follows `http_filter_order/`: a `package.json` pinning `@nestjs/common`,
`@nestjs/core`, `@nestjs/platform-express`, `@nestjs/platform-ws` and `@nestjs/websockets` 11.2.1 (plus `ws`,
`reflect-metadata`, `rxjs`), a plain-JS `generate.mjs` that applies decorators as functions, binds `127.0.0.1` only and
writes `lifecycle.json`. Cases:

1. **Order.** Root module `App` imports `A`, which imports `B` (distances 0, 1, 2); each module has a controller, two
   providers and the module class, each implementing all five hooks. Every hook appends `"<phase>:<class>"` in its
   synchronous part (so the order of `Promise.all` starts is what is recorded). Recorded: the sequence for `init()` and
   for `close()`.
2. **A failing destroy hook.** As 1, with the first provider of `A` rejecting in `onModuleDestroy`. Recorded: the
   sequence, whether `close()` rejected and with what message, and whether the HTTP server still accepts a connection
   after `close()` settled (an HTTP `GET` with a 2-second client timeout).
3. **A failing `beforeApplicationShutdown`.** Same shape, the hook rejects in `B`.
4. **A gateway client at close.** A `@WebSocketGateway()` on the `ws` adapter (`WsAdapter`), one client connected; the
   client records whether it received a close frame, its code and reason, or an abrupt end (1006).

The Corvis test `crates/corvis/tests/lifecycle_fixture.rs` builds the same module shapes and compares: case 1 exactly (the
start order within a module equals Corvis' sequential order); cases 2 and 3 against the original's sequence *plus* the
deliberate difference of Decision 2, written as an explicit list in the test (`EXPECTED_DIFFERENCES`) with the reason;
case 4 records the original's outcome in the docs' compatibility note (Decision 6 settles what Corvis sends).

Alternative considered: rely on the source reading above. Rejected: the repository records every claim about the original
as a fixture (`http_filter_order`, `ws_gateways`, `throttler`), and `Promise.all` start order is easy to misread.

### 2. Shutdown completes and reports every failure

- `Container::call_hook` keeps its signature for the init phases (first error ends the phase, as today and as the
  original: a failed start is single-use, `core/lifecycle`). A new `call_shutdown_hook(kind, signal) -> Vec<HookFailure>`
  runs **every** hook of the phase, in order, and collects failures: `HookFailure { phase: HookKind, provider: String
  (display name of the token), error: anyhow::Error }`. A hook that panics is caught (`catch_unwind`, as the scheduler
  does) and recorded as a failure with the panic message.
- `close_with_signal` runs, whatever the failures: close the Engine.IO sessions (as today), stop the background servers
  (Decision 4), close the plain gateway connections (Decision 6), the destroy phase, `stop_scheduler`, the
  before-shutdown phase, the shutdown phase, then unregister the gateway servers (Decision 7). Each failure is logged when
  it happens (`Logger::new("CorvisApplication").error("on_module_destroy of CatsService failed: <error>")`).
- If any failed, `close()` returns `ShutdownError` (public, in `application.rs`, re-exported from the crate root next to
  `InitFailedError`, `#[non_exhaustive]`): `Display` is `"<n> lifecycle hook(s) failed during shutdown; first:
  on_module_destroy of CatsService: <error>"`, `failures() -> &[HookFailure]` gives all of them, `std::error::Error::source`
  is the first failure's error. `HookFailure` is public with accessors `phase()`, `provider()`, `error()`.
- `closed` stays set before the work starts: a second `close()` returns `Ok(())` without running anything. A failing
  shutdown is not retried; it is reported once.

Deliberate difference from the original, documented in `lifecycle-events.md`: the original stops at the first rejection
(the later modules, the later phases and the HTTP server close do not run) and relies on the process exiting; Corvis
completes the cleanup and returns every failure, because a Rust process that closed an application (a test binary, a
server that embeds several applications, `listen_in_background`) keeps running, and the skipped cleanup would leave timers
and servers working against destroyed providers. When the original's own sequence has no failure, the two are the same.

Verified by: `crates/corvis/tests/shutdown.rs` (fails before): a module with three providers where the second's
`on_module_destroy` fails and the root module has an `#[interval]` handler: the third provider's destroy hook, every
`before_application_shutdown` and `on_application_shutdown` ran, `SchedulerRegistry::get_intervals()` is empty, the
interval counter does not move after `tokio::time::advance(1 s)` on the paused clock, and the error downcasts to
`ShutdownError` with one failure naming the provider; a hook that panics is reported the same way; two failures in two
phases give `failures().len() == 2` in order; the fixture cases 2 and 3 through `lifecycle_fixture.rs`.

Alternative considered: return the first error unchanged (no new type). Rejected: the other failures would only be in
the log, and callers could not tell a shutdown failure from any other `anyhow::Error`.

### 3. Shutdown hooks run root first

`Container::modules_for_shutdown()` returns the reverse of `modules_for_hooks()` (stable for equal distances: the reverse
of the init order exactly, as the original reverses its sorted list). The three shutdown phases use it; within a module the
order stays controllers, providers, module class (the original's start order). The comment and the dead reassignment at
`container.rs:752-756` go. Verified by fixture case 1 (fails before: Corvis runs `B` first). Visible to applications:
listed in "Behavior visible". The docs sentence "Shutdown follows the same order as startup" becomes "Shutdown runs in the
reverse order: the root module first, the most deeply imported last, so a module is closed after the modules that import
it".

Alternative considered: keep Corvis' order and record the difference. Rejected: a module imported by others (a database
or queue module) must outlive its importers' destroy hooks; the original's order is also the safe one.

### 4. `close()` stops the background servers; `listen` always closes

- `listen_in_background_with` keeps a `BackgroundServer { stop: oneshot::Sender<()>, task: JoinHandle<()> }` in a new
  field `background: Vec<BackgroundServer>`. The `shutdown` future passed to `serve` is `async move { let _ = rx.await; }`,
  which resolves when the sender is used **or dropped**.
- `close_with_signal` sends every `stop` and awaits every task before the destroy phase: `serve` stops accepting and
  drains its connections (bounded by `ServerOptions::shutdown_timeout` of `q-http-server-robustness`, 10 s by default;
  today a fixed 10 s at `server.rs:2345-2352`), so no request reaches a provider whose destroy hook ran. This is the order the signal path of `listen` already has ("the HTTP connections have already
  finished before `on_module_destroy`", `lifecycle-events.md`).
- `listen_with` becomes `let served = serve(..).await; let closed = self.close_with_signal(sig).await; served?; closed`,
  the shape of `listen_until_in_background`. N10: the oneshot without a signal is replaced by
  `futures::future::pending()` when shutdown hooks are off.

Verified by: `shutdown.rs` (fails before): after `listen_in_background("127.0.0.1:0")` and `close()`, a new TCP connect to
the address is refused (`connect` returns an error) and a request started before `close()` completes with its response;
`listen` on a bound port drops the app and its scheduler stops (Decision 5).

### 5. Dropping an application, and failed starts, stop the background work

- `impl Drop for CorvisApplication`: when the application is not closed, it calls `stop_scheduler` (synchronous: it aborts
  task handles; `CronJob::stop` spawns `on_complete` only when a runtime is current) and drops `background`, whose
  senders resolve each server's `shutdown` future. It does not run lifecycle hooks (they are async and may need the
  runtime that is being dropped); the rustdoc says so and points to `close()`. It logs nothing.
- `init()` on failure (the `Err` branch, `application.rs:492-496`) calls `stop_scheduler`: the jobs `mount_scheduler`
  started before `on_application_bootstrap` failed must not run on an application that cannot start. This changes the
  archived decision of `e13-init-is-not-idempotent-after-a-failed-start` (its Decision 4 left them to `close()`), which
  the "Behavior visible" table lists.
- `create_application_context` gets the same through `Drop` (the app is dropped on `?`).
- `SchedulerRegistry::mount` stops what it already started when it returns `Err` half way (it calls `shutdown()` before
  returning the error), so a failed `mount` leaves nothing running even if the caller keeps the application.

Verified by (all fail before; intervals and timeouts run on tokio's clock, so the tests pause it and advance it instead of
sleeping): an application with an `#[interval(..)]` handler dropped after `init()` (counter unchanged after
`advance(10 × period)`); `on_application_bootstrap` failing after the scheduler mounted (same check, and
`get_intervals()` empty); `create_application_context` failing the same way; a `mount` whose second cron expression is
invalid leaves the first job inactive (`is_active() == false`); a background server of a dropped application ends
(the test polls `TcpStream::connect` until it is refused, with a 30-second deadline that only fails the test; no hidden
API is added for it).

### 6. Gateway connections close with the application

`close_with_signal` calls a crate-private `websockets::close_gateways(&shared)`: for every gateway server, every client is
disconnected with a close frame `1001` and reason `"Server shutting down"` through the existing close path (`close_with`,
`queue.close`), so the frames already queued are sent first and the peer gets `close_timeout` to answer before the TCP
connection is destroyed (the `ws/write-deadline` rules are unchanged). Handlers in flight finish as they do when a client
disconnects. Whatever the fixture (case 4) records for the original, Corvis sends 1001: a close frame lets clients
distinguish a server shutdown from a network failure and reconnect on purpose. If the original terminates the socket
without a close frame, the docs' compatibility note says so and the test lists it in `EXPECTED_DIFFERENCES`.

Verified by: `shutdown.rs` (fails before): a client connected to a gateway of an application served with
`listen_in_background` receives a close frame with code 1001 after `close()`, and `WsServer::count()` is 0.

### 7. `WsServer::for_path` forgets closed applications

`close_with_signal` (last step) and `Drop` remove from `SERVERS` each entry whose server is the application's own
(`Arc::ptr_eq` on the server's inner), so an entry another application registered later is kept. `for_path` keeps its
documented fallback (a new, empty server when nothing is registered). Verified by: two applications mounting a gateway at
`/chat`; after the second closes, `for_path("/chat")` returns the first one's server (same `count()`), and after both
close it returns an empty server; a dropped application's entry is gone. (`for_path` itself is reviewed in the third
change of this series, not removed here.)

### 8. Cache bookkeeping with generations

`CacheManager` keeps `HashMap<String, Entry>` and `VecDeque<(String, u64)>`; `Entry` gains `generation: u64` from a
counter in the manager.

- `set_value`: a key that is live keeps its entry's generation and queue position (today's behavior: a re-set does not
  move it); a key that is absent or expired gets a new generation and is pushed at the back with it.
- `get_value` of an expired entry removes it from the map; its queue item becomes stale (its generation no longer
  matches).
- Eviction pops from the front and removes the map entry only when the generation matches; stale items are skipped.
- `del` removes from the map only (O(1)); `clear` clears both.
- After each `set_value`, when the queue holds more than `2 × max(max, 1)` items it is compacted (stale items dropped, order
  kept), so its length is bounded and the work is amortized O(1) per call.
- Crate-private `get_value_at(key, now)` and `set_value_at(key, value, ttl, now)` take the clock, as the throttler's
  `record_hit_at` does; the public methods pass `Instant::now()`. A `#[cfg(test)]` accessor reports the queue length.
- `set` (and `wrap`, through it) with a value whose serialization fails stores nothing, removes a previous entry for the
  key (so a stale value is not served as fresh) and logs an error with the key; the signature stays (`set` mirrors the
  original's `cacheManager.set`).

Verified by unit tests in `cache.rs` with synthetic instants (fail before): the sequence of the table above keeps `a`
(just stored) and evicts `x`; 10,000 cycles of "set with a 1 ms ttl, read after expiry, set again" on one key keep the
queue at most `2 × max`; `del` then `set` of the same key does not evict anything while under `max`; `set` of a value
whose `Serialize` fails leaves `get_value` returning `None` (fails before: `Some(null)`); the existing
`CacheInterceptor` tests pass unchanged.

Alternative considered: `IndexMap` with `shift_remove` on expiry. Rejected: O(n) per expiry with a large `max`.

### 9. Cron jobs: atomic claim, the application's logger, no polling

- `fire_on_tick`: with `wait_for_completion`, `callback_running.compare_exchange(false, true)`; a failed exchange returns
  (a tick is skipped, as today). Without it, the flag is still set while the callbacks are being started, as today. A
  guard struct resets the flag on drop, so a panic in a callback (or its future) cannot leave it set; the guard also calls
  `idle.notify_waiters()` on a new `Notify` in `Inner`.
- `complete()` waits with the usual pattern: create `idle.notified()`, check the flag, await only if still set.
- `eprintln!` at the five sites becomes `Logger::new("Cron")` (`error` for callback and `onComplete` errors and an invalid
  time, `warn` for the missed-deadline lines), so the lines follow the logger's levels and a disabled logger.
- A `#[cfg(test)]` hook in `fire_on_tick` parks the caller between the check and the claim, so the race test is
  deterministic.

Verified by unit tests in `cron_job.rs` (fail before): two `fire_on_tick` calls on a `wait_for_completion` job where the
first is parked by the hook between check and claim: the callback runs once; a hand-built `TickFn` that panics on its first
call (run in a spawned task), then a second `fire_on_tick`: the callback runs again; `stop_and_wait` with a callback blocked
on a `Notify` returns right after the test releases it (no `sleep` in the test, the paused clock does not advance); a
capturing `LoggerService` receives the missed-deadline warning with context `Cron`.

### 10. Listen ports out of range

`ListenAddr for i32` uses `u16::try_from(self)` and returns `io::Error::new(InvalidInput, "port <n> is outside 0..=65535")`.
Verified by: `listen_in_background(70000_i32)` and `(-1_i32)` return that error and bind nothing (fails before).

### 11. The test client fails loudly

`TestRequest` gains `error: Option<TestRequestError>`; `header`/`set`/`bearer` record an invalid name or value, `json`
records a serialization error, `send` records a path that does not parse as a URI. `try_send(self) -> Result<TestResponse,
TestRequestError>` returns the first recorded error without dispatching; `send(self)` calls it and panics with the error's
message (`"invalid test request: header \"x\\ny\": failed to parse header value"`), documented under `# Panics`.
`TestRequestError` is public (`#[non_exhaustive]` enum: `InvalidHeader { name, reason }`, `InvalidPath { path, reason }`,
`Serialize { reason }`). Panicking in `send` is the normal failure of a test utility (`TestClient::new` already panics);
`try_send` is for tests that build requests from generated input (fuzz suites).

Verified by: three unit tests (fail before: today each sends a different request than asked); the whole suite passes, and
any test that relied on the silent fallback is changed to `try_send` and listed in Implementation notes.

### 12. Small items

- N6: `for_feature` computes the value once and passes it to a new `ConfigService::add_value(namespace, value)` (what
  `merge_factory` does with an already computed value). Test: a factory that counts its calls is called once.
- N2: `let _ = self.started;` and the `started` field go (nothing reads them).
- N11, N12: rustdoc. `CorvisApplication` (and `Drop`): cleanup does not rely on dropping the container, which a provider
  holding a `ModuleRef` keeps alive. `ModuleRef::resolve_for`: a request-scoped factory must not await a task it spawned
  that resolves the same provider for the same request (the task cannot see the resolution in progress and waits for it).
- Tests touched in `tests/schedule.rs` that wait 50 ms or 110 ms for a spawned task (`:108`, `:118`, `:205`, `:210`) wait for
  the condition instead (a deadline that only fails the test); the cron tests that wait for wall-clock seconds keep their
  waits, with a comment that cron jobs read `Utc::now()`.

## Order with the other changes

- `e2-socket-io-gateway-adapter` (#35): lands first (being finished on a branch). It edits `init_steps`, `AppShared` and
  most of `websockets.rs`; Decisions 6 and 7 are written against behavior (the close path of a `WsClient`, the `SERVERS`
  map) that its branch keeps. Task 1.1.
- `q-http-server-robustness` (epic #187): lands first. Its configurable `shutdown_timeout` and tracked connection tasks
  make `serve` return only when no handler of that server runs, which is what Decision 4 awaits for background servers;
  without it the wait is bounded at 10 s and handlers may still run after it. Its `server::respond` replaces the body of
  `TestRequest::send`; Decision 11 puts the recorded-error check in front of that call. If this change had to land first,
  Decision 4 still stops the servers (only the completeness of the drain depends on the other change) and its
  `TestRequest::send` rewrite keeps the check.
- `q-core-request-hot-path` and `q-core-macros-diagnostics-and-api-hygiene` land after this change.
- `e2-socket-io-hot-path-and-api-hygiene`: touches `websockets.rs` only for the `tokio-tungstenite` upgrade. Either order.

## Security invariants this change comes near

| Invariant | Source | Decisions | Pinned by |
|---|---|---|---|
| A failed `init()` is single-use and serves nothing | `core/lifecycle`, archived `e13-init-is-not-idempotent-after-a-failed-start` | 2, 5 | `tests/init_failure.rs` unchanged; init phases keep "first error ends the phase" |
| Pending Engine.IO long-polls are answered before the drain; sessions closed on `close()` | `ws/engine-io` (#32) | 2, 4 | existing engine shutdown tests (`engine_io.rs`), unchanged order: `close_engines` stays first |
| A closing WebSocket waits at most `close_timeout`, then the TCP connection is destroyed; the write deadline applies | `ws/write-deadline`, `ws_security.rs` | 6 | `send_timeout_ws.rs`, `ws_security.rs` unchanged; new test: a peer that never answers the 1001 frame is cut at `close_timeout` (paused clock) |
| Scheduled callbacks contain panics | schedule module (`tick_fn`, `run_logged`) | 9 | existing `tests/schedule.rs`; new panicking-`TickFn` test |
| Cache memory bounded by `max` | new here | 8 | new bounded-queue unit test |
| Test servers bind `127.0.0.1` only | repository rule | 4, 6, 10 | review of the new tests (task 12.9) |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Order of shutdown hooks across modules | Deepest module first (same as init) | Root module first (reverse of init), as the original | A module's destroy hook now runs after those of the modules that import it; apps that relied on the old order (none in the workspace or the ports, task 3.3 greps) see a different order |
| A shutdown hook fails | Later hooks, scheduler stop and later phases skipped; `close()` returns that error | Everything runs; each failure logged; `close()` returns `ShutdownError` with all failures | Code that downcast the hook's own error from `close()` reads it from `failures()`; deliberate difference from the original (Decision 2) |
| `close()` after `listen_in_background` | Server keeps serving | Server stops accepting and drains before the hooks | Tests that kept using the address after `close()` fail (none known) |
| Plain gateway connections on `close()` | Stay open | Close frame 1001 "Server shutting down" | Clients see the shutdown |
| Application dropped without `close()` | Timers and background servers keep running | Stopped (hooks still do not run) | Tests that drop apps stop leaking tasks |
| `init()` fails after the scheduler mounted | Jobs run until `close()` | Jobs stopped at once | Changes Decision 4 of the archived init change |
| `listen(70000_i32)` | Listens on 4464 | `InvalidInput` error | Misconfiguration surfaces |
| Test client with invalid header, path or JSON | Silently changed request | `send()` panics with the reason; `try_send()` returns it | Tests that passed by accident fail |
| Cache under expiry and eviction | Could evict the entry just stored; queue grew | Evicts the oldest live entry; bounded queue | Fewer misses |
| `CacheManager::set` of a value that fails to serialize | Stored `null` | Stores nothing, removes the key, logs an error | A later `get` misses instead of returning `null` |
| Cron warnings | `eprintln!` | Through the logger, context `Cron` | Follows log levels and a disabled logger; format of those lines changes |
| `WsServer::for_path` after an app closed | Returned the closed app's server | Returns the live app's server or an empty one | Fewer stale handles |

## Risks / Trade-offs

- Running every shutdown hook means a hook can run after another provider's destroy hook failed half way → hooks must
  tolerate that, as they must tolerate any order; documented. The alternative (stop) is the leak this change removes.
- Awaiting the background servers' drain makes `close()` take up to `shutdown_timeout` (10 s by default) with a stuck
  connection → the same bound `listen`'s signal path has; documented.
- `Drop` cannot run async hooks → documented; `close()` remains the way to run them.
- The shutdown order change can surprise an application that tuned its destroy hooks to the old order → listed, and it is
  the original's order.
- The fixture needs Node and network access to record → recorded once and committed, like the other fixtures; the Corvis
  test reads the JSON and needs no Node.

## Open questions

1. Return the first error unchanged, or a `ShutdownError` with every failure? Recommendation: `ShutdownError`.
2. Should `Drop` log a warning when the application had shutdown hooks that will not run? Recommendation: no; document.
3. Should `close()` stop background servers before the destroy hooks (Corvis' signal-path order) or between
   `before_application_shutdown` and `on_application_shutdown` (where the original's `dispose` sits)? Recommendation:
   before, as the signal path already does.
4. Change the cache's eviction policy (the original store may be least-recently-used) while fixing the bookkeeping?
   Recommendation: no; a policy change needs its own fixture.
5. What close code for gateway clients if the original sends none? Recommendation: 1001 anyway.

## Review notes

These settle the open questions and override the design where they differ.

1. `close()` returns `ShutdownError` (Decision 2); the hook's own error is its `source()` and is in `failures()`.
2. `Drop` logs nothing; the rustdoc of `CorvisApplication` and of `close()` says that dropping stops timers and servers but
   runs no hook.
3. Background servers stop before the destroy hooks, matching `listen`'s signal path; the docs' "Connections during
   shutdown" box is extended to `close()`.
4. The eviction policy does not change. If a user needs the original store's policy, a follow-up records it in a fixture
   first.
5. Gateway clients get 1001 `"Server shutting down"` whatever the original does; the fixture's outcome goes into the
   compatibility note.
6. Implemented after #35 merges; task 1.1 is mandatory and written in "Implementation notes" before any other task.

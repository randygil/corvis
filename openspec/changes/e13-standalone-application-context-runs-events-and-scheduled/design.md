# Design

## Context

See proposal.md for motivation; the guarantees are in the `core/application-context` spec delta. Current state,
verified in the code:

- `CorvisFactory::create_application_context` (`crates/corvis/src/application.rs:62-68`) calls `Self::create::<M>()`
  (scan + `instantiate_all` in `CorvisApplication::bootstrap`, `application.rs:95-140`) and then
  `app.call_init_hooks()`.
- `CorvisApplication::call_init_hooks` (`application.rs:349-358`) is guarded by `hooks_called` and runs
  `OnModuleInit`, `modules::mount_scheduler`, `OnApplicationBootstrap`. It does **not** discover or register handlers.
- `CorvisApplication::init` (`application.rs:361-423`) is guarded by `initialized`. After building the router,
  module middleware, gateways and `AppShared`, it runs `discovery::discover(&self.container)` and
  `modules::register_handlers(&self.container, &discovered)` (`application.rs:418-419`) and then
  `call_init_hooks()` (`application.rs:420`). So in a full application handlers are registered before `on_module_init`
  and mounted between `on_module_init` and `on_application_bootstrap`.
- `discovery::discover` (`crates/corvis/src/discovery.rs:91-108`) only needs instantiated providers and controllers,
  which `bootstrap` already guarantees; it does not depend on the router.
- `modules::register_handlers` (`crates/corvis/src/modules/mod.rs:19-62`) adds event listeners to the `EventEmitter`
  immediately, and queues scheduled handlers in the `SchedulerRegistry` (`add_cron` / `add_interval_handler` /
  `add_timeout_handler`, `crates/corvis/src/modules/schedule/mod.rs:402-434`), recording each name in the
  orchestrator and failing with a duplicate-name error if it was already recorded. Only
  `SchedulerRegistry::mount` (`schedule/mod.rs:436-478`) turns the queue into running timers and jobs; it drains the
  queue and is called once from `call_init_hooks`.
- Consequences today:
  1. A standalone context never registers handlers: no events, no cron jobs, intervals or timeouts.
  2. `create_application_context` followed by `init()` registers handlers during `init()`, but `call_init_hooks`
     returns early (`hooks_called` is already true), so `mount` is never called again: scheduled handlers stay queued
     forever, and event listeners are added after `on_module_init` already ran.
- `close()` / `close_with_signal` (`application.rs:503-517`) already call `modules::stop_scheduler` between
  `OnModuleDestroy` and `BeforeApplicationShutdown`; `SchedulerRegistry::shutdown` (`schedule/mod.rs:480-494`) clears
  timeouts, intervals, initial delays and cron jobs. Nothing to change there.
- Existing coverage: `crates/corvis/tests/modules.rs` (`events_and_schedule`, line 115) exercises events and
  schedules only through `init()`; `crates/corvis/tests/schedule.rs` tests the registry and decorated handlers through
  `init()`. No test calls `create_application_context`.
- Callers: only `create_application_context` and `init` call `call_init_hooks`; `TestingModule::init`
  (`crates/corvis/src/testing.rs:236-238`) delegates to `CorvisApplication::init`.

## Goals / Non-Goals

**Goals:**
- A standalone context runs event listeners and scheduled jobs, with the same order and options as `init()`.
- Handler registration is idempotent per application, whatever sequence of `create_application_context` / `init()` is
  used.

**Non-Goals:**
- Building routes, middleware or gateways in a standalone context.
- Signal handling for standalone contexts.

## Decisions

1. **Registration moves into the init-hooks phase.** A private `register_handlers_once()` (guarded by a new
   `handlers_registered: bool` field) runs `discover` + `modules::register_handlers`. `call_init_hooks` calls it first,
   then `OnModuleInit`, `mount_scheduler`, `OnApplicationBootstrap`. `init()` drops its own two lines
   (`application.rs:418-419`) and keeps calling `call_init_hooks()` at the same point, so the order inside `init()` is
   unchanged: router, middleware, gateways, `AppShared`, handlers, `on_module_init`, mount, `on_application_bootstrap`.
   - Alternative: call `discover` + `register_handlers` directly in `create_application_context` before
     `call_init_hooks`. Rejected: leaves the double-registration and never-mounted bug when `init()` follows.
   - Alternative: reuse `hooks_called` as the only guard. Possible, since registration now lives inside the guarded
     block; a separate flag is kept only if it reads clearer. Implementation may fold it into `hooks_called`.
2. **Parity statement.** In the original model, `ScheduleModule` explores `@Cron`/`@Interval`/`@Timeout` in
   `onModuleInit` and mounts them in `onApplicationBootstrap`, and `EventEmitterModule` subscribes listeners in
   `onApplicationBootstrap`; `createApplicationContext()` runs both hooks, so standalone contexts run them. Corvis
   registers listeners slightly earlier (before `on_module_init`), as `init()` already does; this change keeps that
   existing difference instead of introducing a second ordering.
   - Alternative: a differential test against a Node standalone context with `@nestjs/schedule` and
     `@nestjs/event-emitter`. Not planned: the behavior is "handlers run at all", already covered by the original
     modules' design; listed under Open questions.
3. **No new opt-out API.** Processes that import a module with crons but must not run them (one-shot scripts) use
   the existing `ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, .. })`
   (`schedule/mod.rs:203-212`, `564`), or a separate root module without `ScheduleModule`. The docs show both.
   - Alternative: `create_application_context_with(options)` with a "no handlers" flag. Rejected for now: new API
     surface for a case the module options already cover.

## Risks / Trade-offs

- [Behavior change for existing standalone contexts] → scripts that import a full `AppModule` will now run its cron
  jobs, intervals and timeouts while they are alive. This is the intended fix and matches the original model, but a
  short script can now fire a job it did not ask for (for example, `consi-backend`'s `run-jobs` binary schedules five
  `#[cron]` jobs; the `*/2 * * * *` one can fire during a run). Mitigation: docs explain `for_root_with` /
  separate root module; the parity check covers the port; the maintainer decides how the port should react (Open
  questions).
- [Timeouts fire in short-lived processes] → a `#[timeout(10)]` handler now runs in a script that lives longer than
  10 ms. Same mitigation.
- [Event listeners added before `on_module_init` in standalone contexts] → same order as `init()`; covered by the
  ordering scenario.

## Open questions

- `consi-backend/src/bin/run-jobs.rs` will start scheduling the API's cron jobs during a manual run. Its Node original
  presumably did the same (standalone context with `ScheduleModule`). Should the port keep parity, or switch to a root
  module without `ScheduleModule` / `cron_jobs: false`? Not part of this change (the port lives outside the
  repository); flag it to the port's maintainer.
- Is a differential test against a Node standalone context worth adding for this behavior, or is the Rust regression
  test enough?

## Review notes (2026-11-08)

- Scheduled jobs running in a standalone context matches the original model (its schedule module starts cron jobs on
  application bootstrap in an application context too). The parity run (definition of done) is the check: if the
  `consi-backend` `run-jobs` binary changes behavior in apidiff or the database comparison, stop and report before
  changing the port.
- A Rust regression test is enough; no differential test against Node for this change.

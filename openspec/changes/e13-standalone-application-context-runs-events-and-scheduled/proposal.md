# Proposal

Implements [randygil/corvis#16](https://github.com/randygil/corvis/issues/16) (epic [#13](https://github.com/randygil/corvis/issues/13)).

## Why

`CorvisFactory::create_application_context` is the documented entry point for workers, scripts and CLIs, but it only
runs `on_module_init` and `on_application_bootstrap`. The `#[handlers]` of the application (`#[on_event]`, `#[cron]`,
`#[interval]`, `#[timeout]`) are registered only inside `CorvisApplication::init()`, so a standalone worker never runs
its cron jobs and never receives its events. The docs carry a warning about it on ten pages per language, and they
tell users to build an HTTP application and call `init()` instead, which also builds routes and gateways they do not
need. In the original model a standalone application context runs scheduled jobs and event listeners, because
scheduling and events are wired by their modules' lifecycle hooks, not by the HTTP layer.

There is a second, quieter defect in the same code: calling `app.init()` on an application returned by
`create_application_context` registers the scheduled handlers after the scheduler was already mounted, so they are
queued and never started.

## What Changes

- `create_application_context` registers the discovered `#[on_event]`, `#[cron]`, `#[interval]` and `#[timeout]`
  handlers before `on_module_init`, in the same order as `init()`, so scheduled jobs are mounted before
  `on_application_bootstrap` and stopped by `close()`.
- Handler registration happens once per application: calling `init()` after `create_application_context` does not
  register handlers twice (no duplicate-name errors, no double event delivery) and does not leave scheduled handlers
  unmounted.
- `ScheduleModule::for_root_with(ScheduleModuleOptions { .. })` keeps working as the way to turn off cron jobs,
  intervals or timeouts in a process (for example a one-shot script that imports the full `AppModule`).
- Docs (EN + ES) drop the "does not register the handlers" warnings and show a standalone worker using
  `create_application_context` with `#[cron]`.

## Capabilities

### New Capabilities

- `core/application-context`: what a standalone application context starts and stops (lifecycle hooks, event
  listeners, scheduled jobs) and how it composes with `init()` and `close()`.

### Modified Capabilities

None (no `core/*` spec exists yet).

## Impact

- `crates/corvis/src/application.rs`: handler registration moves into the init-hooks phase shared by `init()` and
  `create_application_context`, guarded so it runs once.
- No public API change; behavior change for every standalone context whose modules declare handlers and import
  `EventEmitterModule` or `ScheduleModule`.
- Docs: `docs/standalone-applications.md`, `docs/fundamentals/lifecycle-events.md`, the eight
  `docs/microservices/*.md` pages that repeat the warning, and their `docs/es/` translations.
- Port apps: `consi-backend/src/bin/run-jobs.rs` uses `create_application_context::<AppModule>()` and `AppModule`
  imports `ScheduleModule::for_root()` with five `#[cron]` jobs. After this change those jobs are scheduled while the
  binary runs (see design, Risks). The HTTP servers of the three ports use `init()`/`listen()`, so the apidiff suites
  are not affected.

## Out of scope

- A dedicated worker API (`create_microservice`-style transports, `app.listen()` for workers): epic-level feature work.
- Making `#[handlers]` discovery lazy or per-module.
- Gateway (`#[subscribe_message]`) handlers in a standalone context: they need the HTTP/WebSocket server and stay part
  of `init()`.
- Graceful shutdown on signals for standalone contexts (`enable_shutdown_hooks` currently acts in `listen()`):
  candidate for a new issue.

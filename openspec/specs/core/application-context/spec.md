# core/application-context Specification

## Purpose
A standalone application context (no HTTP server) starts everything a worker needs: lifecycle hooks, event listeners
and scheduled jobs, in the same order as a full application, and stops them on close. Matches the original model, where
`@nestjs/schedule` and `@nestjs/event-emitter` wire their handlers from module lifecycle hooks and therefore run in a
standalone application context too.

## Requirements

### Requirement: The standalone context registers event and scheduled handlers
`CorvisFactory::create_application_context::<M>()` SHALL register every discovered `#[on_event]`, `#[cron]`,
`#[interval]` and `#[timeout]` handler with the same rules as `CorvisApplication::init()`, including
`ScheduleModuleOptions` and the warning logged when `EventEmitterModule` or `ScheduleModule` is not imported.

#### Scenario: Cron and interval handlers run in a worker
- **WHEN** a module imports `ScheduleModule::for_root()` and registers a provider with an `#[interval("tick", 20)]`
  handler and a named `#[cron(..., name = "hourly")]` handler, and the process calls
  `create_application_context::<M>()` and waits 120 ms
- **THEN** the interval handler has run at least twice and `SchedulerRegistry::get_cron_job("hourly")` returns the
  job

#### Scenario: Event listeners receive events in a worker
- **WHEN** a module imports `EventEmitterModule::for_root(..)` and registers a provider with an
  `#[on_event("order.created")]` handler, the context is created with `create_application_context`, and
  `"order.created"` is emitted
- **THEN** the handler runs once

#### Scenario: A disabled kind is not scheduled
- **WHEN** the module imports `ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, ..Default::default() })`
  and the context is created with `create_application_context`
- **THEN** no cron job is registered, while intervals and timeouts still run

### Requirement: Standalone startup order matches init()
In a standalone context, handlers MUST be registered before any `on_module_init` hook runs, scheduled handlers MUST be
mounted after every `on_module_init` and before any `on_application_bootstrap` hook, exactly as in `init()`.

#### Scenario: An event emitted from on_module_init reaches its listener
- **WHEN** a provider emits `"ready"` from `on_module_init` and another provider listens with `#[on_event("ready")]`
- **THEN** in a standalone context the listener receives the event, as it does after `init()`

### Requirement: Handlers are registered once per application
Handler registration SHALL happen at most once per `CorvisApplication`. Calling `init()` on an application created by
`create_application_context` MUST NOT register any handler a second time and MUST NOT leave a scheduled handler queued
but not running.

#### Scenario: init() after create_application_context
- **WHEN** a module with a named `#[cron(..., name = "hourly")]` handler, an `#[interval]` handler and an
  `#[on_event("x")]` handler is opened with `create_application_context` and then `init()` is called
- **THEN** `init()` succeeds (no duplicate-name error), `"x"` is delivered to the listener exactly once per emit, and
  the interval keeps ticking

### Requirement: Closing a standalone context stops scheduled jobs
`close()` on a standalone context SHALL stop every timeout, interval and cron job registered from handlers before
`before_application_shutdown` runs, as it does for a full application.

#### Scenario: No ticks after close
- **WHEN** a standalone context with an `#[interval("tick", 20)]` handler is closed
- **THEN** the interval counter does not increase over the next 100 ms and `SchedulerRegistry::get_intervals()` is empty

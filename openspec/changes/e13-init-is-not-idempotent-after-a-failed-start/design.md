# Design

## Context

All line references are to `crates/corvis/src/application.rs` unless stated otherwise.

- `CorvisApplication` (`:81-92`) has two lifecycle flags, `hooks_called` and `initialized`, both `false` after
  `bootstrap` (`:135-136`).
- `call_init_hooks` (`:349-358`) returns early when `hooks_called` is true and sets `hooks_called = true` (`:353`)
  **before** running `OnModuleInit`, `mount_scheduler` and `OnApplicationBootstrap`. If `on_module_init` fails, the
  flag is already set.
- `init()` (`:361-423`) returns early only when `initialized` is true (`:362-364`); `initialized = true` is the last
  statement before `Ok(())` (`:421`). In between, in this order:
  1. the provider-registered global enhancers are folded into `self.config.router` (`:365-398`): guards,
     interceptors and pipes are inserted at increasing indexes from 0, filters with `insert(0, x)` (`:394`). Each
     provider with `enhancer: Some(..)` (`crates/corvis/src/di/provider.rs:117`, set by `Provider::app_guard` /
     `app_interceptor` / `app_pipe` / `app_filter` at `provider.rs:259-285`) adds one entry per call;
  2. `build_router` (`:399`, `crates/corvis/src/router/mod.rs:277`), which fails with a `DiError` when a controller or
     route enhancer cannot be resolved (`router/mod.rs:288-291`, `:319`); it also logs the mapped routes;
  3. `build_middleware` (`:400`, runs every module's `configure`) and, with the `websockets` feature,
     `build_gateways` (`:402`);
  4. `self.shared = Some(Arc::new(AppShared { .. }))` (`:403-417`), which `TestClient::new`
     (`crates/corvis/src/testing.rs:255-258`) reads through `shared()`;
  5. `discover` + `register_handlers` (`:418-419`, `crates/corvis/src/modules/mod.rs:19`), which adds event listeners
     with `add_listener` and crons with `add_cron(..)?` (`modules/mod.rs:33-44`);
  6. `call_init_hooks()` (`:420`).
- A failure in step 2 or later leaves the enhancers of step 1 in the global lists; a second call adds them again.
  A failure in step 6 also leaves the listeners of step 5 registered and `hooks_called` set, so a second `init()`
  registers the listeners a second time, skips every hook and returns `Ok(())`.
- Callers of `init()`: `handle()` (`:425-429`, on every request), `bind()` (`:476-477`, used by `listen()` and
  `listen_in_background()`), and `TestingModule::init()` (`crates/corvis/src/testing.rs:236-238`).
  `create_application_context` (`:63-67`) calls `call_init_hooks` directly and drops the application on error, so
  it cannot be retried.
- `crate::Result` is `anyhow::Result` (`crates/corvis/src/lib.rs:78`); `init()` errors are `DiError`s or whatever a
  hook or `register_handlers` returns.
- The only test of provider-registered enhancers is `crates/corvis/tests/enhancer_order.rs:52-63`; no test calls
  `init()` after a failure. `docs/fundamentals/lifecycle-events.md:63` says "`init()` is idempotent: calling it twice
  does not repeat the hooks", which is only true after a success.
- Original model (checked in `@nestjs/core` 11.2.1, from the facten port's `node_modules`): global enhancers registered
  as providers are applied once by `DependenciesScanner.applyApplicationProviders()` (`scanner.js:355`), called
  from `NestFactory.initialize` (`nest-factory.js:114`) at creation, not from `init()`. `NestApplication.init()`
  (`nest-application.js:95-111`) only returns early when `isInitialized`, which is set at the end; after a failure a
  second call re-registers middleware and routes and re-runs `callInitHook` for every module, including hooks that
  already succeeded. With the default `abortOnError`, a failing start ends the process, so retries are not a
  supported path there either.

Related open changes that touch `init()`:
- `e13-standalone-application-context-runs-events-and-scheduled` (#16) moves step 5 into `call_init_hooks` behind a
  "registered once" flag.
- `e1-public-metadata-and-startup-check` (#26) adds a route posture check between steps 2 and 3, a new way for
  `init()` to fail (the case that surfaced this issue).

## Goals / Non-Goals

**Goals:**
- After a failed `init()`, nothing runs twice and nothing is skipped silently: every further attempt fails with an
  error that explains why.
- A successful `init()` stays idempotent, as documented.
- The rule holds whatever steps `init()` contains now or later (#16, #26).

**Non-Goals:**
- Retrying a failed start on the same application.
- Changing the order of global enhancers or of the init steps.
- Behavior of `close()` after a failed `init()` (see proposal, Out of scope).

## Decisions

1. **Single-use after a failure, not idempotent.** A failed `init()` poisons the application; later calls return
   `InitFailedError`. Reason: by the time a later step fails, `on_module_init` hooks of other providers may have run
   (connections opened, background tasks spawned) and cannot be rolled back; re-running them, as the original does,
   duplicates those effects; skipping them, as Corvis does today, reports a start that did not happen.
   - Alternative: make every step idempotent (fold enhancers once, register handlers once, re-run only the hooks that
     did not complete). Rejected: needs per-provider hook bookkeeping, and a hook that failed half way still cannot
     be retried safely.
   - Alternative: copy the original (re-run everything). Rejected: re-registers routes and re-runs
     successful hooks; the original only gets away with it because the process normally aborts.
2. **Implementation: wrap the body.** Rename the current body to a private `async fn init_steps(&mut self)` and make
   `init()`:
   ```rust
   if let Some(first) = &self.init_error { return Err(InitFailedError::new(first.clone()).into()); }
   if self.initialized { return Ok(()); }
   match self.init_steps().await {
       Ok(()) => { self.initialized = true; Ok(()) }
       Err(e) => { self.init_error = Some(format!("{e:#}")); self.shared = None; Err(e) }
   }
   ```
   `init_error: Option<String>` is a new private field set to `None` in `bootstrap`. Storing the message (not the
   `anyhow::Error`, which is not `Clone`) is enough for the later errors. Because the guard sits around the whole
   body, steps added by #16 or #26 are covered without further work, and the order of the existing steps does not
   change. `handle()`, `bind()` and `TestingModule::init()` need no change: they already call `init()` first.
   - Alternative: keep `init()` as is and set `initialized = true` before the first fallible step. Rejected: a failed
     application would then look initialized and serve requests.
3. **`InitFailedError` is a public error type.** `pub struct InitFailedError { first_error: String }` in
   `application.rs`, with `Display`: `"the application failed to initialize earlier and cannot be initialized
   again; create a new application (first error: <message>)"`, `std::error::Error`, `Debug`, and an accessor
   `first_error(&self) -> &str`. Re-exported from the crate root (`lib.rs`, next to the other application types),
   not from the prelude. Callers can match it with `err.downcast_ref::<InitFailedError>()`.
   - Alternative: a plain `anyhow!` message. Rejected: tests and supervisors could only match on text.
4. **Drop the half-built pipeline on failure.** `self.shared` is reset to `None` when `init_steps` fails, so a
   `TestClient` cannot be built from a failed application. `TestClient::new` (`testing.rs:255-258`) checks a new
   `pub(crate) fn init_failed(&self) -> bool` first and panics with `"app.init() failed; create a new application
   instead of reusing this one"`; otherwise it keeps its current message. Background tasks spawned by the failed
   start (scheduler timers mounted before `on_application_bootstrap` failed) are not stopped here; `close()` still
   stops them as today.
5. **No change to the enhancer folding.** With Decision 1 the fold runs at most once per application, which fixes
   the duplicated guards and filters the issue reports. Moving the fold to `bootstrap`, as the original does, would
   also work but touches the enhancer order that `enhancer_order.rs` pins; it is listed under Out of scope.
6. **Tests.**
   - Unit test in `application.rs` (`#[cfg(test)]`, it needs the private `config.router` lists): an app with
     `Provider::app_guard`, `app_interceptor`, `app_pipe`, `app_filter` and a failing `on_module_init`; after two
     `init()` calls each global list holds exactly one entry of each provider.
   - Integration tests in a new `crates/corvis/tests/init_failure.rs`, through the public API: failing
     `on_module_init` then second `init()` (error type, message, `on_application_bootstrap` never ran); `handle()`
     and `listen_in_background("127.0.0.1:0")` after the failure; `request(&app)` panics (`#[should_panic]`); a
     guard counter shows one `can_activate` per request on a fresh application built from the same module once the
     hook succeeds (a static `AtomicBool` makes it fail only once); two successful `init()` calls run hooks once.
   - All sockets bind to `127.0.0.1`.

## Risks / Trade-offs

- [Code that relied on retrying `init()`] → such code got duplicated enhancers or skipped hooks; it now gets an
  explicit error. No caller in the workspace or the ports does this (grep).
- [`format!("{e:#}")` of the first error may be long] → it is only shown in the second error, which is rare; the
  first call still returns the original error with its full chain.
- [A failed application keeps running timers or listeners started before the failure] → unchanged from today;
  `close()` stops the scheduler. Documented in the lifecycle page.

## Open questions

- Should `close()` after a failed `init()` skip `on_module_destroy` for providers whose `on_module_init` did not run?
  Recommendation: leave as is in this change and open an issue if a user hits it.
- Should a failed `init()` honor `CorvisApplicationOptions::abort_on_error` (exit the process) like a failure during
  creation does? Recommendation: no change here; it is a separate behavior decision.

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- `close()` after a failed `init()` keeps its current behavior here; skipping `on_module_destroy` for providers whose
  init never ran is a separate question (follow-up issue if needed).
- `abort_on_error` does not change in this change; applying it to `init()` failures is left for another issue.

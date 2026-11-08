# init() is not idempotent after a failed start

Implements [randygil/corvis#123](https://github.com/randygil/corvis/issues/123) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

`CorvisApplication::init()` is only guarded by an `initialized` flag that is set at the very end. When `init()` fails
part way (a guard that cannot be resolved while building the router, a `configure(consumer)` error, a cron handler
that cannot be registered, an `on_module_init` hook that returns `Err`, or the startup check planned in #26), the
side effects that already happened stay, and a second call repeats some of them and skips others:

- The global enhancers registered as providers (`Provider::app_guard`, `app_interceptor`, `app_pipe`, `app_filter`)
  are inserted into the application's global lists again, so each of them runs twice per request.
- Event listeners (`#[on_event]`) and scheduled handlers are registered again.
- `call_init_hooks` marks the hooks as called before running them, so a retry after a failing `on_module_init` skips
  the hooks and the second `init()` returns `Ok(())` even though some hooks never ran.

`listen()`, `listen_in_background()`, `handle()` and `TestingModule::init()` all go through `init()`, so any of them
can trigger the retry.

The original model does not support retrying either: it applies the provider-registered global enhancers once, when
the application is created, and its `init()` re-runs every step (routes and every `onModuleInit`, including those that
already succeeded) when called again after a failure. Lifecycle hooks that already ran cannot be undone, so a retry
can never be made safe in general.

## What Changes

- `init()` becomes explicitly single-use after a failure: the first failing call returns its original error, as
  today; every later call to `init()` (directly or through `listen()`, `listen_in_background()`, `handle()` or
  `TestingModule::init()`) returns a new public error, `InitFailedError`, that says the application failed to
  initialize earlier, quotes the first error and tells the user to create a new application. Nothing is re-run, so
  global enhancers, event listeners and scheduled handlers are never registered twice and hooks are never skipped
  silently.
- A failed application no longer exposes its half-built request pipeline: `corvis::testing::request(&app)` panics
  with a message about the failed `init()` instead of serving requests through it.
- A successful `init()` keeps returning `Ok(())` on later calls without doing anything (unchanged), and a `listen()`
  that fails to bind after a successful `init()` can still be retried on another address (unchanged).
- Docs: the "`init()` is idempotent" sentence of the lifecycle page (English and Spanish) is corrected to describe
  both cases.

## Capabilities

### New Capabilities
- `core/lifecycle`: what `init()` does when called more than once, after success and after failure.

### Modified Capabilities
- None.

## Impact

- Code: `crates/corvis/src/application.rs` (`init`, a new failure field, the new error type),
  `crates/corvis/src/testing.rs` (`TestClient::new` message), `crates/corvis/src/lib.rs` (re-export of
  `InitFailedError`).
- Behavior change: a second `init()` after a failure now returns an error instead of possibly succeeding with
  duplicated enhancers or skipped hooks. No existing test or example retries `init()` after a failure (checked with
  grep over `crates/corvis/tests` and `examples/`); the port apps start once and exit on error.
- Coordination: `e13-standalone-application-context-runs-events-and-scheduled` (#16) moves handler registration into
  `call_init_hooks`, and `e1-public-metadata-and-startup-check` (#26) adds a failing check inside `init()`. Both stay
  compatible: the failure state wraps the whole `init()` body, whatever steps it contains.
- Tests: `crates/corvis/tests/init_failure.rs` (new) and a unit test in `application.rs`.
- Docs: `docs/fundamentals/lifecycle-events.md`, `docs/fundamentals/testing.md` and their `docs/es/` translations.

## Out of scope

- Making `init()` retryable (rolling back hooks, listeners and scheduled jobs): not possible in general, see
  design.md.
- Moving the folding of provider-registered global enhancers from `init()` to application creation, as the original
  does: no longer needed for this bug once `init()` cannot run twice; worth its own issue only if a caller needs the
  global lists before `init()`.
- What `close()` does after a failed `init()` (today it runs `on_module_destroy` and the shutdown hooks on providers
  whose `on_module_init` may never have run): a separate issue if it matters.
- `abort_on_error` applied to `init()` failures (today it only applies to errors while creating the application).
- `create_application_context` failures: the application is dropped on error there, so it cannot be retried.

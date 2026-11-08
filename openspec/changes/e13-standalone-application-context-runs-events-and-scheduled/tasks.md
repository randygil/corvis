# Tasks

## 1. Regression tests first

- [ ] 1.1 Add `crates/corvis/tests/application_context.rs` with a worker module (`ScheduleModule::for_root()`, `EventEmitterModule::for_root(..)`, a provider with `#[interval("tick", 20)]`, `#[cron(CronExpression::EVERY_HOUR, name = "hourly")]` and `#[on_event("order.created")]`); tests for the spec scenarios "Cron and interval handlers run in a worker" and "Event listeners receive events in a worker" using `create_application_context`; use per-test statics or instance counters so tests do not share state; verify they fail on the current code with `cargo test -p corvis --test application_context`
- [ ] 1.2 Add tests for "init() after create_application_context" (no duplicate-name error, one delivery per emit, interval still ticking) and "No ticks after close"; verify the first fails on the current code
- [ ] 1.3 Add tests for "A disabled kind is not scheduled" (`ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, .. })`) and "An event emitted from on_module_init reaches its listener"; verify with `cargo test -p corvis --test application_context`

## 2. Implementation

- [ ] 2.1 In `crates/corvis/src/application.rs`, add the once-guarded handler registration (design decision 1), call it at the start of `call_init_hooks` (`application.rs:349`), and remove the direct `discover` + `register_handlers` calls from `init()` (`application.rs:418-419`); verify every test from group 1 passes
- [ ] 2.2 Update the rustdoc of `create_application_context` (`application.rs:62-63`) and `init` to say which handlers each one registers; verify with `cargo doc -p corvis --no-deps` without new warnings
- [ ] 2.3 Verify nothing else regressed in the paths that use `init()`: `cargo test -p corvis --test modules --test schedule --test app`

## 3. Documentation (EN + ES)

- [ ] 3.1 `docs/standalone-applications.md`: remove the warning under `## create_application_context`, show the worker example with `create_application_context` and `#[cron]`, and add a short note on turning scheduling off with `ScheduleModule::for_root_with(ScheduleModuleOptions { .. })` or a separate root module; same in `docs/es/standalone-applications.md`; verify by reading both pages
- [ ] 3.2 `docs/fundamentals/lifecycle-events.md` ("Contexts without a server"): remove the warning and state that handlers are registered before `on_module_init` and mounted before `on_application_bootstrap`; same in `docs/es/`; verify by reading both pages
- [ ] 3.3 The eight `docs/microservices/*.md` pages (`basics`, `custom-transport`, `grpc`, `kafka`, `mqtt`, `nats`, `rabbitmq`, `redis`): replace "today `create_application_context` does not register ... so consumption must start from a lifecycle hook" with a sentence that a standalone context runs `#[on_event]`/`#[cron]`/`#[interval]`/`#[timeout]` handlers; same in `docs/es/microservices/`; verify `grep -rn "does not register" docs --include=*.md` no longer matches these pages and the Spanish equivalent ("no registra") neither
- [ ] 3.4 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [ ] 4.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 4.3 `cargo test --workspace` passes
- [ ] 4.4 Regression tests from group 1 are in place and the ones marked in 1.1 and 1.2 fail on the code before this change
- [ ] 4.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 4.6 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences; record in the change whether `consi-backend`'s `run-jobs` binary needs a follow-up (design, Open questions)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #16, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

# Tasks

Start after #35 (`e2-socket-io-gateway-adapter`) and `q-http-server-robustness` are merged into `dev`. Shared Cargo target directory only (never a target
directory per task). Timing rule for every test: wait for a condition with a deadline that only fails the test, use the
paused tokio clock for intervals and timeouts, never `sleep` to synchronize, never compare durations between runs. Test
servers and the fixture generator bind to `127.0.0.1` only. Every bug fix starts with its test, run once to see it fail
(the failure is noted in Implementation notes). Behavior must not change except where design.md "Behavior visible to
applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev` with #35 and
  `q-http-server-robustness` merged (`application.rs`, `websockets.rs`, `testing.rs` and `serve` in particular); write moved lines and any conflict in a new "Implementation
  notes" section of design.md before any other task; verify `cargo test -p corvis` passes on the untouched tree

## 2. Lifecycle fixture (design.md Decision 1)

- [ ] 2.1 `crates/corvis/tests/fixtures/lifecycle/` with `package.json` (the original packages at 11.2.1, `ws`,
  `reflect-metadata`, `rxjs`), `generate.mjs` (the four cases of Decision 1, `127.0.0.1` only, header comment with usage)
  and the recorded `lifecycle.json` (packages and versions included); commit `package-lock.json`; verify `npm ci && node
  generate.mjs` reproduces the committed JSON byte for byte and `node scripts/check-naming.mjs` passes
- [ ] 2.2 `crates/corvis/tests/lifecycle_fixture.rs`: builds the module shapes of cases 1-3 in Corvis, compares case 1
  exactly and cases 2-3 against the recording plus `EXPECTED_DIFFERENCES` (Decision 2, with the reason in a comment); run
  it and record in Implementation notes that the order and failure cases fail on the untouched code

## 3. Shutdown completes, root first (Decisions 2, 3)

- [ ] 3.1 `crates/corvis/tests/shutdown.rs`: the failing-hook tests of Decision 2 (a failing and a panicking
  `on_module_destroy`, failures in two phases, the interval stopped on the paused clock); run them and record that they
  fail
- [ ] 3.2 `di/container.rs`: `call_shutdown_hook` running every hook of a phase with panics caught, returning
  `Vec<HookFailure>`; `modules_for_shutdown()` (reverse of `modules_for_hooks()`); the comment and reassignment at the old
  `:752-756` removed; init phases unchanged; verify `cargo test -p corvis --test di` and `--test app`
- [ ] 3.3 `application.rs`: `ShutdownError` and `HookFailure` (public, `#[non_exhaustive]`, accessors, `Display`, `source`),
  re-exported from `lib.rs` next to `InitFailedError`; `close_with_signal` in the order of Decision 2 with each failure
  logged; grep the workspace and the three ports for code that depends on the old shutdown order or downcasts `close()`
  errors and record the result; verify `shutdown.rs`, `lifecycle_fixture.rs` cases 1-3 and `init_failure.rs` pass
- [ ] 3.4 `listen_with` closes whatever `serve` returned (Decision 4, last item); `futures::future::pending()` instead of
  `std::mem::forget(tx)`; verify `cargo test -p corvis --test app --test application_context`

## 4. Background work (Decisions 4, 5)

- [ ] 4.1 Tests of Decisions 4 and 5 in `shutdown.rs` (background server refused after `close()`, request in flight
  completes; dropped app; bootstrap failure after mount; failed `create_application_context`; half-failed `mount`; dropped
  app's server ends); run and record that they fail
- [ ] 4.2 `application.rs`: `BackgroundServer` kept by `listen_in_background_with` (and the TLS variant), stopped and awaited
  by `close_with_signal` before the destroy phase; `impl Drop for CorvisApplication` (scheduler, background senders, the
  gateway servers of Decision 7); `init()` stops the scheduler on failure; rustdoc of `Drop`, `close()` and
  `listen_in_background`; verify the tests of 4.1
- [ ] 4.3 `modules/schedule/mod.rs`: `mount` calls `shutdown()` before returning an error; verify `cargo test -p corvis
  --test schedule` and the half-failed `mount` test

## 5. Gateways on close (Decisions 6, 7)

- [ ] 5.1 Tests: a connected gateway client receives close 1001 after `close()` and `count()` is 0; a peer that never answers
  is cut at `close_timeout` on the paused clock; `for_path` after one and after both of two applications close, and after a
  drop; run and record that they fail
- [ ] 5.2 `websockets.rs`: `close_gateways(&AppShared)` through the existing close path, called by `close_with_signal`;
  removal of the application's own entries from `SERVERS` (pointer equality) on close and drop; verify the tests of 5.1 and
  `cargo test -p corvis --test websockets --test ws_security --test ws_enhancers --test send_timeout_ws`

## 6. Cache (Decision 8)

- [ ] 6.1 Unit tests in `modules/cache.rs` with synthetic instants: the eviction sequence of design.md, 10,000 expiry
  cycles keeping the queue at most `2 × max`, `del` then `set` under `max`, `set` of a value that fails to serialize; run
  and record that the first two and the last fail
- [ ] 6.2 Generation-tagged queue, stale-item skipping, compaction, O(1) `del`, `get_value_at` / `set_value_at`, `set`
  that stores nothing on a serialization error; verify the unit tests and the `CacheInterceptor` tests
  (`cargo test -p corvis --test modules`)

## 7. Cron jobs (Decision 9)

- [ ] 7.1 Unit tests in `cron_job.rs`: the parked-claim race, the panicking hand-built `TickFn`, `stop_and_wait` released by
  the test, the warning through a capturing logger; run and record that they fail
- [ ] 7.2 `compare_exchange` claim, reset guard with `Notify`, `complete()` without polling, `Logger::new("Cron")` at the
  five `eprintln!` sites, the `cfg(test)` park hook; verify the unit tests and `cargo test -p corvis --test schedule`
- [ ] 7.3 `tests/schedule.rs`: the waits for spawned tasks at the old `:108`, `:118`, `:205`, `:210` become condition waits;
  the wall-clock cron waits get a comment; verify the file passes 5 runs in a row

## 8. Listen ports (Decision 10)

- [ ] 8.1 Test for `70000_i32` and `-1_i32` (run, record the failure), then `u16::try_from` with `InvalidInput`; verify
  `cargo test -p corvis --test app`

## 9. Test client (Decision 11)

- [ ] 9.1 Unit tests in `testing.rs` for an invalid header, an unparsable path and a value that fails to serialize (run,
  record the failures); `TestRequestError`, the recorded error, `try_send`, `send` panicking with the message; rustdoc
  `# Panics`
- [ ] 9.2 Run `cargo test --workspace`; change every test that relied on the silent fallback to `try_send` or to valid
  input, and list them in Implementation notes

## 10. Small items (Decision 12)

- [ ] 10.1 `config.rs`: `for_feature` computes the value once (`ConfigService::add_value`); test with a counting factory
  (run, record the failure first); verify `cargo test -p corvis --lib config`
- [ ] 10.2 Remove `CorvisApplication::started`; rustdoc of `CorvisApplication`, `close()` (cycle through `ModuleRef`, N11),
  `ModuleRef::resolve_for` (spawned-task deadlock, N12), `CorvisFactory::create_with_options` and
  `TestingModuleBuilder::compile` (the logger setting is process-wide, as in the original); verify `cargo doc -p corvis
  --no-deps` without warnings

## 11. Docs

- [ ] 11.1 `docs/fundamentals/lifecycle-events.md`: shutdown order (root first), failures during shutdown and
  `ShutdownError` with the compatibility note (the original stops at the first failure and its process exits), `close()`
  stopping background servers and gateway connections, what dropping an application does, a failed start stopping its
  jobs; translate in `docs/es/fundamentals/lifecycle-events.md`
- [ ] 11.2 `docs/fundamentals/testing.md` (`try_send`, the panic of `send`), `docs/data/caching.md` (eviction order stated),
  `docs/websockets/gateways.md` (close 1001 on shutdown, `for_path` after close), and their `docs/es/` versions; verify
  `npm run docs:check` in `docs/`

## 12. Definition of done

- [ ] 12.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 12.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 12.3 `cargo test --workspace` passes; `lifecycle_fixture.rs` and `shutdown.rs` pass 5 runs in a row
- [ ] 12.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls, socket-io,
  auth-local, auth-api-key, auth-session and auth-oauth2, with no warnings, and `cargo check -p corvis --no-default-features`
- [ ] 12.5 Regression tests: one per spec scenario, each seen failing before its fix (Implementation notes list them)
- [ ] 12.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 12.7 Framework runtime code changed (`application.rs`, `di/container.rs`, `modules/`, `websockets.rs`, `testing.rs`):
  run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences and their own test suites pass
- [ ] 12.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 12.9 Every new test binds `127.0.0.1` only and has no `sleep` used to synchronize (grep the new files and record the
  result)
- [ ] 12.10 Security review with the `security-reviewer` agent, scoped to Decision 6 (closing WebSocket connections: close
  path, `close_timeout`, queued frames) and Decision 4 (draining servers on `close()`); record findings and fixes in design.md
  under "Security review"

## Workflow follow-up

- Archive with `/opsx:archive` (deltas of `core/lifecycle`, `core/modules`, `core/testing`, `ws/gateways` applied).
- Close the issue, tick it in epic #187 and move the card to Done (see the `corvis-task` skill).
- Open the follow-ups the maintainer accepts: an injectable clock for cron jobs; recording the original cache store's
  eviction policy if a user asks for it.

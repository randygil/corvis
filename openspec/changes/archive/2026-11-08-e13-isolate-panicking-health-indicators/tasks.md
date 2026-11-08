# Tasks

## 1. Per-indicator panic isolation

- [x] 1.1 In `crates/corvis/src/modules/health.rs`, add the private `async fn run_indicator(ind: &HealthIndicator)
  -> Result<Value, Value>` that wraps `(ind.check)()` in `AssertUnwindSafe(..).catch_unwind()` and maps a panic to
  `Err(json!({ "message": "health indicator panicked" }))`; make `HealthCheckService::check` (`:100-134`) use
  `join_all(indicators.iter().map(run_indicator))`. If `e13-honest-health-checks` already landed, keep its
  `tokio::time::timeout` in the same function, outside the `catch_unwind` (design.md, Decision 2); verify the
  existing `cargo test -p corvis --test modules` still passes
- [x] 1.2 Log the panic once with `Logger::new("HealthCheckService").error(..)`, including the key and the payload
  from `cron_job::panic_message` (re-export it as `pub(crate) use cron_job::panic_message;` in
  `crates/corvis/src/modules/schedule/mod.rs`, since `mod cron_job` is private at `:27`); verify with the log-capture test of 2.2

## 2. Regression tests

- [x] 2.1 In `crates/corvis/tests/health.rs` (create it if #19 has not): a panicking indicator next to an `Ok` one
  → `Err(HttpException)` with status `503`, `error.<key>` `{ "status": "down", "message": "health indicator
  panicked" }`, the other key in `info`, both in `details`; a second test where the closure panics before returning
  its future; verify `cargo test -p corvis --test health`
- [x] 2.2 A test whose indicator panics with `"postgres://user:secret@db/app"`: the serialized body does not contain
  `"secret"`, and a test `LoggerService` installed with `corvis::logger::set_logger` records one error line with the
  key and the payload; keep the logger and any panic hook changes local to this test (separate test binary or
  restore after use) so other tests are unaffected; verify `cargo test -p corvis --test health`
- [x] 2.3 HTTP test: a controller route returning `self.health.check(..)` with one panicking indicator, requested
  through `corvis::testing::request`, answers `503` with the health body (not `500`); verify
  `cargo test -p corvis --test health`

## 3. Docs

- [x] 3.1 `docs/reliability/terminus.md`: in "Response format" (or a short "Failures" note under "Indicators"), say
  that an indicator that panics is reported `down` with `"message": "health indicator panicked"`, the others are
  still reported, the status is `503`, and the panic message only goes to the log; if the Open question on the
  original behavior is confirmed, mention the divergence; update the module doc comment at the top of `health.rs`;
  verify the example JSON matches the test output
- [x] 3.2 Same change in `docs/es/reliability/terminus.md`; verify `npm run docs:check` in `docs/`

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of group 2
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [x] 4.5 (the maintainer runs it) corvis-parity: the three port apps (consi, facten, houseplan) keep 0 apidiff differences (framework runtime
  code changed, even though the ports do not use health checks) (maintainer batch run on batch/phase03-d 2780936: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#126](https://github.com/randygil/corvis/issues/126)
- [x] Tick #126 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [x] Move the card to Done in the Corvis Roadmap project

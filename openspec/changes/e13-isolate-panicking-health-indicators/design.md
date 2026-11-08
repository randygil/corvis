# Design

## Context

All health code is in `crates/corvis/src/modules/health.rs` (148 lines).

- `HealthIndicator` (`health.rs:31-53`) stores a key and `check: Check`, an
  `Arc<dyn Fn() -> BoxFuture<'static, Result<Value, Value>>>` (`health.rs:29`). `HealthIndicator::new` (`:39-52`)
  wraps the user closure as `async move { f().await }.boxed()` (`:47-50`), so the user closure itself is only called
  when the boxed future is first polled: a panic in the closure body and a panic inside its future both happen while
  the future is being polled.
- `HealthCheckService::check` (`health.rs:100-134`) does
  `futures::future::join_all(indicators.iter().map(|i| (i.check)())).await` (`:103`) with no `catch_unwind`. A panic
  in any indicator unwinds out of `join_all` and out of `check`.
- The panic is then caught by the route pipeline: `execute_route` wraps `run_pipeline` in
  `AssertUnwindSafe(..).catch_unwind()` (`crates/corvis/src/router/mod.rs:577-580`) and turns it into
  `panic_exception` (`router/mod.rs:772-779`), an `HttpException::internal` whose body is
  `{ "statusCode": 500, "message": "Internal server error" }` (`crates/corvis/src/http/exception.rs:141-150`). That
  is the `500` the issue reports. Outside a route (a caller awaiting `check` directly), the panic reaches the caller.
- Existing panic helpers: `cron_job::panic_message` (`crates/corvis/src/modules/schedule/cron_job.rs:58-60`,
  `pub(crate)`) turns a payload into a `String` (`&str`, `String`, else `"panic"`); the scheduler uses it with
  `AssertUnwindSafe(call()).catch_unwind()` (`modules/schedule/mod.rs:514-519`). `health.rs` already imports
  `futures::FutureExt` (`:22`), which provides `catch_unwind`.
- The workspace profiles do not set `panic = "abort"` (`Cargo.toml`), so unwinding is available.
- Tests: the only health test is `crates/corvis/tests/modules.rs:87-95` (controller with one `Ok` and one `Err`
  indicator) and `:162-164` (expects `503`). No test panics in an indicator.
- Docs: `docs/reliability/terminus.md` "Response format" (`:50`) and "Limits" (`:107`) say nothing about panics.

Relation to `e13-honest-health-checks` (#19, not archived yet): its design (Decision 6) wraps each indicator future
in `tokio::time::timeout` inside `check`, keeping `join_all`, and its proposal lists "catching a panicking indicator"
as out of scope. That item became this issue. Both changes edit the same lines of `check`, so this change defines one
private per-indicator runner that both wrappers live in (Decision 2), whichever change lands first.

## Goals / Non-Goals

**Goals:**
- One panicking indicator is reported `down` and never hides the results of the others.
- The response keeps the documented health format and the `503` status of any other failure.
- No internal data from a panic payload is exposed on the health endpoint.

**Non-Goals:**
- Time limits, the HTTP ping check, RSS measurement (all #19).
- Changing how panics are handled in routes, gateways or the scheduler.

## Decisions

1. **Catch per indicator with `AssertUnwindSafe(fut).catch_unwind()`.** Each indicator future is wrapped before it
   goes into `join_all`, so indicators keep running concurrently and a panic in one only resolves that one. Because
   `HealthIndicator::new` defers the user closure into the future (`health.rs:47-50`), this also covers panics in the
   closure body. `AssertUnwindSafe` is sound here: the future is consumed by the wrapper and nothing observes its
   state after a panic, the same reasoning the router and the scheduler already rely on.
   - Alternative: `tokio::spawn` each indicator and map `JoinError::is_panic()`. Rejected: needs `'static` futures
     (they are, but it adds a task per indicator and a runtime requirement for no gain), and detaches indicators
     from the request's cancellation.
   - Alternative: catch around the whole `join_all`. Rejected: loses the other indicators' results, which is the bug.
2. **One private runner per indicator.** Add `async fn run_indicator(ind: &HealthIndicator) -> Result<Value, Value>`
   in `health.rs` that calls `(ind.check)()`, wraps it in `catch_unwind`, logs and maps a panic to
   `Err(json!({ "message": "health indicator panicked" }))`. `check` becomes
   `join_all(indicators.iter().map(run_indicator))`. If #19 lands first, its `tokio::time::timeout` goes inside the
   same function (timeout outside, `catch_unwind` inside, so a panic is reported as a panic and a hang as a timeout);
   if this change lands first, #19 adds the timeout there.
3. **Fixed message in the body, payload in the log.** Body: `"message": "health indicator panicked"`. Log:
   `Logger::new("HealthCheckService").error(format!("health indicator \"{key}\" panicked: {payload}"))`, payload
   through `cron_job::panic_message`, made reachable with `pub(crate) use cron_job::panic_message;` in
   `modules/schedule/mod.rs` (the `cron_job` module is private, `schedule/mod.rs:27`). Health endpoints are often public (the docs example marks the controller with
   `IS_PUBLIC_KEY`, `docs/reliability/terminus.md:27`), and a panic message can carry a connection string or a query;
   the router does the same for handler panics (generic `500` body, cause kept server-side).
   - Alternative: put the payload in `message`. Rejected for the leak risk; see Open questions.
4. **Status stays `503`.** A panicking indicator is a `down` indicator, so `check` returns the same
   `HttpException::with_error(body, "Service Unavailable", 503)` (`health.rs:132`).
5. **Tests in `crates/corvis/tests/health.rs`.** Direct calls to `HealthCheckService::check` for the panic-in-future
   and panic-in-closure cases, a log capture through `corvis::logger::set_logger` (`crates/corvis/src/logger.rs:171`) with a test
   `LoggerService` to check the key and payload are logged and absent from the body, and an HTTP test through
   `corvis::testing::request` asserting `503` instead of `500`. Tests use `std::panic::set_hook` sparingly or accept
   the default hook's stderr output; they must not change the global hook for other tests in the same binary.

## Risks / Trade-offs

- [Divergence from the original library] → see Open questions; the result for an orchestrator is the same
  (non-2xx), and the body becomes useful.
- [A panicking indicator might leave shared state (a `Mutex`) poisoned for the next request] → that indicator keeps
  reporting `down` (it will panic again or see the poisoned lock), which is the honest result.
- [The default panic hook still prints the payload to stderr] → acceptable: stderr is server-side, like the log line.

## Open questions

- Original behavior: not verified here (no copy of `@nestjs/terminus` exists in the workspace or the port apps'
  `node_modules`). Its health check executor is understood to re-throw an indicator error that is not a health-check
  error, which gives a `500`, the behavior this issue calls a bug. The issue asks for `down`,
  so this change diverges on purpose. Recommendation: when the #19 differential fixture is recorded, add a scenario
  with an indicator that throws a plain `Error` to confirm the original behavior, and document the divergence in
  `docs/reliability/terminus.md` if confirmed.
- Should the panic message be included in the body (more useful while debugging, risk of leaking secrets on a public
  endpoint)? Recommendation: no; keep it in the log only.

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- The panic message is never included in the response (only in the log): health endpoints are often public.
- The original's behavior for a throwing indicator is confirmed with the differential fixture of #19 before this
  change is archived; if the original answers 500, the 503 here is recorded as an intentional divergence in the docs.
- The change folder was renamed to `e13-isolate-panicking-health-indicators` (shorter); it implements issue #126.

# A panicking health indicator turns the whole health response into a 500

Implements [randygil/corvis#126](https://github.com/randygil/corvis/issues/126) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

`HealthCheckService::check` awaits all indicators together with `futures::future::join_all`. When one indicator
panics (an `unwrap()` on a driver error, a poisoned lock, an index out of bounds in a custom check), the panic leaves
`check`, reaches the route's panic handler and the health endpoint answers `500 Internal Server Error`. The response
then says nothing about which indicator failed or which ones are still healthy, and an operator reading it sees a
framework error instead of a health report.

The change for #19 (`e13-honest-health-checks`) already isolates each indicator against hangs with a per-indicator
time limit; this change does the same for panics, so one broken indicator is reported as `down` next to the others.

## What Changes

- `HealthCheckService::check` runs every indicator so that a panic inside it, whether raised when the check is
  started or while it is awaited, is caught for that indicator only. The panicking indicator is reported as
  `{ "status": "down", "message": "health indicator panicked" }` in `error` and `details`, the other indicators keep
  their own results, and the response is the usual `503` with the health body.
- The panic payload is not put in the response (health endpoints are usually public); it is logged once with the
  indicator key through the `HealthCheckService` logger context.
- Docs (`reliability/terminus`) in English and Spanish describe how a panicking indicator is reported.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `reliability/health-checks`: adds the panic-isolation requirement. The capability is introduced by
  `e13-honest-health-checks` (#19); this delta only adds a requirement, so it applies whether #19 is archived before
  or after this change.

## Impact

- Code: `crates/corvis/src/modules/health.rs` (`HealthCheckService::check` and a private per-indicator runner).
- No public API change: `HealthIndicator`, `HealthCheckService::check` and the response format keep their
  signatures and shape.
- Behavior change: a panicking indicator produces a `503` with the health body instead of a `500`. Orchestrators treat
  both as unhealthy, so liveness and readiness decisions do not change.
- Tests: `crates/corvis/tests/health.rs` (created by #19, or created here if this change lands first).
- Docs: `docs/reliability/terminus.md` and `docs/es/reliability/terminus.md`.

## Out of scope

- Time limits per indicator, the HTTP `ping_check`, `tcp_check` and RSS on every platform: all in
  `e13-honest-health-checks` (#19), which lists panic isolation as its own out-of-scope item; this change is that
  item.
- Panics inside tasks an indicator spawns with `tokio::spawn` (they surface as a `JoinError` the indicator itself
  handles).
- Deduplicating the panic-payload helpers (`router/mod.rs:772` `panic_exception` and
  `modules/schedule/cron_job.rs:58` `panic_message`) into one shared function: a small cleanup issue.
- Builds with `panic = "abort"`: a panic ends the process there, and nothing in the health module can change that.

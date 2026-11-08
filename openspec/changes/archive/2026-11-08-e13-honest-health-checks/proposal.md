# Honest health checks

Implements [randygil/corvis#19](https://github.com/randygil/corvis/issues/19) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

A health endpoint exists so an orchestrator can trust it. Today three built-in behaviors make it report `up` when the
thing it claims to check was never checked, or never answer at all:

- `HttpHealthIndicator::ping_check` only opens a TCP connection. A web server that accepts connections but answers
  `500` (or is a different service on the same port) is reported `up`. The original library's `pingCheck` makes an HTTP
  request and fails on a non-2xx response.
- `MemoryHealthIndicator::check_rss` reads `/proc/self/statm`, so on Windows and macOS it reports `up` with no data:
  the threshold is never enforced. On Linux it also assumes 4 KiB pages.
- `HealthCheckService::check` waits for every indicator with no time limit, so one hung indicator hangs the health
  endpoint until the orchestrator gives up, and the response never says which indicator was stuck.

## What Changes

- `HttpHealthIndicator::ping_check(key, url)` makes an HTTP `GET` to an `http://` (or, with a new optional feature,
  `https://`) URL and is `up` only on a 2xx response; failures report `message` and, when a response arrived,
  `statusCode` and `statusText`, with the same body shape as the original library.
- The current TCP behavior stays available under an honest name: `HttpHealthIndicator::tcp_check(key, "host:port")`.
- `MemoryHealthIndicator::check_rss` measures resident memory on Linux, Windows and macOS (real page size on Linux);
  where it cannot measure, it reports `down` with a message that says so instead of a silent `up`.
- Every indicator runs with a time limit: 5 seconds by default, overridable per indicator with
  `HealthIndicator::timeout(Duration)`. An indicator that exceeds it is reported `down` with
  `"message": "timeout of <ms>ms exceeded"` and the others are still reported.
- Docs (`reliability/terminus`, `deployment`) in English and Spanish drop the "Limits" caveats this change removes and
  describe the new behavior.

## Capabilities

### New Capabilities
- `reliability/health-checks`: what `HealthCheckService` and the built-in indicators check, their time limits and the
  response format.

## Impact

- Code: `crates/corvis/src/modules/health.rs`, `crates/corvis/src/lib.rs` (re-exports), `crates/corvis/Cargo.toml`
  (hyper `client` feature; `memory-stats`; optional TLS feature).
- Behavior change for existing users of `ping_check` with a `host:port` argument: it now reports `down` with a message
  pointing to `tcp_check`. The crates are not published yet and none of the three port apps nor the examples call
  `ping_check` (verified with grep), so no caller breaks silently.
- Tests: `crates/corvis/tests/health.rs` (new), a differential fixture against `@nestjs/terminus`.
- Docs: `docs/reliability/terminus.md`, `docs/deployment.md` and their `docs/es/` translations.

## Out of scope

- New built-in indicators (`checkHeap`, disk, database, microservice): the docs already say to write them with
  `HealthIndicator::new`; a separate issue can add them.
- Catching a panicking indicator (today a panic inside one indicator fails the whole request with a 500): worth its own
  issue.
- Graceful-shutdown behavior of the health endpoint (`503` while shutting down).
- A module-level option to change the default timeout (`TerminusModule::for_root`): see Open questions in design.md.

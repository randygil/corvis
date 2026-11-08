# Design

## Context

All health code lives in `crates/corvis/src/modules/health.rs` (148 lines), re-exported from
`crates/corvis/src/lib.rs:144` and the prelude (`lib.rs:166`).

- `HealthIndicator` (`health.rs:31-53`) is a key plus a boxed async closure returning `Result<Value, Value>`
  (`Ok` = up, `Err` = down). It has no timeout field.
- `HttpHealthIndicator::ping_check` (`health.rs:58-73`) takes `addr` as `host:port` and only runs
  `tokio::net::TcpStream::connect` inside a fixed 3 s `tokio::time::timeout`; success is `Ok({})`, errors are
  `{ "message": e.to_string() }` or `"timeout of 3000ms exceeded"`. No HTTP request is sent.
- `MemoryHealthIndicator::check_rss` (`health.rs:78-88`) calls `rss_bytes()` (`health.rs:90-94`), which reads
  `/proc/self/statm` and multiplies the second field by a hard-coded 4096. When the file does not exist (Windows,
  macOS) it returns `None` and the indicator reports `Ok({})`, i.e. `up` with no measurement. When it measures, the
  details include `"rss"`.
- `HealthCheckService::check` (`health.rs:100-134`) runs all indicators with `futures::future::join_all` and no time
  limit, then builds `{ status, info, error, details }`; any `down` makes it return
  `HttpException::with_error(body, "Service Unavailable", 503)`.
- `TerminusModule` (`health.rs:136-148`) is a static module that provides `HealthCheckService` as a value.
- Dependencies (`crates/corvis/Cargo.toml`): `hyper` 1 with `server`, `http1`, `http2` only; no HTTP client, no TLS
  crate, no `libc`/`windows-sys`.
- Tests: the only health test is in `crates/corvis/tests/modules.rs:87-95` and `:162-165` (two custom indicators, one
  down → 503). Neither built-in indicator is tested.
- Callers: `examples/cats-app/src/app_controller.rs:25` uses `check_rss`; nobody uses `ping_check` (grep over the
  workspace and the three port apps in `corvis-tests/corvis/`, which do not use health checks at all).
- Docs: `docs/reliability/terminus.md` ("Built-in" table and "Limits") already documents the TCP-only ping, the
  Linux-only RSS and the missing timeout; `docs/deployment.md:106-135` shows `check_rss`.

## Goals / Non-Goals

**Goals:**
- A built-in indicator reports `up` only after it has really checked what its name says.
- A hung indicator cannot hang the health endpoint, and the response names it.
- `ping_check` and `check_rss` produce the same JSON as `@nestjs/terminus` for the same situation, verified with a
  differential fixture.

**Non-Goals:**
- New indicators (heap, disk, database), panic isolation, shutdown-aware health (see proposal, Out of scope).
- A general-purpose HTTP client in the public API of `corvis`.

## Decisions

1. **`ping_check` takes a URL and sends `GET` with hyper's client connection API.** Enable hyper's `client` feature and
   use `hyper::client::conn::http1::handshake` over a `tokio::net::TcpStream` (the same `hyper-util::rt::TokioIo`
   adapter the server already uses). No new crate for plain HTTP. `up` only for 2xx, matching axios'
   default `validateStatus` that `pingCheck` relies on. Down details: `message` always; `statusCode` and `statusText`
   when a response arrived. Redirects (3xx) are followed up to 5 times, as axios does by default; a redirect to a
   different scheme is followed only if that scheme is supported. Alternatives: `reqwest` (heavy, pulls its own TLS
   stack and a connection pool for a single request); keep TCP but rename only (does not fix the issue).
2. **HTTPS behind an optional feature `health-tls`** (`tokio-rustls` + `webpki-roots`), off by default so the default
   build gains no TLS dependency. Without it, an `https://` URL is reported `down` with a message naming the feature;
   it never downgrades to plain HTTP. Alternative: TLS on by default (see Open questions).
3. **The TCP check keeps existing as `tcp_check(key, "host:port")`.** It is useful for services without HTTP (Redis,
   SMTP) and is what users get today. `ping_check` with a value that has no scheme reports `down` with a message that
   names `tcp_check`, so a caller of the old API sees the problem on the first health request instead of a silent
   change of meaning. Alternative: accept `host:port` in `ping_check` and fall back to TCP; rejected, that is exactly
   the dishonest behavior the issue is about.
4. **RSS through the `memory-stats` crate.** It reads the RSS on Linux (`/proc/self/statm` with `sysconf` page size),
   Windows (`GetProcessMemoryInfo`) and macOS (`task_info`), with no further dependencies beyond `libc` /
   `windows-sys`. When it returns `None`, the indicator is `down` with `"message": "rss could not be measured on this
   platform"`. Alternative: hand-written per-OS code with `libc` and `windows-sys` in corvis (more `unsafe` to own for
   no gain).
5. **The RSS detail shape follows `@nestjs/terminus`.** The differential fixture decides whether the `rss` key stays in
   the details (today corvis adds it; see Open questions). The message `"Used rss exceeded the set threshold"` is
   already the original's.
6. **Time limit per indicator, applied by `HealthCheckService`.** `HealthIndicator` gains a `timeout: Duration` field
   (default `DEFAULT_INDICATOR_TIMEOUT = 5 s`) and a builder `timeout(self, Duration) -> Self`. `check` wraps each
   future in `tokio::time::timeout` and keeps `join_all`, so indicators stay concurrent. The timeout message reuses the
   wording `ping_check` already emits (`"timeout of <ms>ms exceeded"`, axios' wording). `ping_check`'s own fixed 3 s
   limit is removed: it uses the indicator limit like every other indicator. Alternatives: a single global timeout for
   the whole `check` (would not say which indicator hung); no default (keeps the hang the issue reports). 5 s is below
   common load-balancer health timeouts and above a normal database round trip.
7. **Differential fixture like `crates/corvis-orm/tests/decimal_js.rs`.** A Node script under
   `crates/corvis/tests/fixtures/terminus/` starts local HTTP servers (200, 500, 302→200, closed port) and records the
   `HealthCheckService.check` bodies of `@nestjs/terminus` (`pingCheck`, `checkRSS` with huge and tiny thresholds) into
   `terminus.json`; the Rust test replays the same scenarios and compares keys and values (except volatile text such
   as OS error messages, compared by presence). The fixture is committed, so `cargo test` needs no Node.

## Risks / Trade-offs

- [Behavior change for `ping_check("…", "host:port")`] → reported `down` with an explicit message; release notes and
  both docs pages say to switch to `tcp_check`.
- [5 s default can be too long for an orchestrator with a 1 s probe timeout] → per-indicator `.timeout()`; documented.
- [`memory-stats` is a small third-party crate] → license and advisories go through `cargo deny check` (`deny.toml`);
  the fallback (`down` with a message) keeps behavior honest if it ever returns `None`.
- [Reporting `down` on an unsupported platform restarts instances there] → only affects platforms outside Linux,
  Windows and macOS, where corvis is not tested anyway; the message makes the cause obvious.

## Open questions

- Should `health-tls` be enabled by default? Recommendation: no, keep the default build lean; revisit if users need
  `https` pings often.
- Keep the extra `rss` key in `check_rss` details (useful, but not in `@nestjs/terminus`) or drop it for parity?
  Recommendation: follow the fixture (drop it) unless a maintainer prefers the extra data; if kept, document it as a
  deliberate divergence.
- Should the default timeout be configurable per application (`TerminusModule::for_root(TerminusOptions)`)?
  Recommendation: not in this change; open an issue if someone asks for it.

## Review notes (2026-11-08)

- `health-tls` off by default, as recommended.
- The response follows the recorded `@nestjs/terminus` fixture exactly, so the extra `rss` field goes.
- A global timeout option in `TerminusModule::for_root` is out of scope.

## Implementation notes

- The original package `@nestjs/terminus` is not available locally, so tasks 5.1 and 5.2 (differential fixture and its
  replay test) are not done and stay unticked. `crates/corvis/tests/health.rs` covers the same scenarios (200, 500,
  302 to 200, closed port, RSS huge and tiny thresholds) and asserts the body shape of the original's documented
  behavior: `message`, `statusCode`, `statusText`, axios' wording `Request failed with status code N`, and `up`
  entries with no extra fields. The `rss` detail was dropped as the review notes decided, without the fixture
  confirming it.
- The per-indicator limit lives in the private `run_indicator` in `health.rs`, the single place an indicator runs, so
  the panic isolation of `e13-isolate-panicking-health-indicators` can add `catch_unwind` inside the timeout there.
- Task 3.3: the TLS tests are limited to the cases that need no certificate: feature disabled (down, names the
  feature, no byte reaches the server) and feature enabled against a plain-HTTP server (handshake fails, down). A
  successful HTTPS ping against a self-signed server is not tested. `tokio-rustls` uses the `ring` provider, so no
  C build tools are needed.
- `memory-stats` reports `physical_mem` as RSS on every supported OS; the Linux page size handling is inherited from
  that crate. `cargo deny check` passes.
- `tokio` with `test-util` was added to the dev-dependencies of `corvis` for the paused-clock tests.
- Task 7.5 (corvis-parity) is not run here: framework runtime code changed, so the parity check is still needed.

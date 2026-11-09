# Proposal

Implements [randygil/corvis#150](https://github.com/randygil/corvis/issues/150) (epic
[#1](https://github.com/randygil/corvis/issues/1)). Found by the #144 measurement
(`openspec/changes/archive/2026-11-08-e0-h2c-on-by-default/design.md`, "What h2c changes when on").

## Why

With `ServerOptions::h2c(true)` (opt-in since #144) and, later, HTTP/2 over TLS (#117), every HTTP/2 limit is
hyper's default: `serve` configures the HTTP/2 builder with a timer and nothing else
(`crates/corvis/src/server.rs:829`). So:

- one connection carries up to 200 concurrent streams, and `max_connections` bounds sockets, not requests;
- the header list limit is 16 KiB whatever `max_header_size` says (that option only reaches the HTTP/1 builder);
- `headers_timeout` and `keep_alive_timeout` apply to an HTTP/2 connection only while it has no request in flight,
  so one long stream exempts the whole connection, and a dead peer behind a long stream is never noticed;
- there is no HTTP/2 PING keep-alive.

An operator who sets `ServerOptions` to bound resources gets those bounds for HTTP/1 only.

## What Changes

- `max_header_size` also sets the HTTP/2 header list limit (`SETTINGS_MAX_HEADER_LIST_SIZE`, enforced on received
  headers). A request over it is answered `431`, as for HTTP/1. **Behavior change:** the HTTP/2 default goes from
  16 KiB to the documented 32 KiB.
- `ServerOptions::http2_max_concurrent_streams(n)`: streams per HTTP/2 connection, **default 100** (the lowest value
  RFC 9113 recommends; hyper's is 200). **Behavior change** for h2c users.
- `ServerOptions::http2_keep_alive_interval(Option<Duration>)` (default `None`, off) and
  `ServerOptions::http2_keep_alive_timeout(Duration)` (default 20 s): PING frames on an HTTP/2 connection, which is
  closed when a PING is not acknowledged in time. This is what notices a dead peer while a long stream (SSE) keeps the
  connection busy.
- `max_connections` keeps counting connections, like Node's `server.maxConnections`; the docs state that in-flight
  HTTP/2 requests are bounded by `max_connections × http2_max_concurrent_streams`.
- The timeouts are documented per protocol: for HTTP/2, `headers_timeout` and `keep_alive_timeout` apply to the
  connection while no stream is in flight, and `request_timeout` applies to each stream's body. A test proves the
  per-stream `request_timeout` over h2c.
- All HTTP/2 settings are applied in one function that builds the connection builder, so every listener that can
  speak HTTP/2 gets them: h2c today, TLS with ALPN `h2` when #117 lands.
- Tests over h2c for each limit; docs (EN + ES) on the HTTP adapters page (`http/performance`), `security/http-parsing`
  and `deployment`.

## Capabilities

### New Capabilities

- `http/http2`: the limits and timeouts of HTTP/2 connections and how `ServerOptions` sets them.

### Modified Capabilities

None. (`http/connection-close` keeps its requirement that h2c is opt-in; this change does not touch it.)

## Impact

- `crates/corvis/src/server.rs`: `ServerOptions` (three fields and setters, rustdoc of `max_header_size`, `h2c` and
  `max_connections`), a `connection_builder(&ServerOptions)` used by `serve`.
- Tests in `crates/corvis/tests/transport_security.rs` (next to `h2c_can_be_enabled`) or a new
  `crates/corvis/tests/http2_limits.rs`; `h2` as a dev-dependency if the tests read the server's SETTINGS (already in
  the dependency graph through hyper, `Cargo.lock`: h2 0.4.20).
- Docs: `docs/http/performance.md`, `docs/security/http-parsing.md`, `docs/deployment.md` and their copies under
  `docs/es/`.
- HTTP/1 clients see no change: the HTTP/2 builder is never used for them.
- Port apps: they serve HTTP/1 only (h2c is off by default), so apidiff should not move; the parity check confirms it.
- #117 (`e13-tls-termination-in-the-built-in-server`) should build its connections with the same builder function;
  noted there as a dependency, not implemented here.

## Out of scope

- A server-wide cap on in-flight requests across connections (`max_requests`): Node has no such option; possible new
  issue if HTTP/2 deployments need it.
- Per-stream header timeouts for HTTP/2: a header block must arrive as one contiguous HEADERS + CONTINUATION
  sequence, so a stalled block stalls only its own connection (design.md, Decision 5).
- Tuning flow-control windows, `max_send_buf_size`, frame size, rapid-reset limits
  (`max_pending_accept_reset_streams`, `max_local_error_reset_streams`): hyper's defaults stay; a write-progress
  timeout for peers that stop reading responses is a possible new issue for both protocols.
- HTTP/2 over TLS itself (#117) and WebSockets over HTTP/2 (extended CONNECT).

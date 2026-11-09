# Proposal

Implements [randygil/corvis#163](https://github.com/randygil/corvis/issues/163) (epic
[#1](https://github.com/randygil/corvis/issues/1)). Found by the security review of #157
(`e0-write-deadline-for-stalled-clients`); builds on its `server.rs` (branch `batch/phase07-o`, not on `main` yet).
Should land before #117 (TLS turns HTTP/2 on by default).

## Why

`send_timeout` (#157) closes connections that make no progress, but it does not bound the response memory a client
can hold while making minimal progress. Measured against #157 (issue body and its comment; proof of concept
`st157b/src/main.rs`, scenarios `mixed`, `rstcycle`, `trickle`):

- **Mixed:** window 0 on 100 streams of a 1 MiB route; only stream 1 gets 16 KiB of WINDOW_UPDATE per period. The
  connection counts as progressing, so the 99 stalled streams stay: about 103 MiB held for about 0.3 KB/s.
- **RST cycle:** window 0, 100 streams of the 1 MiB route, every 55 s reset them and open 100 new ones: 114 MiB held,
  9.2 KB sent, 0 bytes read. The rapid-reset counter of #150 does not fire: the handlers finished, so those streams
  count as *completed* and even earn credit against the reset limit.
- **Trickle:** 16 KiB per stream per `send_timeout` keeps every stream alive (about 27 KB/s for 100 MiB).
- **Zero-copy tail:** when an HTTP/2 body ends, its last 16 KiB slice can stay in h2's send buffer; it is a slice of
  the whole response allocation, so the full response stays alive while a 1-byte window update per keep-alive
  period keeps the stream open.
- **HTTP/1:** about 64 KiB per `send_timeout` keeps one response alive.

The total is `connections × http2_max_concurrent_streams × response size`, with no default bound (`max_connections`
is `None`). The issue's comment sets the goal: bound `max_concurrent_streams × response size` per connection
regardless of what the client reads.

## What Changes

- **New option `ServerOptions::max_pending_response_bytes(impl Into<Option<usize>>)`, default 8 MiB per
  connection, `None` disables it.** Pending response bytes are bytes of responses the handlers produced and the
  connection has not finished sending: the whole size of an in-memory body (also when compression wraps it) until
  the body ends or is dropped, and the unsent part of the frame a streamed body is holding.
- **Admission before the handler.** Each request reserves an equal share of the cap (the cap divided by the
  requests the connection may run at once: `http2_max_concurrent_streams` over HTTP/2, 1 over HTTP/1) and its
  handler starts only when pending bytes plus reservations leave room for it, or when the connection holds nothing.
  Otherwise the request waits without reaching a handler; a client reset while it waits counts as a cancelled
  request (#150).
- **Overshoot is dropped, not held.** Over HTTP/2, when a handler returns a response that would take a connection
  that already holds pending bytes above the cap, the response is dropped at once and its stream is reset with
  `ENHANCE_YOUR_CALM`; it counts as a cancelled request and the first one per connection is logged. A single
  response is always sent, whatever its size. HTTP/1 never reaches this path (one handler at a time).
- **Streams reset after their handler finished count as cancelled.** A request now counts as completed for the
  rapid-reset rule of #150 when its response body *ends*; a body dropped before its end (client `RST_STREAM`, or the
  overshoot above) counts as cancelled.
- **No zero-copy tail.** HTTP/2 slices that may outlive their body (the last `max_send_buf_size + 16 KiB` of a data
  frame cut by `GuardedBody`) are copied into their own allocation, and hyper's per-stream HTTP/2 send buffer is
  lowered from 400 KiB to 32 KiB, so an ended body leaves at most about 64 KiB of its own per open stream.
- **Security round (2026-11-09):** new option `ServerOptions::max_pending_response_wait` (default 5 s, `None` waits
  without limit): a request that waits longer for room is refused (`REFUSED_STREAM` over HTTP/2, not counted as
  cancelled; the connection is closed over HTTP/1). Over HTTP/1 the charge of a body hyper took ends when hyper flushes
  the socket. A response with data is not "ended" for the rapid-reset rule until its body says so. The copied tail
  covers 64 KiB because h2's codec keeps one frame outside its buffer.
- Regression tests reproducing `mixed`, `rstcycle`, `trickle` and the zero-copy tail, plus tests that legitimate
  clients are not reset; docs (EN + ES).

## Capabilities

### New Capabilities

- `http/response-memory`: how much response memory one connection may hold for a client that does not read it, the
  admission of handlers, what happens above the cap, and what memory the built-in server keeps after a body ends.

### Modified Capabilities

- `http/http2`: "Concurrent streams per connection are capped" — a request whose response body is reset or dropped
  before its end counts as cancelled; it counts as completed when its body ends.

## Impact

- `crates/corvis/src/server.rs`: `ServerOptions` (field, setter, rustdoc, default; `send_timeout` rustdoc points to
  the new option instead of #163); `ConnState` (response budget, its `Notify`, the once-per-connection warning);
  `GuardedBody` (charge, outcome at end or drop, tail copy); `CancelWatch`/`Outcome` (hand-off to the body);
  `finish_response` (records the plain body length before compression); `connection_builder`
  (`max_send_buf_size`); the `service_fn` closure in `serve` (admission, overshoot reset).
- `crates/corvis/Cargo.toml`: `h2 = "0.4"` as a direct dependency (already in `Cargo.lock` as 0.4.20 through hyper)
  to build the `ENHANCE_YOUR_CALM` reset error.
- Tests: new `crates/corvis/tests/response_memory.rs`; unit tests in `server.rs`.
- Docs: `docs/http/performance.md`, `docs/security/http-parsing.md`, `docs/deployment.md` and their `docs/es/` copies.
- **Behavior changes:** HTTP/1 pipelined requests no longer run their handler while the previous response is still
  being written; HTTP/2 bursts of large in-memory responses beyond 8 MiB per connection get some streams reset. Port
  apps answer small JSON over HTTP/1, one request at a time, so apidiff should not move; the parity check confirms it.

## Out of scope

- A server-wide (global) cap on pending response bytes: it trades memory exhaustion for one client blocking the
  handlers of all; the global bound stays `max_connections × max_pending_response_bytes` (possible new issue).
- A default for `max_connections` (Node's `maxConnections` is unlimited too): possible new issue.
- Memory the application keeps outside the response body (an unbounded channel feeding an SSE stream, a shared cache
  sliced into responses): the application's responsibility; docs mention it.
- Request-side memory (HTTP/2 receive windows, request bodies): bounded by hyper's 1 MiB connection window and the
  body limits.
- A minimum transfer rate: still out of scope, as in #157; the cap makes the memory a trickling client holds
  constant instead.
- Write deadline for WebSocket connections: #161 (`e13-websocket-write-deadline`).

# Proposal

Implements [randygil/corvis#157](https://github.com/randygil/corvis/issues/157) (epic
[#1](https://github.com/randygil/corvis/issues/1)). Found by the security review of #150
(`e0-http2-limits-follow-server-options`); both problems predate it.

## Why

The built-in server has no limit on how long it waits for a client to accept response bytes:

- `TimedIo::poll_idle` (`crates/corvis/src/server.rs:617-648`) is only evaluated from `poll_read`, and it does
  nothing while a request is in flight (`:622-629`). `poll_write` (`:772-785`) has no deadline.
- `GuardedBody` (`:438-456`) keeps `ConnState::inflight > 0` until hyper drops the response body, so an unfinished
  response disables every connection timer. `request_timeout` covers only reading the request body (`:274`, `:293-296`).

So a client that stops reading keeps its connection, its `max_connections` slot and the memory of its responses
forever. Two proofs of concept (security review of #150) show it:

1. **HTTP/2 with a zero window.** Preface, `SETTINGS_INITIAL_WINDOW_SIZE=0`, `HEADERS GET /route` (~70 bytes). With
   `max_connections(1)` a second client was not served in 25 s; the PING keep-alive of #150 does not help because the
   client answers PINGs. 3 connections × 100 streams of a 1 MiB route took the server from 7 to 314 MiB with ~5 KB sent
   by the client.
2. **Writes blocked.** ~500 KB of PING frames that the client never reads: h2 stops reading until it can write the
   PONGs (`h2-0.4.20/src/proto/connection.rs:385`, `codec/framed_write.rs:110-120`), so `poll_read`, and with it every
   connection timer, is never called again; hyper's PING timeout needs to write a GOAWAY, which also blocks. With
   `max_connections(1)` a second client was not served in 120 s even with keep-alive 1 s / 1 s. HTTP/1 has the same
   variant with ~480 KB of pipelined requests that are never read.

The first problem becomes high once HTTP/2 is reachable without opting in (TLS with ALPN `h2` by default, #117).

## What Changes

- **New option `ServerOptions::send_timeout(impl Into<Option<Duration>>)`, default 60 s.** The longest the server
  waits for a client to accept response bytes. It measures lack of progress, not total time: every byte the client
  accepts restarts it, and it never runs while the server has nothing to send (slow handlers, idle SSE streams, idle
  keep-alive connections). `None` disables it. Named and defaulted after nginx's `send_timeout`; Node's equivalent,
  `server.timeout`, is off by default, so this is a documented divergence (security over parity).
- **Write deadline in `TimedIo`.** A `poll_write`, `poll_write_vectored` or `poll_flush` that stays pending for
  `send_timeout` without the socket accepting a byte fails with `TimedOut`. The connection ends and the socket is closed
  without the linger drain and without trying to flush a GOAWAY or the rest of the response. Fixes problem 2 for HTTP/1
  and HTTP/2.
- **Per-stream progress watchdog.** Each response body records when it is waiting for hyper to take its next frame
  (it produced a frame, or has not been polled yet) as opposed to waiting for the application (its last poll returned
  `Pending`). A watchdog in the connection task closes the connection, without linger, when any body has waited for
  hyper for `send_timeout`. Fixes problem 1: a stream whose flow-control window the client never opens is detected even
  while the connection keeps exchanging PING, SETTINGS or WINDOW_UPDATE frames.
- **HTTP/2 responses are handed to hyper in slices of at most 16 KiB**, so a stream body cannot finish (and leave the
  watchdog's sight) while megabytes of it sit in h2's send buffer behind a window of a few bytes.
- WebSocket and other upgraded connections are not affected (their send queue is bounded by `max_buffered_amount`).
- Regression tests for both problems over HTTP/1 and h2c, plus tests that a slow-but-reading client, an idle SSE
  stream and a slow handler are not cut off. Docs (EN + ES) of the timeouts.

## Capabilities

### New Capabilities

- `http/send-timeout`: how long the server waits for a client to accept response bytes, over HTTP/1 and HTTP/2, and
  how a stalled connection is closed.

### Modified Capabilities

None. (`http/connection-close` keeps its requirements on the closing drain; a connection closed by this timeout is
closed without it, like the other punitive closes that spec lists.)

## Impact

- `crates/corvis/src/server.rs`: `ServerOptions` (field, setter, rustdoc, default); `TimedIo` (write deadline, a second
  `Sleep`); `ConnState` (registry of response-body clocks, `Notify`); `GuardedBody` (clock updates, HTTP/2 slicing);
  `finish_connection` (watchdog alongside the connection future); the `service_fn` closure in `serve` (passes the
  request version to `GuardedBody`).
- Builds on #150 (`e0-http2-limits-follow-server-options`, branch `worktree-agent-a5dad9b32ed26a489`), which moves the
  builder into `connection_builder` and adds `crates/corvis/tests/http2_limits.rs` with a raw-frame `RawClient`; this
  change should be implemented after #150 is merged and reuse that helper.
- #117 (TLS): `TimedIo` wraps the decrypted stream (`e13-tls-termination-in-the-built-in-server`, decision 4), so the
  write deadline covers TLS connections, including a TLS `poll_flush` that pends.
- Tests: new `crates/corvis/tests/send_timeout.rs`.
- Docs: `docs/http/performance.md` ("Timeouts and connections"), `docs/security/http-parsing.md` (slow clients),
  `docs/deployment.md` (server options); copies under `docs/es/`.
- **Behavior change:** a client that accepts no response byte for 60 s is disconnected. Port apps answer small JSON
  responses to a local client, so apidiff should not move; the parity check confirms it.

## Out of scope

- A minimum transfer rate (a client that accepts one byte per `send_timeout` keeps its connection, as with nginx's
  `send_timeout` and Node's `server.timeout`): possible new issue.
- Bounding the response bytes buffered per connection or server-wide while the timeout runs (bounded today by
  `max_connections × http2_max_concurrent_streams × response size` for up to `send_timeout`): possible new issue.
- Resetting only the stalled HTTP/2 stream (RST_STREAM) instead of closing the connection: hyper exposes no handle to a
  stream from outside its body (design.md, Decision 5).
- A write-progress timeout for WebSocket connections (their memory is bounded by `max_buffered_amount`; a dead peer is
  found by a gateway heartbeat): possible new issue.
- Shortening the timeout during graceful shutdown; the existing 10 s drain cap in `serve` is unchanged.

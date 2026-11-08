# Proposal

Implements [randygil/corvis#130](https://github.com/randygil/corvis/issues/130) (epic [#1](https://github.com/randygil/corvis/issues/1)).

## Why

`crates/corvis/tests/transport_security.rs` fails about 1 run in 10 on Windows (`oversized_head_is_rejected`,
`malformed_chunked_body_is_not_reported_as_too_large`), which would make the Windows job of CI (#6) flaky. The failure
is not in the tests: the client reads `ConnectionAborted` (os error 10053) after 0 bytes instead of the `431`. The
server answers a request it has not finished reading and then drops the socket with input still unread; the operating
system turns that close into a TCP reset, and the reset discards the response before the client reads it. Real clients
(browsers, proxies, `curl`) hit the same race, so the server is losing error responses, not just the tests.

## What Changes

- When the server closes a connection it accepted, it first sends FIN and then reads and discards what the peer still
  sends, bounded to 1 MiB and 2 seconds, before releasing the socket (what Node does after an early error response).
- Connections the server closes to punish the peer (headers timeout, keep-alive timeout, refused h2c) and sockets destroyed by an upgrade listener skip the drain.
- The connection permit of `max_connections` is held until the socket is really closed.
- Regression tests: a request whose oversized head is followed by more input still gets its `431`, and the drain
  is bounded for a peer that never stops sending.

## Capabilities

### New Capabilities

- `http/connection-close`: error responses sent before the request was fully read reach the client, and the
  closing drain is bounded.

### Modified Capabilities

None.

## Impact

- `crates/corvis/src/server.rs` (`TimedIo` owns the socket and closes it gracefully on drop).
- `crates/corvis/tests/transport_security.rs` (two new tests).
- Runtime code of the framework changes: the corvis-parity check is needed (the maintainer runs it).

## Out of scope

- Changing the limits, timeouts or status codes of the transport layer.
- Making the tests tolerant of a reset: the response must not be lost, so the existing assertions stay as they are.
- A configurable drain size or time (constants for now; see design Open questions).

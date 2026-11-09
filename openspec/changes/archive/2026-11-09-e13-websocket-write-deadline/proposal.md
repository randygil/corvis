# Proposal

Implements [randygil/corvis#161](https://github.com/randygil/corvis/issues/161) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Depends on #157 (`e0-write-deadline-for-stalled-clients`,
branch `batch/phase07-o`, not on `main` yet), whose write deadline this change extends.

## Why

#157 added `ServerOptions::send_timeout` (60 s): a connection whose socket accepts no response byte for that long is
dropped. Established WebSocket connections are exempt: `TimedIo::poll_write_deadline` returns `Pending` as soon as
the connection is upgraded (`crates/corvis/src/server.rs:1128-1132`), and the test
`tests/send_timeout_ws.rs` asserts that a WebSocket client that stops reading keeps its connection. Their memory is
bounded by `max_buffered_amount` (8 MiB per connection), but a client that stops reading still keeps its TCP
connection, its `max_connections` slot, its `WsClient` in `server.clients()` and up to 8 MiB of queued frames
forever, as long as the application sends less than the limit. In the plain `ws` runtime (`crates/corvis/src/ws.rs`)
it is worse: when the writer task ends, the connection loop does not notice (`run`, `:946-1013`, has no arm for the
writer), so the socket stays half alive until the peer closes or a later `send` overflows the queue.

## What Changes

- **The write deadline of `send_timeout` also applies to upgraded connections** (WebSocket gateways, the plain
  `WebSocketServer`, any `upgrade` listener). A write that stays pending for `send_timeout` without the socket
  accepting a byte fails; the connection is dropped. It measures lack of progress at the socket: every byte the
  client accepts (data, pings, pongs, close frames) restarts it, received frames never do, and it never runs while
  the server has nothing to send, so idle WebSockets are untouched. `send_timeout(None)` turns it off. The other
  #157 mechanisms (stalled-body watchdog, idle timers) stay off for upgraded connections.
- **No close frame; the connection is destroyed.** The socket accepts nothing, so no closing handshake is possible:
  the TCP connection is closed at once, like `ws.terminate()`. Locally the close is reported as `1006`
  (`WebSocket::on_close`, `closed()`); `handleDisconnect` runs and the client leaves `server.clients()`. The reason is
  logged once as a warning.
- **Both WebSocket runtimes end the connection when their writer fails.** `ws.rs::run` gets the writer arm the
  gateway loop already has; both loops end at once on a write error even while a closing handshake was waiting.
- Pending `send_and_wait` calls fail with the write error.
- Regression tests with a client that never reads (gateway and plain `ws`), a client that sends pings but never
  reads, an idle connection and a slow reader; the #157 test that asserted the exemption is inverted. Docs (EN + ES).

## Capabilities

### New Capabilities

- `ws/write-deadline`: how a WebSocket connection whose peer stops accepting frames is detected and closed, what the
  application observes, and how pings and pongs count.

### Modified Capabilities

- `http/send-timeout` (added by #157, archived before this change): "Upgraded connections are not affected" is
  replaced by "Upgraded connections keep only the write deadline".

## Impact

- `crates/corvis/src/server.rs`: `TimedIo::poll_write_deadline` (drop the upgraded exemption); rustdoc of
  `send_timeout` and `max_connections`.
- `crates/corvis/src/ws.rs`: `write_loop` reports why it ended; `run` watches the writer; log line.
- `crates/corvis/src/websockets.rs`: `run_connection` ends on a writer failure even while closing; log line.
- Tests: `crates/corvis/tests/send_timeout_ws.rs` (rewritten), new cases for the plain `ws` runtime there.
- Docs: `docs/websockets/gateways.md`, `docs/websockets/adapter.md`, `docs/http/performance.md`, and their
  `docs/es/` copies.
- **Behavior change:** a WebSocket client whose socket accepts nothing for 60 s while the server has frames for it
  is disconnected. Node's `ws` never does this (it disables the socket timeout on upgrade); nginx applies its
  `send_timeout` to upgraded connections the same way. The port apps do not use WebSockets in their apidiff suites;
  the parity check confirms nothing moves.

## Out of scope

- A per-gateway or per-`WebSocketServer` override of the timeout: the server-wide `send_timeout` covers it; possible
  new issue if someone needs a different value for WebSockets.
- A built-in heartbeat for gateways (detects dead peers on idle connections, where nothing is written): possible new
  issue.
- A minimum transfer rate for WebSockets (a client that accepts a few bytes per period keeps its connection; its
  memory stays bounded by `max_buffered_amount`).
- Response memory of HTTP connections: #163 (`e0-cap-pending-response-bytes`).

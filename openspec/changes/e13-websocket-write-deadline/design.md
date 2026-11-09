# Design

## Context

See proposal.md for motivation; the behavior is in the spec deltas. Current state verified on branch `batch/phase07-o`
(merge `da5f508`: #157, #158 and #115 on top of `main` `a353e1c`); paths under `crates/corvis/` unless stated.
Locked dependencies: hyper 1.12.0, tokio-tungstenite 0.24.0, tungstenite 0.24.0.

**The write deadline of #157 and the exemption**

- `TimedIo` wraps every accepted socket (`src/server.rs:870-903`, created in `serve`, `:1446`) and hyper serves it
  with `serve_connection_with_upgrades(TokioIo::new(io), ...)` (`:1499`). After a `101`, hyper hands that same IO to
  the upgrade (`hyper::upgrade::Upgraded`), so every byte a WebSocket writes still goes through
  `TimedIo::poll_write` / `poll_write_vectored` / `poll_flush` (`:1330-1382`). With TLS (#117) `TimedIo` wraps the
  decrypted stream, so `wss` is covered the same way.
- `poll_write_deadline` (`:1128-1146`) arms a `Sleep` at the first pending write and fails the write with `TimedOut`
  (`no_linger` set) when the socket accepted nothing for `send_timeout`; `write_progress` (`:1149-1174`) clears the
  stall on every accepted byte. It returns `Pending` without a deadline when `ConnState::upgraded` is set
  (`:1130-1132`). The flag is set by the service closure when the response is a `101` (`:1491-1493`).
- The other #157 mechanisms skip upgraded connections too: the watchdog (`:970-972`) and `poll_idle` (`:1178-1180`);
  `finish_connection` closes an upgraded socket without the linger drain (`:1040-1043`).
- `ServerOptions::send_timeout` rustdoc says it does not apply to WebSockets (`:113-115`); `max_connections` advises
  a gateway heartbeat for dead clients (`:136-139`). `tests/send_timeout_ws.rs` asserts the exemption: a client that
  does not read for three timeouts keeps its connection and receives all 24 messages of 128 KiB.

**WebSocket runtimes**

- Gateways (`src/websockets.rs`): `upgrade` (`:716-760`) builds the `WebSocketStream` from the upgraded IO (`:746`)
  and runs `run_connection` (`:966-1070`). Outgoing frames go through `OutQueue` (`src/ws.rs:80-180`, bounded by
  `GatewayOptions::max_buffered_amount`, default 8 MiB, `websockets.rs:369-371`; a frame is always accepted when
  nothing is pending, `ws.rs:101-118`) and a writer task, `write_loop` (`ws.rs:182-199`), which stops on the first
  write error. `run_connection` ends when the writer stops unless the client is closing (`websockets.rs:1017-1023`);
  while closing it waits for `close_timeout` (30 s, `:1011-1014`). Exceeding `max_buffered_amount` discards the queue
  and closes with `1008` (`send_text`, `:266-276`). Pings are answered by tungstenite (`:1026`).
- Plain `WebSocketServer` (`src/ws.rs`): `handle_upgrade` (`:815-912`) runs `run` (`:946-1013`) with the same
  `OutQueue` and `write_loop`. `run` has no arm for the writer: when the writer stops, the loop keeps reading until
  the peer closes, a `send` overflows the queue (then `1008`, `:488-502`) and `close_timeout` passes, or a read fails.
  When no close frame was received the close is reported as `1006` (`:1011-1012`, also in `Cleanup`, `:915-934`), the
  code `terminate()` reports (`:565-573`).
- Close codes in use: `CLOSE_POLICY_VIOLATION` = 1008 for a peer that does not read fast enough (`ws.rs:53-54`),
  `CLOSE_INTERNAL_ERROR` = 1011 for a panicking listener or hook (`:55-56`).
- tungstenite answers pings while reading: `WebSocketContext::read` flushes the pending pong first and, on
  `WouldBlock`, keeps reading; any other write error ends the read with that error
  (`tungstenite-0.24.0/src/protocol/mod.rs:387-404`). tokio-tungstenite wakes both the reading and the writing task
  on IO readiness (`tokio-tungstenite-0.24.0/src/compat.rs:13-73`, `WakerProxy`).

**References**

- `ws` 8.22.0 (the version under the original WebSocket adapter in the scratchpad; the adapter pins 8.21.2):
  `setSocket` disables the socket timeout of the upgraded connection, `socket.setTimeout(0)`
  (`lib/websocket.js:260`); the only timer is `closeTimeout` (30 s) for the closing handshake (`:1324-1326`). So `ws`
  never closes a peer that stops reading; `bufferedAmount` grows without limit.
- The original adapter (`@nestjs/platform-ws` 11.2.9, `adapters/ws-adapter.js`): pauses reading from a client while
  replies above 1 MiB are pending (`SEND_BUFFER_HIGH_WATER_MARK`, `:25`, `:108-124`) and `terminate()`s a client with
  more than `maxSendBufferSize` queued (default 128 MiB, `:26`, `:56-57`, `ensureSendBufferWithinLimit`, `:187-205`):
  no close frame, so its close event carries `1006`. It has no time-based limit either.
- nginx: `ngx_http_upstream_process_upgraded` (`src/http/ngx_http_upstream.c:3713`) arms `send_timeout` on the
  client side of an upgraded (WebSocket) connection whenever a write to the client is blocked (`:3896-3897`), the
  same semantics as #157's write deadline.
- RFC 6455: §7.4.1 defines 1008 ("an endpoint is terminating the connection because it has received a message that
  violates its policy", used generically when no better code fits) and 1011 ("a server is terminating the connection
  because it encountered an unexpected condition that prevented it from fulfilling the request"); §7.1.5 and §7.4.1:
  1006 is never sent, it reports a connection closed without a close frame.

## Goals / Non-Goals

**Goals:**
- A WebSocket peer whose socket accepts nothing for a configurable time while the server has frames for it loses its
  connection, in both runtimes, on by default.
- Idle connections and slow but live readers are unaffected; pings and pongs cannot be used to look alive.
- The application observes the closure through its usual hooks.

**Non-Goals:**
- A per-gateway override, a built-in heartbeat, a minimum rate (proposal, Out of scope).

## Decisions

1. **Reuse `ServerOptions::send_timeout`, applied at the socket.** The upgraded exemption is removed from
   `poll_write_deadline` (`:1130-1132`); nothing else in `TimedIo` changes. The question it answers is the same as for
   HTTP ("how long may a client refuse bytes?"), the measure is the same (no byte accepted by the socket), and it is
   already on every upgraded connection's IO, including TLS. nginx applies its `send_timeout` to upgraded connections
   (References), which is the semantics #157 copied.
   Alternatives considered:
   - *A gateway option (`GatewayOptions::send_timeout`, `WebSocketServerOptions::send_timeout`).* Rejected for now:
     it needs a handle from the request extensions into `ConnState` for every runtime, including raw `upgrade`
     listeners, for a value nobody has asked to differ yet; a later override can be added on top (Out of scope).
   - *A timeout around `sink.send` in `write_loop`.* Rejected: it measures time per frame, not progress, so an 8 MiB
     message to a reader on a slow link would be cut while it moves, and it would not see pongs written by the read
     half.
   - *Keep the exemption and rely on `max_buffered_amount`.* Rejected: an application that sends less than the
     limit leaves the connection, its slot and up to 8 MiB alive forever; security over parity with `ws`.
2. **Destroy the connection; report `1006`.** When the deadline fires the socket accepts nothing, so a close frame
   cannot be delivered: the failed write ends the writer, the runtime ends the connection and drops the socket (no
   linger, `no_linger` is already set by `poll_write_deadline`), like `ws.terminate()` and like the original adapter
   for `maxSendBufferSize`. Locally the closure is reported with `1006`, as RFC 6455 prescribes for a connection
   closed without a closing handshake and as `ws.rs` already does for lost connections. If a code had to be chosen
   for the cause, it would be `1008`: the peer violates the server's policy of reading what it is sent, the code
   Corvis already uses for the same cause when the queue overflows; `1011` reports a failure of the server itself and
   would put the blame on the wrong side. The warning names the cause (`send_timeout`) instead.
   Alternatives considered: queue a `1008` close frame and wait for it (it can never leave a blocked socket; it would
   only add `close_timeout`); report `1008` locally without sending it (a code that was never exchanged, unlike every
   other `1008` the application sees).
3. **Progress = bytes the socket accepts, of any frame; received frames never count.** It falls out of Decision 1:
   `write_progress` runs for every byte written, whether data, ping, pong or close; reading does not touch the write
   stall. TCP gives a peer no way to accept the server's pongs while refusing its data, so a peer that pings without
   reading blocks on the pongs and is cut like any other (tungstenite flushes pongs on the read path and returns the
   `TimedOut` error from there too, Context). The deadline is armed only while a write is pending, so a connection on
   which the server has nothing to send never times out: dead peers on idle connections still need a heartbeat, as
   the `max_connections` rustdoc says.
4. **Both runtimes end on a writer failure.** `write_loop` returns why it stopped (`Closed` after writing a close
   frame or on shutdown, `Failed(io::ErrorKind)` on a write error). `websockets.rs::run_connection` breaks at once on
   `Failed`, also while closing (today it waits for `close_timeout` when closing); `ws.rs::run` gains the same writer
   arm and breaks on `Failed`. When the kind is `TimedOut`, each logs one warning: `WebSocketGateway` "client {id}:
   accepted no data for {send_timeout:?}, connection dropped" and `WebSocket` "socket {id}: ...". Pending
   `send_and_wait` acks already receive the write error (`write_loop`) or fail on `shutdown`.
5. **The other #157 mechanisms stay off for upgraded connections.** The watchdog watches HTTP response bodies, which
   an upgraded connection no longer has, and the idle timers would cut quiet WebSockets. `finish_connection` keeps
   skipping the linger drain for upgraded sockets.
6. **Spec placement.** A new capability `ws/write-deadline` for what the WebSocket runtimes do; in `http/send-timeout`
   (added by #157) "Upgraded connections are not affected" is removed and "Upgraded connections keep only the write
   deadline" added. #157 must be archived first so that the removal applies.
7. **Tests.** `tests/send_timeout_ws.rs` is rewritten (feature `websockets`): the existing gateway (`/feed`, 24 × 128
   KiB) now expects the connection to end, a twin with `send_timeout(None)` keeps the old assertion, and new cases
   cover the plain `WebSocketServer` behind an `upgrade` listener (`on_close` 1006, `send_and_wait` fails), the
   `max_connections(1)` slot, a pinging client that never reads, an idle connection and a slow reader. Clients use a
   4 KiB receive buffer, as the current test does, and `send_timeout(1 s)` with windows of several seconds, the
   practice of `tests/send_timeout.rs`.

## Risks / Trade-offs

- [Legitimate clients that stop reading for 60 s lose the connection] → a suspended mobile app or a frozen tab whose
  socket buffer filled while the server kept sending. They reconnect on resume (socket.io-style clients do already;
  its own heartbeat drops them after about 45 s); `send_timeout(None)` or a larger value restores the old behavior.
  Documented.
- [One knob for HTTP and WebSockets] → an application that disables `send_timeout` for long downloads also disables
  it for WebSockets; acceptable until someone needs different values (Out of scope).
- [Raw `upgrade` listeners that implement other protocols get the deadline too] → the same reasoning applies to any
  upgraded protocol; documented in the `send_timeout` rustdoc.
- [A peer that accepts a few bytes per period keeps its connection] → memory stays bounded by `max_buffered_amount`;
  a minimum rate is out of scope.
- [Timing tests flaky on Windows] → 1 s timeout, small receive buffers, assertions within several seconds.

## Open questions

None that needs a maintainer: reuse of `send_timeout`, destroying the connection with a local `1006` (and why not
`1008`/`1011` on the wire), and how pings and pongs count were settled above in favor of security, then parity.

## Review notes (2026-11-09)

- Approved as drafted: reuse `send_timeout` for upgraded connections, destroy the connection without a close
  frame (local `1006`), and count only bytes the socket accepts as progress.
- Archive after `e0-write-deadline-for-stalled-clients` (#157), since it modifies `http/send-timeout`.

## Review notes, security round (2026-11-09)

1. **Panic in `ws.rs::run` (high).** The writer arm consumed the `JoinHandle` on `WriteEnd::Closed`, and the
   protocol-error branch awaited it again. Fixed with the same `if !writer_done` guard the gateway loop has. Regression
   test `an_invalid_frame_during_the_closing_handshake_closes_the_plain_socket_once`.
2. **Pongs bypass `max_buffered_amount` (high).** Decision: read backpressure, not a cap on `max_write_buffer_size`
   (which would fail large data messages). `TimedIo::read_paused`: on an upgraded connection, while the last write is
   blocked, `poll_read` returns `Pending` instead of reading. It probes `poll_write_ready` of the socket, because the
   blocked write may have been issued by the read task itself (a pong flushed on the read path), in which case nobody
   else would retry it; when the socket is writable again it clears the flag and wakes the writer task. A write that
   makes progress wakes the parked reader (`write_progress`). The send timeout is still polled from the parked read,
   so a stall ends the connection. Pending pongs are bounded by one read chunk plus what the kernel buffers hold.
   Tests: `a_ping_flood_without_reading_is_bounded_on_a_gateway` and `..._on_a_plain_websocket` (a raw client sends
   masked pings until its socket stops accepting bytes; with the pause the server takes far less than 24 MiB; with the
   pause disabled the same tests failed with 37-47 MiB accepted). The false comment about "one pending pong at most"
   in `websockets.rs` was replaced.
3. **`Failed(Other)` ended the plain connection (low).** `write_loop` now reports `WriteEnd::Refused` for errors that
   are not socket errors (for example a send after the closing handshake began). `ws.rs::run` ends at once only on
   socket errors (`Failed`); on `Refused` it behaves as on `Closed` (keeps reading until the peer closes or
   `close_timeout`), so a pending close reply is flushed. The gateway loop only breaks on `Failed`, as before.

Reviewer PoCs, run from the review folder against this branch (release build):
- `panic`: no panic; the server logs `on_close(1006)` once and `on_close called: true`.
- `e13poc 15 0 60` (ping flood, no reading, send_timeout 60 s): no ping batch completed, server heap delta 0 MiB
  throughout (was +254 MiB in 15 s).
- `e13poc 13 1 10` (trickle reader, send_timeout 10 s): 2 MiB sent in 13 s, heap delta 0 MiB (was +508 MiB).

## Implementation notes

- `write_loop` returns `WriteEnd::{Closed, Failed(io::ErrorKind)}`; the kind is taken from `tungstenite::Error::Io`, any
  other write error is `Other`.
- `ws.rs::run`: on `Closed` the loop ends only if a close frame was already received; otherwise it keeps waiting for the
  peer's reply until `close_timeout`.
- Tests: one test per spec scenario in `tests/send_timeout_ws.rs`, serialised with a static mutex because they share
  counters. The closing-handshake test probes the connection by reading after four seconds instead of timing the exact
  moment of closure. The stall tests were not run against the unchanged code to see them fail first.
- `cargo clippy -p corvis --no-default-features --all-targets` fails to compile several existing test files (they need
  the default features); this predates the change, so `--lib --bins` was checked instead.

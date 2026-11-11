# Proposal

Implements [randygil/corvis#185](https://github.com/randygil/corvis/issues/185) (epic
[#30](https://github.com/randygil/corvis/issues/30)) and absorbs
[#183](https://github.com/randygil/corvis/issues/183) and [#184](https://github.com/randygil/corvis/issues/184), which
change the same lines (see "Absorbed issues"). Source: an engineering-quality review (not a security review) of the
Socket.IO / Engine.IO server after #32, #33 and #34, read against `dev` at `db5b4f6`. It lands before the Redis adapter
(#36, `openspec/changes/e2-socket-io-redis-adapter/`), which publishes the `Adapter` trait as this change leaves it, and
after the gateway adapter (#35, `openspec/changes/e2-socket-io-gateway-adapter/`), which is being finished on a branch
and touches `socket_io/client.rs` and `socket_io/gateway.rs`: task 1.1 re-checks every line cited here against `dev` once
#35 is merged.

## Why

The review found no new bug or vulnerability. It found cost on the hottest paths and API surface that should not be
published with the adapter trait:

- Every received message (the connection loop, every WebSocket frame, every polling `POST`) builds a `Vec`, up to three
  boxed `Notified` and a `select_all` to wait for a release of the unread budget, whether or not it will wait; every
  release calls `notify_waiters()` at each level of the gauge chain, which takes the notifier's mutex even with nobody
  waiting, so the server-wide gauge serializes every connection.
- The handshake `Request` is deep-copied for every socket and for every polling request, outside every budget, while two
  comments say the opposite.
- An emit copies its payload about four times; a broadcast copies the shared encoding once per WebSocket recipient when
  it writes.
- The connect path holds the connection's namespace-table mutex while it queues CONNECT and calls the adapter: an adapter
  that disconnects the socket from `add_all` deadlocks the connection. The same lines are why a client can know its socket
  id before `io.of(..).socket(id)` finds it (#183), and the CONNECT written back to back with the first emit of a
  connection handler is why the upstream conformance suite fails 2 to 3 runs in 8 (#184).
- The answer budget of broadcast acknowledgements keys a client by a different address than the session does, so a
  malformed forwarded address gets a fresh share.
- Test hooks sit on the public trait and types; seven public enums cannot grow without a breaking change; several tests
  synchronize with `sleep` or bound wall-clock time.

There is no Socket.IO benchmark yet, so the first task builds one and every performance decision names the number it must
move.

## What Changes

- **Benchmark.** `bench/socket-io/`: a release-mode benchmark (server and load generator in separate processes, on
  `127.0.0.1`) for `emit`, `server-emit` and `broadcast` over WebSocket and polling, N connections by M messages, reporting
  messages per second, latency percentiles and the server's allocations and bytes allocated per message. Run before and
  after; the numbers go to design.md.
- **Release wakeups.** Waiting for room no longer allocates: a reader checks the limit, registers only when it is reached,
  and checks again (the lost-wakeup protocol of the earlier security reviews is kept); a release notifies a level only when
  a waiter is registered there. The same rule for the outbox's watchers of a pending poll.
- **One handshake request per session.** The session keeps `Arc<Request>`; sockets share it; a polling request of an
  existing session is not copied and its query is read for `sid`, `transport` and `EIO` only. The comments that claimed
  this are corrected to say what is held and why it is not charged.
- **Fewer copies.** The encoder writes the header and the JSON into one buffer; an encoding is moved, not cloned, into the
  queue; a shared encoding carries the Engine.IO message type, so a WebSocket frame reuses it; `push_group` stops
  allocating a vector per send; `tokio-tungstenite` moves to a release with `Bytes` payloads (0.26 or later).
- **Connect sequence (#183 and the critical-section finding).** After the middleware, the socket is added to the namespace
  and joins the room of its own id while in a new internal phase (`Joining`) that broadcasts, bulk operations and emits
  skip; then, under the socket's own state lock, CONNECT is queued and the socket marked connected. No connection lock is
  held across the adapter or a send. `InMemoryAdapter::add_all` checks the ids of other sockets against its own map
  instead of looking each name up in the namespace under its write lock.
- **CONNECT alone (#184).** A poll response ends after a CONNECT packet, so what the connection handlers emit leaves in a
  later response, as with the reference server when a poll was waiting (a WebSocket already writes each packet as a
  frame of its own). The connection handlers still run right after CONNECT is queued: the first design made them wait
  for the CONNECT write, which the security review found let a polling client that never polls skip them (design.md,
  "Security review (round 1)").
- **Address key.** The answer budget uses the session's address key.
- **Hygiene.** One `Connection` struct for the connection task; one function each for room validation, pending-ack
  registration and the engine's rate-limited lines (`WarnLimiter`, which stops allocating before it knows it will log);
  test hooks moved to one hidden `corvis::socket_io::testing` module; `#[non_exhaustive]` on `EmitError`,
  `DisconnectReason`, `AckError`, `RoomError`, `AdapterError`, `BroadcastAckErrorKind` and the Engine.IO `CloseReason`;
  integration and unit tests wait for conditions instead of sleeping; dead code, contradicting comments, errors typed by
  text and the nice-to-have items of the issue (design.md Decision 13); `adapter.rs` split into four files with the same
  public paths.

## Absorbed issues

- **#183** is the same lines as the critical-section finding (`client.rs`, the block that queues CONNECT under the
  namespace-table lock): one redesign fixes both, so its requirement ("a lookup by id never misses a socket whose client
  knows the id", "nothing the application emits precedes CONNECT") and its tests are part of this change, and the waits the
  #34 tests added for it are removed.
- **#184** is the engine's writer and the connect path: the fix considered (CONNECT in a write of its own) touches the
  order this change redesigns, so it was taken up here; the upstream suite stays unstable for reasons of its own and #184
  stays open (design.md, "What this change does not claim").

## Capabilities

### New Capabilities

- `bench/socket-io`: the Socket.IO benchmark (scenarios, what it reports, where it builds, how results are recorded).

### Modified Capabilities

- `ws/socket-io`: added requirements for the connect order, when the connection handlers run, CONNECT alone in its poll
  response, no lock across the adapter, one encoding per emit, extensible public enums and the place of test hooks.
- `ws/socket-io-rooms`: the requirement on adapter failures while a socket connects changes (a failure joining the own
  room now refuses the CONNECT instead of closing the connection), and an added requirement on the address key of the
  answer budget.
- `ws/engine-io`: added requirements for release wakeups and for the handshake request kept once per session.

## Impact

- New: `bench/socket-io/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member, `publish = false`),
  `crates/corvis/src/socket_io/{testing,ack,info}.rs`, `crates/corvis/src/socket_io/adapter/{mod,memory}.rs` (the split of
  design.md Decision 14, replacing `adapter.rs`), `crates/corvis/tests/socket_io_connect_order.rs`.
- Edited: `crates/corvis/src/ws.rs`, `crates/corvis/src/websockets.rs` and `crates/corvis/src/server.rs` (the
  `tokio-tungstenite` upgrade only), `crates/corvis/src/socket_io/{mod,client,socket,namespace,server,adapter,broadcast,
  parser,value,warn,budget}.rs`, `crates/corvis/src/socket_io/engine/{mod,session,websocket,polling,packet}.rs`,
  `crates/corvis/Cargo.toml`, `Cargo.lock`, `Cargo.toml` (workspace member), `examples/tertulia/Cargo.toml`, the tests
  named in tasks.md group 12, `docs/websockets/adapter.md`, `docs/es/websockets/adapter.md`.
- Public API: `#[non_exhaustive]` on seven enums (code that matches them exhaustively needs a wildcard arm);
  `Adapter::{membership_size, check_invariants, has_socket}`, `InMemoryAdapter::size`, `IoSocket::buffered_amount`,
  `IoNamespace::adapter`, `probe_decode`, `probe_auth` and `DecodeProbe` leave their current places for the hidden
  `testing` module (all were `#[doc(hidden)]`). Nothing else changes signature.
- Behavior visible to clients: a failure of the adapter joining the own room answers CONNECT_ERROR
  `Internal server error` instead of closing the connection after CONNECT; over polling, CONNECT and what the connection
  handlers emit leave in separate responses. Visible to applications: a lookup by id finds a socket from the moment its
  client can know the id; a Socket.IO packet counts its buffer's allocation against `max_buffered_amount`. Listed in design.md "Behavior visible to applications and clients".
- Dependencies: `tokio-tungstenite` and `tungstenite` move from 0.24 to the release chosen in task 5.4 (same licenses,
  already allowed by `deny.toml`). No new crate.
- Port apps do not enable `socket-io`, but `ws.rs`, `websockets.rs` and `server.rs` change with the upgrade, so the
  corvis-parity check is run.

## Out of scope

- A server-wide outbound budget (#176) and the frames held by slow WebSocket recipients (#175): the copies removed here do
  not change what a slow recipient holds.
- The Redis adapter (#36): this change only leaves the trait in the shape design.md "What the Redis adapter needs" lists.
- A Node comparison in the Socket.IO benchmark: the benchmark measures Corvis before and after; comparing with the Node
  reference server is a possible later issue.
- Charging the handshake request to the unread budget (design.md Decision 3 explains why it stays outside).
- Changing `IoSocket::ip()` and `Handshake::ip()` to return `&str` (design.md Decision 13, Review note 4).
- Gateways (#35), the harness CI job (#37), the user guide (#38).

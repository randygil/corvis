# Proposal

Implements [randygil/corvis#34](https://github.com/randygil/corvis/issues/34) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: design.md Decisions 2, 3 and 5,
delta `specs/websockets/socket-io-rooms`, tasks.md group 5). Builds on #33
(`openspec/changes/e2-socket-io-namespaces-events-and-acks/`, drafted together with this change) and, through it, on
#32 (`openspec/changes/e2-engine-io-handshake-polling-upgrade-and-heartbeat/`, approved and being implemented in
another worktree; its Engine.IO API is referred to **as designed there**). The issue and `ROADMAP.md` name the change
`e2-rooms-and-broadcast`; this change uses `e2-socket-io-rooms-and-broadcast`, the name it was assigned when drafted
(design.md Open question 1).

## Why

After #33 a Socket.IO server can only talk to one socket at a time: `IoSocket::emit`. Applications built on the
original model broadcast: `io.emit(...)`, `io.to("room").emit(...)`, `socket.broadcast.emit(...)`,
`socket.to(room).except(other).emit(...)`, with `socket.join(room)` / `socket.leave(room)` deciding who is in each
room. The official client cannot observe rooms directly, only which sockets receive each event, and that set must be
the one Node's in-memory adapter (`socket.io-adapter` 2.5.8) selects.

Corvis has rooms only in the `ws`-style gateways, and they do not fit: a set per client
(`crates/corvis/src/websockets.rs:239`, `join` / `leave` at `:284-289`), a broadcast that walks every client of the
gateway and filters (`RoomEmitter::emit`, `:514-523`), `except` for a single client id (`:510-513`), no namespace
level, no room named after the socket id, and no abstraction another backend could implement. #36 (Redis) needs that
abstraction, compatible in shape with `@socket.io/redis-adapter`.

## What Changes

- New files under `crates/corvis/src/socket_io/`, behind the opt-in `socket-io` feature: `adapter.rs` (the `Adapter`
  trait, `BroadcastOptions`, `BroadcastFlags`, `AckSink`, `SocketInfo`, the adapter factory, and `InMemoryAdapter`,
  the default) and `broadcast.rs` (`BroadcastOperator`, `RemoteSocket`).
- Rooms on sockets: `IoSocket::join`, `leave`, `rooms`; every socket is in the room named after its id from the moment
  it connects; rooms are scoped to their namespace; a room is deleted when its last socket leaves; a disconnecting
  socket is still in its rooms during `disconnecting` and in none at `disconnect`.
- Broadcast operators equal to Node's: `to` / `within` (Node's `in`), `except`, `local`, `timeout`, chained, on
  `IoServer` (main namespace), `IoNamespace` and `IoSocket` (`to`, `within`, `except`, `broadcast`, which exclude the
  sender); `emit`, `emit_with_ack` (collects the acknowledgement of every recipient or fails with
  `operation has timed out` and the answers received so far), `fetch_sockets`, `sockets_join`, `sockets_leave`,
  `disconnect_sockets`; `IoNamespace::emit` to every socket.
- The adapter trait is shaped for #36: cluster-wide operations are `async` and fallible, room membership of local
  sockets is synchronous, broadcast options carry `rooms`, `except` and `flags` as the Redis adapter's messages do,
  broadcasts receive the packet before encoding, `fetch_sockets` returns serializable descriptions, and
  `InMemoryAdapter` exposes the local delivery a cluster adapter wraps.
- Bounded defaults, documented, each with a regression test and, where a client can observe a difference from Node,
  an entry in `parity/socket-io/expected-differences.json`: 1,000 rooms per socket, room names up to 256 bytes (also
  in `to` / `except`, where an invalid name fails the emit instead of widening it), one shared encoding per broadcast
  with each recipient's send buffer bounded by the Engine.IO layer, the emitter never waiting on a recipient,
  broadcast acknowledgements with a deadline and at most 16 MiB of collected answers, no membership left behind by a
  disconnected socket. The full table is design.md Decision 8.
- Harness: the scenario app gains room and broadcast commands, and scenarios with several clients cover scoping,
  unions, exclusions, sender exclusion, rooms at disconnect, broadcast acknowledgements and the limits.
- Docs (EN + ES): rooms and broadcast in the "Socket.IO (preview)" section of `docs/websockets/adapter.md`.

## Capabilities

### New Capabilities

- `ws/socket-io-rooms`: rooms, broadcast operators, the adapter interface and the in-memory adapter of Socket.IO
  namespaces, with their limits. (The spike named it `websockets/socket-io-rooms`; the main spec tree uses `ws/`.)

### Modified Capabilities

None. `ws/socket-io` (#33) keeps its requirements; its disconnect order gains the room cleanup between
`disconnecting` and `disconnect`, which that capability already allows for. `ws/gateways` is untouched: the `ws`-style
`WsClient::join` and `RoomEmitter` keep their behavior.

## Impact

- New: `crates/corvis/src/socket_io/adapter.rs`, `crates/corvis/src/socket_io/broadcast.rs`,
  `crates/corvis/tests/socket_io_rooms.rs`, `crates/corvis/tests/socket_io_rooms_security.rs`,
  `parity/socket-io/scenarios/socket-io/rooms/**`.
- Edited: `crates/corvis/src/socket_io/{mod,options,server,namespace,socket}.rs` (adapter per namespace, operators,
  room cleanup on disconnect, own-id room on connect, `adapter` and room limits in `IoOptions`),
  `crates/corvis/tests/socket_io_diff.rs` and `parity/socket-io/reference/socket-server.mjs` (room and broadcast
  commands of the scenario app), `parity/socket-io/expected-differences.json`, `docs/websockets/adapter.md`,
  `docs/es/websockets/adapter.md`.
- No new dependency (`async-trait` is already used, `crates/corvis/Cargo.toml:20`).
- Public API: additions under `corvis::socket_io`. The `Adapter` trait is public and documented as unstable until
  #36 has implemented it once.
- Port apps do not use socket.io and the code is behind a feature they do not enable; the corvis-parity check is still
  run because the definition of done asks for it when framework runtime code changes.

## Out of scope

- The Redis adapter, cluster requests and mixed Node / Corvis clusters: #36. This change only keeps the trait shaped
  for it and tests the shape with a second, test-only adapter.
- Gateways over socket.io (`#[websocket_server]` as an `IoServer`, `#[connected_socket]` as an `IoSocket`,
  `client.join` in gateway handlers): #35. The rooms of the `ws`-style gateways are not changed or reimplemented on
  the adapter.
- Adapter events (`create-room`, `delete-room`, `join-room`, `leave-room`): possible later issue.
- `volatile` and `compress` flags, connection state recovery (`SessionAwareAdapter`), `serverSideEmit`,
  `allSockets()` (deprecated in Node in favor of `fetchSockets()`): possible later issues, as in #33.
- A cap on the number of rooms named in one `to` / `except` chain: the list is the application's own and the cost is
  linear in it (design.md Decision 8).
- User guide and example application: #38.

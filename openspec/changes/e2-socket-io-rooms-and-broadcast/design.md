# Design

> Reconciled with the code #33 merged (task 1.1): the layers described below were as designed, with the renames and
> shape changes recorded in "Implementation notes" at the end (`Packet` public and opaque, `EngineMessage::Shared`,
> `PendingAck::target`, the disconnect order, `HandshakeInfo`).

## Context

Design source: the Socket.IO spike #31 (`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`):
Decision 2 (`adapter.rs`: trait plus in-memory adapter, `redis.rs` later behind its own feature), Decision 3
(`to/in/except`, `broadcast` on the low-level API), Decision 5 (bounded send buffers), its delta
`specs/websockets/socket-io-rooms` (the starting point of this change's delta) and its notes on
`@socket.io/redis-adapter` 8.3.0. Code verified at `6f58ee1` (`origin/dev`).

### What exists on `origin/dev`

Rooms exist only in the `ws`-style gateways (`crates/corvis/src/websockets.rs`):

- Membership is a set per client: `ClientInner.rooms: Mutex<HashSet<String>>` (`:239`), `WsClient::join`, `leave`,
  `rooms`, `in_room` (`:284-295`). Nothing bounds the number of rooms or the length of a name.
- A gateway server holds its clients in one map (`ServerInner.clients`, `:441`). `WsServer::emit` (`:486-489`) sends
  to all of them; `WsServer::to(room)` (`:491-493`) takes one room and returns a `RoomEmitter` (`:502-506`) whose
  `except` takes one client id (`:510-513`) and whose `emit` clones the client list and tests `in_room` on each
  (`:514-523`): every room broadcast costs the whole gateway, and the JSON frame is built once per recipient
  (`WsClient::emit`, `:261-264`).
- There is no namespace level, a client is not in a room named after its id (ids are a timestamp plus a counter,
  `:982`), and there is no adapter interface.
- What does carry over is the delivery rule: a send never waits for the peer, each client has a byte-bounded queue,
  and overflowing it closes that client only (`WsClient::send_text`, `:268-279`; `OutQueue::push`,
  `crates/corvis/src/ws.rs:101-116`; default 8 MiB, `ws.rs:46`). `tests/ws_security.rs` covers it for gateways.

No `crates/corvis/src/socket_io/` exists yet; it arrives with #32 and #33.

### The layers this builds on (as designed, not yet merged)

- #32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`, approved, being implemented in another worktree;
  described here as its design.md states): each Engine.IO session has an outbox bounded by `max_buffered_amount`
  (8 MiB); a send over it fails, discards the outbox and closes that session with `transport error`; sends never
  block. `max_sessions` (10,000) bounds the sessions of a server.
- #33 (`e2-socket-io-namespaces-events-and-acks`, drafted with this change): `IoServer`, `IoNamespace`, `IoSocket`,
  the `pub(crate)` `Packet` and `encode`; `EngineSocket::send_all`, which queues the messages of one packet whole
  (its Decision 3); per-socket pending acknowledgements with `max_pending_acks` (1,000), `ack_timeout` (30 s) and a
  per-namespace id counter (its Decision 7); the disconnect order `on_disconnecting`, removal, `on_disconnect` (its
  Decision 8); `IoSocket::data` as a JSON value; socket ids of 20 characters from `engine/sid.rs`.

### Node behavior relied on

Read from the packages installed by the harness lockfile (`socket.io` 4.8.4 `dist/{broadcast-operator,socket,
namespace}.js`, `socket.io-adapter` 2.5.8 `dist/in-memory-adapter.js`), 2026-11-09:

- `Adapter` (one per namespace, `new Adapter(nsp)`): `rooms: Map<room, Set<sid>>` and `sids: Map<sid, Set<room>>`.
  `addAll(id, rooms)`, `del(id, room)`, `delAll(id)`; a room is deleted when its set becomes empty. `apply(opts, fn)`
  selects recipients: with rooms, the union of their members, each once, skipping ids in the except set and ids
  without a local socket; without rooms, every id of `sids` not in the except set. The except set is the union of the
  members of the `except` rooms; since every socket is in the room of its own id, excluding a socket id works the
  same way. `broadcast` encodes the packet once and writes it to each selected socket. `broadcastWithAck` takes an id
  from the namespace counter, registers the callback in each recipient's `acks` and reports the number of recipients.
  `sockets(rooms)`, `socketRooms(id)`, `fetchSockets(opts)`, `addSockets(opts, rooms)`, `delSockets(opts, rooms)`,
  `disconnectSockets(opts, close)`, `serverCount()` (1), `init()`, `close()`.
- `Socket`: `_onconnect` joins the room of its id before answering CONNECT; `join(rooms)` is `adapter.addAll`,
  `leave(room)` is `adapter.del`, `rooms` is `adapter.socketRooms(id)` or an empty set; `to`, `in`, `except` and
  `broadcast` start a `BroadcastOperator` whose except set holds the socket's id; `_onclose` emits `disconnecting`,
  then `_cleanup` (`leaveAll`, removal from the namespace, `join` becomes a no-op), then `disconnect`.
- `BroadcastOperator`: `to` / `in` and `except` take a room or an array and return a new operator with the union;
  `local`, `timeout`, `compress`, `volatile` set flags. `emit` rejects reserved event names, then calls
  `adapter.broadcast`, or with a callback `adapter.broadcastWithAck` and completes when every server reported its
  count and every counted client answered; a timer of `flags.timeout` fails it with
  `new Error("operation has timed out")` and the answers so far (`err.responses`) and removes the id from the
  sockets' `acks`. Without `timeout()` the timer's delay is `undefined`, so Node fires it at once: a broadcast
  acknowledgement without a timeout practically always fails. `fetchSockets` returns local `Socket`s or
  `RemoteSocket`s (`id`, `handshake`, `rooms`, `data`, and `emit` / `join` / `leave` / `disconnect` through an operator
  targeting the id's room).
- Nothing in Node bounds rooms per socket, room names (any value is accepted as a key), or the answers collected by
  a broadcast acknowledgement.
- `@socket.io/redis-adapter` 8.3.0 (spike Context): a broadcast is published as `[uid, packet, {rooms, except,
  flags}]` and delivered on each node by the in-memory logic; requests (sockets, fetch, join, leave, disconnect,
  broadcast with ack) are asynchronous and time out. This is the consumer the trait must fit.

## Goals / Non-Goals

**Goals:**

- Every broadcast reaches exactly the sockets Node's in-memory adapter selects, verified by the harness with several
  clients.
- A bounded default for what clients can make the server hold through rooms and broadcasts, with a test each.
- An adapter trait #36 can implement for Redis without changing namespaces, sockets or the public operators.

**Non-Goals:**

- Redis (#36), gateways (#35), adapter events, `volatile` / `compress`, state recovery (proposal, Out of scope).
- Replacing the room code of the `ws`-style gateways.

## Decisions

### 1. Files and where the adapter sits

`socket_io/adapter.rs` (trait, option types, `InMemoryAdapter`) and `socket_io/broadcast.rs` (`BroadcastOperator`,
`RemoteSocket`). Each `IoNamespace` owns one `Arc<dyn Adapter>`, created when the namespace is registered by the
factory in `IoOptions::adapter(factory)` (default: `InMemoryAdapter`), as Node's `new (server.adapter())(nsp)`. The
factory receives a `LocalNamespace`: the namespace name plus crate-provided access to its connected sockets (look up
by id, iterate, the ack id counter). It does not hand out the `IoNamespace` itself, so an adapter cannot keep the
namespace alive in a cycle.

Alternative considered: one adapter per server with the namespace as a parameter of every call. Rejected: Node's
adapters, the Redis channel names (`socket.io#<nsp>#`) and the room maps are all per namespace.

### 2. The adapter trait

```rust
#[async_trait]
pub trait Adapter: Send + Sync + 'static {
    async fn init(&self) -> Result<(), AdapterError>;
    async fn close(&self);
    async fn server_count(&self) -> Result<usize, AdapterError>;

    // Membership of sockets connected to this process (Node: addAll, del, delAll, socketRooms).
    fn add_all(&self, id: &SocketId, rooms: &[Room]) -> Result<(), RoomError>;
    fn del(&self, id: &SocketId, room: &Room);
    fn del_all(&self, id: &SocketId);
    fn socket_rooms(&self, id: &SocketId) -> Vec<Room>;

    // Operations that span the cluster.
    async fn broadcast(&self, packet: Packet, opts: BroadcastOptions) -> Result<(), AdapterError>;
    async fn broadcast_with_ack(&self, packet: Packet, opts: BroadcastOptions, sink: AckSink)
        -> Result<(), AdapterError>;
    async fn sockets(&self, rooms: &[Room]) -> Result<HashSet<SocketId>, AdapterError>;
    async fn fetch_sockets(&self, opts: BroadcastOptions) -> Result<Vec<SocketInfo>, AdapterError>;
    async fn add_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError>;
    async fn del_sockets(&self, opts: BroadcastOptions, rooms: Vec<Room>) -> Result<(), AdapterError>;
    async fn disconnect_sockets(&self, opts: BroadcastOptions, close: bool) -> Result<(), AdapterError>;
}
```

What makes it implementable by #36:

- Cluster operations are `async` and return `Result`: Redis publishes, waits for other nodes and can fail or time
  out. The in-memory adapter resolves them without awaiting anything.
- Membership is synchronous and local, as in Node, where `socket.join` returns at once even with the Redis adapter:
  a socket lives on one node, and that node owns its rooms.
- `broadcast` receives the `Packet` before encoding (the Redis adapter serializes it with msgpack; the in-memory
  adapter encodes it once). `Packet`, `SocketId` and `Room` become public types for this, opaque except for what an
  adapter needs (`Room` and `SocketId` are cheap-to-clone strings).
- `BroadcastOptions { rooms, except, flags }` with `BroadcastFlags { local, timeout }`, both `#[non_exhaustive]`:
  the three fields of the Redis adapter's message, with room for `volatile` / `compress` later.
- `broadcast_with_ack` mirrors Node's two callbacks with an `AckSink`: `sink.expect(clients)` once per server and
  `sink.respond(args)` per answer. The operator owns the deadline, the completeness rule (`server_count` servers
  reported, every counted client answered) and the byte cap of Decision 6, so they are not re-implemented per
  adapter.
- `fetch_sockets` returns `SocketInfo { id, handshake, rooms, data }`, plain serializable data (handshake: `auth`,
  headers, query, URL, address, issue time; `data`: the socket's JSON `data` of #33). The operator turns each into a
  `RemoteSocket`, which exposes `local() -> Option<IoSocket>` when the socket lives in this process.
- `InMemoryAdapter` is public and its local operations take `&self`, so the Redis adapter holds one and calls it for
  local delivery where Node's subclass calls `super.broadcast(...)`.

The trait is documented as unstable until #36 lands. To keep the shape honest before then, the tests include a second
adapter: a test-only wrapper around `InMemoryAdapter` that adds an `await` point and an injected failure to every
cluster operation and reports `server_count` 2 with a simulated second node (task 2.4).

Alternatives considered: a synchronous trait with Redis I/O spawned inside the adapter (errors and backpressure would
be invisible to callers, and publishes could pile up without bound); passing pre-encoded packets (forces the Redis
adapter to decode again); returning `IoSocket` from `fetch_sockets` (impossible for sockets on other nodes).

### 3. The in-memory adapter

State: `RwLock<{ rooms: HashMap<Room, HashSet<SocketId>>, sids: HashMap<SocketId, HashSet<Room>> }>` with the
standard randomly keyed hasher (room names can come from clients through application code).

- `add_all` takes the write lock, checks that the socket is still connected (through `LocalNamespace`), checks the
  limits of Decision 8 for the whole call before changing anything, then inserts. `del` and `del_all` remove and
  delete rooms that became empty. A join for a socket that is not connected does nothing and returns `Ok`, as Node's
  no-op `join` after cleanup.
- Selection (`apply` in Node) takes the read lock, computes the recipients exactly as Node does (union of rooms, each
  socket once, except set as the union of the members of the except rooms, or every socket when no room is given),
  collects their ids and releases the lock. Delivery happens after the lock is released, so a handler that joins or
  leaves during a broadcast cannot deadlock and a large fan-out does not block joins.
- `broadcast` encodes the packet once into an `Encoded` whose text and attachments are reference counted, and queues
  that same value on each recipient with `send_all`. A recipient whose outbox refuses it is closed by the Engine.IO
  layer (`transport error`); the others are unaffected and the broadcast still returns `Ok`.
- `broadcast_with_ack` takes one id from the namespace counter, registers a pending acknowledgement on each
  recipient (subject to #33's `max_pending_acks`), queues the packet and calls `sink.expect(recipients)`.

Alternative considered: keeping membership inside each socket, as the gateways do (`websockets.rs:239`). Rejected:
every room broadcast would scan the namespace, and the room-to-sockets map is what `except` and the Redis requests
need.

### 4. Rooms on sockets and the disconnect order

- On connect, after middleware and before CONNECT is answered, the socket joins the room of its id (Node's
  `_onconnect`). This cannot fail: `max_rooms_per_socket` is validated to be at least 1.
- `IoSocket::join(rooms) -> Result<(), RoomError>` accepts one name or several and is all-or-nothing; `leave(room)`;
  `rooms() -> HashSet<String>`. A socket may leave the room of its own id, as in Node; it then stops receiving emits
  addressed to its id.
- Disconnect (#33 Decision 8) becomes: `on_disconnecting` handlers (rooms intact, so a handler can tell the rooms
  that the socket is leaving), the socket is marked disconnected, `adapter.del_all(id)`, removal from the namespace,
  pending acks failed, `on_disconnect` handlers (`rooms()` is empty). Marking before `del_all`, with the connected
  check of `add_all` under the adapter's write lock, guarantees that no membership of a disconnected socket remains,
  whatever a concurrent `join` does.

### 5. Broadcast operators

`BroadcastOperator` is a value holding the adapter, `rooms`, `except`, `flags` and a deferred error. `to(rooms)` and
`within(rooms)` (Node's `in`, a keyword in Rust) add to `rooms`; `except(rooms)` adds to `except`; `local()` and
`timeout(duration)` set flags; each returns a new operator, so a base operator can be reused. Entry points:
`IoNamespace::{emit, to, within, except, local, timeout, fetch_sockets, sockets_join, sockets_leave,
disconnect_sockets}`; the same on `IoServer`, acting on `/` as Node's `io.to(...)`; `IoSocket::{to, within, except,
broadcast, local}`, whose operator starts with the socket's id in `except`.

Terminal operations are `async` and return `Result`: `emit(event, args)`, `emit_with_ack(event, args)`,
`fetch_sockets()`, `sockets_join(rooms)`, `sockets_leave(rooms)`, `disconnect_sockets(close)`. With the in-memory
adapter they complete without waiting on any client.

A room name that breaks the limits of Decision 8 does not panic and is not dropped: dropping a name from `to` could
leave the room set empty, which means "everyone". The operator records the error and every terminal operation returns
it (`EmitError::InvalidRoom`) without sending anything.

`emit` rejects the reserved event names of #33 before reaching the adapter, and binary arguments above
`max_attachments` as a single emit does.

Alternative considered: synchronous `emit` for the in-memory adapter and `async` only with Redis. Rejected: the same
application code must work when the adapter is swapped.

### 6. Broadcast with acknowledgements

`emit_with_ack` returns `Result<Vec<Args>, BroadcastAckError>`, where the error carries its kind and the answers
received so far (Node's `err.responses`).

- Deadline: `timeout()` or `IoOptions::ack_timeout` (30 s, #33). Node without `timeout()` fails immediately; Corvis
  applies the default deadline instead, which only server code can see.
- Completion: every server reported its count and every counted client answered, as Node. A recipient that
  disconnects, or whose pending table is full, never answers, so the call ends at the deadline with
  `operation has timed out` and the partial answers, as in Node.
- At the deadline the id is removed from the recipients that have not answered. An answer is accepted once per
  recipient (the entry is removed when it arrives) and only from sockets that were sent the packet (#33's per-socket
  table).
- Collected answers are bounded by `max_broadcast_ack_bytes` (default 16 MiB, the size of the received-data bound of
  the Engine.IO layer): each answer can be as large as a packet, and a broadcast to 10,000 sockets could otherwise
  collect gigabytes. Once the cap is reached further answers are dropped and the call fails with
  `BroadcastAckError::ResponsesTooLarge` and what was collected.

### 7. `RemoteSocket` and bulk operations

`fetch_sockets()` returns `Vec<RemoteSocket>`: `id()`, `handshake()`, `rooms()`, `data()`, and `emit`,
`join`, `leave`, `disconnect(close)`. *Corrected by review round 1:* for a socket of this process they act on that
`IoSocket` directly, as Node returns the local socket; implementing them as an operator on the room of the id (the first
design, "exactly as Node") let a client that had joined that room be reached, joined and disconnected with the socket. For
a socket held by another process they reach the adapter as an exact single-socket target (`BroadcastOptions::socket`),
so they keep working when #36 returns sockets of other nodes. `sockets_join` / `sockets_leave` apply the room limits per
socket; a socket that cannot join because of its cap is skipped and logged, the others join (Node has no cap).
`disconnect_sockets(close)` calls #33's `disconnect(close)` on each selected socket.

### 8. Limits and their defaults

| Limit | Default | Node | When exceeded |
|---|---|---|---|
| `max_rooms_per_socket` (own id room included) | 1,000 | none | `RoomError::TooManyRooms`, nothing joined by that call |
| `max_room_name_length` (join, `to`, `except`) | 256 B | none (any value) | `RoomError::NameTooLong`; in an operator, `EmitError::InvalidRoom` and nothing sent |
| Rooms per namespace | sockets × `max_rooms_per_socket`; empty rooms deleted | unbounded | derived |
| Membership of disconnected sockets | none: join after disconnect is a no-op, cleanup under the adapter's lock | same intent, single threaded | n/a |
| Encodings per broadcast | 1, shared by all recipients | 1 | n/a |
| Send buffer per recipient | 8 MiB (#32 `max_buffered_amount`) | none | that session closes with `transport error`; emitter and other recipients unaffected |
| Emitter waiting on recipients | never | never | n/a |
| Membership lock during delivery | not held | n/a | n/a |
| Broadcast ack deadline | `timeout()` or `ack_timeout` 30 s (#33) | `timeout()`, else immediate failure | `operation has timed out` with partial answers |
| Pending ack per recipient | counts in `max_pending_acks` 1,000 (#33) | none | recipient gets the event, its answer is not awaited |
| `max_broadcast_ack_bytes` | 16 MiB per broadcast | none | further answers dropped, `ResponsesTooLarge` |
| Answers per recipient | 1 | 1 | later ACKs ignored |
| Rooms named in one `to` / `within` / `except` chain | `max_rooms_per_socket` in all (*review round 1: was uncapped*) | not capped | `RoomError::TooManyTargets` on the terminal operation, nothing sent |
| Names in one `join` / `sockets_join` | at most `max_rooms_per_socket`, refused before the lock | not capped | `RoomError::TooManyRooms`, no name examined under the lock |
| Adapter call of an operator or `init` | `adapter_timeout`, 5 s | none | `EmitError::Adapter(AdapterError::Timeout)` |

Validation in `IoOptions` (error naming the option): zero `max_rooms_per_socket`, `max_room_name_length`,
`max_broadcast_ack_bytes` or `adapter_timeout`; `max_room_name_length` below 20 (a socket id must fit).

*Corrected by review round 1:* the first design left the rooms of one `to` / `except` chain uncapped on the argument that
the application builds the list. A list that comes from a client is the same list, and `to(empty)` reaching everyone made
that worse; the operator now names at most `max_rooms_per_socket` rooms and an empty `to` selects nobody (see "Security
review (round 1)").

Client-observable divergences (through what the scenario app acknowledges): the 1,001st room of a socket, a room
name of 257 bytes, and (round 1) a join of another socket's id, leaving the own id room and an empty `to`. Each gets a
scenario and an entry in `expected-differences.json`. The other rows equal Node or are
visible only to server code; they are documented and tested in Rust.

### 9. Verification

- Unit tests of `InMemoryAdapter` against a table of cases derived from Node's `apply` (rooms, unions, except rooms,
  except ids, empty rooms, unknown rooms, sockets without a local entry), plus proptest properties comparing the
  selection with a naive model (a list of `(socket, rooms)` filtered in the obvious way) and checking the invariant
  that `rooms` and `sids` mirror each other and hold no empty room after any sequence of operations. The properties
  live in `socket_io/fuzz_adapter.rs` so `cargo test --lib fuzz` selects them.
- `crates/corvis/tests/socket_io_rooms.rs`: in-process application with several raw clients, one test per spec
  scenario that does not need Node. Absence of delivery is asserted without sleeping: after the broadcast the server
  emits a marker event to every socket, and a client that receives the marker without the broadcast did not get it.
- `crates/corvis/tests/socket_io_rooms_security.rs`, in the style of `tests/ws_security.rs`: room cap, name length in
  `join` and in operators (nothing sent), broadcast to many clients that do not read (emit returns promptly, each
  outbox stays within its bound, slow sessions closed, healthy ones served, one shared encoding measured by reference
  count), join / disconnect races (no membership left, run many times), broadcast ack deadline on the paused clock,
  answers cap, second ACK ignored, ACK from a socket outside the broadcast ignored, handler joining during a
  broadcast (no deadlock).
- Harness: the scenario app of #33 (Node reference and the Corvis app in `socket_io_diff.rs`) gains commands sent as
  events with acknowledgements: `join`, `leave`, `rooms`, `broadcast` (`{from: "server" | "socket", to, except,
  event, args}` followed by the marker event `done` to the whole namespace), `broadcast-ack` (`{to, timeout}`,
  acknowledged with the error message and the sorted answers), `sockets-join`, `sockets-leave`,
  `disconnect-sockets`, `fetch-sockets` (sorted ids and rooms), and the server-side log records `rooms` in
  `disconnecting` and in `disconnect`. Scenarios under `parity/socket-io/scenarios/socket-io/rooms/`: `scoping`,
  `socket-id`, `union`, `from-socket`, `except`, `leave-and-disconnect`, `bulk` (`sockets-join`, `sockets-leave`,
  `disconnect-sockets`, `fetch-sockets`), `broadcast-ack`, `limits` (the two expected differences). Each uses three or
  four official clients named in the transcript, so the normalizer's per-client ordering applies
  (`parity/socket-io/lib/normalize.mjs:12-20`), and waits on the marker event, never on time.

## Risks / Trade-offs

- #33 and #32 are not merged: names used here may shift → the tasks start by reconciling with the merged code; the
  decisions depend on behavior (grouped sends, per-socket ack table, disconnect order), not on signatures.
- The trait is designed before its second implementation → marked unstable, exercised by the test-only adapter, and
  #36 may adjust it; public operators do not expose it.
- `async` terminal operations make `io.emit(...)` need `.await` → same code works with Redis; documented with
  examples; forgetting `.await` is a compiler warning (`#[must_use]` on the operator's futures).
- Snapshotting recipients allocates per broadcast → proportional to the recipients, which the delivery already is.
- A socket that joined or left between selection and delivery may receive or miss that one broadcast → inherent to
  concurrent servers; per-task ordering is kept (a task that joins and then broadcasts sees the join).
- Room caps can reject legitimate joins in unusual applications → configurable, errors are explicit, listed as
  expected differences.
- Scenarios with several clients are the most timing-sensitive of the harness → marker events instead of waits, per
  client ordering in the normalizer, no retries.

## Open questions

1. The issue and `ROADMAP.md` name this change `e2-rooms-and-broadcast`; the draft uses
   `e2-socket-io-rooms-and-broadcast` as assigned. Rename one of them when the change is accepted (#32 settled the
   same question in favor of the issue's name).
2. `max_rooms_per_socket` 1,000 and `max_room_name_length` 256 bytes: confirm the numbers. Recommendation: keep; both
   are options.
3. `sockets_join` over a selection where some sockets are at their room cap: skip those and log (the draft), or fail
   the whole call? Recommendation: skip and log, so one socket cannot block a bulk operation.
4. Broadcast acknowledgement without `timeout()`: use `ack_timeout` (the draft) or fail at once as Node effectively
   does? Recommendation: the default deadline; it is what application code means.
5. `within` as the name of Node's `in`. Alternative: `in_`. Recommendation: `within`.
6. Should the `Adapter` trait be `#[doc(hidden)]` until #36, instead of public and documented as unstable?
   Recommendation: public and unstable, so #36's review can judge it as users will see it.

## Review notes

These settle the open questions and override the design where they differ.

1. The change keeps the name `e2-socket-io-rooms-and-broadcast`; `ROADMAP.md` is updated to match.
2. 1,000 rooms per socket and 256 bytes per room name are confirmed.
3. `sockets_join` skips the sockets that are at their cap and logs a warning, as designed.
4. A broadcast with an ack and no `timeout()` uses `ack_timeout`. It is a listed expected difference.
5. The operator is named `within`.
6. The `Adapter` trait and `InMemoryAdapter` are `#[doc(hidden)]` and documented as unstable until #36 implements
   the Redis adapter against them; #36 makes them public. This overrides the design, which made them public now.
7. Implemented after #33 lands: the first task compares this design with the code #33 merged and records the
   adjustments in Implementation notes.
8. The security review is mandatory before the merge (room limits, broadcast fan-out, slow receivers, ack
   accumulation).

## Implementation notes

### Comparison with the code #33 merged (task 1.1)

The decisions held; the names and a few shapes moved. Adjustments, in the order a reader of "The layers this builds on"
meets them:

- `Packet`, `Body` and `encode` were `pub(crate)` in `socket_io/parser.rs`. `Packet` is now `pub` and opaque (its fields
  stay `pub(crate)`) so the adapter trait can name it; `encode` still returns `Encoded` (a `String` plus attachments).
  `Encoded::share()` turns it into `SharedEncoded` (`Arc<str>` plus `Bytes` attachments), the one value a broadcast queues
  on every recipient.
- `EngineSocket::send_all` exists as designed (it queues the messages of a packet whole). It takes `EngineMessage`s, whose
  text was a `String`, which cannot be shared: the hidden Engine.IO enum gained a variant `EngineMessage::Shared(Arc<str>)`
  (sent only, never received; a received one closes the connection). It is the only change in the Engine.IO layer: its
  `len`, `polling_len` and `encode_polling` handle the variant like a text, so plain Engine.IO users, polling and
  WebSocket, behave as before. The `Packet::encode_ws` copy into a frame is unchanged: it happens per session, at write
  time (see the memory formula).
- `IoSocket::send_encoded` existed unused (`#[allow(dead_code)]`); `send_shared` is the one the broadcast uses. It checks the
  phase first, so a socket that disconnected is never sent a packet.
- Pending acknowledgements: `SocketState.acks` held a `oneshot` sender per entry; an entry is now
  `PendingAck { target: AckTarget::One(oneshot) | AckTarget::Broadcast(AckSink), deadline }`, so a broadcast shares the
  per-socket table, the `max_pending_acks` cap and the lazy purge by `deadline` with `emit_with_ack`. The namespace id
  counter (`NamespaceInner::next_ack_id`) is the one the adapter draws from through `LocalNamespace`.
- Disconnect order. #33: `disconnecting` handlers, `remove_socket`, `forget`, phase `Disconnected` and pending acks failed,
  `disconnect` handlers. Now: `disconnecting` handlers (rooms intact), phase `Disconnected` and pending acks taken,
  `adapter.del_all`, `remove_socket`, `forget`, acks failed, `disconnect` handlers. The mark comes before `del_all` and the
  connected test of `add_all` runs under the adapter's write lock: that pair is what leaves no membership behind.
- `IoSocket::data` is a `serde_json::Value`, as designed; `SocketInfo.data` carries it. `Handshake` is not serializable
  (it holds the request), so `fetch_sockets` returns `HandshakeInfo` (auth, headers, URL, address, issue time in ms): plain
  data a remote adapter can produce. `RemoteSocket::handshake()` returns that, not a `Handshake`. *Round 1:*
  `HandshakeInfo` no longer copies: for a socket of this process it shares the socket's `Handshake` (now an `Arc` of its
  data), and serializes and deserializes as plain data for a remote adapter; its accessors are `auth()`, `headers()`,
  `url()`, `address()` and `issued_at()`.
- `IoSocket::timeout(d)` already exists (a deadline for one acknowledged emit, Node's `socket.timeout`): the broadcast
  version is `socket.broadcast().timeout(d)`.
- The own-id room is joined in `client::connect`, right after `add_socket` and before the connection handlers run.
- Socket ids are 20 characters (`engine/sid.rs`), which is where the minimum of `max_room_name_length` (20) comes from.

### Decisions and deviations

- **Types.** `Room` and `SocketId` are `Arc<str>` (`pub type`). Membership methods that only look a name up take `&str`
  (`del`, `del_all`, `socket_rooms`, `add_all`'s id), so no allocation is needed to leave a room.
- **`LocalNamespace`** holds a `Weak` to the namespace (the namespace owns the adapter: no cycle) and offers lookup by id,
  `sockets_by_ids` (one read lock), `is_live`, the two room limits and the ack id counter. Created with `Arc::new_cyclic`,
  so the factory gets a handle that cannot be used until it returns (documented).
- **Public error types.** `EmitError` gained `InvalidRoom(RoomError)` and `Adapter(AdapterError)`; an adapter's
  `InvalidPacket` is mapped back to `TooManyAttachments` / `TooDeep`. `AdapterError` has `Timeout`, `Closed`,
  `Unavailable(String)` and `InvalidPacket(PacketError)`.
- **Hidden adapter module.** Review note 6: `pub mod adapter` is `#[doc(hidden)]`, and so is `IoOptions::adapter`. The types
  an application meets (`RoomError`, `AdapterError`, `SocketInfo`, `HandshakeInfo`, the operator, `RemoteSocket`, the ack
  error) are re-exported at `corvis::socket_io` and documented. The trait has three provided, hidden test hooks
  (`membership_size`, `check_invariants`, `has_socket`) that the in-memory adapter implements.
- **`init` and `close`.** `init` runs once per namespace before its first cluster operation (a `tokio::sync::OnceCell`),
  not at registration, because `IoServer::of` is synchronous and may run outside a runtime. `IoServer::close` and the
  application's shutdown spawn `Adapter::close` for every namespace when a runtime is available; round 1 made it run
  once (it ran twice, once from each path).
- **`AckSink`** gained `on_finish(cleanup)`: the adapter registers what it must undo when the call ends. The in-memory
  adapter registers one closure holding a `Weak` per recipient, which removes the pending entry; it runs at completion, at
  the deadline and when the call's future is dropped (`Collection` guard), so no recipient keeps an entry.
- **A recipient whose pending table is full** is sent the event and is not counted in `expect`, so it does not hold the call
  until the deadline (design Decision 6 counted it; its own table row, "answer is not awaited", is the behavior kept). A
  recipient that disconnects after being counted never answers, as in Node, and the call ends at the deadline.
- **Responses.** Node's broadcast acknowledgement callback receives only the first value of each client's answer
  (`ack.apply(this, packet.data)` into a one-parameter function). Corvis returns the whole `Args` of each answer; the
  harness records the first value on both sides.
- **Selection.** `Membership::select` follows Node's `apply`. The except rooms are tested the cheaper way: per candidate when
  `candidates x except rooms` is not above the members of those rooms, else through one set of their members, so neither a
  long `to` list with a big except room nor the opposite is a product of two large numbers.
- **Delivery yields** to the runtime every 512 recipients, so a very large fan-out does not hold a worker thread. What
  this keeps and what it does not (round 1): two broadcasts of one caller, the second awaited after the first returned,
  reach every common recipient in that order, because the first finished delivering before the second started; the
  broadcasts of concurrent tasks interleave at a yield and have no order between them. The handlers of one connection
  run concurrently, so two relays of one client are two tasks (Node runs its handlers to completion, which orders
  them). The yield stays.
- **Harness.** The room commands of the scenario apps are `join`, `room-leave` (`leave` already disconnects a socket),
  `rooms`, `broadcast`, `broadcast-ack`, `sockets-join`, `sockets-leave`, `disconnect-sockets`, `fetch-sockets`. The
  `disconnect` log entries gained `roomsDuring` and `roomsAfter` (one entry, so no existing scenario that counts entries
  changes), and every socket says `left` to the people in its rooms from `disconnecting` (harmless without shared rooms).
  The expected differences are nine paths of two scenarios: the room cap and name rows of `rooms/limits`, and the default
  deadline of `rooms/broadcast-ack` (the clients answer after 100 ms, so Node's immediate failure is not a race); round 1
  added nine paths of two scenarios (`rooms/id-rooms`, `rooms/empty-to`), see "Security review (round 1)".
- **Tests.** The test-only adapter lives in `tests/socket_io_rooms.rs` and uses the public API only. The slow-client and
  shared-encoding tests use long-polling sessions that never poll: what is queued stays queued, deterministically, with no
  dependency on operating-system socket buffers. The 1,000-round join / disconnect race runs on 4 worker threads and was
  run five times in a row. No test compares durations.

### Memory of a broadcast in flight

For an encoding of `E` bytes (text plus attachments) delivered to `N` recipients, the process holds
`E + 24 N` bytes for the broadcast (the shared buffer, and the snapshot of `N` socket ids and handles) plus, in each
recipient's queue, one reference to the buffer and one queue entry. It never holds `N x E`.
`tests/socket_io_rooms_memory.rs` measures it with a counting allocator: a 1 MiB packet to 10 and to 100 sockets holds
1,048,608 bytes in both cases. While it is queued, a recipient's connection holds the reference until its transport has
written the message, and then a frame copy of that message for as long as the write takes (one frame per session). Every
recipient is bounded by its own `max_buffered_amount`, and with no server-wide outbound budget (#176) `S` slow recipients
hold at most `S x max_buffered_amount` queued (plus the frame in flight each): that is the figure to size.

A broadcast with acknowledgements adds, until it ends, a pending entry of one `AckSink` reference and one `u64` per
recipient (about 40 bytes with the table's overhead) and at most `max_broadcast_ack_bytes` (16 MiB) of collected answers.
*Corrected by review rounds 1 and 2.* (Round 2: the answers are charged to the calls' own budget, see "Security review
(round 2)"; the next paragraph describes round 1's intermediate design.) The first implementation said the answers are charged to their sessions "while their
handler runs" and that the collector holds them beyond that, bounded by that figure. That was the defect (F2): the charge of
an ACK packet was dropped right after the answer was handed to the collector, so answers held until the deadline counted
nowhere, and the 16 MiB bound is per call while the number of calls is not bounded. Now the charge travels with the
answer and is held until the call ends (see "Security review (round 1)"), so what a pending call holds counts against the
answering connection's session, address and server gauges, counted with the decoded-size model of `value.rs`.

### Lock order

Outermost first: a connection's namespace table (`join_own_room` runs under it; the first version of this section omitted
it), the adapter's membership lock, the namespace's socket table, a socket's state. The membership lock is held
for a lookup or an update and never across an await or while application code runs; no other lock is taken while the socket
table is held for writing. Broadcast selection takes the membership read lock, then the table read lock once for all ids
(`sockets_by_ids`), and delivers with neither held, locking one socket's state at a time for the phase check and the
engine's queue. `add_all` first validates and deduplicates the names without a lock (round 1), then takes the write lock and asks the
table and the socket's phase (`is_live`) once for the call and once per name for the ids of other sockets; the disconnect path
sets the phase (state lock, released) and only then calls `del_all` (write lock), so whichever of a join and a disconnect
takes the membership lock first, nothing of the socket survives. `AckShared`'s mutex is a leaf: answers are delivered to it
from the socket's resolve path after the state lock was released, and its cleanups run after its own lock was released.

### The bounds, with their tests

| Bound | Default | Enforced by | Test |
|---|---|---|---|
| `max_rooms_per_socket` | 1,000 | `Membership::add`, under the write lock, all or nothing | `a_socket_cannot_join_more_rooms_than_the_cap_and_a_refused_join_changes_nothing`, `the_room_cap_and_the_name_length_are_enforced_all_or_nothing`, `fuzz_the_room_cap_is_all_or_nothing`, harness `rooms/limits` |
| `max_room_name_length` | 256 B (min 20) | `IoSocket::join`, `add_all`, the operator's deferred error | `a_room_name_over_the_limit_is_refused_in_join`, `a_long_room_name_in_an_operator_fails_every_operation_and_never_widens_the_broadcast`, `a_long_name_in_to_sends_nothing` |
| `max_broadcast_ack_bytes` | 16 MiB | `AckSink::respond` | `answers_past_the_cap_end_the_call_with_what_fit` |
| Ack deadline | `timeout()` or `ack_timeout` | `tokio::time::timeout` in `emit_with_ack` | `without_a_timeout_the_default_deadline_applies`, `a_silent_recipient_ends_the_call_at_the_deadline_with_the_answers_so_far` |
| No membership left behind | maps empty after churn | mark then `del_all`; `add_all` checks `is_live` under the lock | `churn_leaves_the_maps_empty` (100,000 join / leave cycles, 200 disconnects), `a_join_racing_a_disconnect_leaves_no_membership_behind` (1,000 rounds), `fuzz_the_two_maps_mirror_each_other_and_hold_nothing_empty` |
| One encoding per broadcast | 1 | `Encoded::share`, `EngineMessage::Shared` | `a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches` |
| Recipient queue | `max_buffered_amount` | Engine.IO outbox | `many_slow_clients_never_hold_the_broadcast_back_and_each_is_cut_at_its_own_bound` |
| Answers once, only recipients | 1 per recipient | the per-socket entry is removed when it answers | `an_outsider_is_ignored_and_a_second_answer_counts_once` |
| Collected answers count on their connection (round 1) | the unread budget of the answering session, address and server | the `Charge` travels with the `Args` into `AckState` and is released by `AckShared::finish` | `the_answers_a_broadcast_holds_count_on_the_answering_connection_until_the_call_ends`, `answers_dropped_by_the_cap_release_their_charge_too`, `answers_held_by_pending_broadcasts_stay_within_the_quota_of_the_answering_address` |
| Id rooms (round 1) | a socket joins no other socket's id and leaves no own id | `InMemoryAdapter::add_all` and `del`; the id given at CONNECT is never a room | `a_client_cannot_join_the_room_of_another_sockets_id`, `no_socket_is_given_an_id_that_is_already_a_room`, `a_remote_socket_reaches_only_its_own_socket_even_when_another_is_in_its_id_room` |
| Work under the membership lock (round 1) | at most `max_rooms_per_socket` names examined | input bound and deduplication before the lock; `Membership::add` stops at the first name over the cap | `a_join_naming_more_rooms_than_the_cap_is_refused_before_the_lock`, `a_join_stops_at_the_first_name_that_passes_the_cap` |
| `adapter_timeout` (round 1) | 5 s | `IoNamespace::bounded`, `ready_adapter` | `an_adapter_that_never_returns_fails_emit_at_the_adapter_timeout`, `an_adapter_whose_init_never_returns_fails_the_call_and_is_tried_again` |

## Security review (round 1)

The first review (a reviewer with proofs of concept, nine areas) found two medium issues and several low ones. What held
and keeps holding, with its tests: one shared encoding per broadcast (text and binary attachments), a broadcast never
waits on a slow recipient and each is cut at its own bound, the room limits (bytes, multi-byte names, exactly 1,000 rooms
under concurrency), cleanup on six ways out, no delivery after `leave` or `disconnect` returned, no deadlock, namespaces
isolated, reserved names refused in both directions, `EngineMessage::Shared` lengths. Corrections to earlier statements are
marked "Corrected by review round 1" where they were made (Decisions 7 and 8, the memory formula, the lock order).

**F1 (medium): a client in the room of another socket's id, and `RemoteSocket` dragging it along.** `join` accepted any
name, a socket id included, so with an application that joins the rooms a client names, `join(<victim id>)` made the
attacker receive what the application emits to that id. Worse than Node: `RemoteSocket::join("admins")` of the victim joined
the attacker too and `RemoteSocket::disconnect` disconnected it, because every `RemoteSocket` operation went through the id
room, also for local sockets (Node's `fetchSockets` returns the local socket). Fixed in two parts.
(1) `RemoteSocket::{emit, join, leave, disconnect}` act on the local `IoSocket` when it is local; otherwise the operation
goes to the adapter addressed by `BroadcastOptions::socket`, an exact single-socket target that `Membership::select`
honours (rooms ignored, `except` still applied). `Adapter` gained `room_exists` (default `false`). The Redis adapter of #36
(`openspec/changes/e2-socket-io-redis-adapter`) carries `BroadcastOptions` as the `rooms`, `except` and `flags` of its
wire messages, which Node peers also read, so it has no field for a target: #36 decides how to send `socket`. Sending it as
`rooms: [id]`, as Node does, is exact on a node that applies hardening (2) (no client can be in another socket's id room),
which is every Corvis node; that design's sentence "`RemoteSocket` operations go through an operator on the room of the
socket's id" is superseded by this one and needs updating when #36 starts.
(2) Hardening, security over parity: `add_all` refuses (`RoomError::SocketIdRoom`), under the membership lock and all or
nothing with the rest of the call, a room named like the id of another live socket of the namespace; `sockets_join` and
`RemoteSocket::join` obey it (bulk skips and logs); `del` ignores leaving the own id room (that room is how the socket is
addressed); and at CONNECT an id that is already a room is never assigned (a new id is drawn). Listed expected differences
(scenario `rooms/id-rooms`): Node lets a socket join any name, including another socket's id, and leave its own.
Tests: the four effects of the reviewer's f3b (join refused and `io.to(id)` reaching only the victim; and, with the
hardening bypassed by a defect simulated through a probe adapter, `emit`, `join`, `leave` and `disconnect` of the victim's
`RemoteSocket`, directly and through its operator, reach only the victim).

**F2 (medium): answers collected by a broadcast with acknowledgements were outside every inbound budget.** The `Charge` of
an ACK packet was dropped right after `resolve_ack`, while `AckSink::respond` kept the `Args`: 32 calls in flight from one
handler, 17 members answering 960 KB each and one silent member held 540 MB until the deadline, against an address share
of 16 MiB. Fixed: `resolve_ack` takes the `Charge` and a broadcast target hands it, with the `Args`, to the collector
(`AckSink::respond_charged`), which keeps it in `AckState` until `AckShared::finish` (completion, deadline, drop of the
call, or an answer dropped for the cap). The charge sits on the answering session's gauge, chained to its address and the
server, so held answers count there and the connection's reads pause at its quota like for any held packet. It composes
with the chain of #33 without touching the gauges: no new gauge code runs; a `Charge` is released exactly once by its drop
(outside the collector's lock, since a release wakes waiters), and holds its gauge chain alive, so no charge outlives its
gauge. When the answering session closes while its answer is held, the charge stays on the address and server levels until
the call ends, as the charges of lingering handlers do. A single `emit_with_ack` to one socket still drops the charge as
before (its answer is returned to the caller). Tests: gauges during and after, with the session closed meanwhile and with the
call dropped, in `tests/review1.rs`; and the reviewer's scenario scaled down with the counting allocator
(`tests/socket_io_rooms_review_memory.rs`: 32 calls, 6 answerers of 300 kB, one silent member; measured 13.4 MB held against
64 MB without the fix, bound 17.5 MB, everything given back at the end).

**B1 (low): a refused join of 200,000 names held the write lock for 106 ms.** The input is now bounded before the lock: a
`join` or `sockets_join` naming more than `max_rooms_per_socket` rooms is refused at once; names are validated and
deduplicated outside the lock; under it `Membership::add` stops at the first new name that would pass the cap. Tests count
work (`Membership::examined`), not time.

**B3 (low): `fetch_sockets` copied rooms, headers and `auth`** (296 KB a socket). `HandshakeInfo` shares the socket's
handshake (`Handshake` is an `Arc` of its data, including the `auth` charge, so a retained result keeps its charge), and the
rooms are shared names (`Vec<Room>`, 16 bytes each, at most `max_rooms_per_socket`; they serialize as plain strings).
Counting allocator: 200 sockets with 14 KiB of `auth` and 20 rooms of 200 bytes allocate 671 bytes a socket, bound
3 KiB. `socket.data` is the application's own value and is still copied.

**B4 (low): a panic in the adapter's `del_all` leaked the socket, and `emit` had no deadline.** The call on the disconnect
path is contained and logged (rate limited), so the socket always leaves the namespace and releases its charges. Every
adapter call made by an operator, and `init`, is bounded by the new option `adapter_timeout` (default 5 s, validated like
the other durations) and fails with `AdapterError::Timeout`; `emit_with_ack` was already bounded by its deadline and now also
bounds `server_count` and `broadcast_with_ack` by `adapter_timeout`. A panic in the adapter's other methods still
propagates to the caller of the operator.

**B5 (low): `to(Vec::new())` selected every socket.** An operator given an explicitly empty `to` / `within` selects nobody
(the call succeeds and delivers to nobody) unless another call adds rooms; no `to` at all still means everyone; an empty
`except` has no effect. Expected difference from Node, scenario `rooms/empty-to`.

**Debug output.** `RemoteSocket`, `SocketInfo` and `HandshakeInfo` have hand-written `Debug`: header names without the values of
`cookie`, `authorization` and `proxy-authorization`, the keys of `auth` without values, the URL without its query. The older
`Handshake` type keeps its derived `Debug` (issue #178).

**Notes fixed while there.** `Adapter::close` ran twice per namespace (`IoServer::close` and the engine's `on_closing` both
called `release`): it runs once. Chaining 5,000 `.to(r)` calls took 509 ms because every call cloned both sets: the operator is
now a shared, linked list of the rooms of each call, built into sets once by the terminal operation, and it carries at most
`max_rooms_per_socket` rooms (`RoomError::TooManyTargets` beyond it; the exclusion of the sender in `socket.broadcast()` is
not counted); a counting allocator measures 1.1 MB for 5,000 chained calls and an emit. Order of delivery: the 24,400 of
50,000 out-of-order deliveries came from concurrent handlers (each of the 32 may deliver, yielding every 512 recipients),
not from one caller; the guarantee is stated as two broadcasts of one caller, the second awaited after the first returned,
and is tested with 1,300 recipients while another task broadcasts; broadcasts of different tasks have no order between them
(documented). The yield stays.

**Recorded against existing issues, not fixed here.**
- #175 / #176: `S` WebSocket recipients that do not read hold `S` frames of the encoding at once (104.85 MB for a 1 MiB
  text to 100 recipients), not "one session at a time" as the docs said (corrected in the limits page); and one address can
  pin `256 x (8 MiB + one frame)` of outbound memory, because there is no server-wide outbound budget.
- B2: each membership costs about 475 bytes outside every budget: 121.6 MB per namespace for one address at the defaults
  (256 sessions x 1,000 rooms) and 4.75 GB per namespace server-wide (10,000 sessions); it is the derived bound the design
  declares, now with its measured figure in the limits page and the options that lower it.
- Engine.IO logged one warning per session closed by a send buffer overflow, without a limit (older than this change); a
  broadcast closes many at once, so it is now one line a minute with the number left out.

**Review status.** Round 1 findings: F1, F2, B1, B3, B4, B5, Debug and the notes are fixed with a regression test each (the
ones recorded against issues have none by design). Round 2 is for the maintainer's session.

## Security review (round 2)

The second review (the closing pass over the round 1 fixes) confirmed that rounds 1's fixes hold: id rooms, memory of
the held answers, the bounds on joins and chaining, the shared handshake, adapter hangs and panics on the async paths and
`del_all`, the rate limited overflow warning. Findings of this round, with their fixes and tests (all in
`crates/corvis/src/socket_io/tests/review1.rs`):

- **M1 (medium): the victims pay for the answers.** Round 1 kept the answerer's `Charge` until the call ended, so one
  address that triggered ack-collecting broadcasts made other clients' budgets pay: 256 calls with one silent member held
  the whole server-wide unread budget on 17 victim addresses, every session above its fair share stopped being read for
  the call's duration, and honest clients were disconnected with `AdmissionTimeout` when the deadline passed 30 s. Fix: the
  answerer's charge is released the moment the collector has taken the answer (or refused it), and what the call keeps is
  charged to a server-wide **answer budget** (`AckBudget`: one atomic counter and a cap, shared by the namespaces of a
  server; `AckHold` releases on drop, so `finish`, a dropped call and an abandoned call all give it back). The charge is a
  try-charge by compare-and-swap that never waits; with no room the call ends with `ResponsesTooLarge` and keeps the
  answers it had. Cap: a quarter of the engine's `max_total_unread_bytes`, never less than `max_broadcast_ack_bytes` (64
  MiB with the defaults), so the memory the held answers can add is bounded by an amount that is not part of the unread
  budget and costs no reader a pause. **Why the server measure and not the caller's chain:** the caller of an operator is
  application code with no session, address or gauge (a timer, a handler of another connection, a task); making the
  caller's chain pay would need a measure the API does not have and would put the cost on whichever client happened to
  trigger the call, which is the defect again. The unread chain (session, address, server) keeps its invariant untouched:
  it holds only what the client sent and is still being read or handled. The trade-off, stated: a client that makes
  calls hold the whole answer budget can make *other calls* fail with `ResponsesTooLarge` (a failure the application sees
  and handles per call) instead of making *other clients* stop being read and be disconnected; each call is still bounded by
  `max_broadcast_ack_bytes` and its deadline. Tests:
  `the_answers_a_broadcast_holds_are_charged_to_the_call_and_never_to_the_answering_connection` (the unread gauges stay 0
  while the call holds the answer, session closed meanwhile, call dropped),
  `the_answers_of_all_calls_together_are_bounded_by_the_server_answer_budget_without_waiting`,
  `a_call_that_finds_no_room_in_the_answer_budget_ends_with_responses_too_large`, and the cap test
  `answers_dropped_by_the_cap_release_their_charge_too`.
- **L1 (low): panic in `room_exists` at CONNECT** killed the connection task and left an Engine.IO session open as long as
  the client answered pings. `IoSocket::new` runs the id draw under `warnings.guard` and returns `None` on a panic; the
  attempt is refused with CONNECT_ERROR `Internal server error` and the namespace slot is freed. Test:
  `a_panic_in_room_exists_during_connect_refuses_the_attempt_and_leaves_no_session_behind`.
- **L2 (low): panic in `add_all` of the own room at CONNECT** left a socket in the namespace table, in no room, with its
  auth charge. `join_own_room` runs under `warnings.guard`; when it fails, after the namespace table is released, the
  socket is disconnected (`disconnect_with`) and the connection closed. Test:
  `a_panic_in_add_all_of_the_own_room_during_connect_closes_the_connection_and_removes_the_socket`.
- **Lead: the id race window during the connect middleware.** The id is drawn before the middleware and the socket goes
  live after it (up to `middleware_timeout`), so a client that learned the id there could join that name before it was a
  live id. Adding a socket to the room named like its own id now expels every other member of that room under the same write
  lock (`Membership::expel_others`, documented on `Adapter::add_all` for other adapters). Test:
  `the_own_room_of_a_connecting_socket_expels_whoever_joined_its_id_during_the_middleware`.
- **Lead: `Debug` of the handshake** printed the values of the headers outside a three-name deny list (`x-api-key` and the
  like). It is now an allow list of nine names without credentials (`host`, `user-agent`, `origin`, `accept`,
  `accept-encoding`, `accept-language`, `connection`, `upgrade`, `sec-websocket-version`); every other header prints
  `<hidden>`. Test: `debug_output_hides_credentials`.
- **Lead: adapter contract.** The `Adapter` docs now say that `room_exists` defaults to `false` (a distributed adapter must
  implement it, or the id rule is silently off) and that the synchronous methods cannot be bounded by `adapter_timeout`, so
  they must not block. The review notes of the change of #36 (`e2-socket-io-redis-adapter`) were extended: implement
  `room_exists`, target exactly one socket through its own field, redact the handshake (query and credential headers) before
  serializing it.

Recorded, not fixed: a server shutdown leaves calls holding their answers until their deadline (bounded by the deadline;
Node alike); a hanging synchronous adapter method pins a runtime worker for the whole hang (documented above).

### Not done by this implementation

Task 6.3 (the security review), 7.6 (the `corvis-parity` check), 7.7 and the archive and GitHub steps belong to the
maintainer's session.

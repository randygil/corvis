# Design

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

`fetch_sockets()` returns `Vec<RemoteSocket>`: `id()`, `handshake()`, `rooms()`, `data()`, and `emit`, `timeout`,
`join`, `leave`, `disconnect(close)` implemented through an operator on the room of the id, exactly as Node, so they
keep working when #36 returns sockets of other nodes. `sockets_join` / `sockets_leave` apply the room limits per
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
| Rooms named in one `to` / `except` chain | not capped | not capped | cost linear in the application's own list |

Validation in `IoOptions` (error naming the option): zero `max_rooms_per_socket`, `max_room_name_length` or
`max_broadcast_ack_bytes`; `max_room_name_length` below 20 (a socket id must fit).

Not capped on purpose: the rooms of one `to` / `except` chain. The list is built by the application (for instance the
followers of a user), the server already holds it, and selection is linear in the list plus the members reached.

Client-observable divergences (through what the scenario app acknowledges): the 1,001st room of a socket and a room
name of 257 bytes. Each gets a scenario and an entry in `expected-differences.json`. The other rows equal Node or are
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

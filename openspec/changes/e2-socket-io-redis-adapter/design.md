# Design

## Context

Design source: the Socket.IO spike #31 (`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`):
Decision 1 (compatibility with `@socket.io/redis-adapter` is why Corvis owns the implementation), Decision 2
(`redis.rs` behind its own feature), Decision 4 (Redis scenarios run only with `CORVIS_REDIS_URL`; mixed clusters
must deliver what two Node instances deliver), its Context notes on the adapter, Review note 4 ("Redis client: the
`redis` crate, same as the queues spike, #49") and its delta `specs/websockets/socket-io-cluster`, the starting point
of this change's delta, adjusted below. Code verified at `e8218d3` (`origin/dev`).

### What exists on `origin/dev`

- No Socket.IO code yet: `crates/corvis/src/socket_io/` and the `socket-io` feature arrive with #32
  (`crates/corvis/Cargo.toml:56-62` lists `jwt`, `websockets`, `tls`, `health-tls`, `compression`).
- No Redis client and no MessagePack crate: `Cargo.lock` has no `redis`, `fred`, `rmp` or `rmpv` package. The queues
  spike recorded the candidates on 2026-11-08 (`openspec/changes/archive/2026-11-08-e4-spike-bullmq-data-format-in-redis/design.md:19`):
  `redis` 1.8.1 (released 2026-10-30, MSRV 1.88) and `fred` 10.1.0 (last release 2025-02-27).
- Dependency policy (`deny.toml`): licenses allowed are MIT, Apache-2.0 (also with the LLVM exception), BSD-2-Clause,
  BSD-3-Clause, ISC, Zlib, Unicode-3.0, BSL-1.0, Unlicense, CDLA-Permissive-2.0 (`:23-26`); the graph is evaluated
  with all features (`:4`); unmaintained and unsound advisories are reported (`:10-11`); only crates.io sources
  (`:42-45`). `THIRD_PARTY_NOTICES.md` lists third-party code.
- CI (`.github/workflows/ci.yml`): the Linux `test` job declares a `postgres:16` service container on port 55432 with
  a health check (`:81-92`), exports `CORVIS_ORM_PG` and `CORVIS_ORM_PG_REQUIRED: "1"` so the Postgres tests fail
  instead of skipping (`:96-98`); the Windows job runs the same tests without a server, and they skip (`:136`). The
  `clippy` job builds each feature on its own in a loop (`:71-73`); `deny` runs `cargo deny check` (`:194-196`). No
  job mentions Redis.
- Harness (#131): `parity/socket-io/package.json` pins `@socket.io/redis-adapter` 8.3.0 and `redis` 6.3.1 (the Node
  client) next to `socket.io` 4.8.4; `lib/targets.mjs` knows the capability `redis` (`:8`) and skips scenarios that
  require it unless `CORVIS_REDIS_URL` is set (`plan`, `:127-137`; main spec `ws/socket-io-parity`, "Redis scenarios
  need CORVIS_REDIS_URL"). There is no Redis scenario and the reference server does not use the adapter yet.

### The layers this builds on (as designed, not yet merged)

- #34 (`e2-socket-io-rooms-and-broadcast`, Decision 2): one adapter per namespace, created by the factory in
  `IoOptions::adapter(factory)` with a `LocalNamespace` (name, connected sockets by id, the ack id counter). The trait:
  `init`, `close`, `server_count`; synchronous local membership (`add_all`, `del`, `del_all`, `socket_rooms`);
  asynchronous, fallible `broadcast(packet, opts)`, `broadcast_with_ack(packet, opts, sink)`, `sockets`,
  `fetch_sockets -> Vec<SocketInfo>`, `add_sockets`, `del_sockets`, `disconnect_sockets`. `BroadcastOptions { rooms,
  except, flags }` with `BroadcastFlags { local, timeout }`; `AckSink::expect(clients)` once per server and
  `respond(args)` per answer, with the operator owning the deadline, the completeness rule (`server_count` servers
  reported) and `max_broadcast_ack_bytes` (16 MiB); `SocketInfo { id, handshake, rooms, data }`; `RemoteSocket`
  operations go through an operator on the room of the socket's id. `InMemoryAdapter`'s local operations take `&self`
  so a cluster adapter can hold one. Limits: `max_rooms_per_socket` 1,000, `max_room_name_length` 256 B. Its Review
  note 6: trait and in-memory adapter are `#[doc(hidden)]` until this change. Its tests include a wrapper adapter
  that simulates a second node.
- #33 (`e2-socket-io-namespaces-events-and-acks`): `Packet` (crate-private), `IoValue` / `Binary` / `Args`,
  `max_attachments` 10, JSON nesting depth 128, `max_pending_acks` 1,000, `ack_timeout` 30 s, the reserved event
  names, namespace names of at most 256 bytes without `,` or control characters, socket ids of 20 base64url
  characters.
- #32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`): each session's outbox is bounded (8 MiB) and a send
  never waits; `max_http_buffer_size` 1,000,000 B.

### The Node adapter (read from the pinned package)

Read on 2026-11-10 from the packages the harness lockfile installs: `@socket.io/redis-adapter` 8.3.0
(`dist/index.js`, `dist/util.js`), `notepack.io` 3.0.1 (`lib/encode.js`, `lib/decode.js`), `uid2` 1.0.0,
`socket.io-adapter` 2.5.8 (`dist/in-memory-adapter.js`), `socket.io` 4.8.4 (`dist/namespace.js`, `dist/socket.js`).

Identity and channels (constructor of `RedisAdapter`, one per namespace):

- `uid = uid2(6)`: six characters of `A-Za-z0-9-_` from random bytes; request ids are `uid2(6)` too.
- `key` (option, default `socket.io`). `channel = key + "#" + nsp + "#"`, `requestChannel = key + "-request#" + nsp +
  "#"`, `responseChannel = key + "-response#" + nsp + "#"`, `specificResponseChannel = responseChannel + uid + "#"`.
  The namespace keeps its leading `/`.
- Subscriptions: `PSUBSCRIBE channel + "*"` and `SUBSCRIBE` to the three request / response channels, receiving
  buffers.
- Options: `requestsTimeout` (5000 ms), `publishOnSpecificResponseChannel` (false), `parser` (notepack.io).
- No heartbeat and no node registry: `serverCount()` is `PUBSUB NUMSUB requestChannel`, the number of connections
  subscribed to the request channel.

Broadcast (`broadcast`, `onmessage`):

- Unless `flags.local`, publish `msgpack([uid, packet, {rooms: [...], except: [...], flags}])` to `channel`, or to
  `channel + room + "#"` when the broadcast targets exactly one room; then deliver locally. `packet` is
  `{type: 2, data: [event, ...args], nsp}`; binary arguments stay inline as MessagePack `bin`, and the packet type
  stays 2 (the Socket.IO encoder turns it into a binary event when it writes to a client).
- On a message: the channel must start with `channel`; `room = channel.slice(channel.length, -1)`; if the room is not
  empty and no local socket is in it, the message is ignored before decoding; then decode, ignore own `uid`, default
  `packet.nsp` to `/`, ignore when it is not this namespace, and deliver with the in-memory logic.
- Rooms can be numbers in Node (`io.to(42)`): they are encoded as MessagePack integers and appear in the channel as
  their decimal text.

Requests (`onrequest`, `onresponse`), on `requestChannel`; a payload whose first byte is `{` (0x7b) is JSON,
anything else MessagePack:

| Type | Sent as | Fields | Answer |
|---|---|---|---|
| 0 `SOCKETS` | not sent by 8.3.0 | `requestId`, `rooms` | JSON `{requestId, sockets: [id]}` |
| 1 `ALL_ROOMS` | JSON | `uid`, `requestId` | JSON `{requestId, rooms: [room]}` |
| 2 `REMOTE_JOIN` | JSON | `uid`, `opts: {rooms, except}`, `rooms` | none (legacy form with `sid`, `room`: `{requestId}`) |
| 3 `REMOTE_LEAVE` | JSON | `uid`, `opts`, `rooms` | none (legacy form as above) |
| 4 `REMOTE_DISCONNECT` | JSON | `uid`, `opts`, `close` | none (legacy form as above) |
| 5 `REMOTE_FETCH` | JSON | `uid`, `requestId`, `opts` | JSON `{requestId, sockets: [{id, handshake, rooms, data}]}` |
| 6 `SERVER_SIDE_EMIT` | JSON | `uid`, `data: [event, ...args]`, `requestId` when an ack is expected | JSON `{type: 6, requestId, data: arg}` on `responseChannel` |
| 7 `BROADCAST` | MessagePack | `uid`, `requestId`, `packet`, `opts: {rooms, except, flags}` | type 8 and type 9 below |
| 8 `BROADCAST_CLIENT_COUNT` | JSON (answer) | `requestId`, `clientCount` | |
| 9 `BROADCAST_ACK` | MessagePack (answer) | `requestId`, `packet` (the first argument of the client's ack) | |

- Answers go to `responseChannel`, or to `responseChannel + request.uid + "#"` when
  `publishOnSpecificResponseChannel` is set; every node subscribes to both. `SERVER_SIDE_EMIT` answers always go to
  `responseChannel`.
- A node ignores a request whose `requestId` is one of its own pending requests, and a `SERVER_SIDE_EMIT` with its own
  `uid`.
- `fetchSockets`: local sockets first; `numSub = serverCount()`; with one server the local result is returned;
  otherwise the request is published and completes when `msgCount` (starting at 1 for itself, incremented by every
  answer, whatever its content) equals `numSub`, or fails after `requestsTimeout` with
  `timeout reached while waiting for fetchSockets response`. A fetched socket's `handshake` is the whole Socket.IO
  handshake object without `sessionStore`: `headers` (every request header, cookies and authorization included),
  `time`, `address`, `xdomain`, `secure`, `issued`, `url`, `query`, `auth`.
- `serverSideEmit` with an ack: expects `serverCount() - 1` answers; after `requestsTimeout` the ack gets
  `timeout reached: only N responses received out of M` and the answers so far. Binary arguments are not supported
  (JSON). The in-memory adapter throws `this adapter does not support the serverSideEmit() functionality`.
- `broadcastWithAck`: the requester publishes the `BROADCAST` request, keeps `ackRequests[requestId]` for
  `flags.timeout` ms, and runs the local broadcast with ack. Each other node runs the in-memory
  `broadcastWithAck`, publishes its `clientCount` and one `BROADCAST_ACK` per answering client. Only the first
  argument of a client's ack crosses (`ack.apply(this, packet.data)` into a one-parameter callback). On the answering
  nodes nothing removes the pending ack of a client that never answers.
- `addSockets`, `delSockets`, `disconnectSockets`: unless `flags.local`, the sender only publishes the request and
  applies nothing directly. These request types are not filtered by `uid`, so the sender's own subscription delivers
  the request back to it and it is applied there as on every other node.
- Errors of the Redis clients are not handled by the adapter; a decode error of a request or an answer is swallowed,
  a decode error of a broadcast is not caught.

`notepack.io` 3.0.1 encoding: integers in the smallest form, any non-integer number as float 64, strings as
`fixstr` / `str 8` / `str 16` / `str 32`, `Buffer` and `ArrayBuffer` views as `bin`, `undefined` as `nil`, object
properties whose value is `undefined` or a function omitted (so `flags` can be absent), `Date` as the timestamp
extension (type -1), objects with `toJSON` through it. Decoding: timestamps become dates; extension type 0 is a
legacy `ArrayBuffer` (`ext 8`), `undefined` (`fixext 1`) or date (`fixext 8`); any other extension becomes
`[type, bytes]`.

Not verified here and left to the mixed-cluster scenarios: how `flags` looks for every operator combination
(`socket.broadcast`, `timeout`), and the exact `handshake.time` text.

## Goals / Non-Goals

**Goals:**

- A Corvis instance and a Node instance with `@socket.io/redis-adapter` 8.3.0 on the same Redis deliver the same
  broadcasts and answer each other's requests, verified by mixed-cluster scenarios compared with two Node instances.
- Every byte read from Redis is treated as untrusted: bounded work and memory per message, no panic, no state that
  grows without a limit, and never a wider delivery than the message asked for.
- Losing Redis never breaks local delivery or the process; the adapter recovers by itself.
- The adapter trait is public after being implemented twice.

**Non-Goals:**

- Sharded pub/sub, Redis Cluster / Sentinel, the streams adapter, the emitter, replay of missed broadcasts, a session
  store or queues on Redis (proposal.md, Out of scope).
- Byte-equal MessagePack: any valid encoding that `notepack.io` decodes to the same value is acceptable; fixtures
  compare decoded structures.

## Decisions

### 1. Redis client crate: `redis` (redis-rs)

| Criterion | `redis` (redis-rs) | `fred` |
|---|---|---|
| Maintenance (crates.io, 2026-11-08, from the queues spike) | 1.8.1 released 2026-10-30, 2.0 release candidates in progress | 10.1.0, last release 2025-02-27 |
| License | BSD-3-Clause, allowed by `deny.toml` | MIT, allowed |
| Async on Tokio | yes (`tokio-comp`) | yes |
| Pub/sub | `get_async_pubsub()` (RESP2, binary-safe channels and payloads), also RESP3 push on multiplexed connections | yes, with built-in resubscription |
| Reconnect | `ConnectionManager` for commands; RESP2 pub/sub connections do not resubscribe by themselves | built in |
| Already decided for other epics | queues spike #49 (through the BullMQ crate, `redis ^1.7.1`), spike #31 Review note 4 | no |
| MSRV | 1.88; the workspace toolchain is newer | n/a |

Decision: `redis`, confirming Review note 4 of the spike. `fred`'s built-in resubscription is its one advantage here,
and a crate without a release in twenty months would trip the `unmaintained` policy sooner or later; one Redis client
for the whole workspace (queues, the session store planned after #44, this adapter) matters more. The dependency is
optional, with default features off and only what is used: the Tokio runtime integration, the async pub/sub
connection and the connection manager (`tokio-comp`, `aio`, `connection-manager`; exact feature names are confirmed
against the manifest of the pinned version in task 1.2). `rediss://` URLs need the crate's rustls integration; it is
enabled only when Corvis' own `tls` feature is also on (`tls = [..., "redis?/<rustls feature>"]`), and a `rediss://`
URL without it fails with an error naming the feature.

The subscriber uses RESP2 (`SUBSCRIBE` / `PSUBSCRIBE` on a dedicated connection), which is what the Node adapter's
clients use and works on every Redis 6 or later; the reconnect and resubscribe loop is Corvis code (Decision 6), so
its behavior is specified and tested here instead of depending on a client feature tied to RESP3.

Alternatives considered: `fred` (above); a minimal RESP2 subscriber written in Corvis so that oversized messages
could be skipped without buffering them (rejected: authentication, TLS and protocol edge cases re-implemented for one
bound that Redis' own `proto-max-bulk-len` and network placement address; see Decision 10); depending on RESP3
automatic resubscription of the connection manager (rejected: RESP3 only, and the behavior during the gap would be
the client's, not ours).

### 2. MessagePack: `rmp` with a bounded codec written against `IoValue`

`rmp` (MIT; marker, integer, float and length primitives) plus a codec in `socket_io/redis/codec.rs` that reads
straight into `IoValue` and the adapter's message structs, and writes what `notepack.io` writes.

- Decoding rules: `nil` → `Null`; booleans; every integer form → `Number`; float 32 / 64 → `Number`, with NaN and
  infinities → `Null` (what `JSON.stringify` makes of them when Node sends the packet to a client); `str` must be
  valid UTF-8; `bin` → `Binary`; arrays; maps with string keys only (insertion order kept); timestamp extensions
  (type -1, and notepack's legacy date) → the ISO 8601 text `Date.prototype.toISOString` gives, which is what a client
  of a Node instance would receive; notepack's legacy `undefined` → `Null` and legacy `ArrayBuffer` → `Binary`; any
  other extension, a non-string map key, invalid UTF-8, a reserved marker, trailing bytes or truncated input → the
  whole message is rejected.
- Bounds while decoding: nesting depth at most 128 (the JSON bound of #33); no allocation sized by a declared length
  (collections grow as elements arrive, and a declared length larger than the bytes left is an error before reading
  elements); the input itself is at most `max_message_bytes` (Decision 10). Decoding is iterative or depth-checked, so
  the stack is bounded.
- Encoding rules: integers in the smallest form; a `Number` that holds an integral float within ±2^53 as an integer
  and other floats as float 64 (JavaScript has one number type, so Node reads the same value either way); strings
  with `str 8` where it fits; `Binary` as `bin`; objects as maps in insertion order; `Null` as `nil`.

Alternatives considered: `rmpv` (a dynamic `Value` that would be converted to `IoValue` afterwards: two allocations
per node, and its recursion and pre-allocation behavior would have to be audited and pinned); `rmp-serde` (binary
versus array is ambiguous through serde, `IoValue` needs a custom `Deserialize`, and the bounds are harder to state);
a hand-written codec without `rmp` (MessagePack is small, but the number and length primitives are exactly what
`rmp` already tests). The version is the current `rmp` release at implementation time, recorded in Implementation
notes.

### 3. Feature, files and the public entry point

`socket-io-redis = ["socket-io", "dep:redis", "dep:rmp"]` in `crates/corvis/Cargo.toml`, off by default. Layout:

```
socket_io/redis/mod.rs          RedisAdapter, RedisState, RedisAdapterStats; re-exports
socket_io/redis/options.rs      RedisAdapterOptions, defaults, validation
socket_io/redis/connection.rs   URL handling (credentials never logged), publisher, subscriber loop, reconnect
socket_io/redis/channels.rs     channel names, pattern escaping, room extraction
socket_io/redis/codec.rs        MessagePack <-> IoValue and the message structs; JSON request / response structs
socket_io/redis/requests.rs     pending requests, remote ack trackers, incoming request queue
socket_io/redis/adapter.rs      the per-namespace Adapter implementation on top of InMemoryAdapter
socket_io/redis/fuzz_codec.rs   proptest properties (selected by `cargo test --lib fuzz`)
```

```rust
let redis = RedisAdapter::connect("redis://127.0.0.1:6379", RedisAdapterOptions::new()).await?;
let io = IoServer::new(IoOptions::new().adapter(redis.clone()))?;
// with gateways (#35): app.use_websocket_adapter(IoAdapter::new().options(IoOptions::new().adapter(redis)));
```

- `RedisAdapter` is a cheap handle to one connection pair (publisher and subscriber), shared by every namespace of
  the server it is given to, like the `pubClient` / `subClient` pair of the Node adapter. It implements the adapter
  factory of #34: each namespace gets an internal `RedisNamespaceAdapter` with its own `uid`, registered in the
  handle's subscription registry.
- One handle serves one `IoServer`. `IoServer::new` with a handle that another live server already holds fails with
  an error naming the adapter: two servers on one handle would share one subscription per request channel, so
  `PUBSUB NUMSUB` would count them as one instance and every operation that waits for `server_count` answers would
  be wrong (Decision 4). A process with several servers (tests, or a gateway application with two Engine.IO paths,
  #35) creates one `RedisAdapter` per server; servers that must not see each other's broadcasts also need different
  `key`s, as two Node servers on one Redis would.
- `RedisAdapter::connect(url, options)` waits for the first connection of both sides, at most `connect_timeout`
  (10 s), and fails otherwise: a wrong URL or password should stop the application at startup.
  `RedisAdapter::connect_lazy(url, options)` returns at once and connects in the background, for deployments that
  must start without Redis.
- `state() -> RedisState` (`Connecting`, `Connected`, `Disconnected`), `stats() -> RedisAdapterStats` (messages
  published and received, dropped by reason: oversized, malformed, queue full, not connected; reconnects), and
  `close()`. A health indicator can read them.
- `Debug` and every log line show the URL without user information.

### 4. Channels and identity

- `uid` and request ids: six characters of `A-Za-z0-9-_` from `OsRng`, as `uid2(6)`. Node treats both as opaque
  strings; keeping the length makes a Corvis node indistinguishable on the wire.
- Channel names exactly as the Node adapter builds them, with `key` from `RedisAdapterOptions::key` (default
  `socket.io`; validated: not empty, at most 128 bytes, no control characters).
- The pattern subscription escapes the glob metacharacters (`*`, `?`, `[`, `]`, `\`) of the key and the namespace
  before appending `*`. Node does not, so a namespace such as `/a*` subscribes it to more channels than its own; the
  per-message namespace check makes that harmless there, and Corvis keeps the same check: the channel must start with
  the adapter's prefix and end with `#`, and the decoded `packet.nsp` (default `/`) must equal the namespace.
- Room filter before decoding, as Node: the text between the prefix and the final `#` is the room; when it is not
  empty and no local socket is in that room, the message is dropped without decoding.
- One subscriber connection carries the subscriptions of every namespace, and each namespace subscribes to its
  request channel exactly once, so `PUBSUB NUMSUB` counts a Corvis instance once, as it counts a Node instance.

### 5. Messages

Outgoing, identical in shape to the table in Context:

- Broadcast: `[uid, {type: 2, data: [event, ...args], nsp}, {rooms, except, flags}]`, to the room channel when there
  is exactly one room. `flags` is a map holding only what #34's `BroadcastFlags` carries (`timeout` in
  milliseconds); never `local` (a local broadcast is not published).
- `REMOTE_FETCH`, `REMOTE_JOIN`, `REMOTE_LEAVE`, `REMOTE_DISCONNECT`, `SERVER_SIDE_EMIT`: JSON with the fields of the
  table. `BROADCAST`: MessagePack. Answers: JSON, except `BROADCAST_ACK` (MessagePack, `packet` is the first argument
  of the client's acknowledgement or `nil`).
- Answers are published on the common response channel unless
  `RedisAdapterOptions::publish_on_specific_response_channel(true)` (default false, as Node, so every Node version of
  the 8.x line receives them); Corvis subscribes to both.

Incoming, validated before anything is done (a message that fails a check is dropped whole, counted in
`stats().dropped_malformed` and logged at most once every 10 seconds per reason, without its content):

- Broadcast: a three-element array; `uid` a string (own `uid`: stop before decoding the rest); `packet` a map with
  `type` 2 (5 is accepted and treated the same), `data` a non-empty array whose first element is a string or a
  number, not one of Socket.IO's reserved event names (the official client drops its connection when it receives one
  as an event), at most `max_attachments` binary values (what a single emit allows, #33), `id` ignored; `opts` a map
  whose `rooms` and `except` are arrays of strings or integers (integers become their decimal text, matching rooms a
  Node node addressed by number), every name within `max_room_name_length`. An invalid room or except entry rejects
  the message; it is never skipped, because dropping an `except` entry or reducing `rooms` to an empty list would
  deliver to more sockets than the sender asked for (the rule of #34 Decision 5).
- Requests: JSON object or MessagePack map by the first byte; `type` an integer 0-7; `uid` and `requestId` strings of
  at most 64 bytes; `opts.rooms`, `opts.except`, `rooms` as above; `close` a boolean when present. Own `uid` or a
  `requestId` this node is waiting on: ignored. Types 0 and 1 are answered (cheap, and older peers send them); the
  legacy `sid` forms of types 2-4 are ignored (proposal.md, Out of scope).
- Answers: `requestId` must be pending on this node, and the answer's shape must fit the pending request's type
  (`sockets` an array of objects with a string `id` for a fetch; `clientCount` a non-negative integer of at most
  `max_sessions` magnitude for a broadcast); anything else is ignored.

Delivery of an accepted broadcast is the in-memory adapter's (`InMemoryAdapter::broadcast` with the decoded packet
and options): one shared encoding, per-recipient bounded outboxes, never waiting on a recipient (#34).

### 6. Connections, publishing and reconnect

- Publisher: one connection (the crate's connection manager, which reconnects for commands) behind a single
  publisher task fed by a queue bounded in bytes (`max_publish_queue_bytes`, 16 MiB). `broadcast`, the bulk
  operations and answers enqueue and return; the task sends in order, so two emits of one task keep their order on
  the wire, as with Node's single `pubClient`. When the queue is full or the state is not `Connected`, the message is
  dropped, counted (`dropped_queue_full`, `dropped_not_connected`) and logged with the rate limit; nothing is kept
  for later. Node's client would queue commands while offline without bound and replay stale broadcasts on
  reconnect.
- A plain broadcast therefore returns `Ok` once it was delivered locally and handed to the queue: cross-instance
  delivery is best effort, as in Node, where a publish error is not reported to `emit` either. Operations that need
  an answer are different: `fetch_sockets`, `emit_with_ack`, `server_side_emit_with_ack` and `server_count` fail with
  `AdapterError::Unavailable` at once when the state is not `Connected`, and with the timeout error after
  `requests_timeout` when the request was sent and not every instance answered.
- Order of a broadcast: validate and encode, check the size (`AdapterError::MessageTooLarge` before anything is
  delivered, so a broadcast is never delivered locally and silently not to the cluster because of its size), enqueue,
  deliver locally.
- `server_count` is `PUBSUB NUMSUB <requestChannel>` on the publisher connection with `requests_timeout` as its
  deadline; not cached (Node asks every time).
- Subscriber: one task owning one pub/sub connection. It only reads: broadcasts are delivered inline (the in-memory
  delivery never waits), answers are matched inline, and requests are handed to a bounded queue
  (`max_queued_requests`, 1,024) drained by a worker, because answering needs a publish. A request that does not fit
  is dropped and counted; its sender times out. The task never awaits anything but the socket, so a backlog builds in
  the kernel buffer and then in Redis, whose `client-output-buffer-limit pubsub` disconnects a subscriber that cannot
  keep up; that disconnect is handled like any other.
- Reconnect: when the subscriber connection ends or errors, the state becomes `Disconnected`, pending requests are
  left to their deadlines, and the task reconnects with exponential backoff from `reconnect_min_delay` (100 ms) to
  `reconnect_max_delay` (5 s) with jitter, without a retry limit. After connecting it subscribes again for every
  namespace in the registry (the registry, not the lost connection, is the source of truth), and only then the state
  becomes `Connected`. Broadcasts published by other instances during the gap are lost: Redis pub/sub is at most
  once, and the Node adapter behaves the same. The publisher's state is tracked the same way; `state()` is
  `Connected` only when both are.
- `Adapter::init` registers the namespace and, when connected, waits for the subscription to be confirmed, so a
  server that starts with Redis up never misses a message published after `init` returned. `Adapter::close`
  unsubscribes, removes the namespace from the registry and fails its pending requests with `AdapterError::Closed`.
  `RedisAdapter::close()` and the server's shutdown do it for every namespace and end both tasks.
- Each connection sets a client name (`RedisAdapterOptions::client_name`, default `corvis-socket-io`), which shows in
  `CLIENT LIST` for operators and lets tests cut exactly their own connections.

### 7. Cluster operations on the trait

- `broadcast`: Decisions 5 and 6. With `flags.local`, only the in-memory delivery.
- `broadcast_with_ack(packet, opts, sink)`: with `flags.local`, in-memory only. Otherwise a request id is taken, the
  `BROADCAST` request is published, an entry `request id → sink` is kept until the operator's deadline
  (`flags.timeout`, which #34 always sets), and the in-memory `broadcast_with_ack` runs with the same sink.
  `BROADCAST_CLIENT_COUNT` answers call `sink.expect(count)`, `BROADCAST_ACK` answers `sink.respond([value])`. The
  operator of #34 completes when `server_count` servers reported and every counted client answered, fails at the
  deadline with the partial answers, and enforces `max_broadcast_ack_bytes`.
- Answering a `BROADCAST` request: run the in-memory `broadcast_with_ack` with a sink that publishes the count and
  each answer. The tracker lives until the request's `flags.timeout`, clamped to at most 10 minutes, or `ack_timeout`
  (30 s, #33) when the request carries none or an invalid one; then the pending acknowledgements it registered on
  local sockets are removed. Node leaves them until the socket disconnects. At most `max_remote_ack_requests` (1,000)
  trackers per namespace; a request above it is answered with a client count of 0 and delivers nothing.
- `fetch_sockets`: local sockets; with `flags.local` or one server, done. Otherwise publish `REMOTE_FETCH` and wait
  for `server_count - 1` answers or `requests_timeout`; the collected answers are bounded by `max_response_bytes`
  (16 MiB) per request. Timeout text as Node: `timeout reached while waiting for fetchSockets response`. Remote
  sockets become `SocketInfo`s parsed tolerantly from Node's handshake object (unknown fields ignored, missing ones
  empty).
- Answering `REMOTE_FETCH`: the local sockets that match, as `{id, handshake, rooms, data}` with the handshake in
  Node's field names. By default the handshake is redacted (Decision 8). If the encoded answer is larger than
  `max_message_bytes` nothing is published and an error is logged; the requester times out, which is an honest
  failure where a truncated list would be a wrong answer.
- `add_sockets`, `del_sockets`, `disconnect_sockets`: with `flags.local`, in-memory only; otherwise apply locally and
  publish the request (Node applies its own request when it comes back through its subscription; Corvis ignores its
  own `uid`, so it applies first; the observable result is the same). Requests from other nodes are applied with
  #34's limits: a socket at its room cap is skipped and logged.
- `sockets(rooms)`: local only, as the Node adapter 8.3.0 (it no longer sends `SOCKETS`).
- `server_count`: Decision 6.
- Server-side emit, new in this change. On the trait: `server_side_emit(args)` and
  `server_side_emit_with_ack(args) -> Result<Vec<IoValue>, ServerSideEmitError>`, with default implementations that
  fail with `AdapterError::Unsupported` and Node's text (`this adapter does not support the serverSideEmit()
  functionality`), which is what `InMemoryAdapter` keeps. On the API: `IoNamespace::server_side_emit(event, args)`,
  `server_side_emit_with_ack(event, args)`, `on_server_side_emit(event, handler)` (handler:
  `Fn(Args, Option<ServerSideAck>)`), and the same on `IoServer` for `/`. Reserved event names and binary arguments
  are rejected before publishing (the wire format is JSON). With an ack: `server_count - 1` answers are expected; none
  expected completes at once with an empty list; the deadline is `requests_timeout`, failing with
  `timeout reached: only N responses received out of M` and the answers so far; answers are bounded by
  `max_response_bytes`. Incoming: handlers run on the request worker under panic containment; `ServerSideAck::send`
  consumes the ack, so an instance answers at most once; an event without a handler is ignored.
- Pending requests (fetch, server-side emit with ack, broadcast with ack) are capped at `max_pending_requests`
  (1,000) per namespace; above it the call fails at once with `AdapterError::TooManyRequests` and nothing is
  published.

### 8. Trust boundary and the handshake redaction

Whoever can publish on the Redis used by the adapter can, by design of the protocol, emit any event to any client,
join sockets to rooms and disconnect them. No validation in Corvis can change that, and the docs say so plainly:
the Redis instance belongs inside the application's trust boundary (authentication, TLS, network isolation, and an
ACL limited to the adapter's channels when the Redis is shared). What Corvis guarantees for anything read from Redis
is narrower and absolute: bounded memory and work, no panic, no delivery wider than the message asked for, no
reserved event forwarded to clients, and local state (rooms, pending tables) that stays within the limits of #33 and
#34.

One behavior of the Node adapter is a disclosure rather than a protocol property: answering `REMOTE_FETCH` publishes
every matching socket's full handshake, including the `cookie` and `authorization` request headers and the `auth`
payload, on a pub/sub channel that every subscriber of that Redis can read. Corvis diverges (security over parity):
`RedisAdapterOptions::shared_handshake(SharedHandshake::Redacted)` is the default and removes the `cookie`,
`authorization` and `proxy-authorization` headers and sends `auth` as an empty object; `SharedHandshake::Full` sends
what Node sends, for applications whose other instances read those fields from remote sockets. The query string and
the URL are sent in both modes; the docs warn against credentials in them. What Node instances publish is outside
Corvis' control. Listed as an expected difference with its scenario.

### 9. Making the adapter interface public

After this change the items of #34 lose `#[doc(hidden)]` and get rustdoc: `Adapter`, `InMemoryAdapter`,
`LocalNamespace`, `BroadcastOptions`, `BroadcastFlags`, `AckSink`, `SocketInfo`, `Room`, `SocketId`, `AdapterError`,
`Packet`. Adjustments the second implementation needs, made before publishing (task 2.1 compares them with the code
#34 merged):

- `Packet` gets what an adapter outside the crate needs and no more: `Packet::event(name, args)`, `event_name()`,
  `args()`, `namespace()`; its wire encoding stays private.
- `AdapterError` is `#[non_exhaustive]` and gains `Unavailable`, `Timeout { operation, received, expected }`,
  `TooManyRequests`, `MessageTooLarge`, `Unsupported`, `Closed`.
- `server_side_emit` and `server_side_emit_with_ack` with default implementations (Decision 7), so they do not break
  an existing implementation; the rustdoc states that the trait may gain defaulted methods in minor versions and
  required methods only in a major one.
- `LocalNamespace` gains what answering requests needs if #34 did not already provide it: list every local room,
  dispatch a server-side emit to the namespace's handlers.
- The adapter factory of `IoOptions::adapter` gains a defaulted method that `IoServer::new` calls once with the
  server's validated options and whose error fails `IoServer::new`. The Redis factory uses it to compare
  `max_message_bytes` with the server's `max_http_buffer_size` (Decision 10) and to refuse a second server
  (Decision 3); the in-memory factory keeps the default, which accepts.
- The Redis adapter holds an `InMemoryAdapter` for membership and local delivery, the composition #34 planned.

### 10. Limits and their defaults

| Limit | Default | Node adapter 8.3.0 | When exceeded |
|---|---|---|---|
| `max_message_bytes`: one message from Redis, and one message Corvis publishes | 16 MiB (validated: at least `max_http_buffer_size`) | none (Redis: 512 MB) | incoming: dropped and counted; outgoing: `AdapterError::MessageTooLarge`, nothing delivered |
| MessagePack / JSON nesting depth | 128 | none | message dropped |
| Declared lengths (arrays, maps, strings, binary) | never pre-allocated; must fit in the bytes left | n/a | message dropped |
| Binary values in one remote packet | `max_attachments`, 10 (#33) | none | message dropped |
| Room and except names in remote options | `max_room_name_length`, 256 B (#34); an invalid entry rejects the message | none | message dropped |
| Number of rooms / except entries in one message | bounded by `max_message_bytes` only (as the local operators, #34) | none | n/a |
| Reserved event names from Redis | rejected | forwarded | message dropped |
| `requests_timeout` | 5 s (validated: not zero, at most one day) | 5 s | the operation fails with the timeout error |
| `max_pending_requests` per namespace | 1,000 | none | `AdapterError::TooManyRequests`, nothing published |
| `max_response_bytes` collected per request (fetch, server-side emit) | 16 MiB | none | the operation fails with the answers so far |
| Broadcast acknowledgement answers | `max_broadcast_ack_bytes`, 16 MiB (#34) | none | as #34 |
| `max_queued_requests` (incoming requests awaiting the worker) | 1,024 | none | request dropped and counted |
| `max_remote_ack_requests` (trackers for other nodes' broadcasts with ack) | 1,000 per namespace; lifetime `flags.timeout` clamped to 10 min, or 30 s | none, never removed | answered with client count 0 |
| Pending acks registered on local sockets for a remote request | count in `max_pending_acks`, 1,000 per socket (#33); removed at the tracker's deadline | none | that socket is not awaited |
| `max_publish_queue_bytes` | 16 MiB | unbounded offline queue of the client | message dropped and counted |
| `connect_timeout` | 10 s | client default | `connect` fails; a reconnect attempt is abandoned and retried |
| Reconnect backoff | 100 ms to 5 s, jitter, no retry limit | client default | n/a |
| Handshake sent to other instances | redacted (Decision 8) | full | n/a |
| `key` | `socket.io`; not empty, at most 128 B, no control characters | `socket.io` | options rejected |
| Ids from Redis (`uid`, `requestId`) | at most 64 B | none | message dropped |
| Log lines about dropped messages | one per reason every 10 s, never the payload or the URL's credentials | n/a | n/a |

Two bounds are not Corvis' to give and are documented as such. The transient allocation for one incoming message
happens inside the client crate before Corvis sees its size, so it is bounded by Redis (`proto-max-bulk-len`, 512 MB
by default) and not by `max_message_bytes`; the docs recommend lowering it on a dedicated Redis and task 1.2 checks
whether the pinned client version offers a read limit (Open question 3). And the subscriber's backlog is bounded by
Redis' `client-output-buffer-limit pubsub`, provided the client crate applies backpressure to the socket; task 1.2
verifies that, and if the crate buffers internally without bound the finding goes to the security review with the
measured behavior.

`RedisAdapterOptions` validation (the error names the option): zero `requests_timeout`, `max_message_bytes`,
`max_pending_requests`, `max_response_bytes`, `max_queued_requests`, `max_remote_ack_requests` or
`max_publish_queue_bytes`; `max_message_bytes` below the server's `max_http_buffer_size` (checked when the adapter is
attached to a server); `reconnect_min_delay` zero or above `reconnect_max_delay`; an invalid `key`; a URL that is not
`redis://` or `rediss://`.

Client-observable divergences (each with a scenario and an entry in `expected-differences.json`): the redacted
handshake of fetched sockets. The other limits are visible only to server code or only under forged input; they are
documented and tested in Rust.

### 11. Verification

- Codec unit tests and proptest properties (`fuzz_codec.rs`): arbitrary bytes never panic and never allocate beyond a
  multiple of the input; depth bound; declared lengths larger than the input; round trip `IoValue → bytes →
  IoValue`; every message struct round trips. Fixtures: `parity/socket-io/scripts/record-redis-adapter.mjs` drives the
  Node adapter 8.3.0 with stub clients that capture what it publishes (no Redis needed) for every operation (plain
  and single-room broadcast, binary argument, numeric room, each request and answer type) and writes
  `crates/corvis/tests/fixtures/redis_adapter_8.3.0.json` with the package versions inside; Rust tests decode every
  recorded message into the expected structure and check that Corvis' encoding of the same operation decodes to the
  same structure. The fixture test also compares the versions stored in the fixture with the ones
  `parity/socket-io/package-lock.json` pins and fails, naming the recorder script, when they differ, so a version
  bump of the harness cannot leave stale fixtures behind.
- `crates/corvis/tests/socket_io_redis.rs` (`#![cfg(feature = "socket-io-redis")]`): two or three `IoServer`s in one
  process on the Redis of `CORVIS_REDIS_URL`, each test with a unique `key` so tests run in parallel and reruns do not
  interfere; raw Socket.IO clients from #33's test helpers. One test per spec scenario that does not need Node:
  broadcasts (all, room, several rooms, except, from a socket, binary, local flag), bulk operations, `fetch_sockets`
  (both handshake modes), broadcast with acks (all answer, partial with timeout), server-side emit (plain, with ack,
  timeout), server count, the channels seen with `PUBSUB CHANNELS` / `NUMSUB`. Without `CORVIS_REDIS_URL` the tests
  print a skip notice; with `CORVIS_REDIS_REQUIRED=1` they fail instead (the Postgres rule).
- Reconnect tests: a control connection finds the adapter's connections by their client name in `CLIENT LIST` and
  cuts them with `CLIENT KILL ID`; assertions: local broadcasts keep working during the gap, `fetch_sockets` fails
  with `Unavailable`, publishes are dropped and counted, the state returns to `Connected`, the subscriptions are back
  (`PUBSUB NUMSUB`), and a cross-instance broadcast is delivered again. `connect_lazy` against a closed loopback port
  covers the never-connected case, and `connect` against it the startup failure. Backoff timing is tested on the
  paused clock at the unit level.
- `crates/corvis/tests/socket_io_redis_security.rs`: a raw Redis client publishes forged messages on the adapter's
  channels: truncated and malformed MessagePack, depth 129, declared lengths of 2^32 - 1, unknown extensions,
  non-string keys, invalid UTF-8, wrong field types, reserved event names, 11 binary values, a 257-byte room in
  `rooms` and in `except` (nothing delivered), an oversized message, another namespace's packet on this namespace's
  channel, answers for unknown and for other types' request ids, a flood of requests beyond the queue, a flood of
  `BROADCAST` requests beyond the tracker cap, `flags.timeout` of 2^53; and the caller-side caps (pending requests,
  response bytes). Each asserts that nothing was delivered that should not be, the counters moved, the process and the
  other tests' sockets are unaffected, and memory bounds are observed as table and queue sizes, not RSS.
- Harness, mixed cluster: `reference/socket-server.mjs` accepts `--redis-url` and `--redis-key` and installs the Node
  adapter; the Corvis scenario app of `socket_io_diff.rs` takes the same. A scenario with `requires: ["socket-io",
  "redis"]` and `cluster: 2` runs twice: against two Node instances (the reference) and against one Node and one
  Corvis instance, with clients on both and commands issued from both, each run with its own key. Scenarios under
  `parity/socket-io/scenarios/cluster/`: `broadcast` (all, room, except, from socket, binary, numeric room),
  `bulk`, `fetch-sockets` (Corvis in `Full` mode), `fetch-sockets-redacted` (default mode; the expected difference),
  `broadcast-ack` (all answer; one silent with a timeout), `server-side-emit` (plain, with ack), `wire/messages` (a
  raw subscriber records what each instance publishes for the same commands and compares the decoded structures with
  ids normalized). Waits are on marker events, never sleeps.
- CI (Decision 12).

### 12. Redis in CI

The Linux `test` job gets a second service container next to Postgres, declared the same way:

```yaml
      redis:
        image: redis:7
        ports:
          - 56379:6379
        options: >-
          --health-cmd "redis-cli ping"
          --health-interval 5s
          --health-timeout 5s
          --health-retries 20
```

with `CORVIS_REDIS_URL: redis://127.0.0.1:56379` and `CORVIS_REDIS_REQUIRED: "1"` in the job's `env`, and a step
`cargo test -p corvis --features socket-io-redis --no-fail-fast`. The fuzz job adds
`cargo test -p corvis --features socket-io-redis --lib fuzz`; the per-feature clippy loop adds `socket-io-redis`;
`cargo deny check` already evaluates all features. The Windows job has no service containers: the Redis tests skip
there, as the Postgres ones do. The Node-gated mixed-cluster scenarios stay out of CI until #37 adds the
`socket-io-diff` job with its own Redis service.

## Risks / Trade-offs

- #34's trait was designed before this implementation → Decision 9 lists the adjustments; task 2.1 applies them to
  the merged code before the trait is published; the simulated second node of #34's tests keeps passing.
- MessagePack details of `notepack.io` (numbers, `undefined`, dates) → fixtures recorded from the pinned adapter and
  mixed-cluster scenarios instead of assumptions; the codec accepts every standard form on input.
- Best-effort publishing hides Redis outages from `emit` callers → `state()`, counters and rate-limited warnings;
  operations that need answers fail loudly; documented with a health-check example.
- Dropping instead of queueing while disconnected loses broadcasts that Node's client might replay → stale replays
  and an unbounded queue are worse; pub/sub already loses what other instances published in the gap.
- The redacted handshake differs from Node for `fetch_sockets` across instances → option to send the full handshake;
  expected difference; Open question 2.
- The client crate buffers a whole message before Corvis can reject it by size → documented with the Redis setting
  that bounds it; Open question 3.
- A second connection pair per `RedisAdapter` and none shared with future Redis users → the connection helper is
  kept free of Socket.IO types so it can be shared later (proposal note).
- Tests against a real Redis in CI can interfere with each other → unique keys per test, connections cut by client
  name, no `FLUSHALL`, no global `CLIENT KILL TYPE`.
- A Redis image tag (`redis:7`) can move → same policy as `postgres:16`; a digest pin is a follow-up for the CI
  owner if wanted.
- Redis Cluster users cannot use the adapter → stated in the docs with the out-of-scope items.

## Open questions

1. The issue and `ROADMAP.md` name this change `e2-redis-adapter-for-multiple-instances`; the draft uses
   `e2-socket-io-redis-adapter` as assigned. Rename one of them when the change is accepted.
2. Handshake redaction on by default (`cookie`, `authorization`, `proxy-authorization` headers and `auth`), with
   `SharedHandshake::Full` to match Node. Confirm the default and the list. Recommendation: keep; add a custom filter
   only if someone asks.
3. If the pinned `redis` crate offers no read limit for incoming messages, is documenting Redis' `proto-max-bulk-len`
   enough, or should a later issue add a size-limited subscriber? Recommendation: document now; decide after the
   security review measures it.
4. Plain broadcasts return `Ok` when the publish was dropped (Redis unreachable or the queue full), with counters and
   warnings. Alternative: return an error although local delivery happened. Recommendation: `Ok`, as Node, because an
   error would make handlers report a failure to a client that did receive the event.
5. `RedisAdapter::connect` fails startup when Redis is unreachable; `connect_lazy` is the opt-out. Confirm the
   default.
6. CI image: `redis:7` (the server the Node adapter is tested with) or Valkey. Recommendation: `redis:7`; the adapter
   uses only `PUBLISH`, `SUBSCRIBE`, `PSUBSCRIBE`, `PUBSUB NUMSUB`, `CLIENT SETNAME`, so both should work, and Valkey
   can be added to the nightly job later.
7. `rediss://` support through Corvis' `tls` feature (no separate `socket-io-redis-tls` feature). Confirm, and
   confirm which root certificates the client should trust (the platform's or `webpki-roots`, which `health-tls`
   already uses).
8. `max_message_bytes` 16 MiB, `max_pending_requests` 1,000, `max_queued_requests` 1,024,
   `max_remote_ack_requests` 1,000, `max_publish_queue_bytes` 16 MiB: confirm the numbers. All are options.
9. One `RedisAdapter` handle per `IoServer`, a second server on the same handle being an error (Decision 3).
   Alternative: let servers share the connection pair and subscribe once per server, which needs one subscriber
   connection per server anyway for `PUBSUB NUMSUB` to count them. Recommendation: keep the error; a process with
   several servers is rare outside tests.
10. Server-side emit (`server_side_emit`, `server_side_emit_with_ack`, `on_server_side_emit`) is part of the Node
    adapter's protocol but is not named by the issue or by the spike's delta. It is included because a mixed cluster
    in which Corvis ignores type 6 requests would make every Node `serverSideEmit` with an acknowledgement time out.
    Alternative: answer type 6 requests with nothing to dispatch and leave the sending API to a later issue.
    Recommendation: keep it in this change.
11. The legacy single-socket requests (types 2-4 addressed by `sid`) and custom parsers are ignored (proposal, Out of
    scope), which rules out mixed clusters with Node adapters older than the 8.x line. Confirm.

## Review notes

These settle the open questions and override the design where they differ.

1. The change keeps the name `e2-socket-io-redis-adapter`; `ROADMAP.md` is updated to match.
2. Handshake redaction is on by default for `fetch_sockets` answers; `SharedHandshake::Full` opts out.
3. If the `redis` crate offers no read limit for incoming messages, the docs name Redis' `proto-max-bulk-len` as the
   bound and the security review measures the exposure; a finding there may add a limit in this change.
4. A plain broadcast returns `Ok` when its publish was dropped, with a counter and a warning.
5. `connect` fails startup when Redis is unreachable; `connect_lazy` opts out.
6. CI uses the `redis:7` image.
7. `rediss://` is enabled through Corvis' `tls` feature, with no separate feature. The client trusts `webpki-roots`
   by default, as `health-tls` does, so behavior does not depend on the host; an option supplies custom roots
   (private CAs).
8. The default limits are confirmed (16 MiB messages and publish queue, 1,000 pending requests, 1,024 queued
   requests, 1,000 remote ack trackers); all are options.
9. One `RedisAdapter` handle per `IoServer`; a second server on the same handle fails `IoServer::new`.
10. Server-side emit is in scope.
11. Legacy `sid` requests and custom parsers are ignored; mixed clusters need the Node adapter 8.x or later. The
    docs state it.
12. The Redis client chosen here is the one the Redis session store (follow-up of #44) reuses; the choice and its
    pinned version are recorded in Implementation notes.
13. (From the security review of #34.) The adapter implements `Adapter::room_exists` (its default says `false`, which
    silently disables the rule that no socket gets an id that is already a room), targets exactly one socket through its own
    field (never through a room named like its id), and redacts the handshake (the query string and the credential
    headers, not only cookie and authorization) before serializing it: never `serde` a `SocketInfo` as it is. Its
    synchronous methods must not block or wait for the network, because `adapter_timeout` cannot bound them.
14. Implemented after #35 lands. The security review is mandatory before the merge (every message from Redis is
    untrusted input: sizes, MessagePack depth, request floods, ack accumulation, redaction).
15. (From #185, `e2-socket-io-hot-path-and-api-hygiene`, which lands first.) What the adapter trait looks like after it,
    and what this change must take into account (task 2.1 compares with the merged code):
    1. The trait has no test hooks: `membership_size`, `check_invariants` and `has_socket` are gone, and the simulated
       second node and the tests use `corvis::socket_io::testing` (`membership_size`, `check_invariants`, `has_socket`
       on a namespace's in-memory adapter, or `adapter_size` and friends on an `InMemoryAdapter` a test adapter wraps).
       The trait has `Any` as a supertrait, so the tests can find the in-memory adapter.
    2. `AdapterError` and `RoomError` are already `#[non_exhaustive]`; this change only adds its variants.
    3. A socket's own-room `add_all` runs while the socket is joining (`Phase::Joining`): `LocalNamespace::is_live` is true
       for it and an adapter must accept the join; `LocalNamespace::sockets_by_ids` leaves joining sockets out of every
       selection. `del_all` may be called for a socket that never reached `Connected` (the teardown of a joining socket).
    4. No connection lock is held when the adapter runs, but its synchronous methods are still called from the connect
       path and must not block (#34's rule). `InMemoryAdapter::add_all` decides "id of another socket" from its own
       membership map; a Redis adapter that wraps it gets that for free and must not call back into the namespace while
       it holds its own lock.
    5. A failure of the own-room `add_all` refuses the CONNECT (CONNECT_ERROR `Internal server error`) instead of
       disconnecting a connected socket: a Redis adapter that fails there is visible to the client as a refused connection.
    6. `AckSink::respond_charged` receives the session's address key; `respond` (answers from other nodes) is unchanged.
    7. `Packet` reaches the adapter before encoding, unchanged; the shared encoding a broadcast queues is now the Engine.IO
       message with its type byte (`EngineMessage::Shared`). The wire encoding stays private.
    8. The module layout: `socket_io/adapter/mod.rs` (the trait, options, errors, `RoomLimits`, `LocalNamespace`, the
       factory), `socket_io/adapter/memory.rs` (`InMemoryAdapter`), `socket_io/ack.rs` (the answer budget and `AckSink`),
       `socket_io/info.rs` (`HandshakeInfo`, `SocketInfo`); every public path is the same as before.
    9. `IoNamespace::adapter()` is no longer public (`corvis::socket_io::testing::adapter` for the tests). If this change
       wants Node's `namespace.adapter`, it publishes it with the trait.

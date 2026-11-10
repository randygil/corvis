# Design

## Context

Design source: the Socket.IO spike #31 (`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`):
Decision 1 (own implementation), Decision 2 (module layout, `parser.rs`, `namespace.rs`, `socket.rs`), Decision 3
(public API shape), Decision 5 (security divergences), Review notes 1 and 5, and its delta
`specs/websockets/socket-io`, which is the starting point of this change's delta, adjusted below. Code verified at
`6f58ee1` (`origin/dev`).

### What exists on `origin/dev`

- No `crates/corvis/src/socket_io/` yet and no `socket-io` feature (`crates/corvis/Cargo.toml:56-62` lists `jwt`,
  `websockets`, `tls`, `health-tls`, `compression`). Both arrive with #32.
- Gateways speak JSON `{ "event", "data" }` frames (`crates/corvis/src/websockets.rs:1-4`). A binary frame is decoded
  as lossy UTF-8 and parsed as JSON (`run_connection`, `websockets.rs:1051`), so there is no binary payload support.
  The acknowledgement extension echoes an `id` of the incoming frame (`dispatch`, `websockets.rs:895-943`).
- Per-connection protections this change copies in shape: handlers of one connection start in arrival order and run
  concurrently up to `max_inflight` (default 32, `websockets.rs:383`, `:400`; the loop stops reading at the limit,
  `:1017`), handler panics are contained with `catch_unwind` (`:911-921`) and hooks with `guarded`
  (`crates/corvis/src/ws.rs:224-237`), a client carries typed extensions (`WsClient::insert` / `get`,
  `websockets.rs:308-313`) and its address resolved once (`ClientInner.ip`, `:241-242`).
- The send path is `OutQueue` (`ws.rs:80-84`): bounded in bytes, `push` (`:101-116`) accepts a frame when nothing is
  pending and otherwise fails with `PushError::Full` (`:62`) above the limit; one writer task drains it (`write_loop`,
  `:193-221`). Defaults: `DEFAULT_MAX_BUFFERED_AMOUNT` 8 MiB (`:46`), `DEFAULT_MAX_UNREAD_BYTES` 16 MiB (`:51`);
  received bytes not yet taken are counted by `UnreadGauge` / `UnreadTicket` (`:241-271`).
- JSON objects keep insertion order: the workspace enables `serde_json`'s `preserve_order` (`Cargo.toml:31`), which
  the byte-equal encoding below relies on.
- Harness (#131): `parity/socket-io/run.mjs` runs every scenario under `scenarios/` against two targets
  (`run.mjs:59-108`); `lib/targets.mjs` starts `reference/socket-server.mjs` (`startNodeTarget`, `:54-117`), knows the
  capabilities `engine-io`, `socket-io`, `redis` (`:8`) and plans where a scenario runs (`plan`, `:127-137`);
  `lib/normalize.mjs` numbers every string found under a property named `sid` (`:22-34`); `expected-differences.json`
  is `[]`. The only scenario is `scenarios/socket-io/handshake-and-ack.mjs`, and the reference server only answers
  `echo` (`reference/socket-server.mjs:30-37`). `crates/corvis/tests/socket_io_diff.rs` has no Corvis target yet.
  Pinned: `socket.io` 4.8.4, `engine.io` 6.6.11, `socket.io-client` 4.8.4 (`parity/socket-io/package.json`); the
  lockfile resolves `socket.io-parser` 4.2.7 and `socket.io-adapter` 2.5.8. `suites.json` pins
  `socketio/socket.io-protocol` at `4f633dd6a4445492d57af3b27ddd2d336014f1f1`.

### The Engine.IO layer this builds on (as designed in #32, not yet merged)

From `openspec/changes/e2-engine-io-handshake-polling-upgrade-and-heartbeat/design.md`. #32 is being implemented in
another worktree; everything in this paragraph is its design, and the names may shift slightly when it lands.

- `corvis::socket_io::EngineOptions` is public; its design says "#33's Socket.IO server options embed it"
  (Decision 2).
- `#[doc(hidden)] pub mod engine`: `EngineServer::new(EngineOptions) -> Result<_, EngineOptionsError>`,
  `attach(&HttpServer)`, `on_connection`, `close()`, `session_count()`; `EngineSocket`: `id()`,
  `handshake() -> &Request`, `ip()`, `transport()`, `send`, `messages()` (stream of text or binary messages),
  `close()`, `closed() -> CloseReason` (Decision 2). Close reasons: `transport close`, `transport error`,
  `ping timeout`, `parse error`, `forced close` (Decision 9).
- A session's outbox is an `OutQueue<Packet>` bounded by `max_buffered_amount` (8 MiB); a send over it fails,
  discards the outbox and closes the session with `transport error`. Received bytes not yet taken are bounded by
  `max_unread_bytes` (16 MiB): above it WebSocket reads pause and polling `POST`s are held (Decision 8).
  `max_http_buffer_size` (1,000,000 bytes) bounds every Engine.IO message; `max_sessions` is 10,000.
- Session ids come from `socket_io/engine/sid.rs`: 15 bytes of `OsRng`, base64url, 20 characters (Decision 4).
- `EngineServer::close()` closes every session with `forced close`; `serve` and `CorvisApplication::close` call it
  (Decision 9). Review note 6: `server shutting down` belongs to this change.
- Review note 1: "The `engine` module stays `#[doc(hidden)]`; #33 decides whether it becomes public." Review note 7:
  timing tests use Tokio's paused clock or lower bounds only. Review note 8: a security review is mandatory.

### Node behavior relied on

Read from the packages installed by the harness lockfile (`socket.io` 4.8.4 `dist/{index,client,namespace,socket}.js`,
`socket.io-parser` 4.2.7 `build/cjs/{index,binary}.js`), 2026-11-09:

- Parser: `<type>[<attachments>-][<nsp>,][<id>][JSON]`; `/` is omitted; `Decoder` defaults to `maxAttachments: 10`
  and throws `too many attachments` above it, `Illegal attachments` for a count that is not an integer >= 1,
  `unknown packet type`, `invalid payload` (CONNECT: an object or nothing; DISCONNECT: nothing; CONNECT_ERROR: string
  or object; EVENT: an array whose first item is a number or a string that is not a reserved event name; ACK: an
  array), `got plaintext data when reconstructing a packet`, `got binary data when not reconstructing a packet`, and
  `illegal attachments` for a placeholder whose `num` is not an index of the received buffers. The same buffer may be
  referenced by several placeholders, and unused buffers are not an error. The ack id is the run of digits before the
  JSON, read with `Number()`.
- `Client` (one per Engine.IO connection): a decoder error calls each socket's error listener and then
  `conn.close()`, so the server-side disconnect reason is `forced close`, not `parse error` (`parse error` is what
  `engine.io` reports for a malformed Engine.IO packet). `ondecoded`: CONNECT for a namespace not joined connects;
  any other packet for a joined namespace is dispatched; everything else (a CONNECT for a namespace already joined, a
  non-CONNECT packet for a namespace not joined, a CONNECT_ERROR) calls `close()`, which closes the Engine.IO
  connection and reports `forced server close` to the sockets still joined. `connectTimeout` (default 45000): if no
  namespace is joined when it fires, the connection is closed. An unknown namespace gets CONNECT_ERROR
  `{"message":"Invalid namespace"}`.
- `Namespace._add` creates the socket, keeps it in `_preConnectSockets` while the middleware chain runs, then either
  sends CONNECT_ERROR `{message: err.message, data: err.data}` (a missing `data` is dropped by `JSON.stringify`) or
  connects. `Client.nsps` is only filled after the middleware accepted, so a second CONNECT for the same namespace
  sent while middleware is still running creates a second pending socket; nothing bounds this.
- `Socket`: the id is `base64id.generateId()`, not the Engine.IO sid ("it's sensitive information"); `_onconnect`
  answers CONNECT `{sid}`. `emit` throws `"<name>" is a reserved event name` for `connect`, `connect_error`,
  `disconnect`, `disconnecting`, `newListener`, `removeListener`. Server ack ids are `nsp._ids++`, a counter per
  namespace starting at 0. `acks` is a `Map` without bound; an entry without `timeout()` stays until answered. With
  `timeout(ms)` the callback gets `new Error("operation has timed out")`. An ACK for an unknown id is ignored. The
  ack function given to a handler sends at most once. `_onclose(reason)`: `disconnecting` listeners, cleanup (rooms,
  namespace), then `disconnect` listeners. `disconnect(false)` sends DISCONNECT and reports
  `server namespace disconnect`; `disconnect(true)` does that for every socket of the connection and then closes the
  Engine.IO connection. A DISCONNECT packet from the client reports `client namespace disconnect`.
- `Server.close()` reports `server shutting down` to every socket and then closes the engine. The constructor
  creates the main namespace `/`; `serveClient` is on by default.

Corrections to the spike's delta that follow from this reading: a malformed Socket.IO packet ends with reason
`forced close`; the attachment limit already exists in Node (10); and the upstream suite at the pinned commit expects
the connection to be closed for an unknown packet type, an invalid payload and an invalid ack id.

## Goals / Non-Goals

**Goals:**

- The official `socket.io-client` 4.8.4 observes the same behavior from Corvis as from Node `socket.io` 4.8.4 for
  namespaces, events, acknowledgements and binary, verified by the harness and the upstream `socket.io-protocol`
  suite.
- A bounded default for everything a client can make the server hold or compute, with a test each.
- A low-level API that #34, #35 and #36 extend without breaking: no implementation type in a public signature.

**Non-Goals:**

- Rooms and broadcast (#34), gateways (#35), Redis (#36), the items listed in proposal.md "Out of scope".
- Byte-equal JSON for floating point numbers (Decision 2).

## Decisions

### 1. Files and visibility

```
socket_io/mod.rs          re-exports IoServer, IoNamespace, IoSocket, AckSender, IoOptions, IoValue, Binary, Args,
                          Handshake, ConnectError, DisconnectReason, EmitError, AckError (EngineOptions from #32)
socket_io/options.rs      IoOptions, defaults, validation
socket_io/value.rs        IoValue, Binary, Args / IntoArgs, serde bridge
socket_io/parser.rs       Packet, encode, Decoder (streaming, with the reassembly state)
socket_io/server.rs       IoServer: namespaces map, attach, close
socket_io/client.rs       Client: one Engine.IO connection (decoder, joined sockets, pending connects, connect
                          timeout, in-flight handlers)
socket_io/namespace.rs    IoNamespace: middleware, connection handlers, sockets map, ack id counter
socket_io/socket.rs       IoSocket, AckSender, pending acks, event handlers, disconnect
socket_io/fuzz_parser.rs  proptest properties (the name makes `cargo test --lib fuzz` select them)
```

`parser.rs`, `client.rs` and the `Packet` type are `pub(crate)`: #34's adapter and #36's Redis adapter live in the
same crate. Alternative considered: a public `parser` module like the npm package; rejected, nothing outside the crate
needs it and it would freeze the packet type.

### 2. Values: `IoValue` and `Binary`

Socket.IO arguments are JSON with binary values at any depth; `serde_json::Value` cannot hold bytes.

- `IoValue` is an enum: `Null`, `Bool`, `Number(serde_json::Number)`, `String`, `Array(Vec<IoValue>)`,
  `Object(indexmap::IndexMap<String, IoValue>)` (insertion order, as `JSON.stringify`), `Binary(Binary)`.
  `Binary` wraps `bytes::Bytes`, so cloning it, or a placeholder that repeats the same `num`, never copies the bytes.
- Conversions: `From<serde_json::Value>`; `IoValue::from_serialize(&T)` through a serializer that turns a `Binary`
  field (serialized as the newtype struct `$corvis::socket_io::Binary`, the technique `serde_json::RawValue` uses)
  into a binary leaf; `IoValue` implements `serde::Deserializer`, so `value.deserialize::<T>()` fills structs whose
  fields are `Binary`. With any other serializer `Binary` is a byte sequence.
- `Args` is the argument list of one event or acknowledgement (`Vec<IoValue>` with `get::<T>(index)`); `IntoArgs` is
  implemented for `()`, tuples up to 8 of `Serialize` values, `Vec<IoValue>` and `Args`, so
  `socket.emit("update", (1, "two", json!({"three": 3})))` reads like the Node call.
- Encoding: strings, integers, booleans, `null`, arrays and objects produce the bytes `JSON.stringify` produces
  (`serde_json` escapes the same control characters, including `U+001E`, the polling record separator, so an encoded
  packet can never split a polling payload). Floating point numbers are valid JSON that parses to the same value but
  may be written differently (`1.0` against `1`): the harness compares decoded values for them, and the byte-equal
  wire scenario uses the other types.

Alternatives considered: `serde_json::Value` plus a side list of buffers with placeholders visible to the application
(leaks the wire format into handlers, and a handler could forge placeholders); a base64 string convention (not what
the client receives).

### 3. On top of the Engine.IO module; three additions to it

`IoServer::new(IoOptions)` builds an `EngineServer` from `IoOptions::engine` and registers `on_connection`; each
Engine.IO connection gets one `Client` task that reads `messages()`. Three things the #32 design does not offer are
needed, all inside the hidden module and without wire changes:

1. **Grouped send.** A binary packet is one text message followed by its attachments, and nothing may be queued
   between them: Node is single threaded, Corvis handlers of one connection run concurrently. `EngineSocket` gets
   `send_all(messages)` that pushes the group under one lock of the outbox and accepts or refuses it as a whole
   (`OutQueue::push`'s rule applied to the sum of the sizes). Alternative considered: a mutex per `Client` around
   single sends; rejected, #34's broadcasts push to many sockets and must not wait on a per-client async lock.
2. **Accounting until released.** In the #32 design a message stops counting against `max_unread_bytes` when it is
   taken from `messages()`. The Socket.IO layer takes messages to reassemble binary packets and to queue them for
   handlers, so each message carries its unread ticket (the `UnreadTicket` idea of `ws.rs:254-271`) and the layer
   drops it when the packet has been handed to a handler (or discarded). Attachments waiting for their siblings and
   packets waiting for a free handler slot therefore stay inside the session's 16 MiB.
3. **Hook before close.** `EngineServer::on_closing(callback)`, run at the start of `EngineServer::close()` before
   any session is closed, so `IoServer` reports `server shutting down` to every socket first (Decision 8). The
   application shutdown path of #32 (`serve`, `CorvisApplication::close`) is unchanged.

`IoServer::attach(&HttpServer)` forwards to `EngineServer::attach`. The `engine` module stays `#[doc(hidden)]`: the
supported surface is `IoServer` and `EngineOptions`, which keeps the `socketioxide` fallback of the spike possible.

If #32 lands with different names, the tasks adapt to them; the three needs stay.

### 4. Packet codec

- `encode(&Packet) -> Encoded { text: String, attachments: Vec<Binary> }`: type digit, `<n>-` for binary types,
  `<nsp>,` unless `/`, the ack id, the JSON. A packet becomes BINARY_EVENT / BINARY_ACK when its arguments contain a
  `Binary` at any depth; placeholders `{"_placeholder":true,"num":n}` are numbered in depth-first order, as
  `deconstructPacket` does. Encoding fails with `EmitError::TooManyAttachments` above `max_attachments` (Node would
  send it and the official client, whose decoder has the same limit of 10, would drop the connection).
- `Decoder` (one per `Client`): `add_text` / `add_binary`, returning a complete packet or nothing. Checks in Node's
  order with Node's outcomes: unknown type; attachment count not a positive integer; count above `max_attachments`;
  namespace (starts with `/`, up to `,` or the end); ack id (digits); payload validity per type; text while
  reassembling; binary while not reassembling; placeholder index out of range. Every failure is a
  `DecodeError`, and the `Client` closes the connection (Decision 8).
- Divergences, each bounded and listed: the ack id must be at most 16 digits and at most 2^53 - 1 (Node reads any run
  of digits as a float and echoes it rounded); JSON nesting deeper than 128 is an invalid payload (`serde_json`'s
  recursion limit; Node has none); the placeholder walk and the `IoValue` conversion reuse that depth bound, so no
  recursion in this layer is deeper than 128.
- An object is a placeholder only when it has exactly the shape Node checks (`_placeholder === true`); as in Node, a
  client can send that shape inside a non-binary EVENT and it is then ordinary data.
- The decoder allocates nothing proportional to a declared count: the attachment list grows as attachments arrive.

### 5. Namespaces and the connection handshake

- `IoServer` holds `RwLock<HashMap<String, IoNamespace>>`; `/` exists from construction; `of(name)` returns the
  existing namespace or registers a new one (a missing leading `/` is added, as Node). Names are validated when
  registered: at most 256 bytes, no `,` (the packet delimiter) and no control characters; `of` returns
  `Result<IoNamespace, NamespaceNameError>`. Clients cannot create namespaces (dynamic namespaces are out of scope),
  so namespaces per session are bounded by what the application registered.
- CONNECT: the namespace string of the packet is looked up as received (no normalization, no allocation beyond the
  packet itself). Unknown: CONNECT_ERROR `{"message":"Invalid namespace"}` echoing the namespace, connection kept.
  Known: a socket is created with a fresh id from `engine/sid.rs` (never the Engine.IO sid, which is the session's
  bearer credential) and a `Handshake` (`auth`, the Engine.IO handshake `Request` for headers / query / URL, `ip()`
  following `set_trust_proxy`, `issued_at`); the middleware chain runs; then CONNECT `{"sid":<id>}` is sent, the socket
  joins the namespace's map and the `on_connection` handlers run.
- One socket per namespace per connection, counting the ones still in middleware: a CONNECT for a namespace that is
  joined **or pending** closes the connection, as Node does for the joined case. Node creates another pending socket
  in the pending case; Corvis does not, which bounds middleware runs per connection by the number of namespaces.
- `connect_timeout` (default 45 s, validated non-zero): a timer per `Client`, started when the Engine.IO connection
  opens; when it fires and no namespace is joined (pending ones do not count, as Node), the connection is closed.
- If the Engine.IO connection closes while middleware runs, the pending socket is dropped without CONNECT_ERROR and
  without connection or disconnect handlers (Node's `"open" !== readyState` branch).

### 6. Connect middleware

`IoNamespace::use_middleware(f)` with `f: Fn(IoSocket) -> Future<Output = Result<(), ConnectError>>`; functions run
in registration order, the first error stops the chain. `ConnectError::new(message).data(value)` becomes CONNECT_ERROR
`{"message":m}` or `{"message":m,"data":d}`. The socket given to middleware is not connected: `emit` returns
`EmitError::NotConnected` (Node silently queues nothing useful there either), `handshake()`, `data` and `insert` work,
which is how middleware attaches the authenticated user.

A panic in middleware is contained (the `catch_unwind` pattern of `websockets.rs:911-921`), logged, and answered with
CONNECT_ERROR `{"message":"Internal server error"}` (Node would crash the process on a throw outside `next`).
Middleware is the place for authentication; it is the `allowRequest`-style check #32 left to this change.

### 7. Events, acknowledgements and the in-flight bound

- `socket.on(event, handler)` with `handler: Fn(IoSocket, Args, Option<AckSender>) -> Future<Output = ()>`; one
  handler per event name per socket (registering again replaces it; `off(event)` removes it). Numeric event names are
  dispatched under their decimal text, as an `EventEmitter` does. An event without a handler is dropped; its
  `AckSender`, if any, is dropped unanswered (Node: no listener, no answer).
- Handlers of one connection start in arrival order and run concurrently, at most `max_inflight` (default 32) at a
  time; at the limit the `Client` stops taking messages, the unread accounting of Decision 3 fills, and the Engine.IO
  transport stops reading. This is the gateway rule (`websockets.rs:1001-1017`) applied to the whole connection, so
  several namespaces cannot multiply it. A panic in a handler is contained and logged; the socket stays connected.
- `AckSender::send(args)` consumes the sender, so an event is answered at most once; it carries the socket and the
  client's ack id, and the server keeps no table of client ack ids. A duplicate id from the client is the client's
  problem, as in Node.
- `socket.emit(event, args) -> Result<(), EmitError>`: `ReservedEvent` for the six reserved names, `NotConnected`,
  `TooManyAttachments`, `BufferFull` (the Engine.IO outbox refused the packet; the session is closing with
  `transport error`, #32).
- `socket.emit_with_ack(event, args) -> Result<Args, AckError>` and `socket.timeout(duration)` for one call. Ids come
  from a per-namespace counter starting at 0 (Node's `nsp._ids++`; visible on the wire). The pending table is per
  socket, so an id is only accepted from the socket it was sent to and predictable ids give a client nothing. An ACK
  for an id that is not pending is ignored.
- Bounds on pending acknowledgements: every call has a deadline, `timeout()` or `IoOptions::ack_timeout` (default
  30 s; Node waits forever without `timeout()`), after which it fails with `AckError::Timeout`, displayed as
  `operation has timed out`; at most `max_pending_acks` (default 1,000) per socket, above which the call fails at once
  with `AckError::TooManyPending` and nothing is sent; when the socket disconnects, pending calls fail with
  `AckError::Disconnected` immediately instead of waiting for their deadline. Timers are `tokio::time`, so the unit
  tests run under `tokio::time::pause()`.

Alternatives considered: no default deadline, as Node (a future that never resolves leaks its table entry and its
task); a global ack table keyed by id (an ACK from another socket of the namespace could then answer it).

### 8. Disconnect, protocol violations and shutdown

| Trigger | Server-side reason | What the client sees |
|---|---|---|
| DISCONNECT packet from the client | `client namespace disconnect` | its own `io client disconnect` |
| `socket.disconnect(false)` | `server namespace disconnect` | DISCONNECT packet, `io server disconnect` |
| `socket.disconnect(true)` | `server namespace disconnect` for every socket of the connection, then the Engine.IO connection is closed | `io server disconnect` |
| Engine.IO close | the Engine.IO reason: `transport close`, `transport error`, `ping timeout`, `parse error`, `forced close` | `transport close` / `transport error` / `ping timeout` |
| Undecodable Socket.IO packet (Decision 4) | `forced close` | `transport close` |
| CONNECT for a joined or pending namespace, non-CONNECT packet for a namespace not joined, CONNECT_ERROR from the client | `forced server close` for the sockets still joined | `transport close` |
| `connect_timeout` with no namespace joined | none (no socket) | `transport close` |
| `IoServer::close()` / application shutdown | `server shutting down` | `transport close` |

- Order for one socket: `on_disconnecting` handlers (the socket is still in its namespace; #34 keeps its rooms here),
  removal from the namespace and the connection, pending acks failed, `on_disconnect` handlers. Each runs once;
  `disconnect()` on a disconnected socket does nothing. Handlers run under `guarded`.
- `DisconnectReason` is an enum whose `as_str()` is Node's text.
- `IoServer::close()` is idempotent: sockets get `server shutting down`, then the engine closes (`forced close` at
  that level reaches no socket). Through the `on_closing` hook of Decision 3 the same happens when the application
  shuts down.

### 9. Limits and their defaults

| Limit | Default | Node 4.8.4 | When exceeded |
|---|---|---|---|
| Packet size: one string packet, and each attachment | `EngineOptions::max_http_buffer_size`, 1,000,000 B | same | Engine.IO: `413` / close `1009` (#32) |
| `max_attachments` per binary packet, both directions | 10 | 10 when decoding, none when encoding | decode: connection closed (`forced close`); encode: `EmitError::TooManyAttachments` |
| Decoded cost of one packet (`max_decoded_bytes`) | 32 MiB; text decodes to 11 to 20 times its size, at most 58 times | none | connection closed before anything is built; reserved in the server-wide budget before decoding |
| Server-wide unread budget, per client address | `max_total_unread_bytes` 256 MiB; an address holds a quarter of it (at least one packet) plus the fair share of each session it may open; a session always holds its fair share | none | the packet waits, reads pause, heartbeat served |
| Bytes held while reassembling one binary packet | at most `(max_attachments + 1) × max_http_buffer_size` (11 MB), counted in `max_unread_bytes` (16 MiB) | not counted | reads pause; options rejected at construction when the product exceeds `max_unread_bytes` |
| Reassembly state | one packet per connection, no allocation from the declared count | same | text during reassembly closes the connection |
| JSON nesting depth | 128 | none | invalid payload, connection closed |
| Client ack id | at most 16 digits and 2^53 - 1 | any digit run, read as a float | invalid packet, connection closed |
| Namespaces per connection | the namespaces the application registered; one socket each, pending included | joined: same; pending: unbounded | connection closed (`forced server close`) |
| Namespace name (registered) | 256 B, no `,`, no control characters | none | `of` returns an error |
| `connect_timeout` | 45 s | same | connection closed |
| `max_inflight` handlers per connection | 32 (`websockets.rs:400`) | none | reads pause |
| `max_pending_acks` per socket | 1,000 | none | `AckError::TooManyPending` |
| `ack_timeout` (default deadline of `emit_with_ack`) | 30 s | none | `AckError::Timeout` |
| Server ack ids | per-namespace counter, accepted only from the socket they were sent to | same | unknown id ignored |
| Socket id | 120 random bits, never the Engine.IO sid | 96 bits + counter, never the sid | n/a |
| Send buffer per session | 8 MiB (#32) | none | `transport error` (#32), `EmitError::BufferFull` |
| Sessions, and so sockets per namespace | `max_sessions` 10,000 (#32) | none | handshake `503` (#32) |

`IoOptions::new` validation (`IoServer::new` returns `IoOptionsError` naming the option): zero `connect_timeout`,
`ack_timeout`, `max_attachments`, `max_inflight` or `max_pending_acks`; `(max_attachments + 1) ×
max_http_buffer_size > max_unread_bytes` (a reassembly that could never complete once reads pause); and the engine's
own validation.

Client-observable divergences (each gets a wire scenario and an entry in `expected-differences.json`): the second
CONNECT while middleware is pending, the ack id above 2^53 - 1, nesting deeper than 128, and (round 1) a lone
surrogate in a string. The other limits either equal
Node or are only visible to server code; they are documented and tested in Rust.

### 10. Verification

- Unit tests and proptest properties for the codec: round trip `Packet -> Encoded -> Packet`; decoding arbitrary text
  and arbitrary interleavings of text and binary never panics; decoded size proportional to input; attachment count
  never above the limit; no encoded text contains `U+001E`; byte strings recorded from the Node parser by
  `parity/socket-io/scripts/record-parser.mjs` (committed as a fixture with the parser version in it).
- `crates/corvis/tests/socket_io.rs`: in-process application, raw Engine.IO over a `tokio-tungstenite` client and over
  polling, one test per spec scenario that does not need Node. `crates/corvis/tests/socket_io_security.rs`, in the
  style of `tests/ws_security.rs`: one test per row of Decision 9 (attachment count, reassembly backlog measured as
  queue sizes, depth, ack id, CONNECT flood during slow middleware, in-flight bound with a blocked handler, pending
  ack cap, ack deadline under paused time, ACK from another socket, placeholder out of range, repeated placeholder
  without copies, namespace name validation, handler and middleware panics).
- Harness: `reference/socket-server.mjs` becomes the scenario app and accepts `--ping-interval`, `--ping-timeout`,
  `--connect-timeout` (`serveClient: false`): namespaces `/` and `/admin`; `/admin` middleware rejecting
  `auth.token !== "admin"` with data, and a `--middleware-delay` for the pending-connect scenario; handlers `echo`
  (ack with the arguments, as today), `sum`, `emit-update` (server emits `update` with three arguments),
  `ask` / `ask-timeout` (server emits with an ack and reports the outcome), `binary-echo`, `leave` (`disconnect()`),
  `kick` (`disconnect(true)`), `emit-reserved`; a server-side log (handshake `auth`, disconnect reasons, ack outcomes)
  served by a loopback-only control route `GET /harness/log` and cleared by `POST /harness/reset`. The same app is
  written with `IoServer` inside `crates/corvis/tests/socket_io_diff.rs`, started in-process on `127.0.0.1:0`, with
  capabilities `engine-io,socket-io`.
- Scenarios under `parity/socket-io/scenarios/socket-io/`: `namespaces`, `middleware`, `events`, `acks`, `binary`
  (polling-only and WebSocket), `disconnect`, `connect-timeout` (raw client, `--connect-timeout 1000`),
  `wire/encoding` (byte-equal payloads), `wire/malformed`, `wire/attachments-limit` (11 attachments, closed by both),
  `wire/pending-connect`, `wire/ack-id`, `wire/depth` (the last three listed as expected differences). Waits are on
  events, never sleeps (`ws/socket-io-parity`).
- Conformance: `crates/corvis/tests/socket_io_conformance.rs`, gated like `socket_io_diff.rs`, starts the server the
  upstream `socket.io-protocol` suite requires (ping 300 / 200 ms, `connect_timeout` 1 s, CORS `*`, `auth` emitted on
  connect, `message` answered as `message-back`, `message-with-ack`, namespace `/custom`) and runs mocha with
  `SUITE_URL`. The suite's Engine.IO half runs again here, against the Socket.IO server.
- CI: #32 already runs `cargo test -p corvis --features socket-io` and `--lib fuzz` with the feature; the new tests
  join by being in the crate. Node-gated tests stay out of CI until #37.

## Risks / Trade-offs

- #32 is not merged: its hidden API may differ from the design → Decision 3 states needs, not signatures; task 1.1
  starts by reconciling them.
- `IoValue` is a second JSON type in the public API → conversions from and to `serde_json::Value` and `Serialize`
  types keep it out of most user code; #35 hides it behind `#[message_body]`.
- A default ack deadline and early failure on disconnect differ from Node for server code → documented; not visible
  to clients; `timeout()` overrides it.
- Closing on a second CONNECT during middleware could break a client that retries CONNECT while waiting → the
  official client sends one CONNECT per namespace per Engine.IO session; listed as an expected difference with its
  scenario.
- The in-flight bound is per connection: one busy namespace can delay another on the same connection → same trade-off
  as gateways; configurable.
- Handlers run concurrently, so two handlers of one socket can interleave their emits → each packet is queued whole
  (Decision 3.1); order between handlers is the application's, as with gateways.
- Timing tests flake → paused clock in unit tests, lower bounds and generous upper bounds elsewhere (#32 Review
  note 7).

## Open questions

1. `ack_timeout` default 30 s and `max_pending_acks` 1,000: confirm the numbers. Recommendation: keep; both are
   options.
2. A second CONNECT for a namespace whose middleware is still running closes the connection (Node starts another
   pending socket). Recommendation: keep; the alternative is to ignore the duplicate silently.
3. `IoValue` / `Binary` as the public value types (Decision 2), or expose only `serde_json::Value` plus a separate
   list of attachments? Recommendation: `IoValue`, because binary values stay where the application put them.
4. Namespace names are limited to 256 bytes without `,` or control characters. Node accepts anything. Recommendation:
   keep; a name with `,` cannot be addressed on the wire anyway.
5. The `engine` module stays hidden (answers #32's Review note 1). Confirm that no standalone Engine.IO API is wanted
   before 1.0.

## Review notes

These settle the open questions and override the design where they differ.

1. `ack_timeout` 30 s and `max_pending_acks` 1,000 are confirmed.
2. A second CONNECT for a namespace whose middleware is still pending closes the connection, as designed. It is a
   listed expected difference.
3. `IoValue` and `Binary` are public. They MUST convert from and to `serde_json::Value` (`From`/`TryFrom`) and work
   with `serde` (`IoValue::from_serialize`, `IoValue::deserialize_into` or equivalent), so a handler that has no
   binary payload never has to name them.
4. The namespace name restrictions (256 bytes, no `,`, no control characters) are confirmed.
5. `engine` stays `#[doc(hidden)]`. The three additions this change needs there (`send_all`, receive-byte
   accounting, `on_closing`) are made against the code #32 merged: the first task compares this design with it and
   records the adjustments in Implementation notes.
6. The security review is mandatory before the merge (parser, attachment reassembly, ack tracking, limits).

## Implementation notes

Recorded while implementing, against `origin/dev` at `cabeb57` (#32 merged).

### The design against the merged Engine.IO module (task 1.1)

- Names that held: `EngineServer::{new, attach, on_connection, close, session_count}`, `EngineSocket::{id, handshake, ip,
  transport, send, messages, close, closed, close_reason}`, `CloseReason` (`transport close`, `transport error`,
  `ping timeout`, `parse error`, `forced close`), `sid::generate`, `EngineOptions` re-exported by `corvis::socket_io`.
- `messages()` already queued `(message, UnreadTicket)` internally and dropped the ticket when the message was taken, as
  the design said. The addition is `EngineSocket::messages_held()`, a stream of `(EngineMessage, Held)` where `Held` is a
  public wrapper of the ticket, and `EngineSocket::hold(bytes) -> Held`, which charges the session's gauge (and the
  server-wide one) for something the layer keeps: the decoded structure. Both share the single inbox with `messages()`.
- `send_all(Vec<EngineMessage>)` is `OutQueue::push_group` (new, in `ws.rs`) under one lock with `push`'s rule applied to
  the sum of the sizes; `Session::send` and `send_all` share the full-buffer handling (`transport error`).
- `EngineServer::on_closing(callback)`: `close()` sets the `closed` flag and takes its snapshot under the sessions lock
  (as before), then runs the hooks (each under `guarded`, so a panic does not stop the close), then closes the sessions.
  Putting the flag first means no session can open between the hook and the close.
- Test helpers behind `#[cfg(test)]`: `EngineServer::open_test_socket` (a WebSocket-first session with no transport),
  `EngineSocket::{feed, take_sent, is_closing}`. They let `src/socket_io/tests.rs` test the layer on Tokio's paused clock
  without a network.
- `tests/engine_io.rs`, `tests/engine_io_security.rs`, `tests/engine_io_ws.rs` and the Engine.IO conformance test pass
  unmodified.

### Decisions and deviations

- **More options than the design lists**, each a bound on something a client can make the server hold or wait for:
  `max_decoded_bytes` (a decoded packet, 32 MiB since round 1; the design only bounded the text), `max_auth_bytes` (16 KiB: the `auth`
  object lives as long as the socket, so 1 MB of it times 10,000 sessions was 10 GB), `max_event_name_len` (256),
  `max_args` (128), `attachment_timeout` (30 s: the design bounded the bytes of a half-sent binary packet but a client
  that answers pings could hold them forever), `middleware_timeout` (10 s). All validated; `max_decoded_bytes` must be at
  least the engine's `max_http_buffer_size` and `max_auth_bytes` at most `max_decoded_bytes`.
- **Charging decoded structures** (superseded: see "Security review (round 1)", item 1, for the cost model, the default of
  `max_decoded_bytes` and the admission rules). `IoValue` is `Object(Box<IndexMap>)` so every node is 40 bytes; the cost model is 48
  bytes per value, plus the text of strings and keys, plus 64 per object entry, whatever the value holds (an empty array
  costs as much as a number). The decoder spends from a budget while it reads, so a 1 MB packet of `[]` is refused before
  the 500,000 arrays exist. The charge is taken from the session's gauge (`hold`) when the packet is decoded and released
  when its handler ends; a binary packet holds its header's cost and every attachment's message ticket until the packet
  is handed over. The connection task takes the next message only while `held < max_unread_bytes` and a handler slot is
  free (a reassembling packet is always completed), so what the layer holds stays inside the Engine.IO budget and no queue
  of its own exists.
- **JSON reader.** The decoder has its own visitor (`parser.rs`) instead of `jwt.rs`'s `PlainJson`: it has to spend a
  budget and build `IoValue`, which `PlainJson` (a `serde_json::Value` builder) cannot do. It has no special keys, never
  uses `IgnoredAny`, never builds a `Value`, and checks the depth itself. Conversion out of `IoValue` is guarded too:
  `deserialize_into::<Value>()`, `Args::get::<Value>`, `to_json()` and `TryFrom<IoValue> for Value` refuse an object with a
  key under `$serde_json::private::` (`IoValueError::ReservedKey`), because `Value`'s deserializer re-enters the parser for
  `RawValue` when `raw_value` is on (sqlx turns it on); the value stays reachable as an `IoValue`.
- **Depth is 127, not 128.** `serde_json` refuses the 128th nested container, and the decoder sits on it, so 127 is the
  effective limit (`MAX_DEPTH`); the encoder and the conversions use the same number so a packet this server writes can
  always be read back. The spec text says "deeper than 128 levels"; 127 is stricter by one level, invisible to a client.
- **Stricter than Node, all fail-closed, listed in `fixtures/parser.json` tests as `STRICTER`:** an event or
  acknowledgement without a payload, an acknowledgement without an id, an id on CONNECT / DISCONNECT / CONNECT_ERROR, an
  empty string or a first character that is not a decimal digit (Node reads `Number(" ")` as CONNECT), an attachment
  count with anything but digits, a placeholder `num` that is not a non-negative integer (Node indexes with
  `buffers[0.5]`), a namespace in a packet longer than 256 bytes, an ack id above 2^53 - 1 or with more than 16 digits.
  Only the last and the depth have a scenario and an expected difference (the official client sends none of the others).
- **Handlers run inside the connection task**, in a `FuturesUnordered`, as the gateways do: they start in arrival order and
  run concurrently, but on one thread per connection; the engine's WebSocket and polling tasks are separate. When the
  connection ends, handlers already running get `close_timeout` (30 s) to finish and are then dropped. An ACK or a
  DISCONNECT queued behind `max_inflight` blocked handlers waits too (reads stop at the limit), so a handler that waits for
  `emit_with_ack` while the connection is at the limit gets the answer when a slot frees, or `operation has timed out`.
- **Connection state.** CONNECT is queued while the connection's namespace table is locked and before the socket is marked
  connected, so nothing the application emits can precede it. A packet that arrives right behind CONNECT, before the
  connection handlers have run, waits for them (`ready`). A packet for a namespace whose middleware is still running is a
  packet for a namespace not joined: the connection closes, as in Node.
- **Lifetimes.** The connection callback of the engine holds the `IoServer` strongly (an application may drop its handle
  after `attach`, as Node keeps `io` alive through the HTTP server); `IoServer::close` and the engine hook replace the
  callback, which breaks the cycle. Sockets drop their handlers when they disconnect.
- **Pending acknowledgements** expire in two ways: the call's own timeout, and a purge of expired entries each time a
  call registers (an abandoned future that is never polled again still frees its slot at its deadline). A call whose
  future is dropped removes its entry.
- **Logging.** A panic in a handler or middleware is logged with the namespace only; undecodable packets and timeouts of a
  connection are not logged at all, so a client cannot write to the log at the rate it sends. (Round 1 added the rate
  limit: one line a minute per kind and namespace.)
- **Harness.** The scenario servers live on `/sio/` and `/sio-short/`, not `/socket.io/`: on Corvis the Engine.IO scenario
  presets own the default path, so `socket-io/handshake-and-ack` moved to `/sio/`. `emitWithAck` of Node resolves with the
  first value of the answer only, so the Node scenario app logs the callback form to compare all values. The upstream
  suite expects `/socket.io/`, so the conformance test starts its own server.
- **`max_args` and the repeated placeholder.** With 128 arguments as the default the spec scenario "Repeated placeholder"
  (10,000 values sharing one buffer) puts the 10,000 placeholders in one array argument instead of 10,000 arguments.
- **API.** `IoNamespace::use_middleware`, `on_connection` and the disconnect handlers are synchronous or async as the
  design says; `IoSocket::disconnect(close: bool)` mirrors Node's `disconnect(close)`; `IoServer::{main, on_connection,
  use_middleware}` delegate to the main namespace. `IntoArgs` requires `Send` so an acknowledged emit can be awaited in a
  spawned task.
- Task 8.5 (security review), 9.6 (parity) and 9.7 belong to the maintainer's session and are not done here.

## Security review (round 1)

The first review (Medium: 1, Info: 3, plus two checks) found that the memory of decoded packets escaped both the
per-session and the server-wide budget. Findings, fixes and the tests that pin them. Where this section differs from
"Implementation notes" above, this section wins.

### 1. Memory of decoded packets escaped the budgets (Medium)

**Evidence.** One session sending 0.99 MB of text, `42["hold",[{},{},...]]` (330,000 empty objects), to a handler that
keeps its arguments held 46.8 MB live while charged 16.8 MB; twelve sessions held 562 MB against a server-wide limit of
16.8 MB; with the default per-address caps one address could pin about 12 GB. Three causes.

**(a) The cost model undercharged.** `NODE_COST` 48 and `ENTRY_COST` 64 were guesses; an empty object really cost about
142 bytes and the buffers carried growth slack. The model is now derived from the types (`value.rs`, 64-bit; every
constant is a `size_of` or a sum of them, and a unit test pins the numbers):

| Constant | Value | Derivation |
|---|---|---|
| `ALLOC_OVERHEAD` | 24 | what the allocator adds to an allocation: glibc's 8-byte header plus rounding to 16 (at most 23); a counting allocator sees requested sizes only, so this is slack on top of what the tests measure |
| `SLOT` | 40 | `size_of::<IoValue>()`: the 32-byte `Bytes` of `Binary` plus the tag. Every value takes one slot, inline in the buffer of the array or entry that holds it |
| `MAP_BASE` | 96 | an object is `Box<IndexMap>`: `size_of::<IndexMap<String, IoValue>>()` = 72 (hash table header 32, entry vector 24, `RandomState` 16) plus `ALLOC_OVERHEAD` |
| `MAP_TABLE` | 100 | what a non-empty object adds once: the overhead of its entry vector and its hash table (2 x 24), the smallest `hashbrown` table (4 buckets x 9 bytes) and one trailing control group (16) |
| `ENTRY` | 56 | per object entry besides the key text and the value's slot: stored hash 8 + key `String` header 24 + the table's index bytes 24 (a table with room for `n` entries has fewer than `16n/7 + 1` buckets of 9 bytes: at most 21 bytes per entry) |
| key, string | `len + 24` (0 when empty) | the bytes and one allocation; an empty string owns no buffer |
| array | `+ 24` when non-empty | the overhead of its buffer; the buffer's bytes are the slots of its elements |
| `NUMBER_HEAP` | 0 | `serde_json::Number` is inline unless another crate turns on `arbitrary_precision`, in which case the constant becomes 40 + overhead (`size_of::<Number>()` is checked at compile time) |
| `PACKET_COST` | `size_of::<Packet>() + 24` | the packet moved into the handler's task; its namespace, event name and argument buffer are charged as strings and an array |

The decoder builds every buffer at its exact final size: a first pass over the JSON text (`measure`, string aware,
bounded by `MAX_DEPTH`, four bytes of memory per container, shrunk to fit) counts the elements of every container, and the
parser allocates `Vec::with_capacity(n)` and `IndexMap::with_capacity(n)` from those counts (limited by what the
remaining budget can pay for, so a wrong count cannot allocate more than charged), then shrinks the one case that ends
smaller (repeated keys). So no growth slack exists and the peak during decoding equals the final size. The same first
pass computes an upper bound of the cost (escapes and repeated keys only make the real value smaller) and the packet is
refused with `TooLarge` before anything is built when the bound exceeds `max_decoded_bytes`.
`tests/socket_io_memory.rs` (its own binary, with a counting global allocator) decodes 21 shapes of 0.5 to 1 MB of text
(arrays of `{}`, `[]`, `""`, `0`, `null`, floats, one-byte strings, one-key objects, one- and two-element arrays, nested
one-key objects, nested arrays, nested empty-key objects, 500-byte keys, 60,000 keys in one object, one key repeated,
escapes, one long string, a hundred arguments, ordinary records, an acknowledgement, a binary header) and asserts, for
each, live heap of the decoded packet <= charged cost <= reservation, peak during decoding <= reservation + text, and
that dropping the packet frees everything. Measured expansion (charged cost over text): `{}` 45.3x, `[]` 13.3x, `0` 20x,
one-key objects 44.6x, nested one-key objects 52.7x, **nested empty-key objects 58.2x (the worst)**, records of six
fields 11.5x, one long string 1.0x. `IoValue::cost` walks with a stack of iterators (one per nesting level) because a
stack of references was itself an allocation as wide as the widest array (found by the peak assertion).

**(b) Nothing consulted the server-wide gauge before taking the next message, and decoding was charged after the fact.**
The connection task now takes a message only while the Engine.IO session is not over its limit by the engine's own test
(`UnreadGauge::over_limit`: its `max_unread_bytes`, or its fair share while the server-wide budget is exceeded: the rule
the transports use; the layer's own counter is gone) and while no measured packet waits for room. A text message is
measured (`Decoder::prepare`: header, first pass, bound), its reservation is made against the server-wide budget
(`Reservation::try_new`), it is decoded with the budget equal to the reservation, and the reservation is shrunk to the
packet's real cost (`UnreadTicket::shrink_to`). When there is no room the message is parked, keeping its wire charge,
and the connection takes nothing else; the select loop wakes on any release of the session's or the server's gauge (the
notification is registered before the gauges are read, as the WebSocket reader does) and the heartbeat is served by the
Engine.IO tasks as always. A binary header is reserved the same way; its attachments are charged as received.

Admission (`budget.rs`): the session may hold the fair share `f = max(16 KiB, max_total_unread_bytes / max_sessions)`
whatever others hold (the guaranteed share), and anything beyond it needs room in the server-wide budget (an atomic
compare-and-swap, so concurrent connections cannot overshoot together). Both are limited per client address (the key the
Engine.IO layer counts sessions under, so `set_trust_proxy` is honored): an address may hold a quarter of the budget
(never less than one packet, never more than the budget) plus `f` for each session it may have open
(`max_sessions_per_ip`), counting the handlers still running for sessions it already closed. **Wrong in round 1, replaced
in round 2:** that count covered decoded packets only; the address share is now an option that covers everything an
address holds, in every state (see "Security review (round 2)", "The accounting rule"). A binary header whose cost
and attachments could not fit together in `max_unread_bytes` is refused when it arrives (it could never complete: the
transport stops reading while the session is over its limit). `max_decoded_bytes` is validated to be at most
`max_total_unread_bytes`, so one packet can always be reserved.

*Invariant.* Let `T` = `max_total_unread_bytes`, `U` = `max_unread_bytes`, `D` = `max_decoded_bytes`, `W` =
`3 x max_http_buffer_size + 64` (the largest message, its accounting, and the first pass's counts, four bytes per
container and at most one container per two bytes of text, which a parked message keeps), `f` the fair share, `Q = min(T, max(T/4, D))` the
address quota, `N` = `max_sessions_per_ip` and `S` the number of sessions open or closed with a handler still running.
- Bytes held by decoded packets, reassembly buffers and unread messages across a server: `H <= T + S x (f + W)`. Every
  reservation beyond a session's guaranteed share is checked against `T` atomically; a guaranteed one keeps the session
  at or below `f`; the transport stops reading a session above `f` while the budget is exceeded, so each session adds at
  most one message in the hand (`W`). With the defaults (`T` 256 MiB, 10,000 sessions) `f` is 26,843 bytes and the
  overshoot term is at most `S x (26 KiB + 1 MB)`, reached only if every session has a message in the hand at once.
- Per session: `U + D + W` (reads stop above `U`; one more packet is reserved; one message in the hand).
- Per client address, closed sessions included: **wrong in round 1** (it said `Q + N x f` for decoded packets, "plus the
  wire messages of its open sessions", which were bounded by nothing but the session and server gauges). Corrected in
  round 2: `A + N x (f + W)` for everything the address holds, with `A` = `max_address_share`.
- The decoding itself never overshoots its reservation (the peak test above), and a reservation never exceeds `D`.

**(c) Closed sessions.** The charge of a packet lives in the handler's future (`Charge`), which the connection task
hands to a grace task for `close_timeout` (30 s) when the session ends: the handler keeps running, and its charge stays
on the server-wide gauge and on the address, until the handler returns or the grace period drops it. The charge holds
its gauges and its address slot by `Arc`, so nothing is released early and nothing is leaked. A client cannot multiply
it by reconnecting: the count belongs to the address (a new session of the same address finds the quota used, so its
packet waits, which is the intended backpressure), and the guaranteed shares of sessions that closed are counted in the
address's cap too (a test reconnects 60 times and the address never holds more than its cap). Other addresses are not
affected because every limit above is checked against the session's own gauge and the address's own slot: the fair
share is the same number for everyone and nobody else's holdings consume it, and an address can use at most `Q + N x f`
of the shared budget, so an honest address keeps at least `T - Q` for packets that need server-wide room, and its small
packets are always admitted. (Round 2: "an honest address keeps at least `T - Q`" holds only while the addresses at
their share do not add up to the budget; with the defaults seven of them do.)
Regression tests: `a_decoded_packet_is_charged_at_its_cost_not_at_what_was_reserved_for_it`,
`a_packet_that_finds_no_room_waits_and_the_connection_takes_nothing_else`,
`an_honest_address_is_served_while_another_holds_its_maximum`,
`the_charge_of_a_running_handler_outlives_its_session_and_is_not_multiplied_by_reconnecting`,
`reconnecting_over_and_over_cannot_multiply_what_an_address_holds_even_in_guaranteed_shares`,
`a_handler_that_never_ends_loses_its_charge_when_the_grace_period_does`, `a_waiting_packet_is_released_with_its_session`,
`a_connection_over_its_unread_limit_takes_no_more_messages` (`src/socket_io/tests.rs`, paused clock);
`many_sessions_of_one_address_hold_a_bounded_amount_while_their_handlers_wait` (twelve sessions, counting allocator,
live heap against the invariant) and `an_honest_address_is_served_while_another_holds_its_maximum` (a client bound to
127.0.0.2 while 127.0.0.1 holds its quota) in `tests/socket_io_memory.rs`; the gauge operations in `engine/session.rs`
tests and the budget rules in `budget.rs` tests.

**Default of `max_decoded_bytes`: 32 MiB (was 16 MiB).** With honest costs a text packet of ordinary JSON (objects with
a handful of string and number fields) decodes to 11 to 20 times its size, so a legitimate 1 MB payload costs 11 to
20 MB: it would have been refused or borderline under 16 MiB. At 32 MiB anything up to 33 times is accepted; the shapes
above 33 times (the worst is 58x) are refused: the connection is closed and nothing is allocated for them. The limit is
documented in the limits table (EN and ES) with the worst-case expansion factor, and in the rustdoc of
`IoOptions::max_decoded_bytes`.

### 2. Unbounded middleware-timeout warning (Info)

The line was written once per timed-out connection attempt. It goes through `WarnLimiter` (`warn.rs`, mirroring the
limiter of the local strategy's hasher, on `tokio::time::Instant` so it can be tested on a paused clock): at most once a
minute per kind and per namespace, the next carrying the count left out. Keys are a static description and the name of
a namespace the application registered (never client data), and the table is capped at 4,096 keys. Tests:
`warn.rs` unit tests and `a_slow_connect_middleware_warns_once_a_minute_per_namespace` in `tests/socket_io_log.rs`.

### 3. Handler panics logged at the packet rate (Info)

The framework's "panicked" lines (event handlers, connection handlers, `on_disconnecting` and `on_disconnect` handlers,
connect middleware) go through the same limiter, one line a minute per kind and namespace. The connection and
disconnect handlers used `ws::guarded`, which wrote the panic payload; they now use `WarnLimiter::guard`, and no line of
this layer includes a panic message or any data of a packet. Documented: handlers should not `unwrap()` the conversion of
an argument. Test: `a_handler_that_panics_on_client_data_writes_one_line_a_minute_without_the_data_or_the_message` and
`panics_in_connection_and_disconnect_handlers_are_limited_and_never_print_the_message`.

### 4. Lone surrogate (Info)

`JSON.parse('"\\ud800"')` keeps the lone surrogate (JavaScript strings are UTF-16) and Node delivers the event; `serde_json`
refuses it (`lone leading surrogate in hex escape`), so Corvis treats the packet as undecodable and closes the
connection: stricter and fail-closed, now a listed expected difference with the scenario `socket-io/wire/surrogate`
(a valid pair is delivered by both), a `STRICTER` entry in the parser fixture recorded from `socket.io-parser`
(`"lone surrogate"`; the recorder marks surrogates it cannot write), the requirement "Lone surrogates are refused" and
the unit test `a_lone_surrogate_is_refused_and_a_pair_of_them_is_a_character`.

### 5. Reserved key and depth tests with and without `raw_value` (check)

The dev-dependency `serde_json` of `corvis` enables `raw_value` (the JWT header tests need it). To run the reserved-key
test (`objects_with_a_key_serde_json_reads_as_an_instruction_are_never_deserialized`), the depth tests and the rest of
`socket_io::value` and `socket_io::parser` without it, the feature was removed from that line of
`crates/corvis/Cargo.toml` (confirmed with `cargo tree -e features -i serde_json` that nothing else enables it), and
`cargo test -p corvis --features socket-io --lib -- socket_io::value socket_io::parser` and `--test socket_io_security`
were run: 33 and 8 tests passed. The line was restored; both runs are manual (there is no dev-dependency feature switch
that does not change the default build), and the default test run keeps `raw_value` on, the worse case.

### Not changed by this round

- Application memory: arguments a handler moves out of `Args` (`into_vec`) or into a task it spawns are the
  application's after the handler returns; their charge ends with the handler. Documented in the limits discussion.
- The Engine.IO layer's own accounting of one message in the hand per session (`W`) is the pre-existing term of the
  invariant; it is the engine's, not this layer's.

## Security review (round 2)

The second review found that round 1's cost model for JSON text held under ten new shapes, and that the accounting
around it did not: the per-address quota covered only decoded packets, so everything else an address holds on the
shared budget (messages in transit or parked, binary attachments, charges of handlers that outlive their session) was
outside it, a fragmented message occupied up to twice what was charged, an honest packet that needed room waited
behind a heartbeat that closed it for memory it did not hold, and the stored handshake `auth` was not charged. Where
this section differs from "Security review (round 1)" and "Implementation notes", this section wins.

### Corrections to round 1

- **The invariant of round 1 was wrong for the address.** It bounded "decoded packets" per address by `Q + N x f` and
  added "the wire messages of its open sessions" as if they were bounded elsewhere. They were not: wire messages above
  a session's guaranteed share, parked messages and every binary attachment (`Reservation::plain`) were charged to the
  session and server gauges only, so one address could hold the whole server budget (69 MB held against a quota of 21 MB
  with 8 sessions of 127.0.0.1 and 0.5 MB attachments; 68 to 85 MB of text against a 67 MB budget). The formulas below
  replace it.
- **"An honest address always keeps at least `T - Q`" was only true while the addresses that could reach their quota
  did not add up to the budget.** With a quarter of the budget four addresses exhausted it. The share is now an option,
  defaults to 1/16, and the number of addresses that exhaust the budget is stated (see "Item 3a").
- **A message was charged for its length, not its allocation.** See "Item 2".

### The accounting rule (one path)

There is one accounting path, in the gauge chain, and `budget.rs` states it. Every byte the Socket.IO layer keeps
that came from a client is charged to the unread gauge of its Engine.IO session, and a session's gauge is chained to the
gauge of its client address, which is chained to the server-wide one (`ws.rs`: `UnreadGauge`; `engine/mod.rs`:
`ServerInner::parent_gauge`). A charge at any level counts at every level above it, so the address gauge holds,
by construction and in every state, the wire messages the transport received and nobody took, the message a connection
took and parked, the attachments of a packet being reassembled or already delivered, the reservations and decoded
packets, the stored `auth` of the sockets, and the charges of handlers that outlive their session (a charge keeps its
whole chain alive through `Arc`, so nothing is released early and an address's gauge exists as long as anything
charged through it does). The separate address counter of round 1 (`AddressBudgets`, `AddressSlot`) is gone.

Let `T` = `max_total_unread_bytes`, `U` = `max_unread_bytes`, `D` = `max_decoded_bytes`, `M` =
`max_http_buffer_size`, `f = max(16 KiB, T / max_sessions)` the fair share, `N` = `max_sessions_per_ip`, `A` =
`max_address_share`, and `held` the bytes a gauge counts.

- **Admission** of a reservation `R` for the message `W_m` that is being decoded (`credit = W_m`: the message is
  released once the packet is decoded, so the packet is checked net of it and a message is not counted twice) by session
  `s` of address `a`:
  1. *guaranteed share*: `s.held + R <= f + credit` and `a.held + R <= A + N x f + credit`;
  2. otherwise *shared budget*: `a.held + R <= A + credit` and `total.held + R <= T + credit`.
  Each test is a compare-and-swap at its level and a failure changes nothing, so concurrent connections cannot
  overshoot together.
- **Reading** (the test the transports and the connection task use): a session is paused iff `s.held > U`, or
  `s.held > f` and (`a.held > A` or `total.held > T`). A session at or below `f` is always read.
- **Whose doing a pause is**: `Own` when `s.held > U` or `a.held > A`; `Pressure` when only the server-wide budget is
  exceeded. The heartbeat is held only for `Pressure` (Item 3b).
- **Floor and default of `A`**: `F = min(T, D + max(max_attachments, 1) x (M + 64))` (one maximum packet: a decoded
  packet next to the wire bytes of the largest binary packet); `A = max(T / 16, F)`, an explicit value must be in
  `[F, T]`. With the defaults `F` = 43,555,072 bytes (41.5 MiB) is above `T / 16` (16 MiB), so **`ceil(T / A)` = 7
  addresses at their share exhaust the 256 MiB budget**; with `max_decoded_bytes` and `max_attachments` small enough for
  `T / 16` to bind, 16 do.
- **Bounds.** An address holds at most `A + N x (f + W)` where `W = 3 x M + 64` is what one session adds beyond its
  share: the message the transport had read when it stopped, the one being received (the WebSocket library buffers an
  incomplete message in a buffer that can reach twice `M`, uncharged until it is a message) and the first pass's counts.
  The server holds at most `T + S x (f + W)` for `S` open sessions (and closed ones with handlers running, for their
  charges). Per session `U + D + W`. `N` and `W` make the address term large with the defaults (`256 x 3 MB`): an
  operator who wants it tight lowers `max_sessions_per_ip` or `max_http_buffer_size`. The tests assert the live heap of
  one address against exactly this formula and against `T / 2`.
- **A charge never fails and nothing is refused beyond what already existed**: wire messages are charged when they
  arrive (they cannot be refused) and the reading limits above stop the next one; only reservations wait.

For a plain Engine.IO user nothing changes unless they set the two new engine options: `max_address_unread_bytes`
(default none: no address gauge exists and a session's gauge is chained straight to the server's) and
`pressure_pause_timeout` (default none: a paused session is closed by the heartbeat). `IoServer` sets both from its own
options. `engine_io_security.rs` (the fair-share and unread-budget tests) passes unmodified. The engine changes that
apply to everyone: a binary WebSocket message is charged for its buffer capacity (equal to its length unless the
WebSocket library over-allocated it, in which case it was a bug), and `UnreadGauge::changed` wakes on a release at any
gauge above the session's.

### Item 1 (Medium): attachments and wire messages outside the quota

Fixed by the rule above. Wire messages already counted at the address in the chain; attachments no longer use a
`plain` reservation that skipped the address (`Reservation::from_held` wraps the ticket the engine made, which is
already on the chain); the credit stops a message from being counted twice at admission. Regression tests:
`socket_io_accounting.rs`: `binary_attachments_of_one_address_stay_inside_its_share_and_do_not_starve_an_honest_session`
(the reviewer's case: 8 sessions of 127.0.0.1, `51-["hold",{"_placeholder":true,"num":0}]` and 524,289 bytes, 40 times
each; live heap against `A + 8 x (f + W) + 2 MiB` and against `T / 2`; an honest session on 127.0.0.200 answered) and
`large_text_events_of_one_address_stay_inside_its_share_and_do_not_starve_an_honest_session` (0.9 MB strings), with a raw
client that writes from the caller's buffers so only the server's memory is counted; unit tests in `tests.rs`
(`binary_attachments_count_against_the_address_in_every_state`) and `engine/session.rs` (the chain: wire messages,
reservations, guarantee cap, quota, credit, wake-up by a sibling's release).

### Item 2 (Medium/low): fragmented messages occupied up to twice their charge

The WebSocket library builds a fragmented message by extending a buffer, which keeps the capacity of its last doubling
(a 524,288 byte frame and a 1 byte continuation: a megabyte for half a megabyte). `websocket::fit` shrinks a binary
buffer whose slack exceeds an eighth of its length and returns the capacity that remains, and
`Session::on_packet_sized` charges `max(length, allocated) + 64`. What the engine keeps of the other messages is already
exact: a text message is copied (`to_string`) to a buffer of its own length, a polling record likewise (no slice of the
request body is kept, so a small record never pins a large buffer), and a base64 binary message keeps at most two bytes
of capacity beyond its length (a unit test pins both). The buffer of an incomplete fragmented message inside the library
is not a message yet: it is the transit term of `W` (at most `2 x M` per open session). Regression tests:
`fragmented_binary_messages_occupy_no_more_than_they_are_charged` (frames of 524,288 + 1 bytes, 513 frames of 1 KiB, and
300,000 + 224,289 bytes: live heap per kept attachment 1.00 to 1.20, was 2.0), `websocket::tests` (the shrink rule),
`packet::capacity_tests`, `session::a_message_is_charged_for_the_allocation_behind_it_when_that_is_larger_than_its_length`.

### Item 3: an honest packet waiting for budget others hold

**(a) The share is an option.** `IoOptions::max_address_share` (and `address_share()`, `address_share_floor()` to read
the values in force): default `max(T / 16, F)`, validated against `[F, T]`; documented with the number of addresses that
exhaust the budget and the need for `trust_proxy` behind a proxy (EN and ES limits tables).

**(b) A session is not closed for pressure it did not cause.** The transports ask `UnreadGauge::pause()`, which says
`No`, `Own` or `Pressure`. While a session is paused for `Pressure` the engine holds its pong deadline
(`Heartbeat::suspend`; `Session::set_pressure_pause`, called by the WebSocket loop and by the polling `POST` that holds
its answer) for at most `pressure_pause_timeout`, which `IoServer` sets to `admission_timeout`; when reading resumes, or
the grace ends, a pong still awaited is awaited for a fresh `ping_timeout`. A session over its own limit, or whose
address is over its share, keeps the ordinary heartbeat. The wait itself is bounded by `admission_timeout` (default
30 s, validated, above zero and at most a day): the connection task keeps the deadline of the message it parked and, when
it passes, closes the connection with the new disconnect reason `admission timeout`
(`DisconnectReason::AdmissionTimeout`, documented, not a Node reason). Messages that wait behind a parked one stay
charged until it is admitted or the connection closes: an address whose own backlog fills its share with nothing
running is therefore released by the admission timeout, not before (a unit test shows it). The suspension is
bounded too, so a session cannot hide behind it: after `pressure_pause_timeout` the heartbeat applies again.

**(c) Wakeups.** A release at any level notifies every gauge above it (`UnreadGauge::release`), and a waiting
connection registers the notifications of its whole chain (`changed()`, created before it reads the gauges, so there is
no lost wakeup and no polling loop). `a_parked_packet_is_admitted_as_soon_as_room_is_released_and_its_deadline_is_forgotten`
(paused clock: admitted with no time passing) and, over real sockets, the delayed-handler test below.

Regression tests: `tests/socket_io_accounting.rs`
`a_session_waiting_for_room_others_hold_is_not_closed_and_is_served_as_soon_as_a_handler_returns` (eleven addresses of
four packets each hold the 8 MiB budget with handlers that return after 8 s; heartbeat window 1.5 s; the honest packet
is answered after more than 2 s, within 3 s of the first handler returning, no disconnect) and
`when_handlers_never_return_the_waiting_session_is_closed_at_the_admission_timeout_not_by_the_heartbeat` (the disconnect
reason is `admission timeout`, after at least `admission_timeout`); both fail with `PingTimeout` after 1.5 s when the
pause is not suspended (checked by disabling it). Unit tests: `Heartbeat::suspend/resume` (fresh interval, bounded
grace, a ping falling due while suspended), `Session` on the paused clock (not closed in the pause, closed after the
grace, unchanged without the option), `a_packet_that_waits_longer_than_the_admission_timeout_closes_the_connection_with_its_own_reason`.

### Item 4 (Low): the stored `auth`

Kept as a `serde_json::Value`, because that is the public type of `Handshake::auth()` and `IoValue` would change the API
of every application that reads it. Its real size is charged instead: `value::json_cost` (the heap a `Value` owns, with
buffers measured by capacity; the slot itself sits in the socket's structures) is taken with `EngineSocket::hold` when the
CONNECT is handled and kept in the `Handshake`, so it is charged to the session and the address for the socket's life and
released when the handshake is dropped. The model follows the compiled `serde_json::Map`: an `IndexMap` when another
crate of the build enables `preserve_order` (the case in this workspace, so what is measured), a B-tree otherwise (the
bound for that build is derived from the node layout and not measured here). A small object is up to about twice its
`IoValue` model (`[{"":0},...]`), so the 16 KiB limit does not bound the stored value; the charge does. The charge is
taken unconditionally (the CONNECT cannot be refused after it was decoded): a flood of CONNECTs with large auth stops at
the address share like any other packet, because the reservations behind it wait. Tests: `socket_io_accounting.rs`
`the_stored_auth_is_charged_at_its_real_size_across_many_sockets_and_namespaces` (five shapes, 200 stored values each,
live heap <= charged <= 3 x live heap, through `corvis::socket_io::probe_auth`), and in `tests.rs` the address holding
exactly the sum over 30 sockets in 10 namespaces and releasing it as they go.

### Item 5 (docs): the default panic hook

`catch_unwind` does not silence the standard library's hook: each handler panic still writes its message to standard
error, at the client's packet rate, and the message may hold client data the handler formatted. Said in the limits
page (EN and ES) and in the rustdoc of `IoSocket::on`: do not `unwrap()` or `expect()` anything derived from client data;
an application that wants silence installs its own hook with `std::panic::set_hook`. The framework installs none (it is
process-wide).

### Not changed by this round

- The memory of the WebSocket library's buffer for an incomplete message (up to `2 x M` per open session) is not
  charged: it is not a message yet. It is the transit term of `W`, bounded by `max_sessions`.
- A session whose own backlog is the whole of its address's share waits for the admission timeout (above).

## Security review (round 3)

The third review found one medium and six low items. Where this section differs from the earlier ones, this section
wins.

### Corrections to earlier rounds

- **`W` in round 2 was wrong.** It named only `2 x M` for an incomplete message. The WebSocket library holds two buffers
  per session outside every gauge: its read buffer, which grows to the largest frame ever received and is never shrunk,
  and the buffer of an incomplete fragmented message. They are not part of `W`; they are the transit term `X` below, they
  are not charged, and they exist already in the Engine.IO server merged as #32.
- **"Handlers keep their charges until they end, for at most `close_timeout`" was stated for every close.** It is now
  true of a close by the server and of a shutdown. When the client drops the connection the handlers are cancelled at
  once (item E).
- **The heartbeat suspension of round 2 was a per-pause allowance**, which a pressure, relief, pressure sequence renewed
  (item D).

### Item A (Medium): transit memory of the WebSocket transport

Measured with default options (counting allocator, raw client that writes from the caller's buffers, 16 sessions of one
address, `max_http_buffer_size` `M` = 1,000,000): a session that received one valid message of nearly `M` keeps
**1,169,504 B** for the rest of its life (the library's read buffer), and a session with an incomplete fragmented
message (a binary frame of `M - 1` bytes, a one byte continuation, a continuation that claims `M` and sends most of it)
holds **3,294,439 B**: the read buffer, and the message buffer that doubled to 2^21 bytes. Neither is charged.

*At the source.* tungstenite 0.24 (the version in the lock file) has no setting that bounds the read buffer: its
`WebSocketConfig` has `max_message_size`, `max_frame_size`, `write_buffer_size`, `max_write_buffer_size` and
`accept_unmasked_frames`, and the read buffer is a `Vec` that only grows. Lowering `max_frame_size` below
`max_http_buffer_size` would refuse messages Node accepts. tungstenite 0.30 (the latest, with tokio-tungstenite 0.30)
adds `read_buffer_size`, but it is the initial capacity: a scratch program outside the repository, same shapes, same
raw client, measured 0.24 at 1,107,002 / 3,232,009 B per session and 0.30 at 1,058,392 / 4,183,349 B (the read buffer is
kept at the size of the frame, and the incomplete message is kept as before and then copied again). The upgrade would
have changed `Message` for the whole workspace (the default `ws` adapter, ports and examples) and fixed neither, so it
was not made. Nothing in the transport code releases a completed message's buffer either: it is the library's own.

*What remains, stated.* Per session, with `M` = `max_http_buffer_size`: after a large message at most
`R(M) = 1.25 x 2^ceil(log2(M + 4,110))` bytes for the session's life, and while a fragmented message is incomplete at
most `R(M) + 2 x 2^ceil(log2(M))`. The factor 1.25 is the excess the measurements show over the power of two
(1,169,504 against 1,048,576 at the default, 133,224 against 131,072 at 100,000, 273,671 against 262,144 at 250,000);
the library alone outside this server shows the same excess. The transit term is `X = S x (R + 2 x 2^ceil(log2 M))` for
`S` sessions, apart from the budget `T`:

| Default options | Per session | One address (`N` = 256) | The server (`max_sessions` 10,000) |
|---|---|---|---|
| after one large message | 1.12 MiB | 286 MiB | 10.9 GiB |
| incomplete fragmented message | 3.14 MiB | 804 MiB | 30.7 GiB |

The ceilings fall with `max_http_buffer_size` (about 0.13 MiB and 0.32 MiB per session at 100,000 bytes),
`max_sessions_per_ip` and `max_sessions`; the limits pages say so (EN and ES, the Engine.IO section and the Socket.IO
paragraph on bounds). The address bound of round 2 reads `A + N x (f + W) + N x (R + 2 x 2^ceil(log2 M))` with `W` the
message in hand only.

*Considered and left:* charging transit bytes to the session. It needs a byte-counting wrapper around the socket under
the WebSocket library (the library reads in chunks and the buffer is internal), changes the stream type given to
`from_raw_socket` and touches reads on every transport; the numbers above are what it would remove. A candidate
follow-up, not done here.

Regression tests, `tests/socket_io_transit.rs`:
`the_transport_holds_at_most_the_documented_bytes_per_session_at_the_default_message_size` and
`max_http_buffer_size_lowers_what_a_session_holds_in_proportion` (idle retention and incomplete message per session
against `R` and `R + 2 x 2^ceil(log2 M)`, at 1,000,000, 250,000 and 100,000 bytes; they also fail if the measurement
saw nothing, so a change that hides the buffer from the counter is caught).

### Item B (Low): the body of a polling `POST` kept alive while the answer waits

`polling::post` decoded the records (charged to the session) and kept the request body, uncharged, through the wait
that holds the answer for the budget. The body is dropped right after decoding. Test:
`socket_io_transit.rs::a_polling_post_body_is_released_while_its_answer_is_held_for_the_budget` (16 sessions, 600 KB of
records each, held: 708,101 B live per session for 600,000 B of records; with the body alive it is about double).

### Item C (Low): a rolled-back reservation did not wake its siblings

A multi-level charge takes the session and address levels and may be refused by the level above; a sibling that looked
while the lower levels were inflated registered for a change and nothing woke it, since nothing was released. A rollback
now wakes the waiters of the level it undoes. **The first version of this fix woke unconditionally, and it livelocked**:
the asker registers for changes before it tries, so its own rollback woke it, it retried, rolled back again and spun
(the lib tests of the Socket.IO layer hung in `reconnecting_over_and_over_cannot_multiply_what_an_address_holds_...`,
a real defect in the code and not a test problem). Two askers refused by the same level above would have woken each other
the same way. A level now counts the charges it refused (`refusals`); a charge records the count when it is taken and its
rollback wakes the waiters only if the count moved, that is, only if someone was refused while the level held a charge
that did not stay. The asker's own rollback moves no count at the levels it took. Test (`engine/session.rs`):
`a_rolled_back_reservation_wakes_the_siblings_that_were_refused_meanwhile_and_not_its_own_asker`, through the budget API
with no timing: a sibling refused inside the window is woken, and the asker is not woken by its own rollback on either
quota path.

### Item D (Low): the heartbeat suspension could be chained

The allowance is per session since its last received pong: `Heartbeat` keeps the suspension time used
(`suspension_used`), a pause may hold the pong deadline only for what remains of `pressure_pause_timeout`, and only a
pong renews it. Once it is used the heartbeat applies, however often the pressure comes and goes. Tests on the paused
clock: `Heartbeat::the_allowance_is_per_session_since_the_last_pong_and_not_per_suspension` and, through a session,
`oscillating_pressure_cannot_chain_the_suspension_and_a_pong_renews_the_allowance`.

### Item E (Low): handlers that outlive their connection

Eight churning connections from one address reached about 86,000 lingering futures and 50 MB for `close_timeout`.
Now: when the client drops the connection (transport close or error, parse error, ping timeout, any reason except the
server's own `forced close`) the handlers still running are dropped at once, which releases their charges, unless
`IoOptions::finish_handlers_after_client_close(true)`. A close by the server, and a shutdown, keep the grace of
`close_timeout`, and the handlers that linger are counted per client address (`EngineSocket::linger`, a guard that gives
the count back as each ends or when dropped). An address at `max_sessions_per_ip x max_inflight` of them
(`IoOptions::max_lingering_per_ip()`, 8,192 by default) is refused new sessions with the existing `503`
(`EngineOptions::max_lingering_per_ip`, set by `IoServer`, unset for plain Engine.IO). Behaviour change visible to
applications: a handler is cancelled when its client goes away, so work that must survive goes in a task the handler
spawns (handler guidance in the page, EN and ES); Node runs handlers to completion. Nothing changes for plain Engine.IO
or the default WebSocket adapter. Tests: `tests/socket_io_lingering.rs` (the churn PoC: 200 sessions, about 6,000
handlers started, none left, heap +2 KB over the baseline; with the grace requested the lingering handlers peak at the
cap of 32 and 32 of 40 sessions are refused) and unit tests
(`handlers_are_cancelled_at_once_when_the_client_drops_the_connection_unless_the_application_opted_in`,
`handlers_get_their_grace_when_the_server_closes_the_connection_and_count_against_the_address_meanwhile`,
`an_address_with_too_many_lingering_handlers_is_refused_new_sessions_until_they_end`). The last two end the connection
from the server with a test helper, `force_close`: the graceful `close()` of a socket with no transport in a unit test
only completes at `close_timeout`, and a second CONNECT is not read while the connection is at `max_inflight`. An earlier
attempt had swapped `close()` for a second CONNECT to get past the hang above; the hang was the livelock of item C and
not the close path, which was checked.

### Item F (Low): a packet for a namespace the server just left

A namespace slot has a third state, `Left`: the connection joined and its socket left (by the server's `disconnect(false)`
or by the client). A packet for it is dropped (`Flow::Continue`) after it was read, measured, charged and, for a binary
event, its attachments consumed by the decoder, so no reassembly slot leaks; a new CONNECT replaces the slot. The set is
bounded by the registered namespaces (one slot each). A packet for a namespace never joined or refused still closes the
connection. Tests: `socket_io_lingering.rs::packets_that_race_the_servers_leave_of_a_namespace_are_dropped_and_the_connection_survives`
(an ACK, an event and a binary event with a 100 KB attachment for the left namespace, then the main namespace answers,
then a packet for a namespace never joined closes the connection) and
`tests.rs::packets_for_a_namespace_the_server_just_left_are_dropped_and_the_other_namespaces_survive`.

### Item G (docs)

The acknowledgements section says a handler that awaits `emit_with_ack` while its connection has `max_inflight` handlers
running cannot receive the answer (the ACK waits behind the same read gate) and fails at `ack_timeout`. The limits page
says clients behind one shared address (carrier or corporate NAT) share one address quota and can delay each other, and
that `trust_proxy` only helps behind a proxy the operator controls. EN and ES.

### Not changed by this round

- The transit bytes are not charged (item A, considered and left).
- The harness `expected-differences.json` gains nothing: the dropped packet for a left namespace is now what Node does,
  and the cancelled handlers are not observable by a client.

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
CONNECT while middleware is pending, the ack id above 2^53 - 1, nesting deeper than 128. The other limits either equal
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
  `max_decoded_bytes` (a decoded packet, 16 MiB; the design only bounded the text), `max_auth_bytes` (16 KiB: the `auth`
  object lives as long as the socket, so 1 MB of it times 10,000 sessions was 10 GB), `max_event_name_len` (256),
  `max_args` (128), `attachment_timeout` (30 s: the design bounded the bytes of a half-sent binary packet but a client
  that answers pings could hold them forever), `middleware_timeout` (10 s). All validated; `max_decoded_bytes` must be at
  least the engine's `max_http_buffer_size` and `max_auth_bytes` at most `max_decoded_bytes`.
- **Charging decoded structures.** `IoValue` is `Object(Box<IndexMap>)` so every node is 40 bytes; the cost model is 48
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
  connection are not logged at all, so a client cannot write to the log at the rate it sends.
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

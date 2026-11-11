# Design

## Context

Design source: the Socket.IO spike #31 (`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`):
Decision 3 (adapter chosen per application, `path` separated from `namespace`, handler results), Review note 1
(`socket-io` stays opt-in), 5 (a separate `IoSocket` type for Socket.IO gateways), 6 (the `exception` payload is
settled by the differential test) and 8 (the original framework's packages are accepted as test-only dependencies of
the harness), and its delta `specs/websockets/socket-io-gateways`, the starting point of this change's delta, adjusted
below. Code verified at `e8218d3` (`origin/dev`).

### What exists on `origin/dev`

Gateways (`crates/corvis/src/websockets.rs`, feature `websockets`, on by default: `crates/corvis/Cargo.toml:57-59`):

- The macro (`crates/corvis-macros/src/gateway.rs`) reads one value: `path = "..."`, `namespace = "..."` or a
  positional literal all set the same `path` (`:13-39`); every other key is ignored without a message (`:33`). A
  `#[websocket_server]` field is forced to `#[init(::corvis::websockets::WsServer::new())]` (`:82-109`) and handed to
  the runtime type-erased (`GatewayInfo.server`, `crates/corvis/src/discovery.rs:120-129`). `GatewayInfo` has `path`,
  the three hooks as `GatewayHook` (`discovery.rs:116`) and the class-level `WsEnhancers` (`:26-33`).
- Lifecycle traits are typed to the `ws` handles: `OnGatewayInit::after_init(&self, server: WsServer)`,
  `OnGatewayConnection::handle_connection(&self, client: WsClient)`, `OnGatewayDisconnect::handle_disconnect`
  (`websockets.rs:68-84`); the macro detects them with the `Probe` trick and the generated hook downcasts its
  argument to that type, doing nothing when the type differs (`hooks`, `websockets.rs:1095-1129`).
- Handlers: `#[subscribe_message("event")]` inside `#[handlers]` produces a `WsCall`
  (`Fn(Arc<dyn Any + Send + Sync>, Vec<Value>) -> Result<Option<Value>, HttpException>`, `discovery.rs:19-21`). The
  generated code obtains `#[connected_socket]` with `client_from_any` (`websockets.rs:355-357`, a downcast to
  `WsClient` that fails with a 500 `invalid socket`) and `#[ip]` with `ws_ip_arg`
  (`crates/corvis/src/__private.rs:388-391`, also through `WsClient`); see
  `crates/corvis-macros/src/handlers.rs:168-181` and `:297-301`. `#[message_body]` parameters are `ParamDef`s whose
  values are JSON (`serde_json::Value`) taken from the frame's `data`.
- `compile_gateways` (`websockets.rs:561-631`) resolves global, gateway and handler enhancers into
  `CompiledWsHandler`s keyed by event and rejects a duplicate event inside one gateway; `start_gateways` (`:634-644`)
  registers each `WsServer` by path and runs `after_init`. `CorvisApplication::init` calls them around the route
  posture check (`crates/corvis/src/application.rs:559-582`) and stores the result in `AppShared.gateways`
  (`crates/corvis/src/router/mod.rs:572`). `apply_global_enhancers_to_gateways` (`application.rs:209-217`) sets
  `RouterConfig::gateway_global_enhancers` (`router/mod.rs:315`) and warns without effect after `init()`.
- A request is matched to a gateway only when it is a WebSocket upgrade whose path equals a gateway path
  (`match_upgrade`, `websockets.rs:671-678`; called from `handle`, `crates/corvis/src/server.rs:558-562`).
- Pipeline for one message: `run_handler` (`websockets.rs:776-801`): guards (a `false` becomes
  `WsException::new("Forbidden resource")`), interceptors, then `invoke_handler` (`:832-847`), which applies the pipes
  to each `#[message_body]` value and calls the handler with the client type-erased. The result is `None`, the marker
  `{"__corvis_same_event": true, "data": ...}` for a plain value or `{event, data}` for a `WsResponse`
  (`encode_reply`, `:225-232`). `dispatch` (`:895-946`) builds the `ExecutionContext` with `for_ws`
  (`crates/corvis/src/common/context.rs:87-94`), contains panics, and sends the reply frame (a plain value under the
  incoming event name) or the `exception` frame built by `exception_data` (`:852-890`: handler and gateway filters,
  then `exception_payload`, `:128-149`), both echoing the frame's `id`.
- Connection: `run_connection` (`:975-`) creates the `WsClient`, runs `handle_connection` to completion (`:997`) and
  only then reads frames; handlers of one connection run concurrently up to `GatewayOptions::max_inflight` (32,
  `:374-405`). Guards never run for the connection itself; `GatewayOptions::verify_client` and `allowed_origins` are
  the handshake checks.
- Guards read the client with `ctx.switch_to_ws().get_client::<WsClient>()` (`context.rs:378-386`); the throttler's
  tracker does so too and otherwise falls back to the handshake request (`request_ip`,
  `crates/corvis/src/modules/throttler.rs:1172-1178`).
- Route posture: `posture_gateways` (`websockets.rs:647-668`) reports each handler with its guards and whether
  `verify_client` is set; the handshake is an unchecked surface (`UncheckedSurface::WsHandshake`,
  `crates/corvis/src/posture.rs:425-434`); message exemptions match the gateway path (`docs/websockets/guards.md`,
  "Route posture for gateways").
- Docs: `docs/websockets/gateways.md` documents `namespace` as a "Synonym for `path`" (`:89-94`) and
  `docs/websockets/adapter.md` says there is no Socket.IO adapter.
- Two gateways declared on the same path replace each other without a message (`start_gateways` inserts by path).
  Not changed here (proposal, Out of scope).

### The layers this builds on (as designed, not yet merged)

Described from the approved changes and their Review notes; names may shift when they land, and task 1.1 reconciles.

- #32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`): `corvis::socket_io::EngineOptions` (public): `path`
  (default `/socket.io/`), `ping_interval` 25 s, `ping_timeout` 20 s, `upgrade_timeout` 10 s, `max_http_buffer_size`
  1,000,000 B, `max_payload_packets` 10,000, `max_buffered_amount` 8 MiB, `max_unread_bytes` 16 MiB, `max_sessions`
  10,000, `close_timeout` 30 s, `cors(CorsOptions)` (off by default, answered by the engine because requests under
  its path never reach the router), `allowed_origins` (both transports). From its security review, being finished in
  the worktree of #32: `max_sessions_per_ip` (256, keyed by the resolved client address), `allow_request` (an async
  hook run for handshakes that returns `Admission::Allow` or `Admission::Reject`; a rejection, a panic or exceeding
  `allow_request_timeout`, 5 s, answers `403 {"code":4,"message":"Forbidden"}`), `first_request_timeout` (10 s) and
  `max_total_unread_bytes` (256 MiB per engine). These were checked against the implementation in the worktree of #32
  (`crates/corvis/src/socket_io/engine/options.rs` there) on 2026-11-10; the stored path always ends with `/`
  (`/socket.io/`), so `path = "/socket.io"` and `path = "/socket.io/"` are the same Engine.IO path. The engine is attached to the application's `HttpServer`
  and consulted in `handle` before gateways and routing; two engines with overlapping paths are rejected at attach.
  The `engine` module is `#[doc(hidden)]`.
- #33 (`e2-socket-io-namespaces-events-and-acks`): `IoServer::new(IoOptions)`, `attach(&HttpServer)`, `of(name)`,
  `close()`; `IoNamespace::use_middleware`, `on_connection`; `IoSocket` (`id`, `handshake()` with `auth`, the
  handshake `Request` and `ip()`, `data`, typed `insert` / `get`, `on(event, handler)`, `emit`, `emit_with_ack`,
  `disconnect(close)`, `on_disconnecting`, `on_disconnect`); handlers receive `(IoSocket, Args, Option<AckSender>)`;
  `AckSender::send` consumes the sender; an event without a handler is dropped; handlers of one connection run
  concurrently up to `max_inflight` (32) with read backpressure; `connect_timeout` 45 s, `max_attachments` 10,
  `max_pending_acks` 1,000, `ack_timeout` 30 s; namespace names at most 256 bytes without `,` or control characters;
  `IoValue` / `Binary` convert from and to `serde_json::Value` (Review note 3); a connect middleware error answers
  `CONNECT_ERROR {message, data?}`.
- #34 (`e2-socket-io-rooms-and-broadcast`): `IoSocket::join` / `leave` / `rooms`, `BroadcastOperator` (`to`,
  `within`, `except`, `local`, `timeout`, terminal `emit`, `emit_with_ack`, `fetch_sockets`, `sockets_join`,
  `sockets_leave`, `disconnect_sockets`, all `async` and fallible, with a deferred error for an invalid room),
  entry points on `IoNamespace`, `IoServer` and `IoSocket`; `IoOptions::adapter(factory)`; `max_rooms_per_socket`
  1,000, `max_room_name_length` 256 B, `max_broadcast_ack_bytes` 16 MiB. The `Adapter` trait and `InMemoryAdapter` are
  `#[doc(hidden)]` until #36 (its Review note 6); this change does not touch them.

### Reference behavior (the original framework's Socket.IO adapter)

The harness lockfile does not include the original framework's packages yet (`parity/socket-io/package.json` pins
only `socket.io` 4.8.4 and its companions), and no checkout on this machine has them installed, so the list below was
written from the published source of the platform package (`adapters/io-adapter.ts`), the gateway binding code
(`web-sockets-controller.ts`, `socket-server-provider.ts`) and the base exception filter
(`exceptions/base-ws-exception-filter.ts`) and is **to verify** against the pinned packages by task 1.2 before any
runtime code is written. Every item has a harness scenario; where the pinned packages disagree with an item, the
packages win and the item is corrected in "Implementation notes".

1. One Socket.IO server per HTTP server and Engine.IO path. The first gateway that needs it creates it with its own
   options; later gateways on the same path get `server.of(namespace)` and their other options are ignored.
2. `afterInit(server)` receives the namespace when the gateway declares one, the server otherwise.
3. On `connection`, `handleConnection(client)` is called, the message handlers are bound to the socket, and
   `handleDisconnect(client)` is bound to its `disconnect` event. `handleConnection` is not awaited: a message sent
   right after connecting can be handled before an asynchronous `handleConnection` finishes.
4. Payload of a message: with one argument, that argument; with several, the array of them; with none, `undefined`.
   An acknowledgement function, when the client sent one, is removed from the arguments first.
5. Result of a handler: `null` / `undefined` sends nothing and does not call the acknowledgement; an object with a
   truthy `event` property is emitted as `socket.emit(result.event, result.data)` whether or not there is an
   acknowledgement; anything else calls the acknowledgement with the value when there is one and is dropped
   otherwise.
6. Guards, interceptors and pipes run per message, never for the connection. A guard that returns `false` raises the
   WebSocket exception `Forbidden resource`.
7. Errors are emitted to the socket as the event `exception`; the acknowledgement is not called. A WebSocket
   exception built with a string gives `{status: "error", message}`, one built with an object gives the object, and
   anything else (HTTP exceptions included) gives `{status: "error", message: "Internal server error"}`. Recent
   versions add a `cause` (`{pattern, data}`) to these payloads unless turned off.
8. Two gateways may share a namespace; when both subscribe to the same event, both handlers run.

Items 3 and 7 matter for the decisions below: Corvis already diverges from 7 under the `ws` adapter for HTTP
exceptions (`exception_payload` forwards their message and fields, documented in `docs/websockets/gateways.md`), and
from 3 by finishing `handle_connection` before reading (`websockets.rs:997`).

## Goals / Non-Goals

**Goals:**

- An application switches its gateways to Socket.IO with one call and a change of handle types; the official
  `socket.io-client` 4.8.4 then observes the same behavior as from the reference gateway, verified by the harness.
- Existing gateways under the default adapter keep compiling and behaving exactly as today
  (`crates/corvis/tests/websockets.rs`, `ws_security.rs`, `ws_enhancers.rs`, `auth_ws.rs` unmodified).
- A mismatch between a gateway and the adapter is a startup error, never a hook or a check that silently does not
  run.
- Every limit of the Socket.IO layers is reachable from the adapter's options and keeps its bounded default; this
  layer adds no unbounded state.

**Non-Goals:**

- A public gateway-adapter trait, manual acknowledgement parameters, binary in the pipeline, the user guide (#38),
  Redis (#36). See proposal.md "Out of scope".

## Decisions

### 1. The adapter is chosen on the application

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_websocket_adapter(IoAdapter::new());            // feature `socket-io`
app.listen("127.0.0.1:3000").await?;
```

- `CorvisApplication::use_websocket_adapter(&mut self, adapter: impl GatewayAdapter) -> &mut Self`, next to
  `apply_global_enhancers_to_gateways` and with its rule: called before `init()`; afterwards it logs a warning and has
  no effect (gateways are built once). It stores a `GatewayAdapterKind` in `RouterConfig`.
- `GatewayAdapter` is a sealed trait in `corvis::websockets` implemented by `WsAdapter` (a unit struct, the default)
  and, with the `socket-io` feature, by `corvis::socket_io::IoAdapter`. Sealed keeps the door open for a public
  adapter trait without committing to its shape now.
- `IoAdapter::new()` uses `IoOptions::default()` for every Engine.IO path. `IoAdapter::options(IoOptions)` sets the
  options of one path (the options' own `EngineOptions::path`); it can be called once per path, and a second call for
  the same path, or options that fail `IoOptions` validation, are remembered and make `init()` fail with the error
  (the builder itself does not return `Result`, like the other application setters).
  `IoAdapter::guard_connections(bool)` is Decision 9.
- Everything the earlier layers bound is configured here, because `IoOptions` embeds `EngineOptions`: session caps,
  `allow_request`, `first_request_timeout`, CORS, `allowed_origins`, buffer sizes, heartbeat, `connect_timeout`,
  `max_inflight`, acknowledgement and room limits, and the cluster adapter of #36 (`IoOptions::adapter`).
- Without the `socket-io` feature `IoAdapter` does not exist; nothing else changes.

Alternatives considered: an attribute per gateway (`#[websocket_gateway(adapter = ...)]`): not in the original, and
two protocols in one application doubles the test matrix; an environment variable or a module option: the original
selects it on the application, and tests need to choose per application instance; making Socket.IO the default:
rejected by spike Review note 1 until 1.0.

### 2. `#[websocket_gateway]` options per adapter

| Form | `ws` adapter (unchanged) | Socket.IO adapter |
|---|---|---|
| `#[websocket_gateway]` | path `/` | namespace `/`, Engine.IO path `/socket.io` |
| `#[websocket_gateway("/chat")]` | path `/chat` | namespace `/chat` |
| `namespace = "/chat"` | path `/chat` (synonym) | namespace `/chat` |
| `path = "/realtime"` | path `/realtime` | Engine.IO path `/realtime`, namespace `/` unless given |
| `cors = true \| "https://a" \| ["https://a", "https://b"]` | no effect, warning at `init()` | CORS of the Engine.IO path |

- `GatewayInfo` keeps `path` exactly as computed today and gains `io: IoGatewayDecl { namespace: Option<String>,
  path: Option<String>, cors: Option<CorsDecl> }`, where `namespace` is the `namespace =` value or the literal and
  `path` only the `path =` value. The `ws` adapter reads `path`; the Socket.IO adapter reads `io`. This is the
  original's rule (`path` is the server path of whichever platform is used), and it means a `ws` gateway written as
  `path = "/chat"` becomes namespace `/` on the Engine.IO path `/chat` when the adapter is switched: the startup log
  line of each gateway names its namespace and Engine.IO path, and the docs table says so.
- The namespace is validated by the macro with #33's rules (at most 256 bytes, no `,`, no control characters), so a
  bad literal is a compile error instead of an `init()` error.
- `cors = true` is `Access-Control-Allow-Origin: *` without credentials (what the reference does for `cors: true`, to
  verify); a string or an array lists the allowed origins; `cors = false` is the same as leaving it out; any other
  value is a compile error naming these forms. Anything richer (credentials, headers, methods, max age)
  is set with `IoAdapter::options(IoOptions::new().engine(EngineOptions::new().cors(CorsOptions...)))`.
- Engine.IO and Socket.IO options other than CORS (`ping_interval`, `transports`, buffer sizes) are not macro
  options: they belong to the server of a path, not to one gateway, and durations and sizes read badly as attribute
  literals. This narrows the spike's delta, which listed them on the macro (Open question 2).
- An unknown option is a compile error naming the accepted ones (`path`, `namespace`, `cors`). Today it is ignored
  (`gateway.rs:33`), which would hide a mistyped `namspace` or an option copied from the original that does nothing.

Grouping at `init()`: gateways are grouped by Engine.IO path; each group gets one `IoServer`. The options of a path
are the adapter's `IoOptions` for it (default otherwise) with the group's declared `cors` applied on top. `init()`
fails, naming the gateways, when two gateways of one path declare different `cors`, when a gateway declares `cors`
and the adapter's options for that path already set one, or when two paths overlap (#32 rejects overlapping engines).
The original lets the first gateway's options win silently (reference item 1); a startup error is the safer reading
of the same configuration.

Several gateways may share a namespace (reference item 8). Their handler maps are merged; the same event in two
gateways of one namespace fails `init()` with the message of the existing duplicate-event check extended to name both
gateways. The original runs both handlers; Corvis keeps one handler per event (`ws/gateways`, "Duplicate events in one
gateway fail startup") because two handlers would race for one acknowledgement.

### 3. Building and starting

`compile_gateways` stays the single place that resolves enhancers. It returns the same `GatewayRuntime`s for both
adapters, with the server handle generalized (Decision 4). After the posture check:

- `ws` adapter: `start_gateways` as today.
- Socket.IO adapter (`socket_io/gateway.rs`): for each Engine.IO path, `IoServer::new(options)`; for each gateway,
  `io.of(namespace)`; the namespace gets the gateway dispatcher (Decision 5), the connection gate and hooks
  (Decision 8) and, when enabled, the connection guards (Decision 9); the gateway's `IoGatewayServer` field is bound;
  `after_init` runs with the `IoNamespace`; finally each `IoServer` is attached to the application's `HttpServer`.
  A failure at any step (options, overlapping paths) fails `init()`, and servers created so far are closed.
- `AppShared.gateways` stays empty under Socket.IO, so `match_upgrade` never matches and WebSocket upgrades outside
  the Engine.IO paths are handled as in an application without `ws` gateways (`on_upgrade` listeners, then routing).
  The `IoServer`s are kept in `AppShared` for `close`; #33 already closes them through the engine's shutdown hook.
- The startup log keeps the existing "subscribed to the ... message" lines and adds one line per gateway:
  `ChatGateway: Socket.IO namespace /chat on /socket.io`.

### 4. Handles and the startup type check

Spike Review note 5 decided a separate `IoSocket` for Socket.IO gateways. The same split is applied to the server
handle and the hooks, with one rule: **what a gateway declares must match the adapter, checked at `init()`**.

- `#[connected_socket]`: the macro stops hard-coding `WsClient`. It emits
  `::corvis::websockets::socket_from_any::<Ty>(&__client)?`, where `Ty: GatewaySocket`, a hidden trait implemented for
  `WsClient` and `IoSocket` with `const KIND: GatewayAdapterKind`. The handler's `WsEnhancers` records the kind
  (`.socket_kind(<Ty as GatewaySocket>::KIND)`). `client_from_any` stays for code generated by older macro versions.
  A parameter of another type is now a compile error (`GatewaySocket` is not implemented) instead of a runtime 500.
- `#[ip]`: `ws_ip_arg` accepts both handles; for `IoSocket` the value is `socket.handshake().ip()`, resolved once per
  connection with the application's `trust proxy` setting (#32, #33). The throttler's `request_ip`
  (`throttler.rs:1172-1178`) gets the same branch.
- `#[websocket_server]`: the macro emits `#[init(<Ty as ::corvis::websockets::GatewayServerField>::__corvis_unbound())]`
  for the field's own type. `WsServer` implements it with `WsServer::new()` (today's behavior). The Socket.IO type is
  `corvis::socket_io::IoGatewayServer`: a cheap handle that is empty until the gateway starts and then holds the
  gateway's `IoNamespace`. It offers the broadcast entry points of #34 (`emit`, `to`, `within`, `except`, `local`,
  `timeout`, `fetch_sockets`, `sockets_join`, `sockets_leave`, `disconnect_sockets`) and `namespace() ->
  Option<IoNamespace>`. Before the gateway starts (a provider constructor, `on_module_init`) the operators carry a
  deferred error, the mechanism #34's operators already have for an invalid room (its Decision 5), with a new variant
  `EmitError::NotStarted` added by this change, so nothing panics and nothing is sent. Once bound, the handle works
  from any provider that injects the gateway.
- Hooks: the three traits get a type parameter with the current type as default:
  `OnGatewayInit<S = WsServer>`, `OnGatewayConnection<C = WsClient>`, `OnGatewayDisconnect<C = WsClient>`. Existing
  `impl OnGatewayInit for Chat` compiles unchanged; a Socket.IO gateway writes
  `impl OnGatewayInit<IoNamespace> for Chat`, `impl OnGatewayConnection<IoSocket> for Chat`. `after_init` receives the
  real `IoNamespace`, so a gateway can register connect middleware there. The `detect!` macro of `hooks`
  (`websockets.rs:1095-1129`) is instantiated for both types and `GatewayInfo` records which kinds of hooks exist.
- Check in `compile_gateways`, before any user code of a gateway runs: under adapter A, a gateway with a
  `#[connected_socket]` parameter, a `#[websocket_server]` field or a hook of the other kind fails `init()`:
  `ChatGateway::on_message takes #[connected_socket] WsClient, but the application uses the Socket.IO adapter; use
  IoSocket`. A hook that silently never runs could be an authentication check, so this is an error, not a warning. A
  gateway may implement the hooks for both kinds and then works under either adapter.
- Guards and interceptors read the socket with `ctx.switch_to_ws().get_client::<IoSocket>()`; `ctx.request()` is the
  handshake request under both adapters, which is what a guard shared between adapters should use
  (`docs/websockets/guards.md`, "One guard for both transports").

Alternatives considered: one handle type for both transports (rejected by Review note 5); making `IoNamespace` itself
late-bound so the field could be an `IoNamespace` (reaches into #33's namespace state for one field; `use_middleware`
and `on_connection` on an unbound namespace have no error to return); `Option<IoNamespace>` behind a lock in user
code (every call site would unwrap); warnings instead of the startup error (a skipped `handle_connection` is a
security hole that a log line does not stop).

### 5. Dispatch through the existing pipeline

- `run_handler`, `call_interceptors` and `invoke_handler` (`websockets.rs:776-847`) take the socket as
  `Arc<dyn Any + Send + Sync>` instead of `&WsClient`; `WsCall` already receives it that way. `exception_data` is
  unchanged. `dispatch` (`:895-946`) keeps the `ws` framing; `socket_io/gateway.rs` has its own `dispatch_event` that
  shares everything up to the result.
- The namespace gets one crate-private dispatcher (`IoNamespace::set_dispatcher`, an addition to #33's namespace
  inside the crate): `Fn(IoSocket, &str, Args, Option<AckSender>) -> Option<BoxFuture<'static, ()>>`, consulted for an
  event when the socket has no handler of its own for it. A handler registered with `IoSocket::on` therefore wins,
  which is the escape hatch for binary payloads and manual acknowledgements. The dispatcher looks the event up in the
  merged handler map of the namespace's gateways; an unknown event returns `None` and is dropped as #33 specifies (the
  original ignores it too). No per-socket state is created per event.
- The future runs inside #33's in-flight accounting: at most `max_inflight` (32) handlers per connection, started in
  arrival order, reads paused at the limit, panics contained (`catch_unwind` as in `dispatch`, `:911-921`, reported
  through the exception path as an internal error).
- `ExecutionContext::for_ws(handshake request, class target, handler target, module ref, Arc<IoSocket>, data)`:
  the same constructor, so `get_handler()`, `get_class()`, metadata and `Reflector` work as today.

### 6. The message body

Reference item 4, in terms of the packet: zero arguments give `null`, one argument gives that argument, two or more
give the array of them. The acknowledgement id is not an argument. The pipeline (`get_data()`, pipes,
`#[message_body]`, `#[message_body("prop")]`) works on `serde_json::Value` as today, so a typed body, a validation
pipe or a zod schema behave the same under both adapters.

Binary: the pipeline has no binary type. The arguments are converted to JSON with every binary value replaced by
`null`; a typed `#[message_body]` that expected bytes then fails its deserialization or validation and the client gets
the `exception` event. The original passes buffers through. Listed as an expected difference (scenario
`gateways/binary-body`) and documented with the alternative: `socket.on("upload", ...)` in `handle_connection`
receives the real `Args`. Extending the pipeline to `IoValue` is left for a later issue (it changes `PipeTransform`).

### 7. Results and exceptions

The handler result reaches `dispatch_event` in the three shapes of `encode_reply`. Mapping (reference items 5 and 7):

| Result | Client sent an acknowledgement | No acknowledgement |
|---|---|---|
| nothing (`()`, `None`), or a value that serializes to `null` | nothing is sent; the acknowledgement is not answered | nothing |
| `WsResponse { event, data }` | `socket.emit(event, data)`; the acknowledgement is not answered | `socket.emit(event, data)` |
| a value that serializes to an object with a non-empty string `event` | `socket.emit(value.event, value.data)` | the same |
| any other value (`false`, `0` and `""` included) | acknowledgement with the value as its single argument | dropped |
| error, guard denial, panic | event `exception` with the payload of `exception_data`; the acknowledgement is not answered | event `exception` |

- The third row is the original's duck typing (`result.event`). A returned object whose `event` is truthy but not a
  string (a number, `true`) is acknowledged as a plain value and logged at debug level; the original would try to
  emit it. Server-authored, not client-observable in a meaningful application; noted in the docs.
- Interceptors see and can replace the result exactly as today (`run_handler` reports the shape after them).
- A `WsResponse` whose event is one of Socket.IO's reserved names fails `IoSocket::emit` (#33, `ReservedEvent`); the
  failure is logged with the gateway and handler and nothing is sent. Emit failures because the socket disconnected or
  its buffer is full are ignored, as `WsClient::send_text`'s `false` is today.
- The `id` echo of the `ws` protocol does not exist here; the acknowledgement is Socket.IO's.
- Exceptions use the same `exception_data`: handler filters, gateway filters, then `exception_payload`. A filter that
  answers with an empty body sends nothing (`ws/exception-filters`).
- Payload parity. `exception_payload` equals the reference for a WebSocket exception with a string or an object and
  for unknown errors. Two points are settled by the harness, as spike Review note 6 asks:
  1. `cause`: if the pinned reference adds `cause: {pattern, data}` by default, the built-in mapping adds it under
     the Socket.IO adapter (the `ws` adapter keeps today's frames); task 1.2 records the answer.
  2. HTTP exceptions (`BadRequestException`, a failed validation pipe): the reference answers
     `Internal server error` for anything that is not a WebSocket exception; Corvis keeps forwarding the exception's
     own message and fields, as it documents for the `ws` adapter, because a client that cannot tell a validation
     error from a server fault is a usability regression and nothing is leaked that the HTTP side would not send.
     Listed as an expected difference with its scenario (Open question 4).

### 8. Connection, hooks and the gate

For each socket that connects to a gateway namespace (after #33's connect middleware accepted it and `CONNECT` was
answered):

1. the socket's gate is created closed;
2. `handle_connection` hooks of the namespace's gateways run in registration order, each to completion, under the
   panic containment of `run_hook` (`websockets.rs:948-958`);
3. the gate opens.

The dispatcher's future awaits the gate before running the pipeline, so an event that arrives while a hook is still
verifying a token is neither dropped (the reference handles it, item 3) nor handled before the hook decided (today's
`ws` guarantee, `websockets.rs:997`). Waiting events hold in-flight slots, so at most `max_inflight` wait and then the
connection stops being read. If a hook disconnects the socket, or panics (the socket is then disconnected with
`disconnect(true)`, the counterpart of today's close code 1011), waiting events are discarded without running guards
or handlers. This is a deliberate, safe-side divergence from reference item 3; it is not client-observable except
through timing.

`handle_disconnect` hooks are registered with `IoSocket::on_disconnect`, so they run after the socket left its rooms
(#34 Decision 4), once, also for sockets disconnected inside `handle_connection`. `after_init` runs once per gateway
with the namespace, before the server is attached and therefore before any connection.

### 9. Guards on connection (opt-in)

By default guards run per message only, as in the reference (item 6) and as under the `ws` adapter. That leaves a
well-known gap in applications built on the original model: a socket that never sends a message never meets a guard,
yet it is connected, can be joined to rooms by server code and receives broadcasts. Ways to check the connection,
from the outermost in:

- `EngineOptions::allow_request` and `allowed_origins` (#32): before a session exists.
- Connect middleware registered in `after_init` (`namespace.use_middleware(...)`, #33): the socket is not connected
  and receives nothing until it passes; a rejection is `CONNECT_ERROR` with a message and optional data.
- `handle_connection` (Decision 8): after connecting; can call `socket.disconnect(true)`.
- **`IoAdapter::guard_connections(true)`** (new, off by default): the adapter registers a connect middleware on every
  gateway namespace that runs the guards that apply to the whole gateway: global guards (when
  `gateway_global_enhancers` is on) and the guards declared below `#[websocket_gateway]` of every gateway of the
  namespace, in that order. The global guards run once, with the first gateway of the namespace as class; each
  gateway's own guards run with that gateway as class. The `ExecutionContext` is a WebSocket context with the pending
  `IoSocket` as client, the handshake as request, `null` data, the gateway as class and a synthetic handler target
  named `connect` that carries the gateway's metadata, so `#[public]` on the gateway is honored by guards that honor
  it and role metadata on individual handlers is not consulted. A guard that returns `false` answers `CONNECT_ERROR
  {"message":"Forbidden resource"}`; a guard that fails answers `CONNECT_ERROR` with the string `message` of
  `exception_payload` and the rest of the payload as `data` (a payload without a string `message` answers
  `Forbidden resource` with the whole payload as `data`); internal errors and panics answer `Internal server error`
  and nothing else. Exception filters do not run for the connection. Handler-level guards still run per message only.
  The pending socket is the one #33 hands to connect middleware: it cannot emit, and what a guard stores on it
  (`insert`, `data`) is there for `handle_connection` and the handlers.

Off by default because it changes what a client of a migrated application observes (a refused connection instead of
`exception` events) and because guards written for messages may read `get_data()`. Whether it should become the
default is Open question 3.

### 10. Route posture

- `posture_gateways` reports Socket.IO gateways with `path` set to the namespace, so the existing error lines
  (`WS  /chat  event "rooms"  ChatGateway::rooms`) and `exempt_message("/chat", "rooms", reason)` keep their form.
  Two gateways on different Engine.IO paths with the same namespace are told apart by the gateway name in the report;
  an exemption matches both (documented).
- The unchecked surface of a Socket.IO gateway is its connection:
  `UncheckedSurface::IoConnection { namespace, path, gateway, guard_connections }`, displayed as
  `Socket.IO connection of ChatGateway on namespace /chat at /socket.io (connection guards on|off)`. Unlike
  `verify_client`, the value is known before `after_init`, so report mode shows it too. Connect middleware
  registered by user code and `allow_request` are not visible to the check and are not claimed.
- With `guard_connections(true)` and enforce mode, a gateway with no connection guard that counts (same rule as for
  handlers: access guards only, at the global or gateway level) and no `#[public]` marker is reported like an
  unguarded handler, with the pseudo-event `connect`; `exempt_message(namespace, "connect", reason)` accepts it.
  `connect` is one of Socket.IO's reserved event names, which a client cannot emit (#33 closes the connection), so
  the pseudo-event cannot be confused with a message handler. Without `guard_connections` nothing new is enforced.

### 11. Limits and their defaults

This layer adds no table, queue or buffer of its own beyond what is listed; it exposes the limits of #32-#34 through
`IoAdapter::options` and keeps their defaults.

| Limit | Default | Reference | When exceeded / effect |
|---|---|---|---|
| Sessions per Engine.IO path, and per client address | 10,000 and 256 (#32) | none | handshake `503` |
| First request after a polling handshake | 10 s (#32) | none | session closed |
| `allow_request` / `allowed_origins` | not set (#32) | not set | `403` |
| Message size, attachments, nesting depth, ack id | 1,000,000 B, 10, 128, 2^53 - 1 (#32, #33) | 1e6, 10, none, none | as #32 / #33 |
| Send buffer per socket; received backlog per session and per engine | 8 MiB; 16 MiB and 256 MiB (#32) | none | `transport error`; reads pause |
| Handlers in flight per connection, **including events waiting for `handle_connection`** | 32 (#33 `max_inflight`) | none | reads pause |
| Events held before `handle_connection` returns | bounded by the row above | not held | discarded if the socket is disconnected |
| `connect_timeout` (no namespace joined) | 45 s (#33) | 45 s | connection closed |
| Pending server-to-client acks, their deadline | 1,000 per socket, 30 s (#33) | none | error to the caller |
| Rooms per socket, room name | 1,000, 256 B (#34) | none | `RoomError` |
| Namespaces a client can reach | the namespaces declared by gateways; no dynamic namespaces | declared ones | `CONNECT_ERROR` `Invalid namespace` |
| Handlers per event per namespace | 1 (startup error otherwise) | several | `init()` fails |
| Acknowledgements per event | at most 1, with one argument (`AckSender` is consumed) | at most 1 per handler | n/a |
| Unknown events | dropped without a reply and without running guards | same | n/a |
| CORS | off; `cors = true` is `*` without credentials | off | no CORS headers |
| Connection guards | off (`guard_connections(false)`) | not available | n/a |
| Errors sent to clients | `exception_payload`: internal errors and panics as `Internal server error`, never the panic text | same for unknown errors | n/a |
| Hook and guard panics | contained; a panicking `handle_connection` disconnects the socket | process-level in Node | n/a |
| Adapter / gateway mismatch | `init()` fails | n/a | n/a |

Two warnings are logged at `init()` because the configuration is legal but risky:

- CORS with credentials on an Engine.IO path without `allowed_origins`: CORS does not apply to the WebSocket
  transport, so a cookie-authenticated namespace is open to cross-site WebSocket hijacking unless the origin
  allowlist (or a connection check) is set. The reflect-origin-with-credentials warning of #32 stays.
- `cors` declared on a gateway under the `ws` adapter (it has no effect there).

Client-observable divergences from the reference, each with a scenario and an entry in
`parity/socket-io/expected-differences.json`: the payload of HTTP exceptions (Decision 7), a binary body seen as
`null` (Decision 6), and, only if task 1.2 finds them, the items corrected there. Events held behind
`handle_connection` and the single handler per event are not observable by one client in the scenario app and are
covered by Rust tests.

### 12. Verification

- Macro tests (`crates/corvis-macros`): the option table of Decision 2, the unknown-option error, the namespace
  validation, `#[websocket_server]` with both field types, `#[connected_socket]` with both socket types and with a
  type that implements neither.
- `crates/corvis/tests/socket_io_gateways.rs` (`#![cfg(feature = "socket-io")]`): in-process applications with a raw
  Engine.IO / Socket.IO client (the helpers #32 and #33 add under `tests/`), one test per spec scenario that does not
  need Node: adapter selection and the no-effect-after-init warning, option grouping and each startup error, body
  mapping, every row of the result table, exceptions with and without filters, hooks and their order, the gate,
  `IoGatewayServer` before and after start, `#[ip]`, global enhancers on and off, two gateways on one namespace, two
  namespaces on one session, raw `corvis::ws` and `on_upgrade` next to Socket.IO gateways.
- `crates/corvis/tests/socket_io_gateways_security.rs`: mismatch errors (socket, server field, hooks, each direction);
  events sent before `handle_connection` returns are not handled early and are discarded when the hook disconnects;
  more than `max_inflight` early events pause reads (measured as #33's tests do); connection guards (deny, error,
  panic, `#[public]`, handler-level guards not consulted, off by default); posture (report entries, enforce with and
  without connection guards, exemption by namespace); panics in handlers, hooks, guards and filters; reserved event in
  a `WsResponse`; the two `init()` warnings.
- The existing gateway suites (`tests/websockets.rs`, `ws_security.rs`, `ws_enhancers.rs`, `auth_ws.rs`,
  `throttler_skip_log_ws.rs`, `send_timeout_ws.rs`, `tls_ws.rs`) pass unmodified: the proof that the default adapter
  did not change.
- Harness: a capability `gateways` in `parity/socket-io/lib/targets.mjs` (`ALL_CAPABILITIES`, `:8`). The Node target
  for it is `parity/socket-io/gateway-reference/` (its own `package.json` and lockfile with exact versions of the
  original framework's core, WebSocket and Socket.IO platform packages and their peers; `npm ci` there is explicit,
  as `ws/socket-io-parity` requires, and `checkInstalledVersions` covers it). The reference app is written in plain
  JavaScript, applying the decorators as functions, so there is no build step. The Corvis side is the same gateway
  application, started in-process by `crates/corvis/tests/socket_io_diff.rs` with capabilities
  `engine-io,socket-io,gateways`. Both expose the loopback-only control routes of #33's scenario app
  (`GET /harness/log`, `POST /harness/reset`) for the server-side log (hook order, what guards saw).
  The app: gateway `/` and gateway `/chat` (two namespaces), a gateway-level guard that denies when
  `auth.token` is missing on `guarded` events, a validation pipe on one handler, an interceptor that wraps one
  result, a gateway exception filter for one exception type, handlers `echo` (returns its body), `silent` (returns
  nothing), `reply` (`WsResponse`), `duck` (object with `event`), `falsy` (returns `0`), `args` (returns the body it
  received, called with zero, one and three arguments), `fail-ws` (string and object), `fail-http`, `fail-unknown`,
  `broadcast` (server field emits to a room), `whoami` (`#[ip]` and handshake), plus `handle_connection` /
  `handle_disconnect` logging and a namespace with CORS for the wire scenario.
  Scenarios under `parity/socket-io/scenarios/gateways/`: `ack`, `no-ack`, `events`, `body-mapping`, `exceptions`,
  `guard`, `pipe`, `interceptor`, `filter`, `lifecycle`, `namespaces`, `server-emit`, `unknown-event`, `wire/cors`,
  and the expected differences `http-exception` and `binary-body`. Absence of a reply is observed through a marker
  event (the scenario asks the server to emit `done` after the handler), never by waiting.
- CI: the tests join `cargo test -p corvis --features socket-io` that #32 adds; Node-gated runs stay out of CI until
  #37.

## Risks / Trade-offs

- #32-#34 are not merged: names may differ → Decision text states needs (a namespace dispatcher, in-flight
  accounting, deferred operator errors), and task 1.1 reconciles before any code.
- The reference behavior was not verified locally → task 1.2 installs the pinned packages and checks the eight items
  first; the harness scenarios encode each one.
- Rejecting unknown `#[websocket_gateway]` options can break a build that carried a no-op option → the error names
  the accepted options; mentioned in the release notes; the three port apps and the examples are checked.
- `path = "/chat"` changes meaning when the adapter is switched → it is the original's rule; startup log line and
  docs table; the handle types have to change anyway, so the switch is never silent.
- Generic lifecycle traits change the trait signatures → defaults keep every existing `impl` and `dyn` use compiling;
  verified by the unmodified test suites and the examples.
- The gate makes a slow `handle_connection` delay the first events of that socket → same as the `ws` adapter today;
  documented; bounded by `max_inflight`.
- Connection guards run guards in a context without message data → opt-in, documented, the synthetic handler target
  keeps metadata lookups well defined.
- Keeping the HTTP exception payload differs from the reference → explicit expected difference; Open question 4.
- Scenarios with the original framework's packages add a second `npm ci` and a slower harness run → separate package,
  only needed for the `gateways` capability: a direct `node run.mjs` without it reports the gateway scenarios as
  skipped, `CORVIS_NODE_DIFF=1` without it fails naming the `npm ci` to run, and #37's job installs it. The main
  spec `ws/socket-io-parity` is not modified: its capability rule already covers a capability a target lacks, and
  the second package follows its version and installation rules (stated in `ws/socket-io-gateways`, "Verified against
  the reference gateway").

## Open questions

1. The issue and `ROADMAP.md` name this change `e2-ioadapter-for-gateways`; the draft uses
   `e2-socket-io-gateway-adapter` as assigned. Rename one of them when the change is accepted (#34 settled the same
   question by updating `ROADMAP.md`).
2. Engine.IO options on the macro: the spike's delta listed `ping_interval`, `ping_timeout`,
   `max_http_buffer_size` and `transports` as `#[websocket_gateway]` options; this design keeps only `namespace`,
   `path` and `cors` there and puts the rest on `IoAdapter::options`. Recommendation: keep; a per-gateway value for a
   per-server setting needs the conflict rules of `cors` for every option.
3. `guard_connections` off by default (parity) or on by default (security over parity)? Recommendation: off for now,
   documented prominently in `docs/websockets/guards.md`, revisited with the 1.0 defaults (spike Review note 1).
4. HTTP exceptions under Socket.IO: keep Corvis' current payload (message and fields of the exception) or answer
   `Internal server error` as the reference does? Recommendation: keep, as an expected difference, so both adapters
   behave the same.
5. Conflicting or duplicated `cors` declarations fail `init()` (the reference lets the first gateway win).
   Recommendation: keep the error.
6. `IoGatewayServer` as the name of the `#[websocket_server]` type for Socket.IO gateways. Alternative:
   `GatewayNamespace`. Recommendation: `IoGatewayServer`, next to `IoServer` / `IoSocket`.
7. Which exact versions of the original framework's packages the gateway reference pins: the latest release of its
   current major at implementation time, recorded in the lockfile and in "Implementation notes". Confirm.
8. The same event in two gateways of one namespace fails `init()`; the reference runs both handlers.
   Recommendation: keep the error (one acknowledgement cannot be answered twice).

## Review notes

These settle the open questions and override the design where they differ.

1. The change keeps the name `e2-socket-io-gateway-adapter`; `ROADMAP.md` is updated to match.
2. The gateway macro takes only `namespace`, `path` and `cors`. Engine.IO options are set on `IoAdapter::options`.
3. `guard_connections` is off by default, as in the reference. The docs page states it prominently and shows how to
   turn it on; the default is revisited with the 1.0 defaults.
4. An HTTP exception thrown under Socket.IO keeps Corvis' payload. It is a listed expected difference.
5. Conflicting `cors` declarations for one path fail `init()`.
6. The server handle is named `IoGatewayServer`.
7. The gateway reference pins the latest release of the current major of the original framework's packages at
   implementation time, recorded in its lockfile.
8. The same event declared by two gateways of one namespace fails `init()`. It is a listed expected difference.
9. Implemented after #34 lands: the first task compares this design with the code #33 and #34 merged and records the
   adjustments in Implementation notes. Task 1.2 (verifying the reference adapter's behavior against the installed
   packages) comes before any runtime code, and whatever it disproves is corrected in the design first.
10. The security review is mandatory before the merge (connection guards and their bypasses, the `connect` exemption
    in route posture, CORS merging, exception payloads, reserved events).

## Implementation notes

Written while implementing, against `origin/dev` at `d290f1d` (#32, #33 and #34 merged).

### 1.1 The layers as merged, and what changed in this design

- `IoOptions` embeds `EngineOptions` (`IoOptions::engine(EngineOptions)`); `EngineOptions::path`, `cors`,
  `allowed_origins`, `allow_request` and `max_sessions` exist as designed; `normalized_path()` (the path with exactly one
  trailing `/`) is crate-private and is the key used to group gateways and to find the options of a path.
  `IoServer::new` validates the options and returns `Result`; `attach(&HttpServer)` fails on overlapping paths; `of(name)`
  returns `Result` (namespace rules) and the main namespace `/` always exists.
- `IoNamespace::on_connection` takes a synchronous `Fn(IoSocket)` (it runs after CONNECT was queued and before the first
  packet is handled), `use_middleware` an async function. There is no hook for events without a handler and the
  in-flight accounting is the `inflight` set of the connection task (`client.rs`): this change adds the crate-private
  `IoNamespace::set_dispatcher` consulted when the socket has no handler of its own for an event (it returns `None` for an
  event nobody handles); the future it returns joins the same `inflight` set, so the 32-per-connection bound, the
  arrival order, the charge of the packet and the cancellation when the client leaves are #33's, unchanged. `IoNamespace`
  and `client.rs` do no extra work when no dispatcher is installed.
- `Handshake::request()` returns `&Request`; the request is now held in an `Arc` (`request_arc()`, crate-private) so the
  execution contexts of a socket share it, and `Handshake::ip()` is resolved once (a `OnceLock`): a gateway reads it for
  every message and resolving it walks `X-Forwarded-For`.
- Deferred operator errors: #34's `BroadcastOperator` always has a namespace, so an operator of a handle that is not
  bound yet cannot be one. `IoGatewayServer` holds the namespace in a `OnceLock` of a **weak** reference (the namespace holds
  the gateway through its dispatcher and hooks, the gateway holds the handle: a strong reference would be a cycle) and
  returns an `IoGatewayBroadcast`, a thin wrapper with the same chain and terminal operations whose terminal operations return
  `EmitError::NotStarted` (new variant) when unbound. `namespace()` is `None` before the gateway started and after its
  server closed.
- Added to #33's code only what the dispatcher needs: `IoNamespace::set_dispatcher`/`dispatcher`, `Handshake::request_arc`,
  `IoValue::to_json_binary_as_null`, `EmitError::NotStarted`; nothing of the Engine.IO, parser, budget or adapter code changed.
- `GatewayAdapterKind` lives in `discovery.rs` (it is needed by `GatewayInfo` and `WsEnhancers`, which are always compiled)
  and is re-exported from `corvis::websockets`; the adapter an application selected is the hidden
  `GatewayAdapterChoice` (`Ws`, or `SocketIo(Arc<IoAdapter>)` with the feature) stored in `RouterConfig`.
- `GatewayInfo` gained `io`, `io_on_*` (the hooks implemented for `IoNamespace` / `IoSocket`) and `server_kind`;
  `WsEnhancers` gained `socket_kinds`. Without the `socket-io` feature the `hooks::detect!` probes for the Socket.IO hooks
  always answer `None` (the macro output is the same with and without the feature).
- Macro: `macro_rules!` forwards a `$x:literal` or `$x:expr` as an invisible group; the option parser looks through it.

### 1.2 The reference, verified against the installed packages

Packages pinned in `parity/socket-io/gateway-reference/package.json` (exact versions, `package-lock.json`):
`@nestjs/common`, `core`, `websockets`, `platform-express` (needed to build the HTTP application the platform adapter
attaches to) and `platform-socket.io` 12.4.2, `reflect-metadata` 0.2.2, `rxjs` 7.8.2; `socket.io` 4.8.4 comes with the
platform package. Read in `web-sockets-controller.js`, `socket-server-provider.js`, `ws-context-creator.js`,
`ws-proxy.js`, `base-ws-exception-filter.js` and `io-adapter.js`, and checked by the harness scenarios (which replace the
throwaway script: Node against Node and Node against Corvis agree on every scenario except the two listed differences).

| Item | Result |
|---|---|
| 1. One server per path, first gateway's options win | Confirmed (`SocketServerProvider.scanForSocketServer` keys servers by port and path and decorates later gateways with `server.of(namespace)`). |
| 2. `afterInit` gets the namespace | Confirmed. |
| 3. `handleConnection` not awaited; handlers bound right after | Confirmed (`createStaticEventHandler` calls the hook and `subscribeMessages` runs next). New in 12.x: a disconnect that happens inside a connection hook is held until every gateway sharing the server handled the connection. |
| 4. Payload | Corrected: with no argument and no callback the payload is `undefined`, with only a callback `undefined`, with one argument that argument, with several the array (also with a callback: `payload.slice(0, n)`). Corvis maps no argument to `null` (JSON has no `undefined`); the reference app states `body ?? null` where the scenario makes it visible. |
| 5. Results | Confirmed: `null` / `undefined` send nothing; a truthy `event` is emitted with `socket.emit(event, data)` whether or not there is a callback; anything else calls the callback when there is one. |
| 6. Guards | Confirmed: a guard that returns `false` raises `WsException("Forbidden resource")`; guards run per message, never for the connection. |
| 7. Exceptions | **Corrected**: the built-in filter adds `cause: { pattern, data }` by default (`includeCause`), to the payload of a `WsException` with a string and to the unknown-error payload, not to a `WsException` with an object (sent as is); `data` is `undefined` (absent in JSON) when the event had no argument. An `Error` wrapped in a `WsException` is an unknown error. |
| 8. Two gateways on one namespace | Confirmed: both handlers run. Corvis keeps one (startup error), an expected difference not observable by one client. |
| `cors: true` | The wire scenario `gateways/wire/cors` compares the preflight and polling headers for a single origin, `true` and a list of origins (a list is not compared against the reference) and a path without `cors`: no difference. |

### 1.3 Corrections applied

- Decision 7 point 1: under the Socket.IO adapter the built-in mapping adds `cause: { pattern: <event>, data: <body> }` to the
  payloads of a `WsException` with a string, of the denial `Forbidden resource` and of internal errors and panics (still
  `Internal server error`, without the text). The `ws` adapter keeps its frames. `data` is left out when the event had no
  argument. A payload built by a filter, or by `WsException::object`, has no `cause`. The cause is the message the client
  itself sent, bounded by the message limits, and is sent back to that client only.
- A message whose arguments cannot be converted to JSON (an object with a key the deserializer treats as an instruction, nesting
  beyond the limit) is answered with `exception` `Invalid message body` and no handler runs.

### Decisions and deviations

- **Hook bound.** The design ran hooks to completion; the review brief for this change asks that a slow or panicking hook cannot
  hold a slot or a charge for good. `IoAdapter::hook_timeout` (default 30 s, `DEFAULT_HOOK_TIMEOUT`) bounds `after_init`,
  `handle_connection` and `handle_disconnect`: a connection hook that times out or panics disconnects the socket with
  `disconnect(true)`; a disconnect hook that times out is abandoned; an `after_init` that panics or times out fails `init()`
  (it would otherwise abort or hang start-up). A connection hook is dropped, not waited for, when the socket disconnects meanwhile.
- **The main namespace.** An `IoServer` always has `/`. When no gateway of an Engine.IO path declares `/`, the server refuses it
  with `CONNECT_ERROR` `Invalid namespace` (a connect middleware), so a client can reach only declared namespaces.
- **Disconnect hooks run once**, through a flag shared by the `on_disconnect` callback and the case where the socket had already
  left when the connection handler registered it; the gate is discarded at the same moment, which releases the events that
  waited for the hooks.
- **Gate and tasks.** One task per connected socket runs its connection hooks (bounded by `hook_timeout`, dropped when the socket
  leaves); one short task per disconnected socket runs its disconnect hooks. No map or queue grows with client input: the gate
  is a `watch` channel in the socket's own extensions.
- **Connection guards.** The connect middleware is `use_middleware`, so `middleware_timeout` bounds it and a panic answers
  `Internal server error` (#33's containment). A guard error with a payload that has a string `message` answers that message and
  the rest of the payload as `data` (so `WsException::new("x")` gives `data: {"status":"error"}`); the design's rule, kept.
- **Route posture.** `GatewayPosture` carries the Socket.IO view (`io`); the connection is the handler `connect` of the gateway
  with the global and gateway guards. `UncheckedSurface::IoConnection` is shown in report mode too.
- **Not done here**: a manual acknowledgement parameter, binary in the gateway pipeline, a public gateway adapter trait,
  duplicate `ws` gateway paths, the user guide (#38); the examples were not changed (the Socket.IO adapter is opt in and the
  examples run under the default one).

## Security review

Pending: task 7.3 belongs to the maintainer's session.

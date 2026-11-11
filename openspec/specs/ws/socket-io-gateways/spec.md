# ws/socket-io-gateways Specification

## Purpose
How `#[websocket_gateway]` gateways are served over Socket.IO (feature `socket-io`) when an application selects the
Socket.IO adapter: adapter selection, the namespace, path and CORS options, message handlers as events, handler
results as acknowledgements or emitted events, exceptions, the socket and server handles, lifecycle hooks, guards on
messages and on the connection, and the limits of this layer. What a `socket.io-client` 4.8.4 observes MUST match the
same gateway written for the original framework with its Socket.IO platform adapter (npm package
`@nestjs/platform-socket.io`), at the versions pinned in `parity/socket-io/gateway-reference/`, verified by the
differential harness of `ws/socket-io-parity`, except the divergences listed in "Security and behavior divergences are
explicit". Implemented by [#35](https://github.com/randygil/corvis/issues/35).

Timing rule for every scenario below: absence of a reply is observed through a marker event emitted after the handler
finished, never by waiting. "On both servers" means the reference gateway application and the same application
written with Corvis gateways.

## Requirements

### Requirement: The adapter is chosen per application
An application SHALL serve its gateways with the `ws`-style protocol (`{ event, data }` frames) unless it selects
another adapter with `use_websocket_adapter` before `init()`. Selecting the Socket.IO adapter MUST serve every gateway
of that application over Socket.IO and MUST NOT change `corvis::ws` servers, `on_upgrade` listeners or other
applications in the same process. A call after `init()` MUST log a warning and have no effect.

#### Scenario: Existing gateways are unchanged
- **WHEN** the existing gateway test suites (`crates/corvis/tests/websockets.rs`, `ws_security.rs`, `ws_enhancers.rs`,
  `auth_ws.rs`) run after this change
- **THEN** they pass without modification

#### Scenario: Opting in
- **WHEN** an application calls `use_websocket_adapter(IoAdapter::new())` and declares
  `#[websocket_gateway(namespace = "/chat")]`
- **THEN** the official client connecting to namespace `/chat` on the default path `/socket.io` reaches that gateway,
  and a plain WebSocket upgrade to `/chat` does not

#### Scenario: Selected too late
- **WHEN** `use_websocket_adapter` is called after `init()`
- **THEN** a warning is logged and the gateways keep the adapter they were built with

#### Scenario: Plain WebSocket servers next to Socket.IO gateways
- **WHEN** an application with Socket.IO gateways also registers an `on_upgrade` listener for `/ws/firehose`
- **THEN** upgrades to `/ws/firehose` reach the listener as before

### Requirement: Gateway options under Socket.IO
Under the Socket.IO adapter `#[websocket_gateway]` SHALL read `namespace` (or the positional literal) as the Socket.IO
namespace, default `/`; `path` as the Engine.IO path, default `/socket.io`; and `cors` (`true`, one origin or a list
of origins) as the CORS configuration of that path. Under the `ws` adapter `namespace`, `path` and the literal MUST
keep their current meaning, the URL path.

#### Scenario: Namespace and default path
- **WHEN** a gateway declares `#[websocket_gateway("/chat")]` under the Socket.IO adapter
- **THEN** it serves namespace `/chat` on the Engine.IO path `/socket.io`

#### Scenario: Custom Engine.IO path
- **WHEN** a gateway declares `#[websocket_gateway(path = "/realtime", namespace = "/chat")]`
- **THEN** a client configured with `path: "/realtime"` reaches namespace `/chat`, and `/socket.io` answers as a path
  without a Socket.IO server

#### Scenario: Path alone
- **WHEN** a gateway declares `#[websocket_gateway(path = "/realtime")]` under the Socket.IO adapter
- **THEN** it serves namespace `/` on the Engine.IO path `/realtime`, and the startup log names both

#### Scenario: Same declaration under the ws adapter
- **WHEN** `#[websocket_gateway(namespace = "/chat")]` runs under the default adapter
- **THEN** it accepts WebSocket upgrades at `/chat`, as before this change

#### Scenario: CORS preflight
- **WHEN** a gateway declares `cors = "https://app.example"` and a browser-style client sends a preflight `OPTIONS`
  for a polling request from that origin
- **THEN** the response carries the same CORS headers as the reference gateway declared with that origin

#### Scenario: CORS for any origin
- **WHEN** a gateway declares `cors = true` and a polling request carries an `Origin` header
- **THEN** the response allows any origin without credentials, as the reference gateway declared with `cors: true`

### Requirement: Gateway options are checked at compile time
The namespace of a gateway MUST be validated when the gateway is compiled, with the namespace name rules of
`ws/socket-io` (at most 256 bytes, no `,`, no control characters). An option `#[websocket_gateway]` does not know MUST
be a compile error that names the accepted options, under either adapter.

#### Scenario: Unknown option
- **WHEN** a gateway declares `#[websocket_gateway(namspace = "/chat")]`
- **THEN** compilation fails with a message that names `path`, `namespace` and `cors`

#### Scenario: Invalid namespace literal
- **WHEN** a gateway declares `namespace = "/a,b"`
- **THEN** compilation fails with a message about the namespace name

#### Scenario: Invalid cors value
- **WHEN** a gateway declares `cors = 3`
- **THEN** compilation fails with a message that names the accepted forms of `cors`

### Requirement: Gateways that share an Engine.IO path share one server
Gateways whose Engine.IO path is the same SHALL be namespaces of one Socket.IO server, so one client connection
reaches all of them. The options of a path MUST come from the adapter (`IoAdapter::options`, one `IoOptions` per path,
the defaults otherwise) with the `cors` declared by the path's gateways applied.

#### Scenario: Two gateways, two namespaces
- **WHEN** two gateways declare namespaces `/a` and `/b` with the default path
- **THEN** one client manager reaches both over a single Engine.IO session, on both servers

#### Scenario: Options from the adapter
- **WHEN** the adapter is given `IoOptions` with `ping_interval` 300 ms for the default path
- **THEN** the handshake of that path announces `pingInterval` 300

#### Scenario: Path without options
- **WHEN** the adapter is given options for the default path only and a gateway declares `path = "/realtime"`
- **THEN** the server of `/realtime` uses the default options

### Requirement: Ambiguous path configuration fails startup
`init()` MUST fail, naming the gateways or the path involved, when the configuration of an Engine.IO path is ambiguous
or invalid: gateways of one path declare different `cors`, a gateway declares `cors` and the adapter's options for
that path already configure CORS, options are given twice for one path or do not pass validation, or two Engine.IO
paths overlap. Servers created before the failure MUST be closed.

#### Scenario: Conflicting CORS
- **WHEN** two gateways on the default path declare `cors = "https://a.example"` and `cors = "https://b.example"`
- **THEN** `init()` fails with a message naming both gateways

#### Scenario: CORS declared twice
- **WHEN** a gateway declares `cors = true` and the adapter's options for its path set `EngineOptions::cors`
- **THEN** `init()` fails with a message naming the gateway and the path

#### Scenario: Options given twice
- **WHEN** `IoAdapter::options` is called twice with options for the same Engine.IO path
- **THEN** `init()` fails with a message naming the path

#### Scenario: Overlapping paths
- **WHEN** one gateway declares `path = "/io"` and another `path = "/io/v2"`
- **THEN** `init()` fails with a message naming both paths

### Requirement: One handler per event per namespace
Several gateways MAY declare the same namespace; their message handlers SHALL be served together. `init()` MUST fail
when two handlers of one namespace, in the same gateway or in different ones, subscribe to the same event, naming
both.

#### Scenario: Two gateways on one namespace
- **WHEN** gateways `A` and `B` both declare namespace `/chat` with different events
- **THEN** a socket connected to `/chat` reaches the handlers of both, and the connection hooks of both run

#### Scenario: Same event in two gateways
- **WHEN** gateways `A` and `B` on namespace `/chat` both subscribe to `message`
- **THEN** `init()` fails with a message naming `A`, `B` and `message`

### Requirement: Handles match the adapter
Under the Socket.IO adapter `#[connected_socket]` SHALL be an `IoSocket`, `#[websocket_server]` an `IoGatewayServer`,
and the lifecycle hooks `OnGatewayInit<IoNamespace>`, `OnGatewayConnection<IoSocket>` and
`OnGatewayDisconnect<IoSocket>`. Under the `ws` adapter they stay `WsClient`, `WsServer` and the hooks without a type
argument, which MUST keep compiling unchanged. A `#[connected_socket]` or `#[websocket_server]` of any other type
MUST be a compile error.

#### Scenario: Socket.IO handles
- **WHEN** a gateway under the Socket.IO adapter takes `#[connected_socket] socket: IoSocket` in a handler and
  implements `OnGatewayConnection<IoSocket>`
- **THEN** the handler and the hook receive the socket that sent the event, with its id, handshake and rooms

#### Scenario: Existing hook implementations
- **WHEN** a gateway written before this change has `impl OnGatewayConnection for Chat` and `impl OnGatewayInit for Chat`
- **THEN** it compiles and runs unchanged under the `ws` adapter

#### Scenario: Socket parameter of another type
- **WHEN** a handler declares `#[connected_socket] client: String`
- **THEN** compilation fails

#### Scenario: Client in a guard
- **WHEN** a guard calls `ctx.switch_to_ws().get_client::<IoSocket>()` for a message of a Socket.IO gateway
- **THEN** it receives the socket that sent the event, and `ctx.request()` is that socket's handshake request

#### Scenario: Client address
- **WHEN** a handler declares `#[ip] ip: String` under the Socket.IO adapter and the application trusts its proxy
- **THEN** the value is the client address resolved from the handshake request with that setting

### Requirement: A gateway written for the other adapter fails startup
`init()` MUST fail, before any code of the gateway runs and naming the gateway and the item, when a gateway declares
a socket parameter or a server field of the adapter that is not selected, or declares a lifecycle hook only for the
adapter that is not selected. A gateway that declares each of its hooks for both adapters and has no adapter-specific
parameter or field MUST start under either.

#### Scenario: Socket handle of the wrong adapter
- **WHEN** a handler takes `#[connected_socket] client: WsClient` and the application uses the Socket.IO adapter
- **THEN** `init()` fails with a message naming the gateway, the handler and `IoSocket`

#### Scenario: Socket.IO gateway under the default adapter
- **WHEN** a gateway has a `#[websocket_server] server: IoGatewayServer` field and the application did not select the
  Socket.IO adapter
- **THEN** `init()` fails with a message naming the gateway, the field and `WsServer`

#### Scenario: Hook of the wrong adapter
- **WHEN** a gateway implements only `OnGatewayConnection` for `WsClient` and the application uses the Socket.IO
  adapter
- **THEN** `init()` fails instead of starting without the hook

#### Scenario: Gateway written for both adapters
- **WHEN** a gateway implements `OnGatewayConnection` for `WsClient` and for `IoSocket` and has no adapter-specific
  parameters
- **THEN** it starts under either adapter and the matching hook runs

### Requirement: The server handle is bound when the gateway starts
An `IoGatewayServer` SHALL offer the broadcast operators of `ws/socket-io-rooms` on the gateway's namespace. Before
the application started its gateways every operation on it MUST return an error without sending anything and without
panicking.

#### Scenario: Server handle before start
- **WHEN** a provider emits through an `IoGatewayServer` before the application initialized its gateways
- **THEN** the call returns an error that says the gateway has not started, nothing is sent and nothing panics

#### Scenario: Server handle after start
- **WHEN** a handler calls `self.server.to("room").emit("news", data)` on its `IoGatewayServer`
- **THEN** exactly the sockets of the gateway's namespace that joined `room` receive `news`

#### Scenario: Namespace of the handle
- **WHEN** code reads `namespace()` of an `IoGatewayServer` before and after the gateway started
- **THEN** it is `None` before and the gateway's `IoNamespace` after

### Requirement: Message handlers are Socket.IO events
A `#[subscribe_message("e")]` handler SHALL run when a socket of the gateway's namespace emits event `e`, through the
same pipeline as under the `ws` adapter: global, gateway and handler guards, interceptors, pipes, the handler, and
the handler's and gateway's exception filters in the order of `ws/exception-filters`. The execution context MUST be a
WebSocket context whose client is the `IoSocket` and whose request is the handshake request.

#### Scenario: Global enhancers
- **WHEN** a global guard is registered and `apply_global_enhancers_to_gateways` was not turned off
- **THEN** the guard runs for every Socket.IO message before the gateway's and the handler's guards

#### Scenario: Global enhancers turned off
- **WHEN** `apply_global_enhancers_to_gateways(false)` was called and a global guard denies everything
- **THEN** Socket.IO messages are handled without that guard

#### Scenario: Pipe on the body
- **WHEN** a handler has a validation pipe on its `#[message_body]` and the client emits an invalid body
- **THEN** the handler does not run and the client receives `exception`

#### Scenario: Handler metadata in guards
- **WHEN** a guard reads the handler's metadata through the execution context for a Socket.IO message
- **THEN** it reads the same metadata as for that handler under the `ws` adapter

### Requirement: The message body is built from the event arguments
The message body (`#[message_body]`, the context's data, the input of pipes) MUST be built from the event's arguments
as the reference adapter does: no argument gives `null`, one argument gives that argument, several give the array of
them. The acknowledgement MUST NOT count as an argument. The body is JSON: a binary value in the arguments MUST be
seen as `null`.

#### Scenario: One argument
- **WHEN** the client emits `echo` with `{"a":1}` and a callback, and the handler returns its body
- **THEN** the callback receives `{"a":1}` on both servers

#### Scenario: Several arguments
- **WHEN** the client emits `args` with `1`, `"two"` and `{"three":3}` and a callback, and the handler returns its body
- **THEN** the callback receives `[1,"two",{"three":3}]` on both servers

#### Scenario: No argument
- **WHEN** the client emits `args` with only a callback
- **THEN** the handler's body is `null`, on both servers

#### Scenario: Property of the body
- **WHEN** a handler declares `#[message_body("room")] room: String` and the client emits `{"room":"r1"}`
- **THEN** the parameter is `r1`, as under the `ws` adapter

#### Scenario: Binary argument
- **WHEN** the client emits an event whose only argument is a buffer to a handler that returns its body
- **THEN** the Corvis handler sees `null`, and the harness lists the difference from the reference as expected

### Requirement: Events without a gateway handler
An event no handler of the namespace subscribes to MUST be dropped without a reply and without running guards. A
handler registered directly on the socket with `IoSocket::on` MUST take precedence over the gateway's handler for
that event.

#### Scenario: Unknown event
- **WHEN** the client emits an event no handler subscribes to, with a callback
- **THEN** the callback is never called and no `exception` event arrives, on both servers

#### Scenario: Handler registered on the socket
- **WHEN** `handle_connection` registers `socket.on("upload", ...)` and the gateway also subscribes to `upload`
- **THEN** the handler registered on the socket runs, with the event's arguments and its acknowledgement, and the
  gateway's handler and guards do not

### Requirement: Handler results map like the reference adapter
A handler's result SHALL reach the client as the reference adapter sends it: nothing for an empty result, an emitted
event for a `WsResponse` or a value that serializes to an object with a non-empty string `event`, and the client's
acknowledgement, with the value as its single argument, for any other value. The acknowledgement MUST be answered at
most once and MUST stay unanswered when the result was empty or was emitted as an event.

#### Scenario: Return value goes to the acknowledgement
- **WHEN** the client emits `echo` with `"ping"` and a callback
- **THEN** the callback receives `"ping"` on both servers

#### Scenario: Falsy value is still acknowledged
- **WHEN** the handler returns `0` and the client sent a callback
- **THEN** the callback receives `0` on both servers

#### Scenario: No acknowledgement requested
- **WHEN** the client emits `echo` without a callback
- **THEN** the client receives nothing before the marker event, on both servers

#### Scenario: Handler returns nothing
- **WHEN** the client emits `silent` with a callback and the handler returns nothing, or a value that serializes to
  `null`
- **THEN** the callback is not called before the marker event, on both servers

#### Scenario: WsResponse is emitted
- **WHEN** the handler returns `WsResponse::new("reply", 1)` and the client sent a callback
- **THEN** the client receives event `reply` with `1` and the callback is not called, on both servers

#### Scenario: Object with an event field
- **WHEN** the handler returns a value that serializes to `{"event":"duck","data":2}`
- **THEN** the client receives event `duck` with `2`, on both servers

#### Scenario: Interceptor replaces the result
- **WHEN** an interceptor wraps the handler's value as `{"wrapped": value}` and the client sent a callback
- **THEN** the callback receives the wrapped value, on both servers

#### Scenario: Reserved event in a WsResponse
- **WHEN** a handler returns `WsResponse::new("disconnect", 1)`
- **THEN** nothing is sent to the client, the failure is logged with the gateway and the handler, and the socket stays
  connected

### Requirement: Exceptions are emitted as the exception event
A guard that denies, a `WsException`, a failing pipe or interceptor, any other error and a panic SHALL be reported to
the socket as event `exception`, and the client's acknowledgement MUST stay unanswered. The payload MUST be the one
the gateway's exception filters produce, or the built-in mapping, which MUST equal the reference's for WebSocket
exceptions and MUST report unknown errors as `Internal server error` without their text.

#### Scenario: Guard denies
- **WHEN** a guard returns `false` for an event
- **THEN** the client receives `exception` with status `error` and message `Forbidden resource`, on both servers

#### Scenario: WsException with a string
- **WHEN** a handler fails with `WsException::new("Room is full")`
- **THEN** the client receives `exception` with the same payload as the reference gateway

#### Scenario: WsException with an object
- **WHEN** a handler fails with `WsException::object({"code":"ROOM_FULL"})`
- **THEN** the client receives `exception` with the same payload as the reference gateway

#### Scenario: Unknown error
- **WHEN** a handler panics or returns an internal error
- **THEN** the client receives `exception` with message `Internal server error` and nothing of the original text, on
  both servers

#### Scenario: Acknowledgement after a failure
- **WHEN** the client emits an event with a callback and the handler fails
- **THEN** the callback is not called before the marker event, on both servers

#### Scenario: Gateway filter
- **WHEN** a gateway declares an exception filter that catches the raised exception and answers a JSON body
- **THEN** the client receives `exception` with that body

#### Scenario: Filter with an empty body
- **WHEN** the filter that catches the exception answers with an empty body
- **THEN** nothing is sent to the client

#### Scenario: HTTP exception
- **WHEN** a handler fails with `BadRequestException::new("bad input")`
- **THEN** the Corvis client receives `exception` with that message, as under the `ws` adapter, and the harness lists
  the difference from the reference (`Internal server error`) as expected

### Requirement: Lifecycle hooks under Socket.IO
`after_init` SHALL run once per gateway with its namespace before any client can connect. `handle_connection` SHALL
run for each socket after the namespace's connect middleware accepted it, in the registration order of the
namespace's gateways, and `handle_disconnect` exactly once per socket, after it left its rooms. A hook that panics
MUST NOT take the server down.

#### Scenario: Connection hook sees the handshake
- **WHEN** a client connects with `auth: { token: "t" }`
- **THEN** the connection hook reads the same `auth` object, on both servers

#### Scenario: Hook order
- **WHEN** a client connects, emits one event and disconnects
- **THEN** the server-side log shows init before the connection, the connection before the message, and one
  disconnect, on both servers

#### Scenario: Middleware registered in after_init
- **WHEN** a gateway registers a connect middleware on the namespace it receives in `after_init` and the middleware
  rejects a client
- **THEN** the client receives `connect_error` and `handle_connection` does not run for it

#### Scenario: Rooms in the disconnect hook
- **WHEN** a socket that joined room `r` disconnects
- **THEN** `handle_disconnect` runs once and reads no rooms for the socket

#### Scenario: Connection hook panics
- **WHEN** `handle_connection` panics for a socket
- **THEN** that socket is disconnected and its connection closed, `handle_disconnect` runs for it, and other sockets
  are unaffected

### Requirement: Events wait for the connection hook
No event of a socket MUST be dispatched before every `handle_connection` hook of its namespace returned. Events
received meanwhile MUST wait, counted in the per-connection in-flight bound, and MUST be discarded without running
guards or handlers when a hook disconnected the socket.

#### Scenario: Early event waits for the hook
- **WHEN** a client sends an event immediately after connecting while `handle_connection` is still running
- **THEN** the handler runs only after the hook returned, and the event is not lost

#### Scenario: Hook rejects the socket
- **WHEN** `handle_connection` disconnects the socket and the client had already sent an event
- **THEN** no guard and no handler runs for that event

#### Scenario: Early events are bounded
- **WHEN** a client sends more events than `max_inflight` while `handle_connection` has not returned
- **THEN** the server stops reading from that connection at the limit and resumes when the hook returns

### Requirement: Guards run per message unless connection guards are enabled
Guards SHALL run per message only, as in the reference, unless the application sets
`IoAdapter::guard_connections(true)`. With it, the global guards (while global enhancers apply to gateways) and the
guards declared on the gateways of a namespace MUST also run when a socket connects to that namespace, before it is
connected. Handler-level guards MUST NOT run for the connection.

#### Scenario: Off by default
- **WHEN** a gateway has a guard that denies and the adapter was created with `IoAdapter::new()`
- **THEN** a client connects, as with the reference gateway, and only its messages are denied

#### Scenario: Context of a connection guard
- **WHEN** `guard_connections(true)` is set and a gateway-level guard runs for a connecting socket
- **THEN** its execution context is a WebSocket context whose client is the socket, whose request is the handshake,
  whose data is `null`, whose class is the gateway and whose handler is named `connect` and carries the gateway's
  metadata

#### Scenario: Handler-level guard
- **WHEN** `guard_connections(true)` is set and a guard that denies is declared on one handler only
- **THEN** clients connect, and only messages to that handler are denied

#### Scenario: Public gateway
- **WHEN** `guard_connections(true)` is set, a global guard honors the public marker and the gateway is marked public
- **THEN** a client without credentials connects

### Requirement: A denied connection is refused with CONNECT_ERROR
When a connection guard returns `false` or fails, the server MUST answer `CONNECT_ERROR` instead of connecting the
socket: its hooks MUST NOT run, it MUST NOT receive broadcasts, and exception filters MUST NOT run. The message is
`Forbidden resource` for a denial, the exception's message for a failure, and `Internal server error` for internal
errors and panics.

#### Scenario: Connection denied
- **WHEN** `guard_connections(true)` is set and the gateway's guard returns `false` for a client
- **THEN** the client receives `connect_error` with message `Forbidden resource` and `handle_connection` does not run

#### Scenario: Guard fails with an exception
- **WHEN** a connection guard fails with `WsException::object({"message":"Token expired","code":"EXPIRED"})`
- **THEN** the client receives `connect_error` with message `Token expired` and data `{"code":"EXPIRED"}`

#### Scenario: Denied socket receives no broadcast
- **WHEN** a client was denied by a connection guard and the server then emits to the whole namespace
- **THEN** that client receives nothing

#### Scenario: Guard panics
- **WHEN** a connection guard panics
- **THEN** the client receives `connect_error` with message `Internal server error` and the server keeps running

### Requirement: Limits of the Socket.IO layers apply to gateways
Every limit of `ws/engine-io`, `ws/socket-io` and `ws/socket-io-rooms` SHALL apply to gateways with its default and
SHALL be configurable through `IoAdapter::options`. This layer MUST NOT add state that grows with client input beyond
them: events waiting for `handle_connection` count as handlers in flight (default 32 per connection), and a client
can reach only the namespaces gateways declare.

#### Scenario: Undeclared namespace
- **WHEN** a client connects to a namespace no gateway declares
- **THEN** it receives `connect_error` with message `Invalid namespace`, on both servers

#### Scenario: Session cap from the adapter options
- **WHEN** the adapter's options set `max_sessions` to 1 and a second client opens a session
- **THEN** its handshake is refused with `503`

#### Scenario: Handlers in flight
- **WHEN** a client emits more events than `max_inflight` to a handler that does not return
- **THEN** the server stops reading from that connection at the limit, as `ws/socket-io` specifies

### Requirement: Risky configurations are reported at startup
`init()` SHALL log a warning when an Engine.IO path has CORS with credentials and no origin allowlist, and when a
gateway declares `cors` under the `ws` adapter, where it has no effect.

#### Scenario: Credentials without an allowlist
- **WHEN** the options of a path enable CORS with credentials and `allowed_origins` is not set
- **THEN** a warning names the path and the origin allowlist

#### Scenario: CORS under the ws adapter
- **WHEN** a gateway declares `cors = true` and the application uses the default adapter
- **THEN** a warning names the gateway, and the gateway serves WebSocket upgrades as without the option

### Requirement: Security and behavior divergences are explicit
Every behavior of this layer that differs from the reference gateway SHALL be documented in
`docs/websockets/adapter.md` (English and Spanish). Each one a client can observe MUST have an entry in the harness's
`expected-differences.json` with the scenario that exercises it.

#### Scenario: Divergences are listed
- **WHEN** the harness runs the gateway scenarios against both servers
- **THEN** every difference reported is listed with its reason, and no listed difference is stale

#### Scenario: Client-observable divergences
- **WHEN** the list of expected differences of the gateway scenarios is read
- **THEN** it holds the payload of HTTP exceptions that are not WebSocket exceptions (Corvis forwards the exception's
  message and fields; the reference answers `Internal server error`) and binary values in a message body (seen as
  `null`), and nothing else unless the reference check of the tasks recorded it

#### Scenario: Divergences a single client cannot observe
- **WHEN** the documentation page is read
- **THEN** it also describes events held until `handle_connection` returns, one handler per event per namespace,
  startup errors for conflicting options, and connection guards

### Requirement: Verified against the reference gateway
The gateway scenarios of the differential harness SHALL run the official client against a reference application
written with the original framework and its Socket.IO adapter, pinned with exact versions and a lockfile in
`parity/socket-io/gateway-reference/`, and against the same gateways written with Corvis. They require the capability
`gateways`; a scenario that requires it MUST be reported as skipped, never as passed, on a target that lacks it.

#### Scenario: Gateway scenarios pass
- **WHEN** the harness runs with `CORVIS_NODE_DIFF=1` and both packages installed
- **THEN** the gateway scenarios report no unlisted and no stale difference

#### Scenario: Reference package not installed
- **WHEN** `CORVIS_NODE_DIFF=1` is set and `parity/socket-io/gateway-reference/node_modules` does not exist
- **THEN** the test fails with a message that names `npm ci` in `parity/socket-io/gateway-reference/`

#### Scenario: Direct run without the reference package
- **WHEN** the harness is started with `node run.mjs`, without `CORVIS_NODE_DIFF`, and the reference package is not
  installed
- **THEN** the gateway scenarios are reported as skipped and the other scenarios run

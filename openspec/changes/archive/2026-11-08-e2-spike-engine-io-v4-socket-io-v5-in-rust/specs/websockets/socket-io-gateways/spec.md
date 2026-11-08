# Spec Delta

## Purpose

Defines how `#[websocket_gateway]` gateways are served over socket.io when an application selects the socket.io
adapter: namespace, path and CORS options, `#[subscribe_message]`, `#[connected_socket]`, `#[message_body]`, handler
results, `WsException` and guards, pipes and interceptors per event. What a socket.io client observes MUST match the
same gateway written for the original framework with its socket.io platform adapter (npm package
`@nestjs/platform-socket.io`, `IoAdapter`), verified by the differential harness of `websockets/socket-io-parity`.
Implemented by [#35](https://github.com/randygil/corvis/issues/35).

## ADDED Requirements

### Requirement: The adapter is chosen per application
An application SHALL keep serving gateways with the current `ws`-style protocol (`{ event, data }` frames) unless it
selects the socket.io adapter explicitly. Selecting it MUST serve every gateway of that application over socket.io, and
MUST NOT change how existing `ws`-style applications, `corvis::ws` servers or `server.on('upgrade')` listeners behave.

#### Scenario: Existing gateways are unchanged
- **WHEN** the existing gateway test suites (`crates/corvis/tests/websockets.rs`, `crates/corvis/tests/ws_security.rs`)
  run after socket.io support lands
- **THEN** they pass without modification

#### Scenario: Opting in
- **WHEN** an application selects the socket.io adapter and declares `#[websocket_gateway(namespace = "/chat")]`
- **THEN** the official `socket.io-client` connecting to `/chat` on the default path `/socket.io` reaches that gateway

### Requirement: Gateway options under socket.io
Under the socket.io adapter, `#[websocket_gateway]` SHALL accept `namespace` (default `/`), `path` (the Engine.IO path,
default `/socket.io`), `cors` and the Engine.IO options (`ping_interval`, `ping_timeout`, `max_http_buffer_size`,
`transports`), with the same defaults as the original adapter. Under the `ws` adapter, `namespace` MUST keep its current
meaning (a synonym of `path`). Gateways that share a `path` MUST share one Engine.IO server.

#### Scenario: CORS preflight
- **WHEN** a gateway declares `cors` with origin `https://app.example` and a browser-style client sends a preflight
  `OPTIONS` for a polling request from that origin
- **THEN** the response carries the same CORS headers as the Node reference gateway

#### Scenario: Two gateways, two namespaces
- **WHEN** two gateways declare namespaces `/a` and `/b` with the default path
- **THEN** one client manager reaches both over a single Engine.IO session

### Requirement: Handler results map like the original adapter
A `#[subscribe_message("e")]` handler SHALL receive the first event argument as `#[message_body]` and the socket as
`#[connected_socket]`. When the handler returns a `WsResponse`, the server MUST emit that event with its data to the
socket; when it returns any other non-null value and the client sent an acknowledgement callback, the value MUST be
sent as the acknowledgement; otherwise the value MUST be dropped. Events without a handler MUST be ignored.

#### Scenario: Return value goes to the ack
- **WHEN** the client emits `ping` with a callback and the handler returns `"pong"`
- **THEN** the callback receives `"pong"` on both servers

#### Scenario: WsResponse is emitted
- **WHEN** the handler returns `WsResponse::new("reply", 1)`
- **THEN** the client receives event `reply` with `1` on both servers, and no acknowledgement

### Requirement: Exceptions and enhancers match the original adapter
Guards, interceptors and pipes declared on the gateway or the handler SHALL run for every event in the same order as
today (`run_handler`). A guard that denies, a `WsException` or any other error MUST be emitted to the socket as event
`exception` with the same payload as the original base WebSocket exception filter, and unknown errors MUST be reported
as `Internal server error` without leaking details.

#### Scenario: Guard denies
- **WHEN** a guard returns `false` for an event
- **THEN** the client receives `exception` with `{"status":"error","message":"Forbidden resource"}` on both servers

#### Scenario: Validation pipe fails
- **WHEN** a validation pipe rejects the message body
- **THEN** the client receives `exception` with the same payload as the Node reference gateway

### Requirement: Lifecycle hooks under socket.io
`on_init`, `on_connection` and `on_disconnect` hooks SHALL run as in the original adapter: init once with the namespace
server, connection after the socket joined its namespace (after connect middleware), and disconnect with the socket
after it left.

#### Scenario: Connection hook sees the handshake
- **WHEN** a client connects with `auth: { token: "t" }`
- **THEN** the connection hook can read the same `auth` object and the client can be disconnected from it

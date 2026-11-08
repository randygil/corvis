# Spec Delta

## Purpose

Defines the Socket.IO v5 layer on top of `websockets/engine-io`: packet encoding, namespaces, connection with `auth`,
connect middleware, events, acknowledgements in both directions, binary attachments, connect errors and disconnect
reasons. Behavior observed by the official `socket.io-client` 4.8.x MUST match Node `socket.io` 4.8.x, verified by the
differential harness of `websockets/socket-io-parity`. Implemented by
[#33](https://github.com/randygil/corvis/issues/33).

## ADDED Requirements

### Requirement: Packet encoding matches socket.io-parser
Packets SHALL be encoded and decoded as `socket.io-parser` 4.2.x does: `<type>[<attachments>-][<nsp>,][<ackId>][JSON]`
with types 0 CONNECT to 6 BINARY_ACK, the main namespace `/` omitted, and binary values replaced by
`{"_placeholder":true,"num":<n>}` with the attachments sent as separate binary Engine.IO packets. A packet that cannot
be decoded MUST close the connection with reason `parse error`.

#### Scenario: Encoding round trip
- **WHEN** the same events (strings, numbers, nested objects, arrays, `null`, several arguments) are emitted by both
  servers
- **THEN** the Engine.IO message payloads observed on the wire are byte-equal

#### Scenario: Malformed packet
- **WHEN** a raw client sends a socket.io packet with an invalid type or invalid JSON
- **THEN** both servers close the connection and report `parse error` as the disconnect reason

### Requirement: Namespaces and connection match Node
A client SHALL join a namespace by sending CONNECT with an optional `auth` object, and the server MUST answer with
CONNECT carrying `{"sid":<id>}`; the socket id MUST differ from the Engine.IO session id. Several namespaces SHALL be
multiplexed over one Engine.IO session. CONNECT to a namespace that does not exist MUST be answered with CONNECT_ERROR
`{"message":"Invalid namespace"}`. A session that joins no namespace within `connectTimeout` (default 45000 ms) MUST be
closed.

#### Scenario: Auth is visible to the server
- **WHEN** the official client connects with `auth: { token: "abc" }`
- **THEN** the server-side handshake exposes the same `auth` object on both servers, and the client receives a `sid`

#### Scenario: Unknown namespace
- **WHEN** the official client connects to `/missing`
- **THEN** its `connect_error` handler receives an error with message `Invalid namespace` on both servers

#### Scenario: Two namespaces on one session
- **WHEN** the client opens `/` and `/admin` from the same manager
- **THEN** both namespaces share one Engine.IO session and each receives only its own events

#### Scenario: No namespace joined in time
- **WHEN** a raw client completes the Engine.IO handshake but sends no CONNECT, with `connectTimeout` 1000 on both
  servers
- **THEN** both servers close the Engine.IO session after about one second

### Requirement: Connect middleware matches Node
Namespace middleware SHALL run in registration order before a socket connects. An error from middleware MUST reject the
connection with CONNECT_ERROR `{"message":<message>}` plus `"data"` when the error carries data, and the socket MUST NOT
be added to the namespace.

#### Scenario: Middleware rejects with data
- **WHEN** a middleware fails with message `not authorized` and data `{"content":"retry later"}`
- **THEN** the client's `connect_error` receives `err.message == "not authorized"` and `err.data` equal to the data, on
  both servers

### Requirement: Events match Node
Events SHALL carry any number of JSON arguments in both directions. Reserved event names (`connect`, `connect_error`,
`disconnect`, `disconnecting`, `newListener`, `removeListener`) MUST be rejected when emitted by server code.

#### Scenario: Several arguments
- **WHEN** the server emits `update` with `1`, `"two"` and `{"three":3}`
- **THEN** the client handler receives the same three arguments on both servers

#### Scenario: Reserved name
- **WHEN** server code emits `disconnect` to a socket
- **THEN** the emit is rejected and nothing is sent, as in Node

### Requirement: Acknowledgements match Node
A client emit with a callback SHALL be answered with an ACK whose payload is the array of arguments given to the
server's ack function. A server emit with an ack MUST resolve with the client's answer or fail with the error message
`operation has timed out` after the configured timeout.

#### Scenario: Client ack with several values
- **WHEN** the client emits `sum` with `[1, 2]` and a callback, and the server acks with `3` and `"ok"`
- **THEN** the callback receives `(3, "ok")` on both servers

#### Scenario: Server ack timeout
- **WHEN** the server emits with a 200 ms ack timeout to a client that never answers
- **THEN** the server side fails with `operation has timed out` on both servers

### Requirement: Binary attachments match Node
Arguments containing binary values (at any depth) SHALL be sent as BINARY_EVENT / BINARY_ACK with attachments and
received with the binary values restored in place, over both polling and WebSocket.

#### Scenario: Binary echo over polling
- **WHEN** a client restricted to polling emits an object containing a 1 KiB buffer and the server echoes it
- **THEN** the client receives an equal buffer at the same position, on both servers

### Requirement: Disconnect reasons match Node
Disconnections SHALL report the same reason as Node on the server side: `client namespace disconnect`,
`server namespace disconnect`, `transport close`, `transport error`, `ping timeout`, `parse error`, `forced close`,
`server shutting down` and `forced server close`. A server-side `disconnect(true)` MUST close the whole Engine.IO
session; `disconnect()` without `true` MUST only leave the namespace.

#### Scenario: Client leaves one namespace
- **WHEN** the client calls `socket.disconnect()` on `/admin` while staying on `/`
- **THEN** the server reports `client namespace disconnect` for `/admin` only, on both servers

#### Scenario: Server closes the connection
- **WHEN** the server calls `disconnect(true)` on a socket
- **THEN** the client's `disconnect` handler receives `io server disconnect` on both servers

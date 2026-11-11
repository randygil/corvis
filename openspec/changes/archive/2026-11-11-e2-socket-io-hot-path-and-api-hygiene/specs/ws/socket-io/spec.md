# Spec Delta

## ADDED Requirements

### Requirement: A connecting socket is registered before its CONNECT
When the connect middleware accepts a socket, the server SHALL add it to the namespace and to the room of its own id
before it queues CONNECT, and SHALL queue CONNECT and mark the socket connected in one step. Until then broadcasts and
bulk operations MUST skip the socket and its emits MUST fail with `NotConnected`. A lookup by id MUST find the socket of
every client that has received its CONNECT.

#### Scenario: Lookup right after CONNECT
- **WHEN** 1,000 clients connect to a server on a runtime with four workers and each, as soon as it reads its CONNECT,
  has the server look its socket id up
- **THEN** every lookup finds the socket, with no retry and no wait

#### Scenario: Nothing precedes CONNECT
- **WHEN** a task broadcasts to the namespace in a loop while 200 clients connect to it
- **THEN** the first packet every client receives for the namespace is its CONNECT

#### Scenario: The connection ends while the socket joins
- **WHEN** the connection closes after the socket was added to the namespace and before its CONNECT was queued
- **THEN** the namespace and the adapter hold nothing of the socket, no handler of it runs and no CONNECT is sent

### Requirement: No connection lock is held across the adapter
The connect path SHALL NOT hold a lock of the connection while it calls the adapter, application code or the transport,
so an adapter that disconnects the socket or closes its connection from `add_all` MUST NOT deadlock the connection.

#### Scenario: Adapter closes the connection while the socket joins
- **WHEN** a test adapter's `add_all` for the room of the socket's own id calls `disconnect(true)` on that socket
- **THEN** the connect attempt ends, the socket is removed and the connection closes, within the test's deadline

### Requirement: Connection handlers run when the socket is announced
The connection handlers of a socket SHALL run right after its CONNECT was queued and the socket marked connected, before
any packet of the client is handled, whatever the transport and whether or not the client has collected CONNECT: from
that moment the socket receives broadcasts, so the hook where an application checks a socket MUST NOT wait for the
client. Packets the client sends meanwhile MUST wait for the handlers.

#### Scenario: Polling client that does not poll
- **WHEN** a client posts CONNECT to a namespace that is broadcast to in a loop, has no poll pending and does not poll,
  and the connection handler disconnects the socket
- **THEN** the handler runs before any poll, and what the client then collects holds its CONNECT, the disconnect and no
  broadcast

#### Scenario: Gateway hook of a polling client that does not poll
- **WHEN** a polling client posts CONNECT to a gateway's namespace with no poll pending and does not poll, and the
  gateway's `handle_connection` disconnects the socket
- **THEN** the hook runs before any poll, and a broadcast to the namespace after it does not reach the client

### Requirement: A socket that never connected reports no disconnect
A socket that never connected (refused by a middleware, or whose connect failed before CONNECT) SHALL run no
`disconnecting` or `disconnect` callback, and what was registered on it MUST be released when the attempt ends.

#### Scenario: Callbacks of a socket that never connected
- **WHEN** a connect middleware registers `disconnecting` and `disconnect` callbacks that hold a clone of the socket, and
  the attempt is refused by the middleware or by a failure to join the socket's own room
- **THEN** neither callback runs and both are dropped

### Requirement: CONNECT leaves alone
A poll response SHALL end after a Socket.IO CONNECT packet, so what the connection handlers emit leaves in a later
response, as with the reference server when a poll was waiting. A WebSocket SHALL write CONNECT as a frame of its own.

#### Scenario: Polling
- **WHEN** a client posts CONNECT with no poll pending and the connection handler emits `auth`
- **THEN** the next poll answers only the CONNECT packet and the poll after it answers the `auth` event

#### Scenario: WebSocket
- **WHEN** the connection handler of a WebSocket client emits an event
- **THEN** the event is written as a frame of its own after the CONNECT frame

### Requirement: An emit is encoded once
The server SHALL encode an event for one recipient into one buffer that holds the Engine.IO message, move it into the
send queue without copying it, and build the WebSocket frame from that buffer without copying it. A broadcast SHALL
queue one shared encoding on every recipient. The polling batch bound MUST count the length of the encoded packet, and
`max_buffered_amount` the allocation that holds it (its length, plus at most an eighth of spare capacity for a buffer of
one recipient).

#### Scenario: One recipient
- **WHEN** a socket emits a 16 KiB event to a WebSocket client
- **THEN** the frame handed to the WebSocket library points into the buffer the encoder produced

#### Scenario: One encoding per broadcast
- **WHEN** an event is broadcast to several recipients
- **THEN** every recipient's queue holds the same shared encoding

#### Scenario: Spare capacity counts against the send buffer
- **WHEN** an encoded packet of 900 bytes held in a buffer of 1,000 is queued
- **THEN** it counts its 1,000 bytes of allocation against `max_buffered_amount`, not its 900 bytes of length

### Requirement: Public enums can grow
`EmitError`, `AckError`, `DisconnectReason`, `RoomError`, `AdapterError`, `BroadcastAckErrorKind` and the Engine.IO
`CloseReason` SHALL be `#[non_exhaustive]`, so adding a variant to one of them is not a breaking change.

#### Scenario: Exhaustive match outside the crate
- **WHEN** code outside the crate matches one of these enums without a wildcard arm
- **THEN** it does not compile (one `compile_fail` doctest per enum)

### Requirement: Test hooks are not part of the API
Functions that exist for tests (the in-memory adapter's map sizes and invariants, decode probes, a socket's buffered
amount, a namespace's adapter) SHALL live in one hidden module, `corvis::socket_io::testing`, documented as unstable,
and MUST NOT be methods of the `Adapter` trait or of other public types.

#### Scenario: The adapter trait
- **WHEN** the `Adapter` trait is read
- **THEN** it has no method that exists only for tests, and the test suites reach those hooks through
  `corvis::socket_io::testing`

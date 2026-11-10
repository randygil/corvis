# Spec Delta

## Purpose

The Socket.IO v5 layer on top of `ws/engine-io` (feature `socket-io`): packet encoding, namespaces, connection with
`auth`, connect middleware, events, acknowledgements in both directions, binary attachments, disconnect reasons and
the limits of this layer. What the official `socket.io-client` 4.8.4 observes MUST match Node `socket.io` 4.8.4
(`socket.io-parser` 4.2.7), verified by the differential harness of `ws/socket-io-parity`, except the bounds listed in
"Security divergences are explicit". Every limit has a bounded default. Implemented by
[#33](https://github.com/randygil/corvis/issues/33).

Timing rule for every scenario below: "within" bounds are generous, "not before" bounds use the configured value
itself, timer logic is tested on a paused clock, and no scenario depends on how fast the machine is.

## ADDED Requirements

### Requirement: Packet encoding matches socket.io-parser
Packets SHALL be encoded and decoded as `socket.io-parser` 4.2.7 does: `<type>[<attachments>-][<nsp>,][<ackId>][JSON]`
with types 0 CONNECT to 6 BINARY_ACK and the main namespace `/` omitted. For strings, integers, booleans, `null`,
arrays and objects (keys in insertion order) the encoded bytes MUST equal Node's; floating point numbers MUST decode
to the same value. An encoded packet MUST NOT contain the character `U+001E`.

#### Scenario: Encoding round trip
- **WHEN** the same events (strings, integers, nested objects, arrays, `null`, several arguments, a custom namespace,
  an acknowledgement id) are emitted by both servers to a raw client
- **THEN** the Engine.IO message payloads observed on the wire are byte-equal

#### Scenario: Record separator inside a string
- **WHEN** the server emits a string argument containing `U+001E` to a polling client
- **THEN** the polling payload holds one packet with the character escaped, and the client receives the same string

### Requirement: Undecodable packets close the connection like Node
A Socket.IO packet that cannot be decoded (unknown type, invalid attachment count, invalid payload for its type, a
reserved event name, an invalid acknowledgement id, text while a binary packet is being reassembled, binary data when
none is expected, a placeholder that is not an index of the received attachments) MUST close the Engine.IO connection,
and the sockets of that connection MUST report the disconnect reason Node reports, `forced close`. Decoding MUST NOT
panic on any input.

#### Scenario: Malformed packet
- **WHEN** a raw client that joined `/` sends `9`, then on new connections `2{"a":1}`, `2["disconnect"]` and
  `42abc["x"]`
- **THEN** both servers close each connection, and the server-side log holds `forced close` for each socket

#### Scenario: Arbitrary input
- **WHEN** the property tests feed arbitrary text and arbitrary interleavings of text and binary messages to the
  decoder
- **THEN** it returns a packet or an error, never panics, and holds no more memory than a constant times its input

### Requirement: Namespaces and connection match Node
A client SHALL join a namespace by sending CONNECT with an optional `auth` object, and the server MUST answer with
CONNECT carrying `{"sid":<id>}`. Several namespaces SHALL be multiplexed over one Engine.IO session. The main
namespace `/` MUST exist without being registered. CONNECT to a namespace the application did not register MUST be
answered with CONNECT_ERROR `{"message":"Invalid namespace"}` and MUST NOT create it.

#### Scenario: Auth is visible to the server
- **WHEN** the official client connects with `auth: { token: "abc" }`
- **THEN** the server-side handshake exposes the same `auth` object on both servers, and the client receives a `sid`

#### Scenario: Unknown namespace
- **WHEN** the official client connects to `/missing`
- **THEN** its `connect_error` handler receives an error with message `Invalid namespace` on both servers, and the
  server still has no namespace `/missing`

#### Scenario: Two namespaces on one session
- **WHEN** the client opens `/` and `/admin` from the same manager
- **THEN** both namespaces share one Engine.IO session and each receives only its own events

### Requirement: Socket ids are not session ids
The id of a socket MUST be generated independently of the Engine.IO session id, with at least 120 random bits from the
operating system's generator and the 20-character shape of Node's ids, and the Engine.IO session id MUST NOT be sent
in any Socket.IO packet nor written to logs.

#### Scenario: Ids differ
- **WHEN** a client connects to `/` and `/admin`
- **THEN** the two socket ids and the Engine.IO session id are three different 20-character strings

### Requirement: A connection that joins no namespace is closed
An Engine.IO connection that has joined no namespace `connect_timeout` (default 45 s, as Node's `connectTimeout`)
after it opened MUST be closed. A connection attempt still waiting in middleware does not count as joined.

#### Scenario: No namespace joined in time
- **WHEN** a raw client completes the Engine.IO handshake but sends no CONNECT, with the timeout set to 1000 ms on
  both servers
- **THEN** both servers close the Engine.IO session, not before 1000 ms

### Requirement: Connect middleware matches Node
Namespace middleware SHALL run in registration order before a socket connects. An error from middleware MUST reject
the connection with CONNECT_ERROR `{"message":<message>}`, plus `"data"` when the error carries data, the socket MUST
NOT be added to the namespace, and connection handlers MUST NOT run for it. A panic in middleware MUST be contained
and reject the connection with the message `Internal server error`.

#### Scenario: Middleware rejects with data
- **WHEN** a middleware fails with message `not authorized` and data `{"content":"retry later"}`
- **THEN** the client's `connect_error` receives `err.message == "not authorized"` and `err.data` equal to the data, on
  both servers

#### Scenario: Middleware rejects without data
- **WHEN** a middleware fails with only a message
- **THEN** the CONNECT_ERROR payload on the wire is `{"message":"not authorized"}`, without a `data` key, on both
  servers

#### Scenario: Middleware panics
- **WHEN** a middleware panics
- **THEN** the client receives CONNECT_ERROR with message `Internal server error`, the panic is logged, and other
  connections are not affected

### Requirement: One socket per namespace per connection
A connection SHALL hold at most one socket per namespace, counting connection attempts whose middleware has not
finished. A CONNECT for a namespace that is joined or pending, a packet other than CONNECT for a namespace that is not
joined, and a CONNECT_ERROR from the client MUST close the Engine.IO connection; sockets still joined report
`forced server close`.

#### Scenario: Duplicate CONNECT
- **WHEN** a raw client that joined `/` sends `40` again
- **THEN** both servers close the connection and report `forced server close` for the socket

#### Scenario: CONNECT flood during slow middleware
- **WHEN** a raw client sends 1,000 CONNECT packets for `/admin` while its middleware takes one second
- **THEN** Corvis runs the middleware once and closes the connection at the second CONNECT, and the harness lists this
  as an expected difference from Node, which starts one pending socket per packet

#### Scenario: Event before CONNECT
- **WHEN** a raw client sends `2["echo",1]` without having joined `/`
- **THEN** both servers close the connection

### Requirement: Events match Node
Events SHALL carry any number of arguments in both directions, delivered to the handler registered for the event name
on that socket. An event without a handler MUST be ignored. Emitting a reserved event name (`connect`,
`connect_error`, `disconnect`, `disconnecting`, `newListener`, `removeListener`) from server code MUST fail and send
nothing.

#### Scenario: Several arguments
- **WHEN** the server emits `update` with `1`, `"two"` and `{"three":3}`
- **THEN** the client handler receives the same three arguments on both servers

#### Scenario: Reserved name
- **WHEN** server code emits `disconnect` to a socket
- **THEN** the emit fails with an error naming the reserved event and nothing is sent, as in Node

### Requirement: Acknowledgements from the server match Node
A client emit with a callback SHALL be answered with an ACK carrying the client's id and the array of arguments the
handler gave to its acknowledgement sender. An event MUST be acknowledged at most once, and an event whose handler
does not answer MUST NOT be acknowledged.

#### Scenario: Client ack with several values
- **WHEN** the client emits `sum` with `[1, 2]` and a callback, and the server acks with `3` and `"ok"`
- **THEN** the callback receives `(3, "ok")` on both servers

### Requirement: Acknowledgements from the client match Node
A server emit with an acknowledgement SHALL resolve with the client's answer. Its id MUST come from a counter per
namespace starting at 0, as in Node. An ACK MUST only be accepted from the socket the event was sent to, and an ACK
with an unknown id MUST be ignored. With a timeout, an unanswered emit MUST fail with the message
`operation has timed out`.

#### Scenario: Client answers
- **WHEN** the server emits `ask` with an acknowledgement and the client answers with `"yes"` and `7`
- **THEN** the server side resolves with `["yes", 7]` on both servers

#### Scenario: Server ack timeout
- **WHEN** the server emits with a 200 ms acknowledgement timeout to a client that never answers
- **THEN** the server side fails with `operation has timed out` on both servers, not before 200 ms

#### Scenario: ACK from another socket
- **WHEN** socket A is sent an event with acknowledgement id 0 and socket B of the same namespace sends ACK id 0
- **THEN** A's pending call is not resolved

### Requirement: Pending acknowledgements are bounded
Every server emit with an acknowledgement MUST have a deadline: the one given for the call, or `ack_timeout` (default
30 s). A socket SHALL hold at most `max_pending_acks` (default 1,000) unanswered acknowledgements; a call over the
limit MUST fail without sending. When the socket disconnects, its pending calls MUST fail at once.

#### Scenario: Default deadline
- **WHEN** server code emits with an acknowledgement and no timeout, and the client never answers (paused clock)
- **THEN** the call fails with `operation has timed out` at 30 s and the socket holds no pending acknowledgement

#### Scenario: Pending acknowledgement cap
- **WHEN** `max_pending_acks` is 2 and server code starts a third acknowledged emit while two are unanswered
- **THEN** the third fails at once, nothing is sent for it, and the first two still resolve when answered

#### Scenario: Socket disconnects with a pending acknowledgement
- **WHEN** a socket disconnects while server code waits for its acknowledgement
- **THEN** the call fails at once and the socket holds no pending acknowledgement

### Requirement: Client acknowledgement ids are bounded
A client acknowledgement id SHALL be a run of at most 16 digits whose value is at most 2^53 - 1; it MUST be echoed
exactly in the ACK. A longer or larger id MUST be treated as an undecodable packet.

#### Scenario: Oversized ack id
- **WHEN** a raw client sends `2` followed by the 20 digits `12345678901234567890` and `["echo",1]`
- **THEN** Corvis closes the connection, and the harness lists this as an expected difference from Node, which
  answers with a rounded id

### Requirement: Binary attachments match Node
Arguments containing binary values at any depth SHALL be sent as BINARY_EVENT / BINARY_ACK with the binary values
replaced by `{"_placeholder":true,"num":<n>}` and the attachments sent as binary Engine.IO messages right after the
packet, and received with the binary values restored in place, over polling and WebSocket. The messages of one packet
MUST be queued together: no other packet of the same connection may be sent between them.

#### Scenario: Binary echo over polling
- **WHEN** a client restricted to polling emits an object containing a 1 KiB buffer and the server echoes it
- **THEN** the client receives an equal buffer at the same position, on both servers

#### Scenario: Binary acknowledgement over WebSocket
- **WHEN** a WebSocket client emits two buffers with a callback and the server acks with both
- **THEN** the callback receives the two buffers in order, on both servers

#### Scenario: Concurrent binary emits
- **WHEN** 32 handlers of one connection each emit a packet with two attachments at the same time
- **THEN** the client decodes 32 packets, each with its own two attachments

### Requirement: Attachments are bounded
A binary packet SHALL carry at most `max_attachments` attachments (default 10, the limit of `socket.io-parser`
4.2.7). A received packet that declares more MUST close the connection before any attachment is accepted; an emit with
more MUST fail without sending. The server MUST NOT allocate memory from the declared count, and a placeholder
repeated many times MUST NOT copy the attachment. Each attachment is bounded by the Engine.IO `max_http_buffer_size`.

#### Scenario: Too many attachments
- **WHEN** a raw client sends `511-["x",...]`
- **THEN** both servers close the connection

#### Scenario: Declared count without data
- **WHEN** a raw client sends `510-["x"]` and no attachment, on 100 connections
- **THEN** each connection holds only the text packet until `connect_timeout` or the heartbeat ends it

#### Scenario: Repeated placeholder
- **WHEN** a raw client sends one 500 kB attachment referenced by 10,000 placeholders
- **THEN** the handler receives 10,000 values sharing one buffer, and the server's memory grows by about the size of
  one attachment

### Requirement: Reassembly and waiting packets stay within the received-data bound
Attachments received for a packet that is not yet complete, and packets waiting for a handler, SHALL count against the
Engine.IO session's `max_unread_bytes` until the packet is handed to a handler or discarded. Options under which one
maximal binary packet could not fit in `max_unread_bytes` MUST be rejected when the server is created.

#### Scenario: Incomplete packets on many connections
- **WHEN** a client sends a binary packet header and 9 of its 10 attachments of 1 MB and then stops
- **THEN** the session holds at most `max_unread_bytes` plus one message, and frees it when the session ends

#### Scenario: Inconsistent options
- **WHEN** a server is created with `max_attachments` 100 and the default `max_http_buffer_size` and
  `max_unread_bytes`
- **THEN** creation fails with an error naming the options

### Requirement: Nesting depth is bounded
A packet whose JSON is nested deeper than 128 levels MUST be treated as an undecodable packet, and no recursion of
this layer (decoding, placeholder substitution, value conversion, encoding) may exceed that depth.

#### Scenario: Nesting above the limit
- **WHEN** a raw client sends an event whose argument is 200 nested arrays
- **THEN** Corvis closes the connection, and the harness lists this as an expected difference from Node, which
  delivers the event

#### Scenario: Extreme nesting
- **WHEN** a raw client sends an event whose argument is 100,000 nested arrays
- **THEN** Corvis closes the connection without overflowing its stack

### Requirement: Handlers in flight are bounded per connection
Handlers of one Engine.IO connection SHALL start in the order their packets arrived and run concurrently, at most
`max_inflight` (default 32) at a time across all its namespaces. At the limit the server MUST stop reading from that
connection until a handler finishes. A panic in a handler MUST be contained and logged, and the socket stays
connected.

#### Scenario: Blocked handlers
- **WHEN** a client sends 10,000 events to a handler that never finishes
- **THEN** 32 handlers are running, the rest of the data is not read beyond `max_unread_bytes`, and other
  connections are served

#### Scenario: Order
- **WHEN** a client emits `set` and then `get` without waiting
- **THEN** the `get` handler starts after the `set` handler started

#### Scenario: Handler panics
- **WHEN** a handler panics
- **THEN** the panic is logged, the socket stays connected and the next event is handled

### Requirement: Namespace names are validated
Registering a namespace SHALL fail for a name longer than 256 bytes or containing `,` or a control character. A name
without a leading `/` gets one, as in Node.

#### Scenario: Name with a comma
- **WHEN** the application registers the namespace `/a,b`
- **THEN** registration fails with an error naming the namespace

### Requirement: Disconnect reasons match Node
Disconnections SHALL report the same reason as Node on the server side: `client namespace disconnect`,
`server namespace disconnect`, `transport close`, `transport error`, `ping timeout`, `parse error`, `forced close`,
`forced server close` and `server shutting down`.

#### Scenario: Client leaves one namespace
- **WHEN** the client calls `socket.disconnect()` on `/admin` while staying on `/`
- **THEN** the server reports `client namespace disconnect` for `/admin` only, on both servers

#### Scenario: Transport closed by the client
- **WHEN** a client's Engine.IO connection is closed without a DISCONNECT packet
- **THEN** the server reports `transport close` on both servers

### Requirement: Server-side disconnect matches Node
`disconnect(true)` MUST leave every namespace of the connection and close the Engine.IO session; `disconnect(false)`
MUST only leave its namespace. For each socket the `disconnecting` handlers MUST run before it leaves its namespace
and the `disconnect` handlers after, each exactly once.

#### Scenario: Handler order
- **WHEN** a socket disconnects and both kinds of handlers look the socket up in its namespace
- **THEN** the `disconnecting` handler finds it, the `disconnect` handler does not, and each ran once

#### Scenario: Server leaves one namespace
- **WHEN** the server calls `disconnect(false)` on the `/admin` socket of a client that is also on `/`
- **THEN** the client's `/admin` socket receives `io server disconnect`, its `/` socket stays connected, and the server
  reports `server namespace disconnect`, on both servers

#### Scenario: Server closes the connection
- **WHEN** the server calls `disconnect(true)` on a socket
- **THEN** the client's `disconnect` handler receives `io server disconnect` and the Engine.IO session is closed, on
  both servers

### Requirement: Server close reports server shutting down
Closing the Socket.IO server, and the application's shutdown, SHALL report `server shutting down` to every socket
before the Engine.IO sessions are closed, and fail their pending acknowledgements.

#### Scenario: Shutdown with connected sockets
- **WHEN** the application shuts down with two sockets connected
- **THEN** both `disconnect` handlers receive `server shutting down` and the clients observe `transport close`, as
  with Node's `io.close()`

### Requirement: Options are validated
Creating a Socket.IO server SHALL fail, with an error naming the option, on a zero `connect_timeout`, `ack_timeout`,
`max_attachments`, `max_inflight` or `max_pending_acks`, and on any option the Engine.IO layer rejects.

#### Scenario: Zero connect timeout
- **WHEN** a server is created with `connect_timeout(Duration::ZERO)`
- **THEN** creation fails with an error naming `connect_timeout`

### Requirement: Security divergences are explicit
Every bound of this layer where Node has none or a different one SHALL be documented in `docs/websockets/adapter.md`
(English and Spanish) and the rustdoc of `IoOptions`, and each one a client can observe MUST have an entry in the
harness's `expected-differences.json` with the scenario that exercises it.

#### Scenario: Documented bounds
- **WHEN** the documentation page and the rustdoc of `IoOptions` are read
- **THEN** they list, with default and Node's value: one pending socket per namespace, acknowledgement id range,
  nesting depth, attachments when encoding, reassembly accounting, handlers in flight, pending acknowledgements and
  their default deadline, namespace names and socket id entropy

#### Scenario: Divergences are listed
- **WHEN** the harness runs the Socket.IO scenarios against both servers
- **THEN** every difference reported is listed with its reason, and no listed difference is stale

### Requirement: Verified against Node and the upstream suite
The Socket.IO scenarios SHALL run against the Node reference server and against Corvis with no unlisted difference,
and the upstream `socket.io-protocol` suite, at the commit pinned by `parity/socket-io/suites.json`, SHALL pass against
a Corvis server configured as the suite requires.

#### Scenario: Protocol suite
- **WHEN** `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_conformance` runs
- **THEN** every test of the suite passes

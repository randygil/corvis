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
finished. A CONNECT for a namespace that is joined or pending, a packet other than CONNECT for a namespace the
connection never joined or was refused, and a CONNECT_ERROR from the client MUST close the Engine.IO connection (sockets
still joined report `forced server close`).

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

### Requirement: Packets for a namespace that was left are dropped
A packet from the client for a namespace its connection joined and whose socket has since left (the server called
`disconnect(false)`, or the client left) MUST be dropped without closing the connection (the Node server closes it). It is read,
measured and charged like any packet, its attachments are consumed, and a new CONNECT joins the namespace again.

#### Scenario: Packets racing a server-side leave
- **WHEN** the server calls `disconnect(false)` on a socket of `/b` and the raw client's ACK, event and binary event for
  `/b` were already on their way
- **THEN** all are dropped, nothing is delivered, the connection and `/` go on, and a packet for a namespace never joined
  still closes the connection

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
maximal binary packet could not fit in `max_unread_bytes` MUST be rejected when the server is created, and a binary
header whose cost and attachments could not fit together MUST close the connection when it arrives.

#### Scenario: Incomplete packets on many connections
- **WHEN** a client sends a binary packet header and 9 of its 10 attachments of 1 MB and then stops
- **THEN** the session holds at most `max_unread_bytes` plus one message, and frees it when the session ends

#### Scenario: Inconsistent options
- **WHEN** a server is created with `max_attachments` 100 and the default `max_http_buffer_size` and
  `max_unread_bytes`
- **THEN** creation fails with an error naming the options

### Requirement: Decoded packets are reserved before they are built
The decoded cost of a packet, an upper bound of the heap its values occupy (derived from the types: a slot per value,
the box and table of an object, the bytes of strings, allocator overhead), SHALL be worked out from its text before
anything is built, and a packet above `max_decoded_bytes` (default 32 MiB) MUST close the connection. The cost MUST be
reserved in the server-wide unread budget before decoding, shrunk to the real cost afterwards, and charged until the
handler ends.

#### Scenario: Cost is an upper bound of the heap
- **WHEN** packets of adversarial shapes (empty objects, empty arrays, empty strings, numbers, nested one-key objects,
  long keys, many keys, repeated keys, escapes) are decoded under a counting allocator
- **THEN** for each, the live heap of the decoded packet is at most its charged cost, the charged cost at most the
  reservation, and the peak during decoding at most the reservation plus the text

#### Scenario: Packet above the budget
- **WHEN** a raw client sends 0.99 MB of text that decodes to 45 MB (330,000 empty objects) under the default limit
- **THEN** the connection is closed after a pass over the text that allocates less than one byte per two of it, and
  nothing of the packet is built

### Requirement: A connection waits while the budget has no room
A connection SHALL NOT take another message from the engine while its session is over its unread limit by the engine's
own test (its limit, or its fair share while its address is over its share or the server-wide budget is exceeded), nor
while a measured packet waits for room. A packet is admitted when the session stays within its fair share, or when the
address's share and the server-wide budget have room, counting the packet net of the message it replaces.

#### Scenario: Many sessions of one address
- **WHEN** twelve sessions of one address each send a packet that decodes to 7 MB, to a handler that keeps its
  arguments, with `max_total_unread_bytes` 16 MiB
- **THEN** one packet is admitted, the others wait, the live heap stays within `max_total_unread_bytes` plus, per open
  session, its fair share and one message, and all twelve are handled once the handlers end

#### Scenario: Honest address
- **WHEN** one address holds its share with handlers that do not end, and a session of another address sends a packet
  that needs room in the server-wide budget
- **THEN** that packet is decoded and handled

### Requirement: One address's share covers everything it holds
Everything the sessions of one client address hold of the unread budget SHALL count against the address's share
(`max_address_share`: default 1/16 of `max_total_unread_bytes`, at least one maximum packet; an explicit value below
that floor or above the budget MUST be refused): unread and parked messages, reservations, decoded packets, binary
attachments before and after delivery, handshake `auth`, and charges of handlers that outlive their session.

#### Scenario: Binary attachments from one address
- **WHEN** eight sessions of one address send binary events with 0.5 MB attachments to handlers that keep them, with
  `max_total_unread_bytes` 64 MiB
- **THEN** the live heap of the server stays within the address's share plus, per session, its fair share and the
  messages in hand, far below the budget, and a session of another address is answered

#### Scenario: Large text from one address
- **WHEN** eight sessions of one address send 0.9 MB text events to handlers that keep them
- **THEN** the same bound holds, including the messages in transit and parked, and an honest session is answered

#### Scenario: Floor of the share
- **WHEN** a server is created with a `max_address_share` below `max_decoded_bytes` plus the wire bytes of its largest
  packet
- **THEN** creation fails with an error naming `max_address_share`

### Requirement: The transit memory of the WebSocket transport is stated and bounded
The WebSocket library's read buffer (kept at the size of the largest frame received) and the buffer of an incomplete
fragmented message are outside the unread budget. Their ceiling per session MUST be measured, bounded by a test, and
documented per session, per address and server-wide with the defaults, apart from the budget, with the options that lower
it (`max_http_buffer_size`, `max_sessions_per_ip`, `max_sessions`).

#### Scenario: Idle retention and incomplete message
- **WHEN** sessions each receive one message of nearly `max_http_buffer_size`, then an incomplete fragmented message
- **THEN** the live heap per session stays within the documented ceilings, and falls in proportion at smaller sizes

### Requirement: Messages are charged for their allocation
A message the engine hands up, text or binary, from either transport, SHALL be charged for the allocation behind it, not
only its length: a buffer holding more than an eighth beyond the length MUST be shrunk, and the remaining capacity
charged. A small slice MUST NOT keep a large buffer alive.

#### Scenario: Polling body released while the answer waits
- **WHEN** a polling `POST` carries more records than the session's unread limit and its answer is held for room
- **THEN** the request body is not alive: the heap holds the records and not twice them

#### Scenario: Fragmented binary message
- **WHEN** a client sends a 524,289 byte binary message as frames of 524,288 and 1 byte, and as 513 frames of 1 KiB,
  to handlers that keep them
- **THEN** the live heap per kept attachment is at most an eighth above its length plus its accounting, not twice it

### Requirement: A wait caused by others does not close the session
While reads are paused for the server-wide budget and the session is over neither its own limit nor its address's share, its pong deadline SHALL be held for at most `admission_timeout` in total and resume with what was left of it, so a client that never answers MUST be closed within `ping_interval + ping_timeout + admission_timeout` of its last pong. A packet not admitted within `admission_timeout` MUST close the connection (reason `admission timeout`). A waiter MUST be woken when room appears.

#### Scenario: Handlers return after a delay
- **WHEN** attackers on several addresses hold the budget with handlers that return after a delay, and an honest
  session sends a packet that needs room
- **THEN** the packet is handled promptly after the first handler returns, and the session is not closed

#### Scenario: Oscillating pressure
- **WHEN** the pressure on a session starts and stops repeatedly without a pong in between
- **THEN** the allowance of `admission_timeout` is spent in total, not per pause, and the heartbeat applies once it is
  used, until a pong renews it

#### Scenario: Handlers never return
- **WHEN** the same handlers never return
- **THEN** the honest session is closed with `admission timeout` at `admission_timeout`, and not with `ping timeout`
  before it

### Requirement: The handshake auth is charged
The `auth` kept in a socket's handshake SHALL be charged, at the size of the stored `serde_json::Value`, to the
session and the address for as long as the handshake lives.

#### Scenario: Many sockets and namespaces
- **WHEN** sessions join many namespaces with the largest auth the limit allows
- **THEN** the address holds the sum of the stored values' sizes, which is at most the model's charge under a counting
  allocator, and releases it as the sockets go

### Requirement: Charges end with their handlers
The charge of a decoded packet MUST stay on the server-wide gauge until its handler ends, including after its session
was closed by the server or by a shutdown (for at most `close_timeout`, after which the handler is dropped), and MUST
count against the client address, so reconnecting does not multiply it and other addresses keep their share.

#### Scenario: Handler running after a server close
- **WHEN** the server closes a session while its handler keeps a packet, and the address reconnects and sends the same
  packet
- **THEN** the gauge still counts the first packet, the second waits, and it is admitted when the first handler ends

### Requirement: Handlers do not outlive a client that left
When the client drops the connection or fails to make progress (transport close or error, parse error, ping timeout, protocol violation, admission timeout) the handlers still running
MUST be cancelled at once, releasing their charges, unless `finish_handlers_after_client_close` is set. Handlers that
outlive their connection MUST count per client address, and an address at `max_sessions_per_ip x max_inflight` of them
MUST be refused new sessions with `503` until some end.

#### Scenario: Client leaves while its packet waits
- **WHEN** a client with handlers running parks a packet for room and closes the TCP connection
- **THEN** its handlers are cancelled at `admission_timeout` at the latest, not after `close_timeout`

#### Scenario: Reconnect churn
- **WHEN** eight connections of one address each start 32 blocked handlers and drop, 25 times over
- **THEN** no handler is left running and the live heap returns to its baseline within a few MiB

#### Scenario: Opted-in grace
- **WHEN** the same churn runs with the grace requested and a short `close_timeout`
- **THEN** the handlers that linger stay at or below the cap, sessions over it are refused, and the address is served
  again when they end

### Requirement: Lone surrogates are refused
A string of a packet holding a lone UTF-16 surrogate escape (`"\ud800"`) SHALL make the packet undecodable and close the
connection, because a Rust string cannot hold it and Node keeps it. The difference is stricter and fail-closed, and
MUST be listed in `expected-differences.json` with its scenario.

#### Scenario: Lone surrogate in an argument
- **WHEN** a raw client sends an event whose argument is `"\ud800"`, and another whose argument is a valid pair
- **THEN** Corvis closes the connection for the first and delivers the second; Node delivers both, and the harness
  lists the first as an expected difference

### Requirement: Log lines a client can provoke are rate limited
A line written because of what a client sent (a handler or connection handler that panicked, a connect middleware that
timed out) SHALL be written at most once per minute per kind and per namespace the application registered, the next
one carrying the count of those left out, and MUST NOT include a panic message or data of a packet.

#### Scenario: Handler that panics on bad input
- **WHEN** a client sends 50 events that make a handler panic with a message and the client's data
- **THEN** one line is logged, naming the namespace, and neither the message nor the data appears in the log

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
`forced server close` and `server shutting down`, and `admission timeout` where Node has no equivalent.

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
`max_attachments`, `max_inflight`, `max_pending_acks` or `admission_timeout`, on a `max_decoded_bytes` above the
engine's `max_total_unread_bytes`, on a `max_address_share` outside its floor and the budget, and on any option the
Engine.IO layer rejects.

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
  nesting depth, attachments when encoding, reassembly accounting, the decoded cost of a packet with its worst-case
  expansion, the address share and how many addresses exhaust the budget, the admission timeout, the
  handlers' panic messages reaching the default panic hook, handlers in flight, pending acknowledgements and their default deadline, namespace
  names and socket id entropy

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

# Spec Delta

## Purpose

The Redis adapter for Socket.IO servers (feature `socket-io-redis`): several application instances share broadcasts,
room operations and requests through Redis pub/sub. It is wire compatible with Node `@socket.io/redis-adapter` 8.3.0
(classic, non-sharded), so Corvis instances and Node instances on the same Redis serve one set of clients during a
migration: the same channels, the same messages and the same request protocol, verified by fixtures recorded from
that package and by mixed-cluster scenarios of the differential harness of `ws/socket-io-parity`. Everything read
from Redis is untrusted input. Every limit has a bounded default; the differences from Node are listed in "Security
divergences are explicit". Implemented by [#36](https://github.com/randygil/corvis/issues/36).

Words used below: an instance is one Socket.IO server process connected to the Redis; "the Node adapter" is
`@socket.io/redis-adapter` 8.3.0; a mixed cluster is one Node instance and one Corvis instance on the same Redis with
the same key; `<key>` is the adapter's key prefix (default `socket.io`) and `<nsp>` a namespace name with its leading
`/`. Timing rule for every scenario: absence of delivery is observed through a marker event sent after the
operation, never by waiting; timer logic is tested on a paused clock; tests that need Redis use a key of their own.

## ADDED Requirements

### Requirement: Same channels as the Node adapter
For each namespace the adapter SHALL use the channels the Node adapter uses: broadcasts on `<key>#<nsp>#`, or on
`<key>#<nsp>#<room>#` when the broadcast targets exactly one room, received through one pattern subscription;
requests on `<key>-request#<nsp>#`; answers on `<key>-response#<nsp>#` and `<key>-response#<nsp>#<uid>#`. The key
prefix MUST be configurable and default to `socket.io`.

#### Scenario: Channels observed in Redis
- **WHEN** a Node instance and a Corvis instance start on the same Redis with namespace `/chat`
- **THEN** `PUBSUB CHANNELS` and `PUBSUB NUMPAT` show the same channels and one pattern for each, uids aside

#### Scenario: Single-room broadcast
- **WHEN** a Corvis instance emits to room `r1` only, and then to rooms `r1` and `r2` together
- **THEN** the first message is published on `<key>#/#r1#` and the second on `<key>#/#`

#### Scenario: Custom key
- **WHEN** two Corvis instances use key `app-a` and a third uses key `app-b` on the same Redis
- **THEN** a broadcast from the first reaches the clients of the second and not those of the third

#### Scenario: Answers on the common channel by default
- **WHEN** a Corvis instance answers a request with the default options, and then with
  `publish_on_specific_response_channel(true)`
- **THEN** the first answer is published on `<key>-response#<nsp>#` and the second on the requester's
  `<key>-response#<nsp>#<uid>#`, and a Corvis requester receives answers on both

#### Scenario: Namespace with glob characters
- **WHEN** an instance serves namespace `/a*` and another instance broadcasts on namespace `/ab`
- **THEN** no socket of `/a*` receives the broadcast

### Requirement: An instance is counted once
Each instance MUST hold exactly one subscription to the request channel of each of its namespaces, so the server
count, read with `PUBSUB NUMSUB` on that channel as the Node adapter does, counts a Corvis instance once. The uid of
an adapter and its request ids MUST have the Node adapter's form: six characters of `A-Z`, `a-z`, `0-9`, `-`, `_`
from a cryptographic random source.

#### Scenario: Server count in a mixed cluster
- **WHEN** one Node instance and two Corvis instances serve namespace `/` on the same Redis and key
- **THEN** every instance reports a server count of 3

#### Scenario: Several namespaces on one connection
- **WHEN** a Corvis server with namespaces `/`, `/a` and `/b` uses one adapter
- **THEN** Redis shows one subscriber connection for it, subscribed once to the request channel of each namespace

#### Scenario: One handle, two servers
- **WHEN** a second `IoServer` is created with a `RedisAdapter` handle that a running server already uses
- **THEN** creating it fails with an error that says the adapter already serves a server

### Requirement: Broadcast messages match the Node adapter
A broadcast SHALL be published as the MessagePack array `[uid, packet, opts]` the Node adapter publishes: `packet` is
`{type: 2, data: [event, ...args], nsp}` with binary arguments inline, and `opts` is `{rooms, except, flags}`. A
broadcast restricted to the local instance MUST NOT be published. An instance MUST ignore its own messages.

#### Scenario: Recorded Node broadcast
- **WHEN** the codec decodes each broadcast recorded from the Node adapter (plain, single room, several rooms with
  except, binary argument, numeric room, with a timeout flag)
- **THEN** it yields the uid, event, arguments, namespace, rooms, except rooms and flags the recorder noted

#### Scenario: Corvis broadcast read by Node
- **WHEN** a raw subscriber in a mixed cluster records what each instance publishes for the same emit
- **THEN** the two messages decode to the same structure, uids aside

#### Scenario: Local broadcast
- **WHEN** an instance emits with the `local` operator
- **THEN** nothing is published and only its own sockets receive the event

#### Scenario: Own message
- **WHEN** an instance receives the broadcast it just published
- **THEN** its sockets receive the event once

### Requirement: Requests and answers match the Node adapter
Requests and answers SHALL use the Node adapter's types and fields: JSON for socket fetch, join, leave, disconnect,
server-side emit and their answers; MessagePack for a broadcast with acknowledgements and for each acknowledgement;
JSON for its client count. On input both encodings MUST be accepted, told apart by the first byte as the Node adapter
does.

#### Scenario: Recorded Node requests
- **WHEN** the codec decodes each request and answer recorded from the Node adapter (types 1 to 9)
- **THEN** it yields the type, uid, request id and fields the recorder noted

#### Scenario: Requests written by Corvis
- **WHEN** Corvis encodes the same operations as the recorded ones
- **THEN** each message uses the encoding the Node adapter used and decodes to the same structure, ids aside

#### Scenario: Requests Node no longer sends
- **WHEN** a peer sends a type 0 request (sockets of rooms) or a type 1 request (all rooms)
- **THEN** the Corvis instance answers with its local socket ids or its local rooms, in the Node adapter's format

#### Scenario: Legacy single-socket requests
- **WHEN** a peer sends a type 2, 3 or 4 request addressed by `sid` instead of `opts`
- **THEN** the Corvis instance ignores it and changes nothing

### Requirement: MessagePack values convert like notepack.io
The codec SHALL read every standard MessagePack form the Node adapter's encoder (`notepack.io` 3.0.1) can write and
SHALL write values that it decodes to the same JavaScript value. Byte-equal output is not required. A message with a
form that has no Socket.IO value MUST be rejected whole.

#### Scenario: Numbers
- **WHEN** Corvis encodes `1`, `-200`, `4294967296`, `1.5` and a float that holds `3.0`
- **THEN** integers and the integral float are written as MessagePack integers in the smallest form, `1.5` as
  float 64, and `notepack.io` decodes each to the same number

#### Scenario: Values only JavaScript writes
- **WHEN** a recorded message holds `undefined`, a `Date`, a NaN and a legacy `ArrayBuffer` extension
- **THEN** they decode to `null`, the ISO 8601 text of the date, `null` and a binary value

#### Scenario: Forms without a Socket.IO value
- **WHEN** a message holds an unknown extension type, a map key that is not a string, invalid UTF-8 in a string, a
  reserved marker or bytes after the value
- **THEN** the whole message is rejected

#### Scenario: Round trip
- **WHEN** any Socket.IO value within the limits is encoded and decoded again (property test)
- **THEN** the result equals the value

### Requirement: Broadcasts reach the whole cluster
A broadcast SHALL reach, exactly once, every socket of every instance that the same broadcast would select on that
instance alone (`ws/socket-io-rooms`): rooms, their union, except rooms and socket ids, broadcasts from a socket,
binary arguments. A mixed cluster MUST deliver what two Node instances deliver.

#### Scenario: Mixed cluster broadcast
- **WHEN** clients are connected to the Node instance and to the Corvis instance, and either instance emits to the
  namespace
- **THEN** every client receives the event once, as with two Node instances

#### Scenario: Room and except from Node to Corvis
- **WHEN** the Node instance emits to room `r` except room `x`
- **THEN** the Corvis instance delivers it only to its sockets in `r` and not in `x`, as a second Node instance does

#### Scenario: Broadcast from a socket
- **WHEN** a socket on the Corvis instance broadcasts to room `r`, which has members on both instances
- **THEN** every other member receives the event and the sender does not, as with two Node instances

#### Scenario: Binary broadcast across instances
- **WHEN** the Node instance broadcasts an event with a 1 KiB buffer, and then the Corvis instance does
- **THEN** the clients of the other instance receive an equal buffer both times

#### Scenario: Numeric room
- **WHEN** the Node instance emits to room `42` given as a number
- **THEN** the Corvis sockets that joined room `"42"` receive the event

#### Scenario: Room without local members
- **WHEN** a broadcast for room `r` arrives on its room channel and no local socket is in `r`
- **THEN** the message is dropped without being decoded

### Requirement: Bulk operations reach the whole cluster
`sockets_join`, `sockets_leave` and `disconnect_sockets` SHALL act on the selected sockets of every instance unless
the operator is local. Requests from other instances MUST be applied with the room limits of `ws/socket-io-rooms`.

#### Scenario: Join across instances
- **WHEN** the Corvis instance makes every socket in `r1` join `r2`, and then the Node instance emits to `r2`
- **THEN** the former members of `r1` on both instances receive the event, as with two Node instances

#### Scenario: Leave and disconnect requested by Node
- **WHEN** the Node instance makes room `r` leave `r2` and then disconnects the sockets in `r`
- **THEN** the Corvis sockets in `r` leave `r2` and are then disconnected, as on a second Node instance

#### Scenario: Remote join over the room cap
- **WHEN** a join request from another instance selects a local socket that is at `max_rooms_per_socket`
- **THEN** that socket is skipped and logged, and the other selected sockets join

### Requirement: Fetching sockets across instances
`fetch_sockets` SHALL return the selected sockets of every instance with their id, handshake, rooms and data, and
each returned socket MUST support `emit`, `join`, `leave` and `disconnect` wherever it is connected. With one
instance, or a local operator, it MUST answer without waiting on Redis.

#### Scenario: Fetch sockets across instances
- **WHEN** a client is connected to the Node instance and the Corvis instance fetches the sockets of its room
- **THEN** the result contains that socket's id, rooms and data, as a Node instance returns it

#### Scenario: Remote socket operations
- **WHEN** Corvis code calls `emit` and then `disconnect` on a fetched socket that is connected to the Node instance
- **THEN** that client receives the event and is then disconnected

#### Scenario: Fetch answered by Corvis
- **WHEN** the Node instance fetches the sockets of a room whose members are connected to the Corvis instance
- **THEN** it receives their ids, rooms, data and the handshake fields Node's `RemoteSocket` exposes

#### Scenario: Single instance
- **WHEN** only one instance is subscribed and it fetches sockets
- **THEN** the local sockets are returned and no request is published

### Requirement: Handshake data shared with other instances is redacted by default
When a Corvis instance answers a socket fetch, the handshake it publishes MUST NOT contain the `cookie`,
`authorization` and `proxy-authorization` request headers, and its `auth` MUST be an empty object, unless the
application selects `SharedHandshake::Full`, which sends what the Node adapter sends.

#### Scenario: Default
- **WHEN** a client connects to a Corvis instance with a cookie, an `Authorization` header and `auth: {token: "t"}`,
  and another instance fetches its socket
- **THEN** the fetched handshake has the other headers, the address, the URL and the query, no credential header and
  an empty `auth`; the harness lists the difference from Node as expected

#### Scenario: Full handshake
- **WHEN** the same happens with `shared_handshake(SharedHandshake::Full)`
- **THEN** the fetched handshake equals the one a Node instance publishes

#### Scenario: Nothing else is published
- **WHEN** a raw subscriber records everything a Corvis instance publishes while clients connect, emit and disconnect
- **THEN** no message contains a client's cookie, authorization header or `auth` payload

### Requirement: Broadcast acknowledgements across instances
A broadcast with acknowledgements SHALL collect the answers of the selected clients of every instance. It completes
when every instance reported its client count and every counted client answered, and fails at its deadline with
`operation has timed out` and the answers received, within the bounds of `ws/socket-io-rooms`.

#### Scenario: Every client answers
- **WHEN** an instance of a mixed cluster broadcasts with a 5 s timeout to a room with clients on both instances that
  all answer
- **THEN** the emitter receives every answer, as with two Node instances

#### Scenario: One remote client silent
- **WHEN** a client on the other instance never answers and the timeout is 200 ms
- **THEN** the call fails with `operation has timed out` and the answers received, not before 200 ms

#### Scenario: Corvis answers for Node
- **WHEN** the Node instance broadcasts with acknowledgements to clients of the Corvis instance
- **THEN** the Corvis instance publishes its client count and one answer per client, each with the first argument of
  the client's acknowledgement, as a Node instance does

### Requirement: Server-side emit
`server_side_emit` SHALL deliver an event to the `on_server_side_emit` handlers of the same namespace on every other
instance, and never to the emitting one. `server_side_emit_with_ack` SHALL resolve with one answer per other
instance. Reserved event names and binary arguments MUST be rejected before publishing. An adapter without cluster
support MUST fail with `this adapter does not support the serverSideEmit() functionality`.

#### Scenario: Emit to the other instances
- **WHEN** the Corvis instance calls `server_side_emit("hello", ("world", 1))` in a cluster of three
- **THEN** the handlers of the two other instances run with `"world"` and `1`, and its own handler does not

#### Scenario: Acknowledged by Node and Corvis
- **WHEN** an instance emits with an acknowledgement and each other instance answers, in a mixed cluster
- **THEN** the emitter receives one answer per other instance, as with two Node instances

#### Scenario: An instance does not answer
- **WHEN** one of two other instances never answers within `requests_timeout`
- **THEN** the call fails with `timeout reached: only 1 responses received out of 2` and the answer received

#### Scenario: Alone
- **WHEN** an instance with no peer emits with an acknowledgement
- **THEN** the call completes at once with an empty list

#### Scenario: Binary argument
- **WHEN** application code passes a binary value to `server_side_emit`
- **THEN** the call fails and nothing is published

#### Scenario: In-memory adapter
- **WHEN** `server_side_emit` is called on a server that uses the in-memory adapter
- **THEN** it fails with `this adapter does not support the serverSideEmit() functionality`

#### Scenario: Handler panics
- **WHEN** an `on_server_side_emit` handler panics
- **THEN** the panic is contained and logged, and later requests are still handled

### Requirement: Requests that wait for other instances are bounded
An operation that waits for answers MUST end no later than `requests_timeout` (default 5 s) with the Node adapter's
timeout error. At most `max_pending_requests` (default 1,000) may wait per namespace, and the answers collected for
one request are bounded by `max_response_bytes` (default 16 MiB); beyond either the operation MUST fail without
publishing or holding more.

#### Scenario: Fetch timeout
- **WHEN** another instance is subscribed and never answers a socket fetch (paused clock)
- **THEN** the call fails at 5 s with `timeout reached while waiting for fetchSockets response` and its pending entry
  is removed

#### Scenario: Too many pending requests
- **WHEN** `max_pending_requests` is 2 and a third request is issued while two wait
- **THEN** it fails at once and nothing is published for it

#### Scenario: Answers too large
- **WHEN** `max_response_bytes` is 1 MiB and peers answer a fetch with 2 MiB of sockets
- **THEN** the call fails with the answers collected so far and holds at most 1 MiB of them

#### Scenario: Answer for an unknown request
- **WHEN** an answer arrives whose request id is not pending, or whose shape does not fit the pending request
- **THEN** it is ignored and no pending request changes

### Requirement: Local delivery never depends on Redis
Room membership and delivery to the sockets of the emitting instance SHALL work whatever the state of Redis.
Operations that need other instances (`fetch_sockets`, acknowledgements, the server count, server-side emit with an
acknowledgement) MUST fail with an error that says the cluster is unavailable while Redis is not connected, without
waiting for `requests_timeout`.

#### Scenario: Broadcast while Redis is down
- **WHEN** the adapter's connections are cut and an instance emits to a room
- **THEN** its own sockets in the room receive the event and the emit succeeds

#### Scenario: Join while Redis is down
- **WHEN** a socket joins and leaves rooms while Redis is not connected
- **THEN** both succeed immediately

#### Scenario: Fetch while Redis is down
- **WHEN** an instance calls `fetch_sockets` while Redis is not connected
- **THEN** the call fails at once with the unavailable error

### Requirement: Publishing is best effort and bounded
Publishing SHALL NOT make an emit wait on Redis. Messages wait in a queue bounded by `max_publish_queue_bytes`
(default 16 MiB) and are published in the order they were emitted. A message that does not fit, or that is produced
while Redis is not connected, MUST be dropped, counted and logged, and MUST NOT be published later.

#### Scenario: Order
- **WHEN** one task emits `a` and then `b` to a room with clients on another instance
- **THEN** those clients receive `a` before `b`

#### Scenario: Queue full
- **WHEN** `max_publish_queue_bytes` is 1 MiB and emits produce 4 MiB while the publisher is stalled
- **THEN** at most 1 MiB is queued, the rest is dropped and counted, every emit returns without waiting and local
  sockets receive every event

#### Scenario: No replay after an outage
- **WHEN** an instance emits ten events while Redis is not connected, and Redis then returns
- **THEN** the other instances receive none of the ten, and the counter of messages dropped while not connected is 10

### Requirement: Outgoing messages are bounded
A message larger than `max_message_bytes` (default 16 MiB) MUST NOT be published. A broadcast that would produce one
MUST fail before anything is delivered, also locally; an answer that would exceed it MUST NOT be sent, truncated or
split.

#### Scenario: Oversized broadcast
- **WHEN** `max_message_bytes` is 1 MiB and application code broadcasts an event of 2 MiB
- **THEN** the emit fails with an error that says the message is too large and no socket of any instance receives it

#### Scenario: Oversized fetch answer
- **WHEN** the sockets selected by a fetch request encode to more than `max_message_bytes`
- **THEN** nothing is published, an error is logged, and the requester ends with its timeout error

### Requirement: The adapter reconnects and resubscribes
Losing a Redis connection SHALL NOT crash or block the application. The adapter MUST reconnect by itself with
exponential backoff and jitter, between `reconnect_min_delay` (100 ms) and `reconnect_max_delay` (5 s), without a
retry limit, and MUST subscribe again for every namespace before it reports itself connected.

#### Scenario: Connections cut
- **WHEN** the adapter's connections are killed on the Redis server while clients are connected
- **THEN** the state becomes disconnected and then connected again, the subscriptions of every namespace are back,
  and a broadcast from another instance is delivered again

#### Scenario: Backoff
- **WHEN** Redis stays unreachable (paused clock)
- **THEN** the delay between attempts grows from 100 ms and never exceeds 5 s plus jitter, and attempts never stop

#### Scenario: Namespace added while disconnected
- **WHEN** a namespace is registered while Redis is not connected and Redis then returns
- **THEN** that namespace is subscribed as well

#### Scenario: Pending requests during an outage
- **WHEN** the subscriber connection is lost while a fetch is waiting for answers
- **THEN** the fetch ends with its timeout error at its deadline, not later

### Requirement: Startup connection
`RedisAdapter::connect` SHALL wait for both connections and fail when they are not established within
`connect_timeout` (default 10 s), so a wrong address or password stops the application at startup.
`RedisAdapter::connect_lazy` SHALL return at once and connect in the background. Closing the adapter or its server
MUST unsubscribe and fail the requests still pending.

#### Scenario: Wrong address
- **WHEN** `connect` is given the address of a closed loopback port
- **THEN** it fails within `connect_timeout` with an error that does not contain the URL's password

#### Scenario: Lazy start without Redis
- **WHEN** `connect_lazy` is given the address of a closed loopback port and a server starts with the adapter
- **THEN** the server serves its clients, local broadcasts work and the state is not connected

#### Scenario: TLS without the feature
- **WHEN** a `rediss://` URL is given and Corvis' `tls` feature is off
- **THEN** creating the adapter fails with an error that names the feature

#### Scenario: Close
- **WHEN** a server with the adapter is closed while a fetch is pending
- **THEN** the fetch fails with an error that says the adapter is closed, and Redis shows no subscription left from
  that instance

### Requirement: Messages from Redis are untrusted input
Every message read from Redis SHALL be validated completely before it has any effect. A message that fails a check
MUST be dropped whole, counted and logged without its content, at most once every 10 seconds per reason. No message
MUST be able to make the process panic or to change state outside the namespace of its channel.

#### Scenario: Malformed messages
- **WHEN** a raw Redis client publishes truncated MessagePack, a two-element array, a packet without `data`, an
  empty `data`, a non-string event name and wrong field types on a namespace's channels
- **THEN** each is dropped and counted, no socket receives anything and the instance keeps serving

#### Scenario: Arbitrary bytes
- **WHEN** arbitrary bytes are given to the broadcast, request and answer decoders (property test)
- **THEN** none panics and each returns a message or an error

#### Scenario: Another namespace's packet
- **WHEN** a message published on the channel of `/a` carries `nsp: "/b"`
- **THEN** it is dropped and no socket of `/a` or `/b` receives it

#### Scenario: Oversized ids
- **WHEN** a request carries a `uid` or a `requestId` longer than 64 bytes
- **THEN** it is dropped

#### Scenario: Log flood
- **WHEN** 10,000 malformed messages arrive within one second
- **THEN** at most one log line per reason is written, the counter shows 10,000, and no line contains message bytes

### Requirement: A remote message never widens delivery
A broadcast or request from Redis whose `rooms` or `except` holds an entry that is not a string or an integer, or a
name longer than `max_room_name_length`, MUST be rejected whole: an entry MUST NOT be skipped. Integers are read as
their decimal text. A broadcast whose event is one of Socket.IO's reserved names, or that carries more binary values
than `max_attachments`, MUST be rejected.

#### Scenario: Invalid except entry
- **WHEN** a forged broadcast targets the namespace with `except` holding a 257-byte name
- **THEN** no socket receives the event

#### Scenario: Invalid room entry
- **WHEN** a forged broadcast has `rooms` holding one valid room and one entry that is an object
- **THEN** no socket receives the event, including the members of the valid room

#### Scenario: Reserved event from Redis
- **WHEN** a forged broadcast carries the event `disconnect` or `connect_error`
- **THEN** it is dropped and no client receives it

#### Scenario: Too many binary values
- **WHEN** a forged broadcast carries 11 binary arguments with the default `max_attachments`
- **THEN** it is dropped

### Requirement: Decoding is bounded
Decoding a message from Redis MUST use memory and time proportional to its size: nesting deeper than 128 is
rejected, nothing is allocated from a declared length, and a declared length larger than the bytes that remain is an
error. A message larger than `max_message_bytes` (default 16 MiB) MUST be dropped without being decoded.

#### Scenario: Depth
- **WHEN** a message nests arrays 129 deep, in MessagePack and in JSON
- **THEN** it is rejected without overflowing the stack

#### Scenario: Declared length
- **WHEN** a 10-byte message declares an array, a map, a string or a binary of 2^32 - 1 elements
- **THEN** it is rejected without allocating for the declared length

#### Scenario: Oversized message
- **WHEN** `max_message_bytes` is 1 MiB and a 2 MiB message is published on a broadcast channel
- **THEN** it is dropped, counted as oversized and not decoded

### Requirement: Work done for other instances is bounded
Incoming requests SHALL wait for their worker in a queue of at most `max_queued_requests` (default 1,024); a request
that does not fit is dropped and counted. Acknowledgement trackers kept for other instances' broadcasts MUST number
at most `max_remote_ack_requests` (default 1,000) per namespace and MUST be removed, with the pending
acknowledgements they registered on local sockets, at their deadline.

#### Scenario: Request flood
- **WHEN** 100,000 fetch requests are published while the worker is stalled
- **THEN** at most 1,024 are queued, the rest are dropped and counted, and broadcasts keep being delivered

#### Scenario: Tracker cap
- **WHEN** 1,001 broadcast-with-acknowledgement requests arrive while none has expired
- **THEN** the 1,001st is answered with a client count of 0 and delivers nothing

#### Scenario: Tracker deadline
- **WHEN** a remote broadcast with acknowledgements carries a timeout of 2^53 ms, and another carries none
- **THEN** the first tracker is removed after 10 minutes and the second after `ack_timeout`, and no local socket
  holds a pending acknowledgement for them afterwards (paused clock)

### Requirement: Redis credentials are never logged
The Redis URL's user information MUST NOT appear in any log line, error message, `Debug` output or panic message of
the adapter.

#### Scenario: Connection failure
- **WHEN** `connect` fails for `redis://user:secret@127.0.0.1:1/`
- **THEN** neither the returned error nor the log contains `secret`

#### Scenario: Debug output
- **WHEN** a `RedisAdapter` created with a password in its URL is formatted with `{:?}`
- **THEN** the output does not contain the password

### Requirement: State and counters are observable
The adapter SHALL expose its connection state (connecting, connected, disconnected) and counters: messages published
and received, messages dropped by reason (oversized, malformed, queue full, not connected) and reconnects. Each
connection MUST carry a configurable client name, default `corvis-socket-io`.

#### Scenario: Counters after an outage
- **WHEN** the adapter's connections are cut once and one broadcast is emitted during the gap
- **THEN** the counters show one reconnect and one message dropped while not connected

#### Scenario: Client name
- **WHEN** `CLIENT LIST` is read on the Redis server while an adapter is connected
- **THEN** its two connections carry the configured client name

### Requirement: Options are validated
Creating a Redis adapter SHALL fail, with an error naming the option, on a zero `requests_timeout` or one above a
day, a zero size or count limit, a `reconnect_min_delay` that is zero or above `reconnect_max_delay`, an invalid
`key`, and a URL that is not `redis://` or `rediss://`. Creating a server MUST fail when the adapter's
`max_message_bytes` is below the server's `max_http_buffer_size`.

#### Scenario: Zero timeout
- **WHEN** an adapter is created with `requests_timeout` zero
- **THEN** creation fails with an error naming `requests_timeout`

#### Scenario: Invalid key
- **WHEN** an adapter is created with an empty key, a key of 129 bytes or a key with a control character
- **THEN** creation fails with an error naming `key`

#### Scenario: Message limit below a packet
- **WHEN** a server whose `max_http_buffer_size` is 1,000,000 is created with an adapter whose `max_message_bytes`
  is 500,000
- **THEN** creating the server fails with an error naming `max_message_bytes`

### Requirement: Security divergences are explicit
Every behavior of the adapter where the Node adapter has no bound or a different one SHALL be documented in
`docs/websockets/adapter.md` (English and Spanish) and in the rustdoc of `RedisAdapterOptions`, together with the
statement that the Redis server is inside the application's trust boundary. Each divergence a client can observe
MUST have an entry in the harness's `expected-differences.json` with the scenario that exercises it.

#### Scenario: Divergences are listed
- **WHEN** the harness runs the cluster scenarios
- **THEN** every difference reported is listed with its reason, and no listed difference is stale

#### Scenario: Client-observable divergence
- **WHEN** the list of expected differences of the cluster scenarios is read
- **THEN** it holds the redacted handshake of fetched sockets and nothing else

#### Scenario: Documented limits
- **WHEN** the documentation page is read
- **THEN** it lists every limit of the adapter with its default and the Node adapter's value, the trust boundary, and
  the two bounds that depend on the Redis server's configuration (`proto-max-bulk-len` and
  `client-output-buffer-limit pubsub`)

### Requirement: Verified against the Node adapter
The message formats SHALL be verified against fixtures recorded from the pinned Node adapter, with the package
versions stored in the fixture, and the behavior by mixed-cluster scenarios whose reference is the same scenario on
two Node instances. Rust tests that need Redis SHALL run when `CORVIS_REDIS_URL` is set and MUST fail instead of
skipping when `CORVIS_REDIS_REQUIRED` is `1`.

#### Scenario: Fixture versions
- **WHEN** the fixture's recorded versions differ from the ones the harness lockfile pins
- **THEN** the fixture test fails and names the recorder script

#### Scenario: Mixed-cluster scenarios pass
- **WHEN** the harness runs with `CORVIS_NODE_DIFF=1` and `CORVIS_REDIS_URL` set
- **THEN** the cluster scenarios report no unlisted and no stale difference

#### Scenario: Local run without Redis
- **WHEN** `cargo test -p corvis --features socket-io-redis` runs without `CORVIS_REDIS_URL`
- **THEN** the tests that need Redis print a skip notice and pass, and the codec, fixture and property tests run

#### Scenario: Redis required
- **WHEN** `CORVIS_REDIS_REQUIRED=1` is set and Redis is not reachable or `CORVIS_REDIS_URL` is not set
- **THEN** the tests that need Redis fail with a message naming `CORVIS_REDIS_URL`

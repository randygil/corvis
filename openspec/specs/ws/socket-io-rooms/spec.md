# ws/socket-io-rooms Specification

## Purpose
Rooms and broadcasting for Socket.IO namespaces (feature `socket-io`): `join`, `leave`, the broadcast operators `to`,
`within`, `except`, `broadcast`, `local` and `timeout`, the adapter interface behind them and its in-memory
implementation, with the limits of this layer. Which sockets receive each broadcast MUST match the in-memory adapter
of Node `socket.io-adapter` 2.5.8 under `socket.io` 4.8.4, verified by the differential harness of
`ws/socket-io-parity`, except the bounds listed in "Security divergences are explicit". Every limit has a bounded
default. Implemented by [#34](https://github.com/randygil/corvis/issues/34).

Timing rule for every scenario below: absence of delivery is observed through a marker event sent after the
broadcast, never by waiting; "not before" bounds use the configured value; timer logic is tested on a paused clock.

## Requirements

### Requirement: Rooms belong to a namespace
Rooms SHALL be scoped to one namespace: joining room `r` in `/` MUST NOT make a socket receive broadcasts to room `r`
in `/admin`. Every socket MUST be in a room named after its own id from the moment it connects, before its connection
handlers run, so emitting to a socket id reaches that socket.

#### Scenario: Same room name in two namespaces
- **WHEN** socket A joins `r` in `/` and socket B joins `r` in `/admin`, and the server emits to `r` in `/`
- **THEN** only A receives the event, on both servers

#### Scenario: Emit to a socket id
- **WHEN** the server emits to the id of socket A
- **THEN** only A receives the event, on both servers

#### Scenario: Rooms right after connecting
- **WHEN** a connection handler reads the rooms of the socket it was given
- **THEN** they are exactly the socket's own id, on both servers

### Requirement: Broadcast operators select the same sockets as Node
`to` / `within` SHALL accept one or more rooms and deliver once to every socket in their union; `except` SHALL accept
one or more rooms (socket ids included) and exclude every socket in them; without `to`, a broadcast reaches every
socket of the namespace not excluded. Operators MUST chain and MUST NOT modify the operator they were called on. A
broadcast started from a socket (`broadcast`, `to`, `within`, `except` on the socket) MUST exclude that socket. A room
nobody is in selects nobody.

#### Scenario: Whole namespace
- **WHEN** the server emits on the namespace `/` with A, B and C connected to `/` and D to `/admin`
- **THEN** A, B and C receive the event and D does not, on both servers

#### Scenario: Union without duplicates
- **WHEN** socket A is in rooms `r1` and `r2` and the server emits to `r1` and `r2` together
- **THEN** A receives the event exactly once, on both servers

#### Scenario: Broadcast from a socket
- **WHEN** socket A, in room `r` with B and C, emits to `r` from its own socket
- **THEN** B and C receive the event and A does not, on both servers

#### Scenario: Except a room
- **WHEN** the server emits to the namespace except room `admins`
- **THEN** every socket of the namespace that is not in `admins` receives the event, on both servers

#### Scenario: Except a socket id inside a room
- **WHEN** A, B and C are in `r` and the server emits to `r` except the id of B
- **THEN** A and C receive the event and B does not, on both servers

#### Scenario: Empty room
- **WHEN** the server emits to a room that no socket has joined
- **THEN** no socket receives the event and the emit succeeds, on both servers

### Requirement: A broadcast is one packet, sent in order
A broadcast SHALL deliver to each recipient the same packet a direct emit of the same event would, binary attachments
included, encoded once for all recipients. Packets emitted by one task to the same socket, by broadcast or directly,
MUST arrive in the order they were emitted; broadcasts of concurrent tasks have no order between them. Reserved event
names MUST be rejected as for a direct emit.

#### Scenario: Binary broadcast
- **WHEN** the server broadcasts an event carrying a 1 KiB buffer to a room of three sockets, one of them on polling
- **THEN** each receives an equal buffer, on both servers

#### Scenario: Order
- **WHEN** a handler emits `a` to a room and then `b` directly to a socket of that room
- **THEN** that socket receives `a` before `b`

#### Scenario: Order across a yield
- **WHEN** one task awaits 20 broadcasts in a row to 1,300 sockets while another task broadcasts to the same sockets
- **THEN** every socket receives the 20 broadcasts of the first task in the order they were issued

### Requirement: Leaving and disconnecting update rooms like Node
`leave` SHALL remove the socket from a room immediately. A disconnecting socket MUST still be in its rooms while the
`disconnecting` handlers run and in none when the `disconnect` handlers run. A room MUST be deleted when its last
socket leaves it. Joining a socket that has disconnected MUST do nothing, and after a socket disconnected the adapter
MUST hold no membership for it, whatever joins ran concurrently.

#### Scenario: Leave
- **WHEN** socket A leaves `r` and the server then emits to `r`
- **THEN** A does not receive the event, on both servers

#### Scenario: Rooms during disconnecting
- **WHEN** a socket in room `r` disconnects and the server reads its rooms in `disconnecting` and in `disconnect`
- **THEN** the first read contains `r` and its own id and the second is empty, on both servers

#### Scenario: Farewell from the disconnecting handler
- **WHEN** a `disconnecting` handler emits `left` to the rooms of the socket from that socket
- **THEN** the other members of those rooms receive `left`, on both servers

#### Scenario: Join racing a disconnect
- **WHEN** many tasks join a socket to rooms while it disconnects, repeated 1,000 times
- **THEN** after each `disconnect` the adapter has no entry for the socket's id and no empty room

### Requirement: Bulk operations match Node
`sockets_join`, `sockets_leave` and `disconnect_sockets` SHALL act on the sockets the operator selects, and
`fetch_sockets` SHALL return them with their id, handshake, rooms and data; a returned socket MUST support `emit`,
`join`, `leave` and `disconnect` whether or not it is connected to this process.

#### Scenario: Make a room join another
- **WHEN** the server makes every socket in `r1` join `r2` and then emits to `r2`
- **THEN** the sockets that were in `r1` receive the event, on both servers

#### Scenario: Disconnect a room
- **WHEN** the server disconnects the sockets in `r` without closing their connections
- **THEN** those clients receive `io server disconnect` for that namespace only and the others stay connected, on both
  servers

#### Scenario: Fetch sockets
- **WHEN** the server fetches the sockets in `r` except the id of B
- **THEN** the ids and rooms returned are those of the other members of `r`, on both servers

### Requirement: Broadcast acknowledgements match Node
A broadcast with acknowledgements SHALL send one packet with one id to every recipient and resolve with the answer of
each once all have answered. At its timeout it MUST fail with the message `operation has timed out` together with the
answers received so far.

#### Scenario: Every recipient answers
- **WHEN** the server broadcasts with a 5 s timeout to a room of three clients that all answer
- **THEN** the server receives the three answers, on both servers

#### Scenario: One recipient silent
- **WHEN** one of the three never answers and the timeout is 200 ms
- **THEN** the server side fails with `operation has timed out` and the two answers received, not before 200 ms, on
  both servers

### Requirement: Broadcast acknowledgements are bounded
A broadcast with acknowledgements MUST have a deadline: the one given for the call, or `ack_timeout` (default 30 s).
At the deadline the server MUST stop waiting on the recipients that did not answer. Each recipient's answer MUST be
counted once, and only answers from recipients of that broadcast count. The answers collected by one broadcast SHALL
be bounded by `max_broadcast_ack_bytes` (default 16 MiB); beyond it further answers are dropped and the call fails
with the answers collected.

#### Scenario: Default deadline
- **WHEN** server code broadcasts with acknowledgements and no timeout to a client that never answers (paused clock)
- **THEN** the call fails with `operation has timed out` at 30 s and no recipient holds a pending acknowledgement

#### Scenario: Answer from outside the broadcast
- **WHEN** a socket that was not a recipient sends an ACK with the broadcast's id, and a recipient sends its ACK twice
- **THEN** the outsider's answer is ignored and the recipient's is counted once

#### Scenario: Answers too large
- **WHEN** `max_broadcast_ack_bytes` is 1 MiB and 20 recipients each answer with 100 kB
- **THEN** the call fails reporting that the answers are too large, with at most 1 MiB of answers held

### Requirement: Rooms per socket are bounded
A socket SHALL be in at most `max_rooms_per_socket` rooms (default 1,000, the room of its own id included). A join
that would exceed it, or that names more rooms than that, MUST fail without joining any room, the second before looking
at its names or taking the membership lock.

#### Scenario: Oversized join
- **WHEN** a join names 200,000 rooms, with `max_rooms_per_socket` at 50
- **THEN** it fails at once and no name is examined under the membership lock

#### Scenario: Room cap
- **WHEN** application code joins a socket to 1,000 more rooms in one call, on a socket that is in its own room only
- **THEN** the call fails, the socket is still in exactly one room, and the harness lists the acknowledged error as an
  expected difference from Node, which has no cap

### Requirement: Room names are bounded and never widen a broadcast
A room name SHALL be at most `max_room_name_length` bytes (default 256). A join with a longer name MUST fail. A
broadcast operator given a longer name in `to`, `within` or `except` MUST make every operation on it fail without
sending or changing anything; the name MUST NOT be silently dropped.

#### Scenario: Long name in join
- **WHEN** application code joins a socket to a room whose name is 257 bytes
- **THEN** the join fails, and the harness lists the acknowledged error as an expected difference from Node

#### Scenario: Long name in to
- **WHEN** application code emits to a single room whose name is 257 bytes
- **THEN** the emit fails and no socket of the namespace receives the event

### Requirement: Broadcasts never wait on recipients
A broadcast SHALL NOT wait for any recipient to read. Each recipient's queued bytes stay within the Engine.IO send
buffer bound; a recipient that exceeds it is closed (`transport error`) without affecting the emitter or the other
recipients. Room membership MUST NOT be locked while packets are delivered, so handlers can join, leave and broadcast
during a broadcast.

#### Scenario: Many slow clients
- **WHEN** the server, with the send buffer bound set to 1 MiB, broadcasts 16 MiB in 64 KiB events to a room of 50
  clients that never read and one that does
- **THEN** every emit returns without waiting for a client, no session queues more than its send buffer bound plus one packet, the 50
  sessions are closed with `transport error`, and the reading client receives every event

#### Scenario: Overflow warnings are rate limited
- **WHEN** a broadcast closes 1,000 sessions by overflow within a minute
- **THEN** one warning is logged, and the next one a minute later says how many were left out

#### Scenario: Join during a broadcast
- **WHEN** a handler triggered by a broadcast joins its socket to another room while the broadcast is being delivered
- **THEN** the join and the broadcast both complete

### Requirement: Adapter abstraction
Room membership and broadcast delivery SHALL go through an adapter interface, one adapter per namespace, whose
cluster-wide operations are asynchronous and can fail. The in-memory implementation SHALL be the default, and another
adapter MUST be able to replace it through the server options without changes to namespaces, sockets or operators.

#### Scenario: Interface shape
- **WHEN** the adapter interface is read
- **THEN** it has synchronous membership of local sockets (add, delete, delete all, rooms of a socket) and
  asynchronous, fallible broadcast, broadcast with acknowledgements, sockets of rooms, fetch sockets, add sockets to
  rooms, remove sockets from rooms, disconnect sockets, server count, init and close; broadcast options carry rooms,
  except rooms and flags; and the packet reaches the adapter before it is encoded

#### Scenario: Default adapter
- **WHEN** an application uses socket.io without configuring an adapter
- **THEN** rooms and broadcasts work in-process through the in-memory adapter

#### Scenario: Replaced adapter
- **WHEN** the room tests run with a test adapter that wraps the in-memory one, suspends on every cluster operation
  and reports two servers, one of them simulated
- **THEN** the same tests pass, and a broadcast with acknowledgements completes only after both servers reported
  their recipients

#### Scenario: Adapter failure
- **WHEN** the test adapter fails a broadcast
- **THEN** the emit returns the adapter's error to the caller and no socket is disconnected

#### Scenario: Close once
- **WHEN** the server is closed
- **THEN** the close of each namespace's adapter runs once, however many paths reach it

### Requirement: Options are validated
Creating a Socket.IO server SHALL fail, with an error naming the option, on a zero `max_rooms_per_socket`,
`max_room_name_length`, `max_broadcast_ack_bytes` or `adapter_timeout` (default 5 s, at most a day), and on a
`max_room_name_length` shorter than a socket id.

#### Scenario: Zero rooms
- **WHEN** a server is created with `max_rooms_per_socket(0)`
- **THEN** creation fails with an error naming `max_rooms_per_socket`

### Requirement: The room of a socket id is not for clients
A socket MUST NOT join a room named like the id of another live socket of its namespace, nor leave the room named like
its own id; a join that names one fails and joins none of its rooms, and `sockets_join` skips such sockets. A new
socket MUST NOT be given an id that is already a room. An operation of a `RemoteSocket` MUST reach that socket only:
directly when it is local, by its id as a single-socket target otherwise, never as the members of a room.

#### Scenario: Join another socket's id
- **WHEN** a client has the application join it to the id of another socket and the server emits to that id
- **THEN** the join fails, only the other socket receives the event, and the harness lists the difference from Node

#### Scenario: Leave its own id
- **WHEN** a socket leaves the room named like its own id and the server emits to that id
- **THEN** it stays in the room and receives the event, and the harness lists the difference from Node

#### Scenario: Id already a room
- **WHEN** the adapter reports that the first two ids drawn for a new socket are rooms
- **THEN** the socket is given a third id

#### Scenario: Remote socket reaches one socket
- **WHEN** a socket was put in the id room of a victim by a defect, and the victim's `RemoteSocket` emits, joins a
  room, or disconnects, directly or through its operator
- **THEN** only the victim receives, joins or is disconnected

### Requirement: An empty list of rooms selects nobody
`to` or `within` given an empty collection MUST select no socket, unless another call of the operator adds rooms: the
operation succeeds and delivers to nobody; a broadcast with no `to` reaches every socket. `except` given an empty
collection MUST have no effect.

#### Scenario: Empty to
- **WHEN** the server emits, fetches, joins and disconnects through `to` given an empty list
- **THEN** no socket receives, is returned, joins or is disconnected, and the harness lists the difference from Node

### Requirement: An operator names a bounded number of rooms
An operator MUST name at most `max_rooms_per_socket` rooms in all (`to`, `within` and `except`); beyond it every
terminal operation MUST fail with a room error and send or change nothing. Building an operator MUST cost what each
call adds, not what the chain already holds.

#### Scenario: Operator naming too many rooms
- **WHEN** `max_rooms_per_socket` is 5 and an operator names 6 rooms through several calls
- **THEN** every terminal operation fails with a room error and sends nothing

#### Scenario: Linear chaining
- **WHEN** 5,000 `to` calls are chained
- **THEN** they allocate less than 1 KiB each in all, measured with a counting allocator

### Requirement: Collected answers count against the calls, not against the clients
The answers held by calls with acknowledgements MUST be charged to a server-wide budget of their own, without waiting, and
MUST NOT stay charged to the unread budget of the connection that sent them beyond the receipt of the answer.

#### Scenario: Answers held by pending calls
- **WHEN** a handler starts 32 calls to a room whose members answer 300 kB each and one never answers
- **THEN** the heap held by the pending calls stays within the answer budget plus a bounded slack, the unread gauges of
  the answering connections, their addresses and the server hold none of it, and the server gives it back when the calls
  end

#### Scenario: One address cannot fill the budget
- **WHEN** one client address answers calls until it holds a sixteenth of the answer budget, and then a call is answered
  from other addresses
- **THEN** the further answers of that address end their calls with `ResponsesTooLarge` and the call answered from other
  addresses completes

#### Scenario: Releasing
- **WHEN** a call ends, however it ends
- **THEN** every answer it held is released from the answer budget

#### Scenario: No room for another answer
- **WHEN** other calls hold the whole answer budget and a call receives one more answer
- **THEN** the call ends with `ResponsesTooLarge` at once and no connection's reads are paused

### Requirement: A panicking adapter does not leave a connecting socket behind
A panic or an error of the adapter while a socket connects (the id check, the join of its own room) MUST be contained: a panic in the
id check MUST refuse the CONNECT with `Internal server error` and free the namespace slot, and a panic joining the own room
MUST disconnect the socket and close the connection. Adding a socket to the room named like its own id MUST expel any other
member of that room.

#### Scenario: Panic in the id check
- **WHEN** `room_exists` panics while a client connects
- **THEN** the client receives CONNECT_ERROR, the namespace holds no socket and the connection holds no session of it

#### Scenario: Panic joining the own room
- **WHEN** `add_all` panics while the socket joins the room of its own id
- **THEN** the socket is removed, its charges released and its connection closed

#### Scenario: A name joined before the socket was live
- **WHEN** a client joins the name that a connecting socket was given, during the connect middleware
- **THEN** the socket's own room expels that client when the socket goes live

#### Scenario: Answering session closes
- **WHEN** a session closes while its answer is held, and the call then ends
- **THEN** the address and server gauges still count the answer until the call ends and are zero afterwards

### Requirement: Adapter calls are bounded
Every call to the adapter made by an operator or by initialisation MUST fail with `AdapterError::Timeout` after
`adapter_timeout`, so an adapter that never returns cannot hang its caller. A panic in the adapter while a socket
disconnects MUST NOT keep the socket in its namespace or keep its charges.

#### Scenario: Adapter never returns
- **WHEN** the adapter's broadcast or init never returns (paused clock)
- **THEN** the emit fails with a timeout error after exactly `adapter_timeout`, and works again once the adapter does

#### Scenario: Adapter panics on disconnect
- **WHEN** `del_all` panics as a client's connection closes
- **THEN** the socket leaves the namespace and its charges are released

### Requirement: Socket descriptions are cheap and keep credentials out of logs
`fetch_sockets` MUST NOT copy the handshake of a socket of this process (it shares it) and MUST allocate a bounded
amount per socket whatever the handshake holds; the description stays serializable as plain data. The `Debug` output of
`RemoteSocket`, `SocketInfo` and `HandshakeInfo` MUST NOT print the values of `cookie`, `authorization` and
`proxy-authorization` headers, the values of `auth`, or a query string.

#### Scenario: Fetch shares the handshake
- **WHEN** 200 sockets with 14 KiB of `auth` each are fetched
- **THEN** less than 3 KiB is allocated per socket and the `auth` object is the socket's own

#### Scenario: Debug output
- **WHEN** a fetched socket whose handshake has a cookie, a bearer token and an `auth` token is printed with `{:?}`
- **THEN** none of those values appears, and the header names and `auth` keys do

### Requirement: Security divergences are explicit
Every bound of this layer where Node has none or a different one (rooms per socket and per operator, room name
length, the rooms of socket ids, an empty `to`, the deadline and bytes of broadcast acknowledgements, the send buffer)
SHALL be documented in
`docs/websockets/adapter.md` (English and Spanish) and the rustdoc of `IoOptions`, and each one a client can observe
MUST have an entry in the harness's `expected-differences.json` with the scenario that exercises it.

#### Scenario: Divergences are listed
- **WHEN** the harness runs the room scenarios against both servers
- **THEN** every difference reported is listed with its reason, and no listed difference is stale

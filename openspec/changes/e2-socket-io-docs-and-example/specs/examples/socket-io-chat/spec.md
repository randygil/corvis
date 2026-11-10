# Spec Delta

## Purpose

The `socket-io-chat` example shows gateways served over Socket.IO on a small real application that the official
`socket.io-client` uses unchanged, with connections authenticated before any event and an optional Redis adapter.

## ADDED Requirements

### Requirement: The example serves a chat gateway over Socket.IO
`examples/socket-io-chat` SHALL be a workspace application (`publish = false`) that selects the Socket.IO adapter
and declares a gateway on the namespace `/chat` of the default Engine.IO path, with handlers to join and leave a
room, send a message and signal typing. It MUST start with `cargo run -p socket-io-chat` and serve its web client
from the same origin.

#### Scenario: Running it
- **WHEN** a user runs `JWT_SECRET=<random> cargo run -p socket-io-chat` and opens the printed address in two
  browsers
- **THEN** both can log in, join the same room and see each other's messages

#### Scenario: Join is acknowledged
- **WHEN** a client emits `join` with a room name and a callback
- **THEN** the callback receives the room's members and recent messages, returned by the handler

#### Scenario: Broadcast without the sender
- **WHEN** a client in a room emits `message`
- **THEN** the other clients of the room receive the `message` event, the sender receives only its acknowledgement,
  and clients of other rooms receive nothing

#### Scenario: Error as an exception event
- **WHEN** a client emits `message` for a room it has not joined
- **THEN** it receives an `exception` event with the code `NOT_IN_ROOM`, and its callback is not called

### Requirement: Connections are authenticated before any event
The example MUST enable connection guards, so that a client whose handshake `auth` holds no valid token is refused
with a connect error and never becomes a socket of the namespace. It MUST have no default signing secret: without
`JWT_SECRET` the application does not start.

#### Scenario: No token
- **WHEN** the official client connects to `/chat` without `auth.token`
- **THEN** it observes `connect_error` with the message `Unauthorized`, the message of the exception the guard
  fails with, and receives no event broadcast afterwards

#### Scenario: Valid token
- **WHEN** a client connects with a token obtained from `POST /api/auth/login`
- **THEN** it connects, and handlers see the user that the connection guard attached to the socket

#### Scenario: Missing secret
- **WHEN** the example starts without `JWT_SECRET`
- **THEN** it exits with an error naming the variable

### Requirement: The example enforces route posture
`create_app()` SHALL enable route posture in enforce mode. Every REST route and every message handler MUST be
guarded or marked `#[public]`, and the report MUST list the `/chat` connection as enforced by connection guards.

#### Scenario: Report in a test
- **WHEN** a test reads `app.route_posture()` after `init()`
- **THEN** no entry is unprotected or exempt, the login route is public, and the `connect` entry of `/chat` is
  protected instead of being listed under "Not checked"

### Requirement: REST handlers reach connected clients
The example SHALL have a REST endpoint, restricted to moderators, that broadcasts an announcement to a room through
the gateway's `IoGatewayServer`, and a REST endpoint that lists the members of a room read with `fetch_sockets`.

#### Scenario: Announcement
- **WHEN** a moderator sends `POST /api/rooms/general/announcements`
- **THEN** every client in `general` receives an `announcement` event and the response reports how many

#### Scenario: Members
- **WHEN** a client requests `GET /api/rooms/general/members` while two users are in the room
- **THEN** the response lists both

### Requirement: A low-level handler shows binary payloads
The example SHALL register, in its connection hook, one handler with `IoSocket::on` that receives a binary argument
and answers through the acknowledgement with a binary value, bounded in size by the server's payload limit.

#### Scenario: Binary round trip
- **WHEN** the official client emits `avatar` with a `Uint8Array` and a callback
- **THEN** the callback receives a binary value of the same bytes

### Requirement: The Redis adapter is optional and chosen at startup
When `REDIS_URL` is set the example MUST use the Redis adapter, and broadcasts SHALL reach clients of every instance
that shares that Redis. Without it the example MUST use the in-memory adapter and need no Redis. The URL MUST NOT
appear in the example's logs.

#### Scenario: Two instances
- **WHEN** two instances run on different ports with the same `REDIS_URL` and a client of each joins the same room
- **THEN** a message sent through one instance is received by the client of the other

#### Scenario: No Redis
- **WHEN** the example runs without `REDIS_URL`
- **THEN** it starts and works with a single instance, and opens no connection to Redis

#### Scenario: Redis unreachable at startup
- **WHEN** `REDIS_URL` points at a closed port
- **THEN** the example exits with an error that does not print the URL's credentials

### Requirement: The web client is the official client at a pinned version
The example's page SHALL load the official `socket.io-client` bundle from the example's own static files, at the
version the differential harness pins, with its license next to it. The page MUST NOT load scripts from another
origin.

#### Scenario: Offline
- **WHEN** the example runs on a machine without network access
- **THEN** the web client loads and connects

#### Scenario: Version drift
- **WHEN** the harness's `socket.io-client` version is changed and the vendored bundle is not
- **THEN** the Node-gated check of the example fails, because the vendored file differs from the installed package's
  bundle

### Requirement: The example is tested with a raw client and with the official client
The example SHALL have Rust tests that start it on `127.0.0.1` and speak Socket.IO over a raw WebSocket, covering
every scenario of this capability that needs one instance. A test gated by `CORVIS_NODE_DIFF=1` MUST drive the same
application with the official client; without the variable it skips with a notice.

#### Scenario: Workspace tests
- **WHEN** `cargo test --workspace` runs without Node and without Redis
- **THEN** the example's raw-client tests run and pass, and the official-client and two-instance tests skip with a
  notice

#### Scenario: Official client in CI
- **WHEN** the `socket-io-diff` job runs
- **THEN** the official-client check connects with and without a token, joins, exchanges a message between two
  clients, receives an `exception` and the binary acknowledgement, and fails the job on any mismatch

#### Scenario: Smoke test
- **WHEN** `node scripts/smoke-examples.mjs socket-io-chat` runs after a build
- **THEN** the example starts on a free loopback port, answers the login route and an Engine.IO handshake, and is
  stopped

### Requirement: The example is safe to run as shipped
The example MUST bind to `127.0.0.1`, declare no CORS (the client is served from the same origin), restrict the
handshake `Origin` to its own origin with the origin allowlist, validate message payloads with a pipe, limit message
length and room names, and state in its README that the login accepts any nickname and must not be exposed as is.

#### Scenario: Oversized message
- **WHEN** a client emits `message` with a text longer than the documented limit
- **THEN** it receives an `exception` with the validation error, and nothing is broadcast

#### Scenario: Handshake from another origin
- **WHEN** a polling or WebSocket handshake carries `Origin: https://evil.example`
- **THEN** it is refused with `403` and no session is created

#### Scenario: No CORS headers
- **WHEN** a polling handshake from the example's own origin is answered
- **THEN** the response carries no `Access-Control-Allow-Origin` header

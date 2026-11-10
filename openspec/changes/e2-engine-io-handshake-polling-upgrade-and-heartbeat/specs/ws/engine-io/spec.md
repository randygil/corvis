# Spec Delta

## Purpose

The Engine.IO v4 server that carries socket.io traffic (feature `socket-io`): handshake, session ids, HTTP
long-polling, CORS of the polling transport, upgrade to WebSocket, heartbeat, packet framing, limits, errors, close
and shutdown. What a client observes MUST match Node `engine.io` 6.6.11, verified by the differential harness of
`ws/socket-io-parity`, except the bounds listed in "Security divergences are explicit". Every limit has a bounded
default. Implemented by [#32](https://github.com/randygil/corvis/issues/32).

Timing rule for every scenario below: "within" bounds are generous (at least ten times the configured value, or a
few seconds), "not before" bounds use the configured value itself, and no scenario depends on how fast the machine
is.

## ADDED Requirements

### Requirement: Engine.IO is served under its own path
An Engine.IO server attached to the application's HTTP server SHALL handle every request whose path equals its path
or starts with it (default `/socket.io/`, the prefix rule of Node's `attach`), before the router, the `ws`-style
gateways and the `upgrade` listeners, for plain HTTP requests and WebSocket upgrades. Requests to other paths MUST
behave exactly as without it. Without the `socket-io` feature nothing changes.

#### Scenario: Polling request is not routed
- **WHEN** a client sends `GET /socket.io/?EIO=4&transport=polling` to an application that has no route for that path
- **THEN** the Engine.IO server answers `200` with an open packet, not the router's `404`

#### Scenario: Other paths are unchanged
- **WHEN** a client requests an ordinary route, or upgrades to the path of a `ws`-style gateway, on an application
  with an Engine.IO server attached
- **THEN** the response is the same as without it (`tests/websockets.rs` and `tests/ws_security.rs` pass unmodified)

### Requirement: Handshake matches Node engine.io
A request without `sid`, with `EIO=4` and a known transport, SHALL open a session and answer an open packet: `0`
followed by JSON with `sid`, `upgrades`, `pingInterval`, `pingTimeout` and `maxPayload`. Defaults MUST equal Node:
`pingInterval` 25000, `pingTimeout` 20000, `maxPayload` equal to `maxHttpBufferSize` 1000000, `upgrades`
`["websocket"]` on polling and `[]` on a WebSocket-first connection.

#### Scenario: Polling handshake
- **WHEN** the same raw handshake is sent to the Node reference server and to Corvis
- **THEN** both answer `200` with equal status, `content-type`, `cache-control` and open packet after `sid`
  normalization

#### Scenario: WebSocket-first handshake
- **WHEN** a client opens `ws://.../socket.io/?EIO=4&transport=websocket` without a polling session
- **THEN** the first frame is the open packet with `upgrades` equal to `[]`, on both servers

### Requirement: The handshake request follows the application's request settings
A session's handshake request SHALL be built with the application's query parser and `trust proxy` setting, as
gateway handshakes are, and its client address SHALL be resolved once per session.

#### Scenario: Address behind a trusted proxy
- **WHEN** the application calls `set_trust_proxy` for `127.0.0.1` and a handshake arrives from `127.0.0.1` with
  `X-Forwarded-For: 203.0.113.7`
- **THEN** the session's handshake `ip()` is `203.0.113.7`

### Requirement: Session ids are unguessable and checked before lookup
Session ids SHALL be 15 bytes from the operating system's CSPRNG encoded as unpadded base64url (20 characters, Node's
length and alphabet, 120 random bits). A `sid` that is not exactly 20 base64url characters MUST be answered as an
unknown session without a lookup. Session ids MUST NOT appear in logs.

#### Scenario: Ids differ
- **WHEN** 100,000 session ids are generated in a unit test
- **THEN** all are distinct, 20 characters long and within `[A-Za-z0-9_-]`

#### Scenario: Oversized sid
- **WHEN** a client polls with a `sid` of 1 MiB
- **THEN** the server answers `400` with `{"code":1,"message":"Session ID unknown"}`, as Node does for any unknown id

### Requirement: Polling errors match Node
Invalid polling requests SHALL be answered as Node `engine.io`: status 400 (403 for code 4), `Content-Type:
application/json`, body `{"code":<n>,"message":<text>}` with 0 "Transport unknown", 1 "Session ID unknown", 2 "Bad
handshake method", 3 "Bad request", 4 "Forbidden", 5 "Unsupported protocol version". A missing or non-`4` `EIO` MUST
be answered with code 5, and a handshake with a method other than `GET` with code 2.

#### Scenario: Error codes
- **WHEN** the harness sends `EIO=3`, `transport=flashsocket`, an unknown `sid` and a `POST` handshake to both servers
- **THEN** status, `content-type` and body are equal on both servers for each request

### Requirement: Query edge cases match Node
A request whose `transport` differs from its session's transport, outside an upgrade, SHALL be answered with code 3. A
query parameter given more than once MUST be treated as an invalid value, as Node's query parsing makes it.

#### Scenario: Duplicate and mismatched parameters
- **WHEN** the harness sends `EIO=4&EIO=4` and a polling request with `transport=websocket` on a polling session
- **THEN** status and body are equal on both servers

### Requirement: Rejected upgrades match Node
A WebSocket upgrade rejected by the Engine.IO checks SHALL be answered with the status line Node's `abortUpgrade`
sends, and the connection MUST be closed without creating a session.

#### Scenario: Rejected upgrade
- **WHEN** a WebSocket upgrade carries an unknown `sid`
- **THEN** both servers answer the same status line and close the connection

### Requirement: Polling framing and answers match Node
Polling payloads SHALL join packets with the record separator `\x1e` and encode binary packets as `b` followed by
base64. `GET` responses MUST carry `cache-control: no-store` and Node's content type. A valid `POST` MUST be answered
`200`, `Content-Type: text/html`, body `ok`. A pending `GET` is answered when packets are queued, at the latest by the
next ping.

#### Scenario: Several packets in one poll
- **WHEN** the server queues several messages, one of them binary, while no poll is pending, and the client then polls
- **THEN** the response carries all of them joined by `\x1e`, byte-equal on both servers

### Requirement: One pending request of each kind per session
A second `GET` while one is pending, or a second `POST` while one is being read, SHALL be answered `400` and MUST close
the session with reason `transport error`.

#### Scenario: Overlapping polls
- **WHEN** a client sends a second `GET` while the first one is pending
- **THEN** both servers answer the second with `400` and the session closes with reason `transport error`

### Requirement: POST bodies are bounded
A `POST` body above `maxHttpBufferSize` SHALL be answered `413` without reading the rest of it, leaving the session open
as Node does. A `POST` with `Content-Type: application/octet-stream` MUST be answered `400`. A body MUST be read within
the server's `request_timeout`.

#### Scenario: Oversized POST
- **WHEN** a client posts `maxHttpBufferSize + 1` bytes with a matching `Content-Length`
- **THEN** both servers answer `413` and a following poll on the same session is served

### Requirement: Polling batches are bounded
A `POST` body SHALL carry at most `max_payload_packets` packets (default 10,000); a body with more MUST be rejected
before any of its packets reaches the application, closing the session with reason `parse error`. A `GET` response
SHALL carry at most `maxHttpBufferSize` bytes of packets (at least one packet); the rest stay queued, in order.

#### Scenario: Packet flood in one POST
- **WHEN** a client posts 10,001 one-byte message packets (well under `maxHttpBufferSize`)
- **THEN** the application receives none of them and the session closes with reason `parse error`; the harness lists
  this as an expected difference from Node

#### Scenario: Large backlog
- **WHEN** the server queues 3 MB of messages for a session and the client keeps polling
- **THEN** no response is larger than `maxHttpBufferSize` plus one packet, and the client receives every message once,
  in order

### Requirement: CORS of the polling transport is off unless configured
Without a `cors` option, polling responses SHALL carry no `Access-Control-*` header and an `OPTIONS` handshake MUST be
answered as Node (code 2). With `cors` set to a `CorsOptions`, preflights and polling responses SHALL carry the headers
the router's CORS gives for those options, and the reflected-origin-with-credentials warning MUST be logged. The
application's `enable_cors` MUST NOT apply to the Engine.IO path.

#### Scenario: No CORS by default
- **WHEN** a preflight `OPTIONS /socket.io/?EIO=4&transport=polling` with `Origin: https://evil.example` reaches both
  servers with default options
- **THEN** both answer `400` with code 2 and no `Access-Control-Allow-Origin` header

#### Scenario: Configured origin
- **WHEN** both servers allow `https://app.example` and a handshake comes from that origin
- **THEN** both answers carry `Access-Control-Allow-Origin: https://app.example`, equal on both servers

### Requirement: Origin allowlist on both transports
When `allowed_origins` is set, a polling or WebSocket handshake whose `Origin` is not in the list (compared ignoring
ASCII case) SHALL be answered `403` with code 4 and no session MUST be created; requests without `Origin` are accepted.

#### Scenario: Origin allowlist on WebSocket
- **WHEN** `allowed_origins` is `["https://app.example"]` and a WebSocket-first handshake has `Origin:
  https://evil.example`
- **THEN** the upgrade is refused with `403` and no session is created

### Requirement: Upgrade to WebSocket matches Node
A polling session SHALL be upgradable with the probe sequence: the client sends `2probe` on the new WebSocket, the
server answers `3probe`, sends a noop `6` on polling every 100 ms while the upgrade is pending, and on `5` switches the
session to the WebSocket without losing, duplicating or reordering queued packets.

#### Scenario: Upgrade with queued packets
- **WHEN** the server emits 100 messages while a client upgrades
- **THEN** the client receives each exactly once and in emission order, on both servers

### Requirement: Upgrade attempts are bounded
A candidate WebSocket that does not send `5` within `upgradeTimeout` (default 10000 ms), or that sends any packet other
than `2probe` or `5`, SHALL be closed while the session keeps working over polling. At most one upgrade per session may
be in progress, and an upgraded session MUST refuse further upgrades, by closing the new WebSocket as Node does.

#### Scenario: Upgrade never completed
- **WHEN** a client sends `2probe` but never `5`, with `upgrade_timeout` 300 ms
- **THEN** the candidate WebSocket is not closed before 300 ms, is closed within 5 s, and a later poll still works

#### Scenario: Second upgrade
- **WHEN** a client opens a second WebSocket with the same `sid` while the first upgrade is pending
- **THEN** the second WebSocket is closed and the first upgrade can still complete

### Requirement: WebSocket messages are bounded
Engine.IO WebSocket messages and frames above `maxHttpBufferSize` SHALL close the connection with `1009` and end the
session with reason `transport error`.

#### Scenario: Oversized WebSocket message
- **WHEN** a client sends a 1,000,001-byte message on an upgraded session
- **THEN** the connection is closed with `1009` and the session ends with reason `transport error`

### Requirement: The WebSocket transport keeps the server's protections
Engine.IO WebSocket connections SHALL be dropped by the server's `send_timeout` when their socket accepts nothing,
pause reads while a write is blocked and the read credit is spent, and wait at most `close_timeout` (default 30 s) for
a closing handshake. A panic in application code reached from a session MUST be logged and close only that session.

#### Scenario: Peer that never reads
- **WHEN** a WebSocket session's client never reads while the server sends 3 MiB, with `send_timeout` of 1 s
- **THEN** the connection is closed within 10 s and the session leaves the server's session count

### Requirement: Heartbeat matches Node
The server SHALL send a ping packet `2` every `pingInterval` after the last pong, and MUST close the session with
reason `ping timeout` when no pong `3` arrives within `pingTimeout` after a ping. A ping sent by the client (other than
the upgrade probe) MUST close the session with reason `transport error`, as Node's "invalid heartbeat direction". A pong
that arrives while no ping is pending MUST NOT change the schedule (Node lets it postpone the next ping, so a client
that only posts `3` could keep a session alive; this is a deliberate divergence, not observable without timing).

#### Scenario: Client stops answering pings
- **WHEN** a raw client stops answering pings, with `ping_interval` 300 ms and `ping_timeout` 200 ms on both servers
- **THEN** on both servers the session is not closed before 500 ms after the last pong and is closed within 10 s with
  reason `ping timeout`

#### Scenario: Timer logic
- **WHEN** the heartbeat state machine runs under paused Tokio time with the default intervals
- **THEN** the first ping is sent at 25 s, a pong at 30 s schedules the next ping at 55 s, and no pong after the ping at
  55 s closes the session at 75 s

### Requirement: Received data waits within a bound
Messages received and not yet taken by the application SHALL be bounded per session by `max_unread_bytes` (default
16 MiB). Above it, the WebSocket transport MUST stop reading from the socket, and a polling `POST` MUST NOT be
answered until the backlog falls below the bound (the session's heartbeat still applies).

Every received message, whatever its length (empty ones included, text or binary), MUST be charged its length plus a
fixed per-message overhead (64 bytes) against that bound, since a queued message costs memory that its payload does not
measure. Messages received and not yet taken SHALL also be bounded across every session of an Engine.IO server by
`max_total_unread_bytes` (default 256 MiB, at least `max_unread_bytes`): while the total is exceeded, every WebSocket
MUST stop being read and every polling `POST` MUST be held, exactly as for the per-session bound. A release of either
bound MUST wake every waiter (a held `POST` and the WebSocket reader of the same session can both wait).

#### Scenario: Application that does not read
- **WHEN** a client sends 64 MiB of messages over WebSocket to a session whose messages are never taken
- **THEN** the server holds at most `max_unread_bytes` plus one message of that session

#### Scenario: Empty messages are charged
- **WHEN** a client sends thousands of empty messages (`4` packets in polling `POST`s, empty text and one-byte binary
  WebSocket messages) to a session whose messages are never taken
- **THEN** the `POST` is held and the WebSocket reader paused once the charged bytes pass `max_unread_bytes`, instead of
  the queue growing without bound

While the total is exceeded, only sessions that hold more than a fair share SHALL be paused or held, with
`fair = max(16 KiB, max_total_unread_bytes / max_sessions)` computed once from the options; a session at or below the
fair share, and under its own limit, MUST keep reading and MUST NOT be paused by what other sessions hold. The worst case
is bounded by `max_total_unread_bytes + max_sessions * fair` (plus one message per session), and the documentation MUST
state it.

#### Scenario: Total unread budget
- **WHEN** `max_total_unread_bytes` is 40,000 and `max_unread_bytes` 30,000, and two sessions each receive a 20,000-byte
  message that the application does not read
- **THEN** the second `POST` is held although its own session is under its limit (it is above the fair share while the
  total is exceeded), and is answered once the application reads the first session's message

#### Scenario: A hoarder does not starve a drained session
- **WHEN** hoarding sessions fill the whole `max_total_unread_bytes` and another session, which the application drains,
  answers its pings and sends messages
- **THEN** that session is never paused, stays open for several ping cycles, and every message it sends reaches the
  application

### Requirement: Send buffers are bounded
Packets queued for a session and not yet written SHALL be bounded by `max_buffered_amount` (default 8 MiB, the
default of `corvis::ws`). A send that would exceed it MUST fail, discard the queued packets and close the session with
reason `transport error`.

#### Scenario: Slow consumer
- **WHEN** a polling client stops polling while the application sends 9 MiB
- **THEN** the send that crosses 8 MiB fails, the session closes with reason `transport error`, and the harness lists
  this as an expected difference from Node

### Requirement: Open sessions are bounded
An Engine.IO server SHALL hold at most `max_sessions` open sessions (default 10,000). A handshake over the limit, on
either transport, MUST be answered `503` with an Engine.IO JSON error body before any session state is created or any
upgrade happens, and a warning MUST be logged at most once per minute.

#### Scenario: Session cap
- **WHEN** `max_sessions` is 2, two sessions are open and a third handshake arrives
- **THEN** the third is answered `503`; after one session closes, a new handshake succeeds

A server SHALL also hold at most `max_sessions_per_ip` open sessions per client address (default 256, zero refused),
keyed by the request's `ip()` so that `set_trust_proxy` is honored and IPv6 addresses counted per `/64`. A handshake
over it, on either transport, MUST be answered like the global cap (`503`, the same body). The per-address count, the
global count and the closed flag of the server MUST be checked and updated under one lock, and a session MUST release
its place when it ends. The limit MUST be configurable, and the documentation MUST state the NAT trade-off, that it does
not stop an attacker with many addresses, and that behind `TrustProxy::All` the client chooses the key. The address MUST
be normalized before it is used as a key (brackets, port and zone removed, IPv4-mapped IPv6 as IPv4); a value that is
no address MUST fall back to the socket's peer address.

#### Scenario: Sessions per address
- **WHEN** `max_sessions_per_ip` is 3 and one address holds three sessions (any transport) and handshakes again
- **THEN** the fourth is answered `503` with `{"code":3,"message":"Too many sessions"}`; after one of the three closes a
  new handshake from the address succeeds, and with a trusted proxy two forwarded clients are counted separately

A polling session that makes no request within `first_request_timeout` after its handshake MUST be closed and its
place released. The timeout defaults to 10 s (or `ping_interval + ping_timeout` when shorter), MUST be above zero and at
most `ping_interval + ping_timeout`. Any request of the session (poll, `POST`, upgrade) stops the timer, and
WebSocket-first sessions are not subject to it.

#### Scenario: Silent polling session
- **WHEN** a client completes a polling handshake and sends nothing else
- **THEN** the session closes at `first_request_timeout`, not before it, and the harness lists this as an expected
  difference from Node; a session that polls, or that connects over WebSocket, is not closed by it

### Requirement: Handshakes can be refused by the application
An Engine.IO server SHALL accept an `allow_request` hook that receives the handshake request (with the application's
request settings) and runs, before any session is created, for polling handshakes and WebSocket-first connections.
When it rejects, panics, or takes longer than `allow_request_timeout` (default 5 s, validated like the other durations;
a warning at most once per minute for timeouts), the server MUST answer `403` with `{"code":4,"message":"Forbidden"}`
and create no session. It MUST run after the session caps are checked: a client over a cap is answered `503` without
the hook being invoked. Requests of existing sessions MUST NOT run it.

#### Scenario: Hook rejects
- **WHEN** the hook rejects a polling handshake and a WebSocket-first connection
- **THEN** both are answered `403` (code 4), no session exists, and a request the hook allows opens a session

### Requirement: Transport confusion is refused
Without an upgrade, only the polling transport of a session that still uses it SHALL be served. A request with
`transport=websocket` that is not a WebSocket upgrade, and any polling request for a session that moved to WebSocket,
MUST be answered `400` with code 3 (`Bad request`) and deliver nothing to the session.

#### Scenario: Polling on a WebSocket session
- **WHEN** a client sends a polling `GET` and a `POST` with a message, with `transport=websocket` and with
  `transport=polling`, for a session that is on WebSocket
- **THEN** every one is answered `400` with code 3 and the message is never delivered

### Requirement: Close and shutdown
A close packet `1` from the client SHALL end the session with reason `transport close`. Closing the Engine.IO server
MUST close every session with reason `forced close`, answer each pending poll with a close packet and refuse new
handshakes; it MUST happen when the HTTP server starts its graceful shutdown and on `CorvisApplication::close`.

#### Scenario: Graceful shutdown with a pending poll
- **WHEN** the application shuts down while a client has a pending poll
- **THEN** the poll is answered with a close packet within 5 s, and the server's shutdown does not wait for its
  10-second drain limit because of it

#### Scenario: Client close
- **WHEN** a client posts `1`
- **THEN** the session ends with reason `transport close` on both servers

### Requirement: Options are validated
Attaching an Engine.IO server SHALL fail, with an error naming the option, on a zero `ping_interval`, `ping_timeout`,
`upgrade_timeout`, `max_http_buffer_size`, `max_payload_packets`, `max_buffered_amount`, `max_unread_bytes` or
`max_sessions`, `max_sessions_per_ip`, `max_total_unread_bytes`, `first_request_timeout` or `allow_request_timeout`, on a path that does not
start with `/`, on any of `ping_interval`, `ping_timeout`, `upgrade_timeout`, `close_timeout`, `allow_request_timeout` above one day (the timers
add them to the clock, and the arithmetic MUST NOT panic whatever the value), on a `first_request_timeout` above
`ping_interval + ping_timeout`, and on a `max_total_unread_bytes` below `max_unread_bytes`.

#### Scenario: Zero ping timeout
- **WHEN** an Engine.IO server is attached with `ping_timeout(Duration::ZERO)`
- **THEN** attaching fails with an error naming `ping_timeout`

#### Scenario: Huge duration
- **WHEN** an Engine.IO server is attached with `ping_interval(Duration::MAX)`
- **THEN** attaching fails with an error naming `ping_interval`, and no timer arithmetic overflows

### Requirement: Security divergences are explicit
Every bound Corvis keeps where Node has none (packets per `POST`, bytes per `GET` response, send buffer, received
backlog, open sessions, session id entropy) SHALL be documented in `docs/websockets/adapter.md` (English and Spanish)
and the rustdoc of `EngineOptions`, and each one a client can observe MUST have an entry in the harness's
`expected-differences.json` with the scenario that exercises it.

#### Scenario: Divergences are listed
- **WHEN** the harness runs the Engine.IO scenarios against both servers
- **THEN** every difference reported is listed with its reason, and no listed difference is stale

### Requirement: Verified against Node and the upstream suite
The Engine.IO wire scenarios SHALL run against the Node reference server in Engine.IO mode and against Corvis with no
unlisted difference, and the upstream `engine.io-protocol` suite, at the commit pinned by #131, SHALL pass against a
Corvis server configured as the suite requires.

#### Scenario: Protocol suite
- **WHEN** `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test engine_io_conformance` runs
- **THEN** every test of the suite passes

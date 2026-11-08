# Spec Delta

## Purpose

Defines the Engine.IO v4 server that carries socket.io traffic: handshake, HTTP long-polling, upgrade to WebSocket,
heartbeat, packet framing, limits and error responses. Everything observable by a client MUST match Node `engine.io`
6.6.x (as bundled with `socket.io` 4.8.x), verified by the differential harness of `websockets/socket-io-parity`,
except the security divergences listed in the last requirement. Implemented by
[#32](https://github.com/randygil/corvis/issues/32).

## ADDED Requirements

### Requirement: Engine.IO is served under its own path
The Engine.IO server SHALL handle every request whose path is the configured Engine.IO path (default `/socket.io/`,
with or without the trailing slash) before the application router, both plain HTTP requests (long-polling `GET`,
`POST` and CORS preflight `OPTIONS`) and WebSocket upgrades. Requests to other paths MUST reach the router and the
existing `ws`-style gateways exactly as before.

#### Scenario: Polling request is not routed
- **WHEN** a client sends `GET /socket.io/?EIO=4&transport=polling` to an application that has no route for that path
- **THEN** the Engine.IO server answers with an open packet, not with the router's 404

#### Scenario: Other paths are unchanged
- **WHEN** a client upgrades to the path of an existing `ws`-style gateway, or requests an ordinary route
- **THEN** the response is the same as before socket.io support existed

### Requirement: Handshake matches Node engine.io
A request without `sid` and with `EIO=4` SHALL open a session and answer with an open packet (`0` followed by JSON with
`sid`, `upgrades`, `pingInterval`, `pingTimeout` and `maxPayload`). Defaults MUST equal Node: `pingInterval` 25000,
`pingTimeout` 20000, `maxPayload` equal to `maxHttpBufferSize` 1000000, and `upgrades` `["websocket"]` on polling and
`[]` on a WebSocket-first connection. The `sid` MUST be unguessable (at least as much entropy as Node's `base64id`).

#### Scenario: Polling handshake
- **WHEN** the official client and a raw request open a polling session against both servers
- **THEN** both servers answer `200` with an open packet whose fields and values are equal after `sid` normalization

#### Scenario: WebSocket-first handshake
- **WHEN** a client opens `ws://.../socket.io/?EIO=4&transport=websocket` without a prior polling session
- **THEN** the first frame is the open packet with `upgrades` equal to `[]`, as in Node

### Requirement: Engine.IO errors match Node
Invalid requests SHALL be answered as Node `engine.io` does: status 400 (403 for code 4), `Content-Type:
application/json`, body `{"code":<n>,"message":<text>}` with code 0 "Transport unknown", 1 "Session ID unknown", 2 "Bad
handshake method", 3 "Bad request", 4 "Forbidden" and 5 "Unsupported protocol version". A missing or non-`4` `EIO`
MUST be answered with code 5 (Engine.IO v3 is not supported).

#### Scenario: Unknown session
- **WHEN** a client polls with a `sid` that does not exist
- **THEN** both servers answer `400` with body `{"code":1,"message":"Session ID unknown"}`

#### Scenario: Old protocol
- **WHEN** a client requests a handshake with `EIO=3`
- **THEN** both servers answer `400` with body `{"code":5,"message":"Unsupported protocol version"}`

#### Scenario: Unknown transport
- **WHEN** a client requests `transport=flashsocket`
- **THEN** both servers answer `400` with body `{"code":0,"message":"Transport unknown"}`

### Requirement: Long-polling transport matches Node
Polling payloads SHALL join packets with the record separator `\x1e`, encode binary packets as `b` followed by
base64, answer a valid `POST` with `200`, `Content-Type: text/html` and body `ok`, and send `GET` responses that are not
cached. A second concurrent `GET` (or `POST`) on the same session MUST be answered with `400` and close the session; a
`POST` body larger than `maxHttpBufferSize` MUST be answered with `413`.

#### Scenario: Several packets in one poll
- **WHEN** the server emits several packets while no poll is pending and the client then polls
- **THEN** the response carries all of them joined by `\x1e`, byte-equal to Node after normalization

#### Scenario: Overlapping polls
- **WHEN** a client sends a second `GET` while the first one is still pending
- **THEN** both servers answer `400` and close the session with reason `transport error`

#### Scenario: Oversized POST
- **WHEN** a client posts a body of `maxHttpBufferSize + 1` bytes
- **THEN** both servers answer `413`

### Requirement: Upgrade to WebSocket matches Node
A polling session SHALL be upgradable to WebSocket with the probe sequence: the client sends `2probe`, the server
answers `3probe`, the pending poll is answered with a noop packet `6`, the client sends `5` and the server switches the
session to the WebSocket transport without losing or reordering buffered packets. A client that does not complete the
upgrade within `upgradeTimeout` (default 10000 ms) MUST have the WebSocket closed while the polling session survives.

#### Scenario: Upgrade with buffered packets
- **WHEN** the server emits events during the upgrade of a client
- **THEN** the client receives every event exactly once and in emission order, as with Node

#### Scenario: Upgrade never completed
- **WHEN** a client sends `2probe` but never sends `5`
- **THEN** after `upgradeTimeout` the WebSocket is closed and the session keeps working over polling

### Requirement: Heartbeat matches Node
The server SHALL send a ping packet `2` every `pingInterval` and MUST close the session with reason `ping timeout` when
no pong packet `3` arrives within `pingTimeout` after a ping.

#### Scenario: Client stops answering pings
- **WHEN** a raw client stops answering pings, with `pingInterval` 300 and `pingTimeout` 200 on both servers
- **THEN** both servers close the session with reason `ping timeout` within the same time window (tolerance set by the
  harness)

### Requirement: Close and shutdown match Node
A close packet `1` from the client SHALL end the session with reason `transport close`. Shutting the application down
MUST close every session with reason `server shutting down` and answer pending polls instead of leaving them open.

#### Scenario: Graceful shutdown with a pending poll
- **WHEN** the application is shut down while a client has a pending poll
- **THEN** the poll is answered with a close packet and the session ends, as with Node `io.close()`

### Requirement: Documented security divergences
Where Node keeps unbounded state, the Corvis server SHALL keep bounds and MUST document each divergence: a byte-bounded
send buffer per session (default 8 MiB, overflow closes the session with reason `transport error`), a bounded number of
handlers in flight per session with read backpressure (default 32), and an optional cap on concurrent sessions
(default off). The differential harness MUST list these as expected differences and exercise each one.

#### Scenario: Slow consumer
- **WHEN** a client stops reading while the server emits more than the send buffer limit
- **THEN** the session is closed with reason `transport error` and the server's memory stays bounded; the harness marks
  this scenario as an expected difference from Node

# ws/write-deadline Specification

## Purpose
How the built-in server detects a WebSocket peer that stops accepting the frames sent to it, and closes its
connection, so that such a peer cannot keep its connection, its `max_connections` slot and its queued frames
forever, while idle connections and slow but live peers keep working.

## Requirements

### Requirement: Stalled WebSocket writes end the connection
On an upgraded connection (WebSocket gateways, `WebSocketServer::handle_upgrade`, any `upgrade` listener), when a
write to the socket stays pending for `ServerOptions::send_timeout` without the socket accepting a byte, the server
SHALL fail the write and close the TCP connection without writing anything else to it (no close frame). It MUST NOT
depend on the connection reading.

#### Scenario: Gateway client that never reads
- **WHEN** a server with `ServerOptions::new().send_timeout(Duration::from_secs(1))` accepts a gateway connection
  from a client with a small receive buffer that never reads, and the gateway emits 3 MiB (less than
  `max_buffered_amount`)
- **THEN** the TCP connection is closed within a few seconds

#### Scenario: The slot is released
- **WHEN** a server has `max_connections(1)` and `send_timeout(Duration::from_secs(1))`, and a WebSocket client
  that never reads holds the only slot while the gateway sends to it
- **THEN** a second client is served within a few seconds

### Requirement: The WebSocket write deadline follows send_timeout
The WebSocket write deadline SHALL use the server's `send_timeout`, and `send_timeout(None)` MUST turn it off for
upgraded connections too.

#### Scenario: Disabled
- **WHEN** a server has `send_timeout(None)` and a WebSocket client stops reading for three seconds while the gateway
  sends less than `max_buffered_amount`
- **THEN** the connection stays open and the client then receives every message

### Requirement: Idle WebSocket connections are not affected
The deadline SHALL run only while a write is pending: a connection on which the server has nothing to send MUST NOT
be closed by it, however long it stays quiet.

#### Scenario: Idle connection
- **WHEN** a gateway connection on a server with `send_timeout(Duration::from_secs(1))` sends and receives nothing for
  five seconds
- **THEN** the connection stays open

### Requirement: Only bytes the peer accepts count as progress
The deadline SHALL restart whenever the socket accepts at least one byte of any frame the server writes (data,
ping, pong or close). Frames received from the peer, including pings and pongs, MUST NOT restart it.

#### Scenario: Peer that pings but never reads
- **WHEN** a WebSocket client on a server with `send_timeout(Duration::from_secs(1))` sends a ping every 200 ms but
  never reads, while the gateway sends 3 MiB to it
- **THEN** the connection is closed within a few seconds

#### Scenario: Slow but live reader
- **WHEN** a WebSocket client reads what its socket holds every 400 ms while the gateway sends 3 MiB to it, on a
  server with `send_timeout(Duration::from_secs(1))`
- **THEN** the client receives every message and the connection stays open

### Requirement: Gateways see the disconnection
When the write deadline closes a gateway connection, the gateway SHALL run `handle_disconnect` and remove the client
from `server.clients()` at once, even if a closing handshake had started.

#### Scenario: Gateway client dropped
- **WHEN** the write deadline closes a gateway connection
- **THEN** `handle_disconnect` runs once and `server.clients()` no longer contains the client

#### Scenario: Deadline during a closing handshake
- **WHEN** a gateway calls `client.disconnect()` on a client that does not read, with `send_timeout` shorter than
  `close_timeout`
- **THEN** the connection ends after `send_timeout`, not after `close_timeout`

### Requirement: Plain WebSockets report an abnormal closure
When the write deadline closes a connection of a `WebSocketServer`, its `WebSocket` SHALL become `CLOSED` at once and
report close code `1006` with an empty reason to `on_close` and `closed()`, and its pending `send_and_wait` calls SHALL
fail.

#### Scenario: Plain WebSocket that never reads
- **WHEN** an `upgrade` listener hands a connection to a `WebSocketServer` on a server with
  `send_timeout(Duration::from_secs(1))`, and the application sends 3 MiB and then a `send_and_wait` to a peer that
  never reads
- **THEN** the connection is closed within a few seconds, `on_close` reports `1006`, and the `send_and_wait` fails

### Requirement: Dropped WebSockets are logged once
The server SHALL log one warning per connection closed by the WebSocket write deadline, naming the client and
`send_timeout`.

#### Scenario: Warning
- **WHEN** the write deadline closes a gateway connection
- **THEN** one warning naming the client id and `send_timeout` is logged

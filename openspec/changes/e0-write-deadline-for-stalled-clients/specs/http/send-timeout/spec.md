# Spec Delta

## Purpose

How long the built-in server waits for a client to accept response bytes, so that a client that stops reading cannot
keep a connection, its `max_connections` slot and the memory of its responses forever, over HTTP/1 or HTTP/2, while
slow but live clients and long-lived responses keep working.

## ADDED Requirements

### Requirement: Send timeout option
`ServerOptions::send_timeout(t)` SHALL set the longest time the server waits for a client to accept response bytes
on a connection. The default MUST be 60 seconds, and `None` MUST disable it. The time SHALL be measured from the last
moment the client made progress (accepted at least one byte on the socket, or let a stalled response stream move),
not from the start of the response.

#### Scenario: Default value
- **WHEN** an application builds `ServerOptions::new()` without calling `send_timeout`
- **THEN** a client that accepts no response byte for 60 seconds is disconnected, and one that does so for less is not

#### Scenario: Disabled
- **WHEN** a server has `ServerOptions::new().send_timeout(None)` and a client stops reading a large response
- **THEN** the server does not close the connection because of it

### Requirement: Blocked socket writes time out
When writing to a connection's socket stays blocked for the send timeout without the socket accepting a byte, the
server SHALL end the connection and close the socket without the closing drain and without writing anything else to
it (no HTTP/2 GOAWAY, no rest of the response). This SHALL apply to HTTP/1 and HTTP/2 connections, including while the
server is shutting down gracefully, and MUST NOT depend on the connection reading.

#### Scenario: HTTP/1 pipelined requests never read
- **WHEN** a server has `ServerOptions::new().max_connections(1).send_timeout(Duration::from_secs(1))` and a client
  pipelines GET requests until its writes block, never reading
- **THEN** that connection is closed within a few seconds and a second client is then served

#### Scenario: HTTP/2 PING flood never read
- **WHEN** a server has `ServerOptions::new().h2c(true).max_connections(1).send_timeout(Duration::from_secs(1))` and
  a client sends PING frames until its writes block, never reading the PONGs
- **THEN** that connection is closed within a few seconds and a second client is then served

#### Scenario: Large response to a client that stops reading
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` sends an 8 MiB response and the client reads the
  first bytes and then stops reading
- **THEN** the connection is closed within a few seconds

### Requirement: Stalled response streams close the connection
A response is stalled while its body waits for the connection to take its next part because the client has not made
room for it. When a response has been stalled for the send timeout, the server SHALL close its connection without the
closing drain, even if other frames (PING, SETTINGS, WINDOW_UPDATE) are still exchanged on it.

#### Scenario: HTTP/2 stream with a zero window
- **WHEN** a server has `ServerOptions::new().h2c(true).max_connections(1).send_timeout(Duration::from_secs(1))`
  with the PING keep-alive on, and a client sends `SETTINGS_INITIAL_WINDOW_SIZE=0`, requests a 1 MiB route and answers
  every PING
- **THEN** the connection is closed within a few seconds and a second client is then served

#### Scenario: HTTP/2 stream with a window of one byte
- **WHEN** a server with `h2c(true)` and `send_timeout(Duration::from_secs(1))` receives, on a connection with
  `SETTINGS_INITIAL_WINDOW_SIZE=1`, a request for a 1 MiB route and the client never sends `WINDOW_UPDATE`
- **THEN** the connection is closed within a few seconds

### Requirement: Responses waiting for the application are not stalled
A response whose body is waiting for the application to produce data (an idle Server-Sent Events stream, a slow
stream source) MUST NOT be considered stalled, however long it waits.

#### Scenario: Idle Server-Sent Events stream
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` serves an SSE route that sends one event and then
  nothing for three seconds, over HTTP/1 and over h2c, to a client that keeps reading
- **THEN** the connection stays open and the next event is delivered

### Requirement: Slow but live clients are not cut off
A client that keeps accepting response bytes SHALL keep its connection however long the whole response takes, and a
request whose handler takes longer than the send timeout before answering MUST NOT be affected by it.

#### Scenario: Slow download
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` sends a 4 MiB response and the client reads it in
  64 KiB pieces every 300 ms (about 6 seconds in total), over HTTP/1 and over h2c
- **THEN** the client receives the whole body

#### Scenario: Slow handler
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` runs a handler that sleeps three seconds and then
  answers
- **THEN** the client receives the response

### Requirement: Upgraded connections are not affected
The send timeout MUST NOT apply to a connection after it has been upgraded (WebSocket gateways, `upgrade` listeners).

#### Scenario: WebSocket client that stops reading
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` accepts a WebSocket connection and the client stops
  reading for three seconds while the gateway sends less than `max_buffered_amount`
- **THEN** the connection stays open and the client then receives the messages

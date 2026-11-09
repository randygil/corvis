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

### Requirement: Stalled connections are closed
A response is stalled while its body waits for the connection to take its next part because the client has not made
room for it. A connection is stalled when at least one of its response bodies is stalled and no response body of the
connection has had a part taken for the send timeout. A stalled connection SHALL be closed without the closing drain,
even if other frames (PING, SETTINGS, WINDOW_UPDATE) are still exchanged on it. A stalled response next to responses
that keep moving MUST NOT close the connection.

#### Scenario: HTTP/2 stream with a zero window
- **WHEN** a server has `ServerOptions::new().h2c(true).max_connections(1).send_timeout(Duration::from_secs(1))`
  with the PING keep-alive on, and a client sends `SETTINGS_INITIAL_WINDOW_SIZE=0`, requests a 1 MiB route and answers
  every PING
- **THEN** the connection is closed within a few seconds and a second client is then served

#### Scenario: HTTP/2 stream with a window of one byte
- **WHEN** a server with `h2c(true)` and `send_timeout(Duration::from_secs(1))` receives, on a connection with
  `SETTINGS_INITIAL_WINDOW_SIZE=1`, a request for a 1 MiB route and the client never sends `WINDOW_UPDATE`
- **THEN** the connection is closed within a few seconds

#### Scenario: One stalled stream next to a stream that moves
- **WHEN** a server with `h2c(true)` and `send_timeout(Duration::from_secs(1))` has one response that the client does
  not read and another that the client keeps reading for four seconds
- **THEN** the connection stays open and the second response keeps arriving

#### Scenario: Every stream stalled
- **WHEN** the client of the previous scenario then stops reading the second response too
- **THEN** the connection is closed within a few send timeouts

### Requirement: Frames that are not responses do not keep an idle connection open
On an idle HTTP/2 connection (no request in flight), only a request frame from the client (HEADERS) and a response
frame from the server (DATA, HEADERS, CONTINUATION) SHALL move the connection out of, or restart, the keep-alive
phase. PING, SETTINGS and WINDOW_UPDATE frames, their acknowledgements and the server's keep-alive PINGs MUST NOT.

#### Scenario: Client PINGs on an idle connection
- **WHEN** a server with `h2c(true)` and `keep_alive_timeout(Duration::from_secs(1))` has an idle connection whose client
  sends a PING every half second
- **THEN** the connection is closed after about the keep-alive timeout

#### Scenario: Client SETTINGS on an idle connection
- **WHEN** the client of that connection sends SETTINGS frames (answered with acknowledgements) instead of PINGs
- **THEN** the connection is closed after about the keep-alive timeout

#### Scenario: Server PING keep-alive on an idle connection
- **WHEN** the same server has `http2_keep_alive_interval` shorter than the keep-alive timeout and the client answers
  every PING
- **THEN** the idle connection is closed after about the keep-alive timeout

### Requirement: Timeouts too long for a clock are off
A `send_timeout`, `headers_timeout`, `keep_alive_timeout` or `request_timeout` of `Duration::MAX` (or any value that
cannot be added to the current time) SHALL behave as no limit and MUST NOT make a connection task panic.

#### Scenario: Maximum durations
- **WHEN** a server is configured with `Duration::MAX` for those timeouts
- **THEN** requests over HTTP/1 and HTTP/2 are answered normally

### Requirement: Responses waiting for the application are not stalled
A response whose body is waiting for the application to produce data (an idle Server-Sent Events stream, a slow
stream source) MUST NOT be considered stalled, however long it waits.

#### Scenario: Idle Server-Sent Events stream
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` serves an SSE route that sends one event and then
  nothing for three seconds, over HTTP/1 and over h2c, to a client that keeps reading
- **THEN** the connection stays open and the next event is delivered

### Requirement: Slow but live clients are not cut off
A client that keeps making progress SHALL keep its connection however long the whole response takes, and a
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

# Spec Delta

## Purpose

The limits and timeouts of HTTP/2 connections served by the built-in server, and how `ServerOptions` sets them, so
that an operator who bounds resources with `ServerOptions` gets the same bounds whichever protocol a client speaks.

## ADDED Requirements

### Requirement: HTTP/2 settings apply to every HTTP/2 connection
The server SHALL apply the HTTP/2 settings of `ServerOptions` (header list size, concurrent streams, PING
keep-alive) to every connection it serves over HTTP/2, however HTTP/2 was selected (prior-knowledge h2c today; ALPN
over TLS when the server supports it). Connections served over HTTP/1 MUST NOT be affected by them.

#### Scenario: HTTP/1 request with HTTP/2 options set
- **WHEN** a server with `ServerOptions::new().h2c(true).http2_max_concurrent_streams(1)` receives HTTP/1.1 requests
  on a keep-alive connection
- **THEN** they are answered as before

### Requirement: The header limit covers HTTP/2
`ServerOptions::max_header_size(bytes)` SHALL also set the HTTP/2 header list limit (`SETTINGS_MAX_HEADER_LIST_SIZE`,
counted as RFC 9113 does: name and value lengths plus 32 per field, pseudo-headers included). A request whose header
list exceeds it MUST be answered with status `431` on its stream, and other streams of the connection MUST keep
working. The default is the documented 32 KiB.

#### Scenario: Oversized header list over h2c
- **WHEN** a server has `ServerOptions::new().h2c(true).max_header_size(8 * 1024)` and an HTTP/2 client sends a
  request with a 12 KiB header value
- **THEN** that request is answered `431`, and a following request with small headers on the same connection is
  answered `200`

#### Scenario: Default allows 32 KiB
- **WHEN** a server has `ServerOptions::new().h2c(true)` and an HTTP/2 client sends a request with a 24 KiB header
  value
- **THEN** the request is answered `200`

### Requirement: Concurrent streams per connection are capped
`ServerOptions::http2_max_concurrent_streams(n)` SHALL set the number of streams a client may have open at once on
one HTTP/2 connection (`SETTINGS_MAX_CONCURRENT_STREAMS`), with a default of 100 and a minimum of 1. The server MUST
never run more handlers at once for one connection than this number.

#### Scenario: Cap of two
- **WHEN** a server has `ServerOptions::new().h2c(true).http2_max_concurrent_streams(2)` and an HTTP/2 client starts
  three requests at once to a handler that waits until released
- **THEN** at most two handlers run at the same time, and all three requests are answered once the handlers are
  released

#### Scenario: Default advertised
- **WHEN** an HTTP/2 client connects to a server with `ServerOptions::new().h2c(true)`
- **THEN** the server's SETTINGS frame advertises `SETTINGS_MAX_CONCURRENT_STREAMS` = 100

### Requirement: max_connections counts connections
`max_connections` SHALL count HTTP/2 connections like HTTP/1 connections, one permit per socket, and MUST NOT count
streams. The documentation SHALL state that the in-flight requests of HTTP/2 clients are bounded by
`max_connections × http2_max_concurrent_streams`.

#### Scenario: One connection, several streams
- **WHEN** a server has `ServerOptions::new().h2c(true).max_connections(1).http2_max_concurrent_streams(4)` and one
  HTTP/2 client starts four requests at once
- **THEN** the four requests are served concurrently on that connection, and a second client's connection waits in the
  accept queue until the first closes

### Requirement: HTTP/2 PING keep-alive
`ServerOptions::http2_keep_alive_interval(interval)` SHALL make the server send a PING frame on each HTTP/2 connection
every `interval` (default `None`: no PINGs), and `ServerOptions::http2_keep_alive_timeout(timeout)` (default 20 s)
SHALL close the connection when a PING is not acknowledged within `timeout`, whether or not streams are in flight.

#### Scenario: Peer that does not answer PINGs
- **WHEN** a server has `ServerOptions::new().h2c(true).http2_keep_alive_interval(Duration::from_millis(200))
  .http2_keep_alive_timeout(Duration::from_millis(300))` and idle timeouts disabled, and a raw client sends the HTTP/2
  preface and SETTINGS and then never acknowledges anything
- **THEN** the client receives a PING frame and the connection is closed within a few seconds

#### Scenario: Peer that answers PINGs
- **WHEN** the same server serves a hyper HTTP/2 client that acknowledges PINGs and keeps a streamed response open for
  two seconds
- **THEN** the connection stays open and the stream completes

#### Scenario: Off by default
- **WHEN** a server has `ServerOptions::new().h2c(true)` and a raw HTTP/2 client stays connected for one second
- **THEN** it receives no PING frame

### Requirement: Timeouts on HTTP/2 connections
For an HTTP/2 connection, `headers_timeout` and `keep_alive_timeout` SHALL apply to the connection while none of its
streams is in flight, as for an idle HTTP/1 connection, and `request_timeout` SHALL apply to the body of each stream
on its own: a stream whose body does not arrive in time MUST be answered `408` without affecting the other streams of
the connection.

#### Scenario: Slow body on one stream
- **WHEN** a server has `ServerOptions::new().h2c(true).request_timeout(Duration::from_secs(1))` and an HTTP/2 client
  opens a `POST` stream with a JSON body it never finishes, and sends a `GET` on another stream of the same connection
- **THEN** the `GET` is answered `200` and the `POST` is answered `408` after about one second

#### Scenario: Idle HTTP/2 connection
- **WHEN** a server has `ServerOptions::new().h2c(true).keep_alive_timeout(Duration::from_secs(1))` and an HTTP/2
  client completes one request and then sends nothing
- **THEN** the server closes the connection after about one second

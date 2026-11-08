# Spec Delta

## MODIFIED Requirements

### Requirement: The closing drain is bounded
Discarding input after an early response MUST stop after at most 1 MiB or 2 seconds per connection, and the
connection MUST keep counting against `max_connections` until the socket is closed. Connections the server closes
because of the headers timeout or the keep-alive timeout, an HTTP/2 preface refused because h2c is off (the default),
or a socket destroyed by an upgrade listener MUST be closed without waiting for the peer. The `408` sent after
`requestTimeout` is an early response and does need the drain.

#### Scenario: Endless sender
- **WHEN** a client keeps sending data after an early error response
- **THEN** the server closes the connection within a few seconds and the client's writes start failing

#### Scenario: Idle connection past the headers timeout
- **WHEN** a connection sends nothing for longer than the headers timeout
- **THEN** the server closes it at the deadline without a drain period

#### Scenario: Slow headers on a keep-alive connection
- **WHEN** a client sends the first bytes of a second request on a keep-alive connection and then drips one byte at a
  time past the headers timeout
- **THEN** the server closes the connection at the headers timeout and the connection permit is free right after it,
  without a drain period

#### Scenario: Destroyed upgrade socket
- **WHEN** an upgrade listener destroys the socket of an upgrade request and the client stays quiet
- **THEN** the server closes the connection at once and the connection permit is free without a drain period

#### Scenario: Refused HTTP/2 preface
- **WHEN** h2c is off and a client sends the HTTP/2 connection preface and then stays quiet
- **THEN** the server closes the connection at once, without a response and without a drain period

## ADDED Requirements

### Requirement: HTTP/2 without TLS is opt-in
`ServerOptions::default()` SHALL serve HTTP/1.x only, like Node's `http` module. A connection that starts with the
HTTP/2 prior-knowledge preface (h2c) SHALL be closed without a response unless `ServerOptions::h2c(true)` was set.
With `h2c(true)` the server SHALL accept HTTP/2 over cleartext by prior knowledge and answer its requests.

#### Scenario: Default refuses prior knowledge
- **WHEN** a client connects to a server with default options and sends the HTTP/2 preface and a SETTINGS frame
- **THEN** the server closes the connection and sends no bytes, and an HTTP/1.1 request on another connection is
  still answered

#### Scenario: Opt-in accepts prior knowledge
- **WHEN** h2c is enabled with `ServerOptions::new().h2c(true)` and an HTTP/2 prior-knowledge client sends a `GET`
- **THEN** the response has version HTTP/2 and status `200`

#### Scenario: Explicitly disabled
- **WHEN** h2c is set to `false` explicitly
- **THEN** the behavior is the same as with the default options

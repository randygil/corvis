# Spec Delta

## Purpose

Guarantees that a response the server sends before it has read the whole request reaches a client that reads it,
instead of being lost to a TCP reset, and that the work done to guarantee it is bounded.

## ADDED Requirements

### Requirement: Early error responses are not lost to a reset
When the server answers a request before reading all of its input (oversized head, malformed body, a handler that
rejects without reading the body) and then closes the connection, it SHALL send the response, shut down its write half
and keep reading and discarding the peer's input until the peer closes or the drain bounds are reached, before
releasing the socket.

#### Scenario: Oversized head followed by more input
- **WHEN** a client sends a request head larger than the head limit and keeps sending bytes after it
- **THEN** the client reads a `431` response head, and does not read a connection reset before it

### Requirement: The closing drain is bounded
Discarding input after an early response MUST stop after at most 1 MiB or 2 seconds per connection, and the
connection MUST keep counting against `max_connections` until the socket is closed. Connections the server closes
because of a timeout or a refused HTTP/2 preface MUST be closed without waiting for the peer.

#### Scenario: Endless sender
- **WHEN** a client keeps sending data after an early error response
- **THEN** the server closes the connection within a few seconds and the client's writes start failing

#### Scenario: Idle connection past the headers timeout
- **WHEN** a connection sends nothing for longer than the headers timeout
- **THEN** the server closes it at the deadline without a drain period

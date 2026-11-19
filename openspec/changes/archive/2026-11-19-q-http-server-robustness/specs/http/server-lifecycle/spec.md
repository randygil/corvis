# Spec Delta

## Purpose

How the built-in HTTP server accepts connections and shuts down: what it does when the operating system refuses new
connections, and what is still running when the application's shutdown hooks start. Introduced by the issue of
`q-http-server-robustness` (epic #187).

## ADDED Requirements

### Requirement: Accept errors do not spin
When accepting a connection fails with an error that concerns that connection only (aborted, reset, refused or
interrupted), the server SHALL retry at once. For any other error it MUST wait before the next attempt, 5 ms after the
first error and doubling up to 1 s, until a connection is accepted, and MUST log a warning at most once a minute with the
number of errors since the last line.

#### Scenario: Out of file descriptors
- **WHEN** accepting fails 20 times in a row with "too many open files" and then succeeds
- **THEN** the attempts are spaced 5, 10, 20 ms and so on up to 1 s, one warning line is logged for the burst, and the
  connection accepted afterwards is served

#### Scenario: A connection aborted before it was accepted
- **WHEN** accepting fails with a connection-aborted error
- **THEN** the server tries again without waiting and logs nothing

#### Scenario: Shutdown while waiting
- **WHEN** the shutdown signal arrives while the server waits after an accept error
- **THEN** the server stops waiting and shuts down at once

### Requirement: The shutdown deadline is configurable
`ServerOptions::shutdown_timeout` SHALL set how long the server waits, after its shutdown signal, for its connections to
finish their requests. The default MUST be 10 seconds; `None` MUST wait without limit.

#### Scenario: Default deadline
- **WHEN** an application shuts down with the default options while a request takes longer than 10 seconds
- **THEN** the server stops waiting for it 10 seconds after the shutdown signal

#### Scenario: No deadline
- **WHEN** `shutdown_timeout(None)` is set and a request in flight never ends
- **THEN** the server keeps waiting for it and the shutdown hooks do not start

### Requirement: The shutdown ends what the server started
When the server stops waiting for its connections, it MUST cancel every connection, TLS handshake and body drain it
started and wait for them to stop before it returns, so that no handler of the server runs once the application's
shutdown hooks start. It SHALL log one warning with the number of connections it cut.

#### Scenario: Handler still running at the deadline
- **WHEN** `shutdown_timeout` is 1 second and a handler is still awaiting when the deadline passes
- **THEN** the handler's future is dropped before `on_module_destroy` and the other shutdown hooks run

#### Scenario: Requests that finish in time
- **WHEN** the shutdown signal arrives while a request is in flight and the request finishes before the deadline
- **THEN** the client receives its whole response and the hooks run after it

#### Scenario: Body drain in progress
- **WHEN** the server is draining the unread body of a client that stopped sending, and the shutdown deadline passes
- **THEN** the drain is cancelled and does not keep the server from returning

#### Scenario: TLS client that sends nothing
- **WHEN** a client opened a TLS connection without sending its first handshake message and the deadline passes
- **THEN** the handshake is cancelled and the server returns

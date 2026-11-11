# Spec Delta

## ADDED Requirements

### Requirement: Waiting for room costs nothing below the limits
A reader (the WebSocket transport, a held polling `POST`, the Socket.IO connection) under its unread limits SHALL NOT
register for release notifications. A reader that finds a limit reached MUST register and read the limits again before
it waits, so no release is lost, and a release MUST notify a gauge only while a waiter is registered on it.
Registering MUST NOT allocate.

#### Scenario: Reading under the limits
- **WHEN** a session receives 10,000 messages while every gauge of its chain is under its limit
- **THEN** no waiter is registered on any gauge at any point

#### Scenario: Releases with nobody waiting
- **WHEN** 10,000 charges are released on gauges that nobody waits on
- **THEN** no notification is sent

#### Scenario: A release between the check and the wait
- **WHEN** a release happens after a reader found a limit reached and before it waits
- **THEN** the reader either sees the release when it reads the limits again or is woken by it

### Requirement: The handshake request is kept once per session
A session SHALL keep the request that opened it once, shared by every socket of its connection. A request of an
existing session MUST NOT copy it, and the engine SHALL read only `sid`, `transport` and `EIO` from that request's
query. The kept request is bounded by the server's header limit, one per session, and is not charged to the unread
budget.

#### Scenario: Sockets share the request
- **WHEN** one connection joins ten namespaces with a 16 KiB `cookie` header
- **THEN** each socket after the first adds less than 16 KiB to the heap, and all ten return the same request

#### Scenario: Polling requests do not copy it
- **WHEN** a session opened with a 16 KiB `cookie` header sends a `POST`
- **THEN** the engine allocates less than 16 KiB beyond what the HTTP server itself allocates for the same request

#### Scenario: Query of a poll
- **WHEN** a poll of an existing session carries 10,000 extra query parameters
- **THEN** the engine keeps none of them and answers the poll as it would without them

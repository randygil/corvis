# Spec Delta

## MODIFIED Requirements

### Requirement: A panicking adapter does not leave a connecting socket behind
A panic or an error of the adapter while a socket connects (the id check, the join of its own room) MUST be contained:
it MUST refuse the CONNECT with `Internal server error`, leave nothing of the socket in the namespace or the adapter,
release its charges and free the namespace slot, so the client never sees that socket connected. Adding a socket to
the room named like its own id MUST expel any other member of that room.

#### Scenario: Panic in the id check
- **WHEN** `room_exists` panics while a client connects
- **THEN** the client receives CONNECT_ERROR, the namespace holds no socket and the connection holds no session of it

#### Scenario: Panic joining the own room
- **WHEN** `add_all` panics while the socket joins the room of its own id
- **THEN** the client receives CONNECT_ERROR `Internal server error` and no CONNECT, the namespace and the adapter hold
  nothing of the socket, its charges are released and the connection stays open

#### Scenario: Error joining the own room
- **WHEN** `add_all` returns an error while the socket joins the room of its own id
- **THEN** the outcome is the same as for a panic

#### Scenario: A name joined before the socket was live
- **WHEN** a client joins the name that a connecting socket was given, during the connect middleware
- **THEN** the socket's own room expels that client when the socket goes live

#### Scenario: Answering session closes
- **WHEN** a session closes while its answer is held, and the call then ends
- **THEN** the address and server gauges still count the answer until the call ends and are zero afterwards

## ADDED Requirements

### Requirement: The answer budget keys answers by the session's address
An answer held by a broadcast with acknowledgements SHALL count against the address share of the answer budget under
the key the session uses for its cap and its unread budget (the resolved address, normalized, falling back to the
peer), so a client cannot obtain another share by sending a forwarded address that does not parse.

#### Scenario: Malformed forwarded addresses from one peer
- **WHEN** two connections from one peer, on a server that trusts every proxy, send different malformed
  `X-Forwarded-For` values and answer broadcasts with acknowledgements
- **THEN** their held answers count against one address share, and the answer past that share ends its call with
  `ResponsesTooLarge`

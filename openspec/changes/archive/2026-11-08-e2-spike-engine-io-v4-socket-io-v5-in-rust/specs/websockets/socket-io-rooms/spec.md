# Spec Delta

## Purpose

Defines rooms and broadcasting for socket.io namespaces: `join`, `leave`, the broadcast operators `to`, `in`, `except`
and `broadcast`, and the in-memory adapter behind them. Which sockets receive each broadcast MUST match the Node
`socket.io-adapter` 2.5.x in-memory adapter, verified by the differential harness of `websockets/socket-io-parity`.
Implemented by [#34](https://github.com/randygil/corvis/issues/34).

## ADDED Requirements

### Requirement: Rooms belong to a namespace
Rooms SHALL be scoped to one namespace: joining room `r` in `/` MUST NOT make a socket receive broadcasts to room `r`
in `/admin`. Every socket MUST be in a room named after its own id from the moment it connects, so emitting to a socket
id reaches that socket.

#### Scenario: Same room name in two namespaces
- **WHEN** socket A joins `r` in `/` and socket B joins `r` in `/admin`, and the server emits to `r` in `/`
- **THEN** only A receives the event, on both servers

#### Scenario: Emit to a socket id
- **WHEN** the server emits to the id of socket A
- **THEN** only A receives the event, on both servers

### Requirement: Broadcast operators select the same sockets as Node
`to`/`in` SHALL accept one or more rooms and deliver once to every socket in their union; `except` SHALL accept one or
more rooms (socket ids included) and exclude every socket in them; operators MUST chain. A broadcast started from a
socket (`socket.broadcast`, `socket.to(...)`) MUST exclude that socket. A broadcast MUST NOT expect acknowledgements
unless a timeout is set, in which case the acknowledgements of every recipient are collected as in Node.

#### Scenario: Union without duplicates
- **WHEN** socket A is in rooms `r1` and `r2` and the server emits to `r1` and `r2` together
- **THEN** A receives the event exactly once, on both servers

#### Scenario: Broadcast from a socket
- **WHEN** socket A, in room `r` with B and C, emits `socket.to("r")`
- **THEN** B and C receive the event and A does not, on both servers

#### Scenario: Except a room
- **WHEN** the server emits to the namespace except room `admins`
- **THEN** every socket of the namespace that is not in `admins` receives the event, on both servers

### Requirement: Leaving and disconnecting update rooms like Node
`leave` SHALL remove the socket from a room immediately, and a disconnecting socket MUST still be in its rooms while the
`disconnecting` handler runs and in none after `disconnect`. An empty room MUST be deleted.

#### Scenario: Rooms during disconnecting
- **WHEN** a socket in room `r` disconnects and the server reads its rooms in `disconnecting` and in `disconnect`
- **THEN** the first read contains `r` and its own id and the second is empty, on both servers

### Requirement: Adapter abstraction
Room membership and broadcast delivery SHALL go through an adapter interface (add, delete, broadcast, broadcast with
acks, sockets, socket rooms, fetch sockets, add/remove sockets to rooms, disconnect sockets) with an in-memory
implementation as the default, so the Redis adapter of `websockets/socket-io-cluster` replaces it without changes to
namespaces or sockets.

#### Scenario: Default adapter
- **WHEN** an application uses socket.io without configuring an adapter
- **THEN** rooms and broadcasts work in-process through the in-memory adapter

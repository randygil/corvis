# Spec Delta

## Purpose

Defines a Redis adapter that lets several application instances share socket.io broadcasts and room operations, wire
compatible with the Node `@socket.io/redis-adapter` 8.x (classic, non-sharded), so Corvis and Node instances can serve
the same clients during a migration. Verified by mixed-cluster scenarios of the differential harness of
`websockets/socket-io-parity`. Implemented by [#36](https://github.com/randygil/corvis/issues/36).

## ADDED Requirements

### Requirement: Same channels as @socket.io/redis-adapter
The Redis adapter SHALL publish and subscribe on the same channels as `@socket.io/redis-adapter` 8.3.x:
`socket.io#<nsp>#`, `socket.io#<nsp>#<room>#`, `socket.io-request#<nsp>#`, `socket.io-response#<nsp>#` and
`socket.io-response#<nsp>#<uid>#`, with the configurable key prefix `socket.io`.

#### Scenario: Channels observed in Redis
- **WHEN** a Node instance and a Corvis instance start on the same Redis and join namespace `/chat`
- **THEN** `PUBSUB CHANNELS` lists the same channel patterns for both (uids aside)

### Requirement: Same messages as @socket.io/redis-adapter
The Redis adapter SHALL encode and decode messages as `@socket.io/redis-adapter` 8.3.x does: msgpack
`[uid, packet, opts]` for broadcasts, JSON or msgpack for requests and responses (chosen as the Node adapter chooses,
both accepted on input), the same request types and the same node uid format.

#### Scenario: Mixed cluster broadcast
- **WHEN** one Node instance and one Corvis instance share a Redis server, clients connect to each, and either instance
  emits to a room
- **THEN** every client in the room receives the event once, the same as with two Node instances

#### Scenario: Room broadcast from Node to Corvis
- **WHEN** the Node instance emits to room `r` except room `x`
- **THEN** the Corvis instance delivers it only to its local sockets in `r` and not in `x`

### Requirement: Cluster-wide operations
`fetch_sockets`, `sockets_join`, `sockets_leave`, `disconnect_sockets`, the list of rooms and broadcasts with
acknowledgements SHALL work across instances, waiting at most `requestsTimeout` (default 5000 ms) for the other
instances and failing with the same error as Node when it expires.

#### Scenario: Fetch sockets across instances
- **WHEN** a client is connected to the Node instance and the Corvis instance calls `fetch_sockets` on its room
- **THEN** the result contains that socket's id, rooms and data, as Node returns it

#### Scenario: Broadcast with acknowledgements across instances
- **WHEN** an instance broadcasts with a timeout and clients on both instances acknowledge
- **THEN** the emitter receives every acknowledgement, as with two Node instances

### Requirement: Failure behavior
Losing the Redis connection SHALL NOT crash the application: local delivery MUST keep working, the adapter MUST
reconnect and resubscribe, and cluster requests issued meanwhile MUST fail with the timeout error.

#### Scenario: Redis restart
- **WHEN** Redis restarts while clients are connected
- **THEN** local broadcasts keep working and cross-instance broadcasts resume after the reconnection

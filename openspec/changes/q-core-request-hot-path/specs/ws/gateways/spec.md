# Spec Delta

## ADDED Requirements

### Requirement: A broadcast is encoded once
`WsServer::emit` and `to(room).emit` SHALL serialize the `{ event, data }` frame once per call and give every recipient a
shared copy of that text, so the bytes allocated by a broadcast do not grow with the number of recipients times the
payload size. Each recipient's queue MUST still count the frame against its `max_buffered_amount`, a full queue MUST
close that recipient with 1008, and data that fails to serialize MUST reach no client.

#### Scenario: Large payload to many clients
- **WHEN** a 16 KiB payload is broadcast to 100 connected clients
- **THEN** every client receives the frame and the call allocates less than 48 KiB

#### Scenario: A slow client during a broadcast
- **WHEN** one client's queue is full when a broadcast reaches it
- **THEN** that client is closed with 1008 and every other client receives the frame

#### Scenario: Data that fails to serialize
- **WHEN** the gateway broadcasts a value whose `Serialize` implementation returns an error
- **THEN** no client receives a frame, `emit` returns 0 and one error line names the event

### Requirement: Room emits reach exactly the room's members
`to(room).emit` SHALL reach every connected client that joined `room` and did not leave it, and no other client, with work
proportional to the room's size. A client that disconnected MUST NOT be reached, and a room with no member left MUST NOT
be kept by the server.

#### Scenario: One member among many clients
- **WHEN** one of 1,000 connected clients joined `vip` and the gateway emits to `vip`
- **THEN** only that client receives the frame and the call allocates less than 2 KiB

#### Scenario: Concurrent joins, leaves and disconnects
- **WHEN** 8 tasks join, leave and disconnect 1,000 clients across 50 rooms and every client then disconnects
- **THEN** the server keeps no room and no emit reaches a disconnected client

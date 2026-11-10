# Spec Delta

## ADDED Requirements

### Requirement: The adapter interface is public
The adapter interface, the in-memory adapter and the types an adapter needs (packet, local namespace, broadcast
options and flags, acknowledgement sink, socket information, room, socket id, adapter error) SHALL be public,
documented items of `corvis::socket_io`. The interface MAY gain methods with default implementations in minor
versions; a method without a default MUST wait for a major version. The packet's wire encoding MUST stay private.

#### Scenario: Adapter outside the crate
- **WHEN** a test crate implements the adapter interface by wrapping the in-memory adapter, using only public items,
  and a server is created with it
- **THEN** it compiles, and rooms and broadcasts work through it

#### Scenario: Documented
- **WHEN** `cargo doc -p corvis --features socket-io --no-deps` runs
- **THEN** the adapter interface, the in-memory adapter and those types appear in the documentation, without
  warnings

#### Scenario: Server-side emit has a default
- **WHEN** an adapter written against the interface of #34 does not define server-side emit
- **THEN** it compiles unchanged, and `server_side_emit` on a server that uses it fails with
  `this adapter does not support the serverSideEmit() functionality`

#### Scenario: Errors can grow
- **WHEN** code outside the crate matches on the adapter error
- **THEN** the compiler requires a wildcard arm, so new error kinds do not break it

#### Scenario: Refused by the adapter
- **WHEN** an adapter refuses the options of the server it is given to
- **THEN** creating the server fails with the adapter's error

# Spec Delta

## Purpose

Completes `ws/socket-io-parity` for [#37](https://github.com/randygil/corvis/issues/37): the catalog is checked as a
whole against the requirements of the Socket.IO capabilities, client scenarios run on every transport, the scenarios
that cross layers are added, and a run can be required to skip nothing. The skeleton of #131 and the scenarios added
by #32 to #36 keep their requirements.

## ADDED Requirements

### Requirement: Every Socket.IO requirement names its verification
`parity/socket-io/coverage.json` SHALL list, for every requirement of `ws/engine-io`, `ws/socket-io`,
`ws/socket-io-rooms`, `ws/socket-io-gateways` and `ws/socket-io-cluster`, what verifies it: harness scenarios,
conformance suites or Rust tests. A requirement marked as comparable with Node MUST name at least one scenario or
suite; one marked as not comparable MUST give the reason and name at least one Rust test.

#### Scenario: Requirement without an entry
- **WHEN** a requirement is added to `openspec/specs/ws/socket-io/spec.md` and `coverage.json` is not updated
- **THEN** `node scripts/check-coverage.mjs` exits non-zero and names the capability and the requirement

#### Scenario: Behavior a client can observe
- **WHEN** the entry of "Events match Node" names only a Rust test and is marked as comparable with Node
- **THEN** the check fails, because a comparable requirement needs a scenario or a suite

#### Scenario: Behavior no client can observe
- **WHEN** the entry of "Options are validated" is marked as not comparable, with the reason that Node has no such
  check, and names a Rust test
- **THEN** the check accepts it

### Requirement: Coverage entries are checked against the repository
The coverage check MUST fail when an entry names a requirement that is not in the main specs, a scenario that no file
under `scenarios/` exports, a suite that is not in `suites.json`, or a Rust test function that is not in the named
file. It SHALL read the specs and the sources as text, without building or starting anything, and it MUST run in the
self-tests of the harness.

#### Scenario: Renamed scenario
- **WHEN** a scenario file is renamed and `coverage.json` still names the old scenario
- **THEN** the check fails and names the entry

#### Scenario: Removed requirement
- **WHEN** `coverage.json` holds an entry for a requirement that was removed from the main specs
- **THEN** the check fails and names the stale entry

#### Scenario: Removed Rust test
- **WHEN** an entry names `crates/corvis/tests/socket_io_security.rs::blocked_handlers` and that function no longer
  exists in the file
- **THEN** the check fails and names the entry

### Requirement: Expected differences cite an existing requirement
The `spec` field of every entry of `expected-differences.json` SHALL have the form `<capability>: <requirement>`,
optionally followed by a scenario name in parentheses, and the coverage check MUST fail when the capability or the
requirement is not in the main specs.

#### Scenario: Citation of a renamed requirement
- **WHEN** an entry cites `ws/engine-io: Polling batches are bounded` and that requirement is renamed in the main
  spec
- **THEN** the coverage check fails and names the entry

#### Scenario: Entries written by earlier changes
- **WHEN** the check runs on the entries added by #32 to #36
- **THEN** every one of them cites a requirement that exists, or the check names the ones that do not

### Requirement: Client scenarios run on every transport
A scenario driven by the official client that declares the transport matrix SHALL run on each target three times:
with the client restricted to polling, restricted to WebSocket, and with its default (polling, then upgrade). Each
variant MUST be compared and reported under its own name, the scenario name followed by `@polling`, `@websocket` or
`@default`.

#### Scenario: A difference on one transport only
- **WHEN** Corvis delivers a binary event correctly over WebSocket and wrongly over polling
- **THEN** the run fails with a difference in `socket-io/binary@polling`, and `socket-io/binary@websocket` passes

#### Scenario: Expected difference on every transport
- **WHEN** an expected difference names a scenario without a variant and the scenario declares the matrix
- **THEN** the entry applies to its three variants, and it is stale if the difference is missing in any of them

#### Scenario: Wire scenarios
- **WHEN** a scenario drives raw HTTP or raw WebSocket requests and does not declare the matrix
- **THEN** it runs once per target, as before

### Requirement: Reconnection matches Node
The catalog SHALL include a scenario in which the server closes the transport of a connected official client without
a Socket.IO disconnect, and the transcripts of both targets MUST be equal: the disconnect reason, the reconnection,
the new socket id, the rooms of the new socket and the delivery of an event emitted while disconnected.

#### Scenario: Transport closed by the server
- **WHEN** a client with reconnection enabled has joined a room and the server closes its transport
- **THEN** on both targets the client observes `disconnect` with `transport close`, reconnects at the first attempt
  with a different socket id and `recovered` false, and the new socket is only in the room named after its id

#### Scenario: Emit while disconnected
- **WHEN** the client emits an event with an acknowledgement between the disconnect and the reconnection
- **THEN** on both targets the event is delivered once after the reconnection and its acknowledgement arrives

### Requirement: Heartbeat and upgrade hold under the official client
The catalog SHALL include scenarios in which an official client stays connected through at least five server pings
with short heartbeat settings, and in which client and server exchange numbered events while the client upgrades. No
event may be lost, duplicated or reordered, and the transcripts of both targets MUST be equal.

#### Scenario: Five heartbeats
- **WHEN** the server pings every 300 ms with a 200 ms timeout and the client has observed five pings
- **THEN** on both targets the client is still connected and has observed no disconnect

#### Scenario: Events during the upgrade
- **WHEN** a client emits 200 numbered events right after connecting, while the server emits 200 numbered events to
  it, and the client upgrades from polling to WebSocket meanwhile
- **THEN** on both targets each side receives its 200 events once and in order, and the client ends on the WebSocket
  transport

### Requirement: The client's view of the payload limit matches Node
The catalog SHALL include a scenario in which an official client emits an event larger than the server's
`maxHttpBufferSize`, on each transport, and the disconnect reason the client observes MUST be the same on both
targets.

#### Scenario: Oversized emit
- **WHEN** a client emits a string of 1,000,001 bytes to a server with the default limit of 1,000,000 bytes
- **THEN** on both targets the client observes the same `disconnect` reason for that transport, the server handler is
  not called, and a second client on the same server keeps working

### Requirement: Gateways broadcast across a mixed cluster
The catalog SHALL include a scenario that requires the `gateways` and `redis` capabilities, in which a gateway
handler broadcasts to a room through its server handle and a client connected to the other instance of the cluster
receives it. The mixed cluster (one reference instance, one Corvis instance) MUST produce the transcripts of two
reference instances.

#### Scenario: Room broadcast from a gateway on the other instance
- **WHEN** a client on instance A sends the gateway event that broadcasts to room `r`, and a client on instance B is
  in `r`
- **THEN** the client on B receives the event with the same arguments in the mixed cluster and in the reference
  cluster, and the sender receives only its acknowledgement

### Requirement: Strict runs skip nothing
With `--require-all` the runner MUST exit non-zero when any scenario or variant was skipped, whatever the reason: a
capability the Corvis target lacks, `CORVIS_REDIS_URL` not set, or the gateway reference package not installed. The
Rust entry point SHALL pass the flag when `CORVIS_NODE_DIFF_STRICT=1`. Without the flag, skipping keeps its current
behavior.

#### Scenario: Redis missing in a strict run
- **WHEN** the runner is started with `--require-all` and `CORVIS_REDIS_URL` is not set
- **THEN** it exits non-zero and lists the cluster scenarios as skipped

#### Scenario: Test built without the features
- **WHEN** `CORVIS_NODE_DIFF=1 CORVIS_NODE_DIFF_STRICT=1 cargo test -p corvis --test socket_io_diff` runs without
  `--features socket-io-redis`
- **THEN** the test fails, because the scenarios that need Corvis capabilities were skipped

#### Scenario: Contributor without Redis
- **WHEN** the runner is started without `--require-all` and without `CORVIS_REDIS_URL`
- **THEN** the cluster scenarios are reported as skipped and the result does not depend on them

#### Scenario: Summary line
- **WHEN** a strict run passes
- **THEN** its last line states the number of scenarios compared and `0 skipped`

### Requirement: Suite exclusions are explicit
A test of an upstream protocol suite MAY be excluded only by an entry in `suites.json` holding its exact title, a
reason and the harness scenario that covers the behavior instead. The conformance tests MUST fail when an excluded
title is not found exactly once in the fetched suite, and MUST run every other test of the suite.

#### Scenario: Racy upstream test
- **WHEN** a test of the `engine.io-protocol` suite fails intermittently against the Node reference server too, and
  `suites.json` excludes it with the reason and a covering scenario
- **THEN** the conformance run skips only that test, against Corvis and against the Node reference

#### Scenario: Stale exclusion
- **WHEN** the pinned commit of a suite changes and an excluded title no longer exists in it
- **THEN** the conformance test fails and names the exclusion

#### Scenario: Exclusion without a covering scenario
- **WHEN** an exclusion names a scenario that no file under `scenarios/` exports
- **THEN** the coverage check fails and names the exclusion

### Requirement: Repeated runs give the same result
The full catalog, in strict mode with Redis, SHALL pass ten consecutive runs on one machine without a retry inside
or between runs. An intermittent failure MUST be fixed at its cause, in the scenario, the harness or Corvis, and
MUST NOT be handled by repeating the scenario or by listing an expected difference.

#### Scenario: Ten runs
- **WHEN** the strict harness run is repeated ten times in a row
- **THEN** all ten runs pass with the same number of scenarios compared and no difference

#### Scenario: A scenario that fails one run in ten
- **WHEN** one of the ten runs reports a difference that the other nine do not
- **THEN** the sequence fails, and the cause is fixed before the change is archived

# Spec Delta

## Purpose

Defines how socket.io parity is verified: the upstream protocol conformance suites run against Corvis, and a
differential harness runs the same scenarios with the official `socket.io-client` against a Node `socket.io` server and
a Corvis server and compares what clients observe. Every requirement of `websockets/engine-io`,
`websockets/socket-io`, `websockets/socket-io-rooms`, `websockets/socket-io-gateways` and `websockets/socket-io-cluster`
is verified here. Implemented by [#37](https://github.com/randygil/corvis/issues/37).

## ADDED Requirements

### Requirement: Pinned, reproducible oracle
The harness SHALL live in the repository with its own `package.json` and committed `package-lock.json` pinning
`socket.io`, `socket.io-client`, `engine.io-client`, `ws` and, for cluster scenarios, `@socket.io/redis-adapter` and a
Redis client. Installation MUST be explicit (`npm ci` in the harness directory); `cargo test` MUST NOT install packages.
Servers MUST bind to `127.0.0.1` on ephemeral ports.

#### Scenario: Fresh machine
- **WHEN** a contributor clones the repository, runs `npm ci` in the harness directory and runs the documented command
- **THEN** the harness runs without any machine-specific path

### Requirement: Same scenarios against both servers
Each scenario SHALL run unchanged against the Node reference server and the Corvis server, record a JSON transcript of
everything each client observes (events, arguments, acknowledgements, errors, disconnect reasons, and for wire
scenarios the HTTP status, relevant headers and raw payloads), normalize values that are random by design (session and
socket ids, timings, interleavings between independent clients), and fail with a readable diff when the transcripts
differ.

#### Scenario: A behavior difference
- **WHEN** the Corvis server answers an event with different arguments than Node
- **THEN** the harness exits non-zero and prints the scenario name and the differing transcript lines

#### Scenario: Ids are normalized
- **WHEN** both servers behave the same but generate different session ids
- **THEN** the harness reports no difference

### Requirement: Expected differences are explicit
Differences that Corvis keeps on purpose (the security divergences of `websockets/engine-io`) SHALL be listed in one
file with the reason and the scenario that exercises them; any difference not listed MUST fail the run, and a listed
difference that no longer occurs MUST also fail so the list stays accurate.

#### Scenario: Stale expected difference
- **WHEN** a listed difference stops occurring
- **THEN** the harness fails and names the stale entry

### Requirement: Conformance suites
The upstream `engine.io-protocol` and `socket.io-protocol` test suites, pinned by commit, SHALL pass against a Corvis
test server configured as the suites require.

#### Scenario: Protocol suites
- **WHEN** the suites run against the Corvis test server
- **THEN** every test passes

### Requirement: Gated in local runs, required in CI
The Rust entry point SHALL skip with a printed notice when Node is missing or `CORVIS_NODE_DIFF` is not set, and MUST
fail instead of skipping when `CORVIS_NODE_DIFF=1` is set and Node or the installed harness is missing. Cluster
scenarios SHALL run only when `CORVIS_REDIS_URL` is set. Flaky results MUST be treated as bugs: the harness MUST NOT
retry a failing scenario.

#### Scenario: Developer without Node
- **WHEN** `cargo test --workspace` runs on a machine without Node and without `CORVIS_NODE_DIFF`
- **THEN** the differential test is skipped with a notice and the run passes

#### Scenario: CI without Node
- **WHEN** the differential job sets `CORVIS_NODE_DIFF=1` but Node is not installed
- **THEN** the test fails

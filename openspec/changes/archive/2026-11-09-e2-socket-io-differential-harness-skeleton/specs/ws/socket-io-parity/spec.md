# Spec Delta

## Purpose

How socket.io parity is verified: a differential harness runs the same scenarios against a Node `socket.io` server and
against Corvis and compares what clients observe, and a script fetches the official protocol test suites at a pinned
commit. This delta covers the skeleton of [#131](https://github.com/randygil/corvis/issues/131); #37 adds the full
scenario catalog, the conformance runs and the CI job.

## ADDED Requirements

### Requirement: Exact versions with a lockfile
The harness SHALL live in `parity/socket-io/` with its own `package.json` listing exact versions (no ranges) of
`socket.io`, `engine.io`, `socket.io-client`, `@socket.io/redis-adapter`, `redis`, `ws` and the packages the protocol
suites import, and a committed `package-lock.json`.

#### Scenario: Fresh clone
- **WHEN** a contributor clones the repository, runs `npm ci` in `parity/socket-io/` and then
  `CORVIS_NODE_DIFF=1 cargo test -p corvis --test socket_io_diff`
- **THEN** the harness runs without any machine-specific path or global package

### Requirement: Installation is explicit
Packages SHALL be installed only by an explicit `npm ci` in `parity/socket-io/`, with install scripts disabled by the
harness's `.npmrc`. `cargo test` MUST NOT install packages.

#### Scenario: Nothing installed by cargo
- **WHEN** `CORVIS_NODE_DIFF=1 cargo test -p corvis --test socket_io_diff` runs without `node_modules`
- **THEN** the test fails and `parity/socket-io/node_modules` still does not exist

### Requirement: Installed versions are checked
The runner MUST refuse to run, before starting any server, when the installed `socket.io` or `engine.io` version
differs from the one pinned in `package.json`.

#### Scenario: Stale node_modules
- **WHEN** `parity/socket-io/node_modules` holds a `socket.io` version other than the one in `package.json`
- **THEN** the runner exits non-zero with a message telling to run `npm ci`

### Requirement: Servers bind to loopback on ephemeral ports
Every server the harness starts SHALL bind to `127.0.0.1` on a port chosen by the operating system and report that
port to the runner. Child processes MUST be terminated when the run ends, including on failure.

#### Scenario: Two runs at once
- **WHEN** two harness runs start at the same time on one machine
- **THEN** both succeed, because no fixed port is used

#### Scenario: Loopback address
- **WHEN** the reference server starts and reports its address
- **THEN** the reported address is `127.0.0.1` with a non-zero port chosen by the operating system

### Requirement: Transcripts record what clients observe
Each scenario SHALL run unchanged against two targets and record, per client and in order, what the client observes:
events with their arguments, acknowledgements, connect errors, disconnect reasons, and for raw requests the HTTP
status, the `content-type` and `cache-control` headers and the decoded packets. Transcripts MUST NOT hold timings.

#### Scenario: Transcript of the first scenario
- **WHEN** the first scenario runs against a Node reference server
- **THEN** `out/<target>/socket-io/handshake-and-ack.json` holds the open packet, the acknowledgement arguments and the
  disconnect reason, and no timestamp

### Requirement: Random ids are normalized
Engine.IO session ids and Socket.IO socket ids SHALL be replaced by placeholders numbered by first appearance,
recognized by the field they come from, so equal ids stay equal and different ids stay different.

#### Scenario: Ids are normalized
- **WHEN** the first scenario runs against two Node reference servers, which generate different session ids
- **THEN** the runner reports no difference and exits zero

#### Scenario: Id identity is kept
- **WHEN** one transcript reuses the same session id in two places and the other uses two different ids there
- **THEN** the comparer reports a difference

### Requirement: Differences fail with a readable diff
When two normalized transcripts differ, the runner SHALL print the scenario name and each differing path with both
values, and exit non-zero. Entries of different clients MUST be compared per client, so interleavings between
independent clients are not differences.

#### Scenario: A behavior difference
- **WHEN** one transcript of the comparer self-test has an acknowledgement argument changed
- **THEN** the comparer reports the scenario and the path of that argument with both values and fails

### Requirement: The first scenario covers a handshake and an acknowledged event
The harness SHALL include `socket-io/handshake-and-ack`: a raw `GET /socket.io/?EIO=4&transport=polling` whose open
packet is recorded, then the official client connecting with an `auth` object, emitting `echo` with `"hello"` and `42`
and an acknowledgement callback, and disconnecting. It requires the `socket-io` capability.

#### Scenario: Node only
- **WHEN** the runner runs without a Corvis target
- **THEN** the scenario runs against two Node reference servers, records an open packet with `pingInterval` 25000,
  `pingTimeout` 20000, `maxPayload` 1000000 and `upgrades` `["websocket"]` and the acknowledgement arguments
  `["hello", 42]`, and the comparison passes

### Requirement: Scenarios run where their capabilities exist
A scenario SHALL declare the capabilities it needs (`engine-io`, `socket-io`, `redis`) and a target the ones it has.
A scenario whose capabilities the Corvis target lacks MUST run on Node only and be reported as skipped on Corvis.

#### Scenario: Corvis target without the capability
- **WHEN** a Corvis target that does not declare `socket-io` is given
- **THEN** the first scenario runs against Node only and the run prints that it was skipped on Corvis

### Requirement: Expected differences are explicit
Intentional differences SHALL be listed in `parity/socket-io/expected-differences.json`, one entry per scenario and
path with a reason. An unlisted difference MUST fail the run, an entry without a reason MUST be rejected, and a listed
difference that does not occur when its scenario ran on both targets MUST fail the run as stale.

#### Scenario: Stale entry
- **WHEN** the comparer self-test lists a difference that its two transcripts do not have
- **THEN** the comparer fails and names the stale entry

#### Scenario: Listed difference
- **WHEN** the comparer self-test lists exactly the difference its two transcripts have
- **THEN** the comparison passes and the run prints the difference as expected

### Requirement: Opt-in with CORVIS_NODE_DIFF
The Rust entry point SHALL skip with a printed notice and pass when `CORVIS_NODE_DIFF` is not `1`. When it is `1`, it
MUST fail instead of skipping if `node` is not on `PATH`, its major version is below 22, or the harness is not
installed.

#### Scenario: Developer without Node
- **WHEN** `cargo test --workspace` runs without `CORVIS_NODE_DIFF`
- **THEN** `socket_io_diff` prints a notice that it was skipped and passes

#### Scenario: Harness not installed
- **WHEN** `CORVIS_NODE_DIFF=1` is set and `parity/socket-io/node_modules` does not exist
- **THEN** the test fails with a message that names `npm ci` in `parity/socket-io/`

### Requirement: Redis scenarios need CORVIS_REDIS_URL
Scenarios that require `redis` SHALL run only when `CORVIS_REDIS_URL` is set, and MUST be reported as skipped
otherwise.

#### Scenario: Redis scenario without Redis
- **WHEN** a scenario requires `redis` and `CORVIS_REDIS_URL` is not set
- **THEN** it is reported as skipped and the run result does not depend on it

### Requirement: No retries and no speed-dependent assertions
The harness MUST NOT retry a failing scenario. Scenario steps SHALL wait for the observed event with a generous
timeout instead of sleeping, and no assertion may depend on how fast the machine is.

#### Scenario: Slow machine
- **WHEN** a self-test runs the first scenario against a reference server started with `--ack-delay 2000` (the
  acknowledgement is sent two seconds late)
- **THEN** the transcript and the result are the same as without the delay

### Requirement: Protocol suites are fetched at pinned commits
`parity/socket-io/scripts/fetch-suites.mjs` SHALL fetch the `test-suite` directories of `socketio/engine.io-protocol`
and `socketio/socket.io-protocol` at the 40-hex commits listed in `parity/socket-io/suites.json`, check that the
checked-out commit is the pinned one, and write them under `parity/socket-io/suites/` (ignored by git), without running
a shell. A malformed or unreachable commit MUST fail the script.

#### Scenario: Suites load
- **WHEN** the script has run and `mocha --dry-run` is pointed at each downloaded suite
- **THEN** both suites load and list their tests without a server

### Requirement: The suites' server URL comes from the environment
The written copies SHALL read the server URL from the `SUITE_URL` environment variable instead of the hard-coded
`http://localhost:3000`, and the script MUST fail when that literal is not found where expected.

#### Scenario: Upstream file changed
- **WHEN** the pinned commit of a suite no longer contains the hard-coded URL in a listed file
- **THEN** the script exits non-zero naming the file, instead of writing a copy that targets `localhost:3000`

# Spec Delta

## Purpose

How the Socket.IO benchmark (`bench/socket-io/run.sh`) builds, drives a Corvis Socket.IO server and reports, so that a
change with a performance claim can show the same figures before and after. Introduced by
[#185](https://github.com/randygil/corvis/issues/185).

## ADDED Requirements

### Requirement: The Socket.IO benchmark drives emit and broadcast
`bench/socket-io/run.sh` SHALL build `corvis-sio-bench` with `cargo build --release -p corvis-sio-bench` in the
workspace target directory, start its `server` on `127.0.0.1` and run its `load` generator in another process, for the
scenarios `emit`, `server-emit` and `broadcast` over WebSocket and polling, with the connections, the messages per
connection and the payload size as parameters.

#### Scenario: Default run
- **WHEN** `bash bench/socket-io/run.sh` runs with no variables set
- **THEN** it measures the three scenarios over WebSocket with 1,000 connections and over polling with 200 connections,
  100 messages of 64 bytes per connection, with one server process running at a time

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/socket-io/run.sh` runs and the release binary does not exist in the target directory
- **THEN** the script exits with a non-zero status before starting a server, naming the path it looked for and the
  build command

### Requirement: The benchmark reports rates, latency and the server's allocations
For each scenario and transport the benchmark SHALL report messages per second, the p50, p99 and maximum latency, the
server's allocations and bytes allocated per message (counted by a counting allocator in the server process over the
measured window only) and the server's peak resident memory. Each measurement SHALL run three times and report the
median with the minimum and maximum.

#### Scenario: Results of a run
- **WHEN** a run finishes
- **THEN** `bench/socket-io/results.md` holds one row per scenario and transport with those figures and the parameters
  of the run, and the raw output is in `bench/socket-io/raw/`, which git ignores

### Requirement: The benchmark is not a test
The benchmark SHALL NOT run in CI or in `cargo test`, and no test SHALL compare its figures, or any timing, between
runs. A change that claims a performance effect SHALL record the before and after figures, taken on the same machine
with the same parameters, in its design.

#### Scenario: Workspace tests
- **WHEN** `cargo test --workspace` runs
- **THEN** no benchmark scenario runs and no server of the benchmark is started

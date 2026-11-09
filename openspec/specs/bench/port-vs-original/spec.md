# bench/port-vs-original Specification

## Purpose
How the port-versus-original benchmark measures a ported application against its original (setup equality check, throughput, latency at a fixed rate, memory) and how its results are published.

## Requirements

### Requirement: Same work on both sides
Before measuring, the benchmark SHALL reset one database per side from the same template, seed both through the HTTP
API with the same requests, and send every measured request once to each side, comparing the responses with apidiff.
If any measured request gets a different status or body (beyond the identifiers, timestamps and tokens apidiff already
maps), the run MUST stop without producing results.

#### Scenario: Responses match
- **WHEN** the setup scenario runs against the original and the port on freshly reset databases
- **THEN** apidiff reports 0 differences for the measured requests and the run continues to the measurements

#### Scenario: A measured request differs
- **WHEN** the port answers a measured request with a different status or body than the original
- **THEN** the run stops with a message naming the request, and no `summary.json` is written

### Requirement: Isolated, repeated measurements
Only one server SHALL be running during a measurement, on `127.0.0.1`, and each scenario MUST start from a freshly
started server process followed by a warmup that is not measured. Each throughput and latency measurement SHALL run
three times; the reported value is the median, and the minimum and maximum are kept in `summary.json`. The server
MUST still answer its health endpoint after every measurement, otherwise the run fails.

#### Scenario: One server at a time
- **WHEN** the port is being measured
- **THEN** the original is not running, and the load generator talks only to `127.0.0.1`

#### Scenario: Server crash
- **WHEN** a server stops answering its health endpoint after a scenario
- **THEN** the run fails and names the side and the scenario

### Requirement: Throughput is reported with its errors
For every scenario and side, the benchmark SHALL report the requests per second at a fixed number of connections
(closed loop), together with the count of errors and of responses with an unexpected status. Any error or unexpected
status MUST be shown next to the numbers it affects.

#### Scenario: Unexpected status during the load
- **WHEN** some responses of a throughput run have a status other than the one the setup recorded for that request
- **THEN** the report shows their count in the same row as that run's requests per second

### Requirement: Latency is compared at the same rate
For every scenario and side, the benchmark SHALL report p50, p90, p99 and maximum latency measured at a fixed request
rate (open loop, with latency correction), and that rate MUST be the same for both sides.

#### Scenario: Latency at the same rate
- **WHEN** the latency phase of a scenario runs
- **THEN** both sides receive the same requests per second, set below the original's measured throughput for that
  scenario, and the report shows p50, p90, p99 and max for each

### Requirement: Memory of the server process is reported
For every scenario and side, the benchmark SHALL report the resident memory of the server process at idle, at its
peak during the scenario, and after the load.

#### Scenario: Memory peak
- **WHEN** a scenario finishes
- **THEN** the report shows the server's peak resident memory during that scenario, taken from the operating system's
  peak counter of a process that only served that scenario

### Requirement: Results are reproducible from committed data
Every published number SHALL come from a committed `summary.json` that records the date, operating system, CPU, Node
and rustc versions, oha version and options, the Corvis commit used by the port, the database pool size of each side,
the scenarios and their parameters. The docs tables MUST be generated from that file, in English and in Spanish, by
the report step.

#### Scenario: Regenerating the docs table
- **WHEN** `node bench/port-vs-original/report.mjs --docs` runs on a committed `summary.json`
- **THEN** the results tables between the markers of `docs/faq/port-vs-original-benchmark.md` and
  `docs/es/faq/port-vs-original-benchmark.md` are rewritten with the same numbers, and the rest of the pages is unchanged

### Requirement: The benchmark never builds
The benchmark scripts SHALL use the binaries and bundles that already exist (the port's release binary and the
original's compiled `dist/`) and MUST NOT run `cargo build` or create a Cargo target directory. When a binary is
missing, the script SHALL stop and print the command that builds it.

#### Scenario: Missing port binary
- **WHEN** the port's release binary does not exist
- **THEN** the script exits with an error that shows the build command for the port, and nothing is built

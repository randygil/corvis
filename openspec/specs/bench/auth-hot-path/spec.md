# bench/auth-hot-path Specification

## Purpose
TBD - created by archiving change e3-authentication-hot-path-and-consolidation. Update Purpose after archive.

## Requirements

### Requirement: The authentication benchmark covers the hot paths
`bench/auth/run.sh` SHALL measure a release build of `corvis-auth-bench` on `127.0.0.1` for the scenarios `jwt-hs256`,
`jwt-provider-rs256`, `service-verify-rs256`, `service-sign-hs256`, `service-sign-rs256`, `session-none`,
`session-read` and `session-write`, and MUST check the status and body of each scenario's request before measuring it.

#### Scenario: Full run
- **WHEN** `bash bench/auth/run.sh --label before` runs with no other server on its ports
- **THEN** every scenario is measured and the server answers its stats endpoint after each one

#### Scenario: Unexpected answer
- **WHEN** a scenario's request does not get its expected status before the measurement
- **THEN** the run stops with a message that names the scenario, and no results file is written

### Requirement: The benchmark reports the cost of a request
For every scenario the benchmark SHALL report requests per second at a fixed number of connections, the p50, p90, p99
and maximum latency at a fixed rate, and the server's allocations and bytes allocated per request, computed from the
server's counters read before and after the measured run.

#### Scenario: Allocations per request
- **WHEN** a scenario has been measured
- **THEN** its row shows allocations and bytes per request, equal to the change of the server's counters divided by the
  change of its request counter

### Requirement: The benchmark builds in the workspace target directory
`bench/auth/run.sh` SHALL build with `cargo build --release -p corvis-auth-bench` from the repository root without
overriding the target directory, and MUST NOT create a Cargo target directory of its own. With `SKIP_BUILD=1` it SHALL
reuse the existing binary, and stop with the full path and the build command when the binary is missing.

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/auth/run.sh` runs and the release binary does not exist
- **THEN** the script exits with a non-zero status before starting a server and prints the path and the build command

### Requirement: Results are labelled and recorded
A run SHALL write `bench/auth/results/<label>.json`, ignored by git, with the date, operating system, CPU, rustc and
`oha` versions, the Corvis commit and the parameters, and print a Markdown table whose rows a change copies into its
design.

#### Scenario: Before and after
- **WHEN** the benchmark runs with `--label before` and later with `--label after`
- **THEN** two results files exist and each printed table has one row per scenario

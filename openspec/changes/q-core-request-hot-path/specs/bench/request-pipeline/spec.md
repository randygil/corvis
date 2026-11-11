# Spec Delta

## Purpose

How the request pipeline benchmark (`bench/pipeline/run.sh`) builds, drives a Corvis application in process and reports
allocations per request for each feature an application can turn on, so that a change with a performance claim can show
the same figures before and after.

## ADDED Requirements

### Requirement: The pipeline benchmark counts allocations per request for each feature
`bench/pipeline/run.sh` SHALL build `corvis-pipeline-bench` with `cargo build --release -p corvis-pipeline-bench` in the
workspace target directory and run it in process, with a counting allocator, for the scenarios `plain`, `body-1k`,
`body-64k`, `cors`, `middleware`, `middleware-miss`, `throttler-1`, `throttler-3` and `request-scope`, reporting
allocations and bytes allocated per request and the difference from `plain`.

#### Scenario: Default run
- **WHEN** `bash bench/pipeline/run.sh` runs with no variables set
- **THEN** each scenario runs 1,000 warm-up and 10,000 measured requests three times, and `bench/pipeline/results.md`
  holds one row per scenario with the median, minimum and maximum allocations and bytes per request

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/pipeline/run.sh` runs and the release binary does not exist in the target directory
- **THEN** the script exits with a non-zero status before running anything, naming the path it looked for and the build
  command

### Requirement: The pipeline benchmark measures contention on the throttler
The benchmark SHALL have a `contention` mode that runs the `plain` and `throttler-1` scenarios on a multi-thread runtime
with 1, 4 and 8 workers sending requests with distinct trackers, and reports requests per second and the ratio of
`throttler-1` to `plain` for each worker count.

#### Scenario: Contention table
- **WHEN** a run finishes
- **THEN** `bench/pipeline/results.md` holds a table with requests per second for both scenarios at 1, 4 and 8 workers
  and their ratio

### Requirement: The pipeline benchmark is not a test
The benchmark SHALL NOT run in CI or in `cargo test`, its raw output SHALL go to `bench/pipeline/raw/`, which git ignores,
and no test SHALL compare its rates between runs. A test MAY pin allocation counts, which are deterministic for a build.

#### Scenario: Running the test suite
- **WHEN** `cargo test --workspace` runs
- **THEN** the benchmark binary does not run and no test reads `bench/pipeline/results.md`

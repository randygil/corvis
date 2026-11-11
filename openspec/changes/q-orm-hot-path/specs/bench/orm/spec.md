# Spec Delta

## Purpose

How the ORM benchmark (`bench/orm/run.sh`) builds, drives corvis-orm against SQLite and Postgres and reports, so that a
change with a performance claim on the ORM can show the same figures before and after. Introduced by the ORM quality
change `q-orm-hot-path` (epic [#187](https://github.com/randygil/corvis/issues/187)).

## ADDED Requirements

### Requirement: The ORM benchmark drives the hot paths
`bench/orm/run.sh` SHALL build `corvis-orm-bench` with `cargo build --release -p corvis-orm-bench` in the workspace
target directory and run its scenarios (`find_unique`, `find_many`, `include`, `include_paginated`, `include_m2m`,
`create_many`, `create_many_and_return`, `update_many`, `transaction`, `distinct`) on SQLite and on Postgres at
`127.0.0.1`, on a seeded data set.

#### Scenario: Default run
- **WHEN** `bash bench/orm/run.sh` runs with the local Postgres server available
- **THEN** it measures every scenario on a SQLite file in a temporary directory and on a scratch Postgres database, which
  it drops afterwards

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/orm/run.sh` runs and the release binary does not exist in the target directory
- **THEN** the script exits with a non-zero status before running any scenario, naming the path it looked for and the
  build command

### Requirement: The ORM benchmark reports rates, latency, allocations and statements
For each scenario and dialect the benchmark SHALL report operations per second, the p50, p99 and maximum latency, the
allocations and bytes allocated per operation over the measured window only, the statements and the rows decoded per
operation, and the peak resident memory. Each measurement SHALL run three times and report the median with the minimum
and maximum.

#### Scenario: Results of a run
- **WHEN** a run finishes
- **THEN** `bench/orm/results.md` holds one row per scenario and dialect with those figures and the parameters of the run,
  and the raw output is in `bench/orm/raw/`, which git ignores

### Requirement: The ORM benchmark is not a test
The benchmark SHALL NOT run in CI or in `cargo test`, and no test SHALL compare its figures, or any timing, between runs.
A change that claims a performance effect on the ORM SHALL record the before and after figures, taken on the same machine
with the same parameters, in its design.

#### Scenario: Workspace tests
- **WHEN** `cargo test --workspace` runs
- **THEN** no benchmark scenario runs

# Spec Delta

## Purpose

How the validation benchmark (`bench/validation/run.sh`) builds, measures the validation library in process and reports,
so that a change with a performance claim can show the same figures before and after. Introduced by
`q-validation-hot-path`.

## ADDED Requirements

### Requirement: The validation benchmark covers the hot paths
`bench/validation/run.sh` SHALL build `corvis-validation-bench` and measure, in one process, the scenarios `dto-ok`,
`dto-fail`, `recursive-union`, `intersection`, `refine-root`, `enum-heavy`, `strict-wide`, `serialize` and `threads`,
and MUST check each scenario's result (the expected output or error body) before measuring it.

#### Scenario: Full run
- **WHEN** `bash bench/validation/run.sh --label before` runs
- **THEN** every scenario is checked and measured, and `dto-ok` has a row for the pipe and a row for `parse(&Value)`

#### Scenario: Unexpected result
- **WHEN** a scenario's parse does not give its expected result
- **THEN** the run stops with a message that names the scenario, and no results file is written

### Requirement: The benchmark reports time and allocations per parse
For every scenario the benchmark SHALL report the median time per parse of five runs with the minimum and maximum, the
allocations and bytes allocated per parse counted by a counting allocator over one run, and the clone ratio: bytes per
parse divided by the bytes one copy of the scenario's input allocates.

#### Scenario: A scenario row
- **WHEN** a scenario has been measured
- **THEN** its row shows time per parse (median, min, max), allocations per parse, bytes per parse and the clone ratio

### Requirement: The benchmark builds in the workspace target directory
`bench/validation/run.sh` SHALL build with `cargo build --release -p corvis-validation-bench` from the repository root
without overriding the target directory, and MUST NOT create a Cargo target directory of its own. With `SKIP_BUILD=1` it
SHALL reuse the existing binary, and stop with the full path and the build command when the binary is missing.

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/validation/run.sh` runs and the release binary does not exist
- **THEN** the script exits with a non-zero status and prints the path it looked for and the build command

### Requirement: Results are labelled and the benchmark is not a test
A run SHALL write `bench/validation/results/<label>.json`, ignored by git, with the machine, the rustc version, the
commit and the parameters, and print a Markdown table. The benchmark SHALL NOT run in `cargo test` or CI, and no test
SHALL compare its timings.

#### Scenario: Before and after
- **WHEN** the benchmark runs with `--label before` and later with `--label after`
- **THEN** two results files exist and each printed table has one row per scenario and entry point

#### Scenario: Workspace tests
- **WHEN** `cargo test --workspace` runs
- **THEN** no benchmark scenario runs

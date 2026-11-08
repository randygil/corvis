# bench/synthetic Specification

## Purpose
How the synthetic throughput benchmark (`bench/run.sh`) builds, runs and reports, and where its build output goes.

## Requirements

### Requirement: The benchmark builds in the workspace target directory
`bench/run.sh` SHALL build the Corvis benchmark application with `cargo build --release -p corvis-app` from the
repository root, without overriding the target directory, so the build goes to the directory Cargo resolves for the
workspace (`CARGO_TARGET_DIR` when it is set, otherwise `<repo>/target`). A run MUST NOT create `bench/target` or any
other Cargo target directory.

#### Scenario: Shared target from the environment
- **WHEN** `bash bench/run.sh` runs with `CARGO_TARGET_DIR` set to the shared target directory
- **THEN** the binary is built at `$CARGO_TARGET_DIR/release/corvis-app.exe` and no `bench/target` folder exists after
  the run

#### Scenario: Default target
- **WHEN** `bash bench/run.sh` runs with `CARGO_TARGET_DIR` unset
- **THEN** the binary is built at `<repo>/target/release/corvis-app.exe` and no `bench/target` folder exists after the
  run

#### Scenario: Leftover folder from older runs
- **WHEN** `bash bench/run.sh` runs and a `bench/target` folder exists from an older version of the script
- **THEN** the script prints a notice that the folder is no longer used and can be deleted, and leaves it in place

### Requirement: The benchmark finds the binary where Cargo builds it
`bench/run.sh` SHALL run the Corvis application from the `release` folder of the target directory that Cargo reports
for the workspace, both after building and with `SKIP_BUILD=1`. When the binary is missing, the script MUST stop
before starting any server, with a message that gives the full path it looked for and the command that builds it.

#### Scenario: Reusing a previous build
- **WHEN** `SKIP_BUILD=1 bash bench/run.sh` runs after a previous build in the shared target directory
- **THEN** the script runs `<target>/release/corvis-app.exe` without building

#### Scenario: Missing binary
- **WHEN** `SKIP_BUILD=1 bash bench/run.sh` runs and `<target>/release/corvis-app.exe` does not exist
- **THEN** the script exits with a non-zero status and the message names that path and
  `cargo build --release -p corvis-app`

### Requirement: The benchmark documents where it builds
`bench/README.md` SHALL describe, in English, how to run the benchmark, its prerequisites and environment variables,
and that the Corvis binary is built in release mode in the shared Cargo target directory, giving its location. The
header comment of `bench/run.sh` MUST point to that README.

#### Scenario: Reader looks for the binary
- **WHEN** a contributor reads `bench/README.md`
- **THEN** it states that the binary is `<target>/release/corvis-app.exe`, where `<target>` is `CARGO_TARGET_DIR` or
  `<repo>/target`, and that `SKIP_BUILD=1` reuses it

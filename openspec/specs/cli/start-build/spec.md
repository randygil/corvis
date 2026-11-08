# cli/start-build Specification

## Purpose
TBD - created by archiving change e13-cli-defaults-that-contradict-the-docs. Update Purpose after archive.

## Requirements

### Requirement: Build profile follows Cargo
`corvis build` SHALL run `cargo build` (debug profile) by default and `cargo build --release` when `--release` or
`--prod` is given. Before running Cargo it MUST print the profile it builds (`debug` or `release`). The README and the
docs site MUST describe this same default.

#### Scenario: Default build
- **WHEN** the user runs `corvis build` in a project
- **THEN** the CLI prints that it builds the debug profile and runs `cargo build` without `--release`

#### Scenario: Release build
- **WHEN** the user runs `corvis build --release`
- **THEN** the CLI prints that it builds the release profile and runs `cargo build --release`

### Requirement: Watch mode explains a missing cargo-watch
Before running `cargo watch`, `corvis start --watch` SHALL check that the `cargo-watch` executable is available where
Cargo looks for external subcommands (the directories of `PATH` and `$CARGO_HOME/bin`, defaulting to `~/.cargo/bin`).
If it is not found, the command MUST exit with a non-zero status and an error that names `cargo-watch`, shows the
install command `cargo install cargo-watch`, and mentions `corvis start` without `--watch` as the alternative; it MUST
NOT run Cargo in that case.

#### Scenario: cargo-watch not installed
- **WHEN** the user runs `corvis start --watch` and no `cargo-watch` executable exists in `PATH` or `$CARGO_HOME/bin`
- **THEN** the command fails with a message containing `cargo install cargo-watch`, and no `cargo` process is started

#### Scenario: cargo-watch installed
- **WHEN** a `cargo-watch` executable exists in one of those directories
- **THEN** the CLI runs `cargo watch -x run` as before

# Spec Delta

## ADDED Requirements

### Requirement: Watch mode explains a missing watchexec
Before starting any process, `corvis start --watch` SHALL check that a `watchexec` executable is available in the
directories of `PATH` or in `$CARGO_HOME/bin` (defaulting to `~/.cargo/bin`). If it is not found, the command MUST exit
with a non-zero status and an error that names `watchexec`, shows the install command
`cargo install --locked watchexec-cli`, mentions that prebuilt binaries and package managers are listed by the
watchexec project, and mentions `corvis start` without `--watch` as the alternative. It MUST NOT run Cargo or any
other process in that case. The CLI MUST NOT look for or run cargo-watch.

#### Scenario: watchexec not installed
- **WHEN** the user runs `corvis start --watch` and no `watchexec` executable exists in `PATH` or `$CARGO_HOME/bin`
- **THEN** the command fails with a message containing `watchexec is not installed`,
  `cargo install --locked watchexec-cli` and `without --watch`, and no `cargo` process is started

#### Scenario: Only cargo-watch installed
- **WHEN** `cargo-watch` exists in `$CARGO_HOME/bin` but `watchexec` does not exist in `PATH` or `$CARGO_HOME/bin`
- **THEN** the command fails with the missing-watchexec message and does not run `cargo watch`

### Requirement: Watch mode passes program arguments unchanged
`corvis start --watch` SHALL run watchexec without a shell (`--shell=none`) and SHALL pass `cargo`, its options and
each program argument after `--` to watchexec as separate arguments, so that each restart runs Cargo with exactly the
arguments `corvis start` would give it. Watch mode MUST accept every program argument that plain `corvis start`
accepts, with no character restriction.

#### Scenario: Argument with a space in watch mode
- **WHEN** the user runs `corvis start --watch -- --name "my app"`
- **THEN** each restart runs `cargo run -- --name "my app"` with `my app` as a single argument given to the application

#### Scenario: Shell metacharacters are not interpreted
- **WHEN** the user runs `corvis start --watch -- "a;b" "$HOME" "%PATH%"`
- **THEN** the application receives the three arguments `a;b`, `$HOME` and `%PATH%` literally

### Requirement: Watch mode ignores build output
`corvis start --watch` SHALL watch the Cargo workspace root of the current directory and MUST NOT restart because of
files written under that root's `target` folder, whether or not the project is a git repository.

#### Scenario: Project created with --skip-git
- **WHEN** `corvis start --watch` runs in a project created with `corvis new my-app --skip-git` (no `.git` folder) and
  the user saves `src/main.rs` once
- **THEN** the application is rebuilt and restarted once, and the build output under `target/` does not trigger
  further restarts

#### Scenario: Started from a subfolder
- **WHEN** the user runs `corvis start --watch` from `my-app/src` and saves `my-app/Cargo.toml`
- **THEN** the application is rebuilt and restarted

## MODIFIED Requirements

### Requirement: Watch mode runs the same command as start
`corvis start --watch` SHALL run
`watchexec --restart --shell=none --project-origin <root> --watch <root> --ignore /target/** -- cargo <run command>`,
where `<root>` is the Cargo workspace root of the current directory and `<run command>` is the same Cargo command
`corvis start` runs with the same options, without the leading `cargo`: `run`, followed by `--release` when `--release`
or `--prod` is given, followed by `--` and the program arguments when arguments follow `--` on the `corvis` command
line. `corvis` MUST NOT add any other option to watchexec.

#### Scenario: Watch without options is unchanged
- **WHEN** the user runs `corvis start --watch` in a project whose workspace root is `/src/my-app`
- **THEN** the CLI runs `watchexec --restart --shell=none --project-origin /src/my-app --watch /src/my-app --ignore
  /target/** -- cargo run`

#### Scenario: Watch with release
- **WHEN** the user runs `corvis start --watch --release` (or `corvis start --watch --prod`)
- **THEN** the watchexec command ends with `-- cargo run --release`

#### Scenario: Watch with program arguments
- **WHEN** the user runs `corvis start --watch --release -- --port 4000`
- **THEN** the watchexec command ends with `-- cargo run --release -- --port 4000`, so each restart starts the release
  build with `--port 4000`, exactly like `corvis start --release -- --port 4000` runs `cargo run --release -- --port
  4000`

### Requirement: Plain start accepts any argument
`corvis start` without `--watch` SHALL pass every argument after `--` to `cargo run` as a separate argument, without
going through a shell.

#### Scenario: Same argument without watch
- **WHEN** the user runs `corvis start -- --name "my app"`
- **THEN** the CLI runs `cargo run -- --name "my app"` with `my app` as a single argument

## REMOVED Requirements

### Requirement: Watch mode explains a missing cargo-watch
**Reason**: cargo-watch is archived and no longer used; replaced by "Watch mode explains a missing watchexec".
**Migration**: install watchexec (`cargo install --locked watchexec-cli`, `cargo binstall watchexec-cli`, or a
package manager); `corvis start --watch` keeps the same flags.

### Requirement: Watch mode rejects arguments the shell would alter
**Reason**: watchexec runs Cargo without a shell, so no argument needs to be refused; replaced by "Watch mode passes
program arguments unchanged".
**Migration**: none; commands that worked keep working, and commands that were refused now run.

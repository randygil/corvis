# Spec Delta

## ADDED Requirements

### Requirement: Watch mode runs the same command as start
`corvis start --watch` SHALL run `cargo watch -x "<cargo run command>"`, where `<cargo run command>` is the same Cargo
command `corvis start` runs with the same options, without the leading `cargo`: `run`, followed by `--release` when
`--release` or `--prod` is given, followed by `--` and the program arguments when arguments follow `--` on the
`corvis` command line. The whole Cargo command MUST be passed as a single `-x` argument, and `corvis` MUST NOT add any
other option to `cargo watch`.

#### Scenario: Watch without options is unchanged
- **WHEN** the user runs `corvis start --watch`
- **THEN** the CLI runs `cargo watch -x run`

#### Scenario: Watch with release
- **WHEN** the user runs `corvis start --watch --release` (or `corvis start --watch --prod`)
- **THEN** the CLI runs `cargo watch -x "run --release"`

#### Scenario: Watch with program arguments
- **WHEN** the user runs `corvis start --watch --release -- --port 4000`
- **THEN** the CLI runs `cargo watch -x "run --release -- --port 4000"`, so each restart starts the release build with
  `--port 4000`, exactly like `corvis start --release -- --port 4000` runs `cargo run --release -- --port 4000`

### Requirement: Program arguments are not read as CLI flags
`corvis start` and `corvis build` SHALL read their own flags (`--watch`, `--release`, `--prod`) only from the
arguments before the first `--`. Arguments after the first `--` MUST be passed to the program unchanged and in order,
and MUST NOT change the command `corvis` builds.

#### Scenario: A program flag named like a CLI flag
- **WHEN** the user runs `corvis start -- --release`
- **THEN** the CLI runs `cargo run -- --release` (debug build, `--release` given to the application)

#### Scenario: Watch flag after the separator
- **WHEN** the user runs `corvis start -- --watch`
- **THEN** the CLI runs `cargo run -- --watch` and does not start `cargo watch`

### Requirement: Watch mode rejects arguments the shell would alter
`corvis start --watch` SHALL accept only non-empty program arguments made of ASCII letters, digits and the characters
`-`, `_`, `.`, `/`, `:`, `=`, `,`, `+` and `@`, because `cargo watch` runs the `-x` command through a shell. For any
other argument the command MUST exit with a non-zero status before running Cargo, with an error that names the
argument and suggests `corvis start` without `--watch`.

#### Scenario: Argument with a space in watch mode
- **WHEN** the user runs `corvis start --watch -- --name "my app"`
- **THEN** the command fails with an error naming `my app` and suggesting `corvis start` without `--watch`, and no
  `cargo` process is started

#### Scenario: Common arguments are accepted
- **WHEN** the user runs `corvis start --watch -- --config=./dev.toml --host 127.0.0.1`
- **THEN** the CLI runs `cargo watch -x "run -- --config=./dev.toml --host 127.0.0.1"`

### Requirement: Plain start accepts any argument
`corvis start` without `--watch` SHALL pass every argument after `--` to `cargo run` as a separate argument, without
going through a shell and without the character restriction of watch mode.

#### Scenario: Same argument without watch
- **WHEN** the user runs `corvis start -- --name "my app"`
- **THEN** the CLI runs `cargo run -- --name "my app"` with `my app` as a single argument

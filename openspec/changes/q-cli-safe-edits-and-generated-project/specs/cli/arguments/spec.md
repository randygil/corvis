# Spec Delta

## ADDED Requirements

### Requirement: Unknown commands and options are refused
The CLI SHALL accept, for each command, only the flags listed for it, and MUST treat an unknown command, an unknown
flag, a flag missing its value, a repeated flag or an extra positional argument as a usage error: a message on stderr
that names the argument (with the closest command or flag when one is within two edits), status 2, and no file created
or changed. For `start` and `build`, only the arguments before the first `--` are checked.

#### Scenario: Misspelled command
- **WHEN** the user runs `corvis nwe my-app`
- **THEN** the command exits with status 2, stderr suggests `new`, and no `my-app` folder exists

#### Scenario: Flag the command does not have
- **WHEN** the user runs `corvis g co users --dry-run` in a project
- **THEN** the command exits with status 2, stderr names `--dry-run`, and no file is created or changed

#### Scenario: Flag without its value
- **WHEN** the user runs `corvis g co profile --path`
- **THEN** the command exits with status 2 and stderr says that `--path` needs a value

#### Scenario: Program arguments still pass through
- **WHEN** the user runs `corvis start -- --anything`
- **THEN** the CLI runs `cargo run -- --anything`

### Requirement: Help is printed only when asked for
`corvis` with no arguments, `corvis help`, `corvis -h` and `corvis --help` SHALL print the help on stdout and exit with
status 0. No other invocation MUST print the help in place of an error.

#### Scenario: Help flag
- **WHEN** the user runs `corvis --help`
- **THEN** the help is on stdout and the status is 0

### Requirement: Output colors follow the terminal
The CLI SHALL color its `CREATE`, `UPDATE`, `SKIP`, `NOTE` and `Error` labels only when the stream they are written to
is a terminal and the `NO_COLOR` environment variable is unset or empty.

#### Scenario: Output captured by a script
- **WHEN** `corvis g s users` runs with its output piped to a file
- **THEN** the output contains no ANSI escape sequence

#### Scenario: NO_COLOR on a terminal
- **WHEN** `corvis g s users` runs on a terminal with `NO_COLOR=1`
- **THEN** the output contains no ANSI escape sequence

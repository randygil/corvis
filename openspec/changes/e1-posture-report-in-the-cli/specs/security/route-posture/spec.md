# Spec Delta

## ADDED Requirements

### Requirement: Report mode of the application
When the environment variable `CORVIS_ROUTE_POSTURE_REPORT` is `text` or `json` (surrounding whitespace ignored),
`init()` SHALL compute the posture report regardless of the configured posture mode, write it to stdout as the text
table or as JSON, flush stdout, and end the process with exit code 0 when no entry is unprotected and 1 otherwise.
This MUST happen after route and gateway compilation, before any `OnModuleInit` or `OnApplicationBootstrap` hook runs
and before any socket is bound.

#### Scenario: Unprotected route in report mode
- **WHEN** an application with posture off and one unguarded, non-public route runs `init()` with
  `CORVIS_ROUTE_POSTURE_REPORT=text`
- **THEN** the process prints a table that lists that route as unprotected, exits with code 1, and no init hook ran

#### Scenario: JSON report with everything accepted
- **WHEN** an application whose routes are all guarded or public runs `listen(0)` with
  `CORVIS_ROUTE_POSTURE_REPORT=json`
- **THEN** the printed report parses as the JSON posture report, the process exits with code 0, and no socket was
  bound

### Requirement: The report is delimited on stdout
In report mode the report SHALL be written between the line `----- BEGIN CORVIS ROUTE POSTURE REPORT -----` and the
line `----- END CORVIS ROUTE POSTURE REPORT -----`, so it can be separated from log lines written to the same stream.

#### Scenario: Logs before the report
- **WHEN** the application logs startup lines to stdout and then runs `init()` in report mode
- **THEN** the report appears after those lines, between the two marker lines, and nothing else is between them

### Requirement: Report mode only with a valid value
An unset or empty `CORVIS_ROUTE_POSTURE_REPORT` MUST leave `init()` unchanged. Any other value than `text` or `json`
MUST make `init()` fail with an error that names the variable and the accepted values, without ending the process.

#### Scenario: Unknown value
- **WHEN** `CORVIS_ROUTE_POSTURE_REPORT=yaml` is set and the application calls `init()`
- **THEN** `init()` returns an error naming `CORVIS_ROUTE_POSTURE_REPORT`, `text` and `json`, and the process is not
  ended by the framework

#### Scenario: Variable not set
- **WHEN** the variable is unset or empty
- **THEN** `init()` behaves exactly as without this feature

### Requirement: Posture report from the CLI
`corvis posture` SHALL start the application of the current project far enough to compute the report, print it as a
table (or as JSON with `--json`), stop before the application listens or runs any init hook, and exit with a non-zero
status when any route is unprotected, whatever the posture mode of the application. It SHALL run the project like
`corvis start` (`cargo run`, honoring `--release`, passing the arguments after `--` to the application).

#### Scenario: CI gate
- **WHEN** `corvis posture --json` runs in a project with one unprotected route
- **THEN** it prints the JSON report, the application does not listen on any port, and the command exits with a
  non-zero status

#### Scenario: Clean project
- **WHEN** `corvis posture` runs in a project whose routes are all guarded, public or exempt
- **THEN** it prints the table and exits with status 0

### Requirement: CLI output and exit status
`corvis posture` SHALL write only the report to its stdout and send Cargo output and the application's other output
to its stderr. Its exit status MUST be 0 when nothing is unprotected, 1 when something is, and 2, with an explanatory
message on stderr, when the application ends without printing a complete report.

#### Scenario: Output stays parseable
- **WHEN** the application logs startup lines to stdout before `init()` and `corvis posture --json` runs
- **THEN** the command's stdout contains only the JSON report

#### Scenario: Application that fails to start
- **WHEN** `corvis posture` runs in a project whose application returns a dependency injection error before `init()`
- **THEN** the command prints on stderr that the application ended without producing a posture report, with the
  application's exit code, and exits with status 2

### Requirement: Timeout for applications that never report
`corvis posture` SHALL stop the application and exit with status 2 when it has not printed a complete report within
`--timeout <seconds>` (default 120) after it started running. The time spent building MUST NOT count.

#### Scenario: Application that never reaches init
- **WHEN** `corvis posture --timeout 3` runs in a project whose `main` blocks before the application initializes
- **THEN** the command stops the application and its child processes, prints on stderr that no report arrived within
  3 seconds, and exits with status 2

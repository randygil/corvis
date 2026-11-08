# Proposal

Implements [randygil/corvis#28](https://github.com/randygil/corvis/issues/28) (epic
[#24](https://github.com/randygil/corvis/issues/24)). The design was settled in the route posture spike (#25,
`openspec/changes/archive/2026-11-08-e1-route-posture-design/`, decisions 2, 11 and 12, the review notes, and task
group 4). This change implements that task group.

## Why

The startup check of #26 stops an application with an unprotected route, and `app.route_posture()` returns the full
report from code. Code review and CI need the same report without writing a test harness: a command that lists every
route with its guards (or `public`, or the exemption and its reason) and fails when anything is unprotected, whatever
posture mode the application uses. New projects should also start with the check on, so the first route someone adds
without a guard is caught on the first run.

## What Changes

- **Report mode in the framework.** When the environment variable `CORVIS_ROUTE_POSTURE_REPORT` is `text` or `json`,
  `init()` computes the posture report right after the posture check point (after route and gateway compilation),
  whatever the configured posture mode. It prints the report to stdout between two marker lines, flushes, and exits the
  process with code 0 when nothing is unprotected and 1 otherwise. It does this before any `OnModuleInit` /
  `OnApplicationBootstrap` hook and before any socket is bound. Any other non-empty value makes `init()` fail with an
  error naming the accepted values.
- **`corvis posture [--json] [--release] [-- <program arguments>]`** in `corvis-cli`. It runs the project with
  `cargo run` (the same resolution as `corvis start`) and the variable set. It prints only the report on its stdout
  (application logs and Cargo output go to stderr) and exits with the application's code: 0 when nothing is
  unprotected, 1 when something is. When the application ends without printing a report (build failure, startup
  error, a framework version without report mode), it prints an explanation and exits with 2.
- **`corvis new` enables the check.** The generated `main` calls `app.require_route_posture()` and the generated
  `get_hello` route is marked `#[public]`, so a new project starts, passes `corvis posture`, and refuses to start once
  a route without a guard or `#[public]` is added.
- `--timeout <seconds>` (default 120): an application that never reaches `init()` is stopped and the command exits
  with 2 (review note of 2026-11-08).
- Help text, the CLI usage page (English and Spanish) and tests.

## Capabilities

### New Capabilities

- `cli/new`: what `corvis new` generates. This change adds only the route posture requirement.

### Modified Capabilities

- `security/route-posture`: adds the report mode contract and the CLI report (the spike's requirement "Posture report
  from the CLI", copied and refined). The capability is created by #26 (`e1-public-metadata-and-startup-check`). This
  delta only adds requirements.

## Impact

- Code: `crates/corvis/src/application.rs` (`init()`: the report-mode branch), `crates/corvis/src/posture.rs`
  (created by #26: marker constants and the report-mode function), `crates/corvis-cli/src/main.rs` (`posture`
  command, help), `crates/corvis-cli/src/templates.rs` (`MAIN`, `APP_CONTROLLER`).
- Depends on #26 (report model, `route_posture()`, `require_route_posture()`, `#[public]`). It should land after #27
  so the report already includes SSE labels and gateway handlers. If it lands before #27, nothing in this change needs
  to be redone: report mode prints whatever report `init()` computed.
- Tests: `crates/corvis/tests/route_posture.rs` (created by #26) gains a report-mode test that re-runs the test binary
  as a child process; `crates/corvis-cli` gains unit tests for argument parsing and report extraction, and a test of
  the generated templates.
- Docs (EN + ES): `cli/usages.md` (`corvis posture` section), `cli/overview.md` (command list), `first-steps.md`
  (it shows the generated `main.rs`, lines 69-91).
- Port apps: report mode only runs with the variable set; nothing changes for them. The parity check runs because
  `init()` changes.

## Out of scope

- The Security page about route posture and the `tertulia` example (issue #29).
- Choosing the binary in a multi-binary package (`--bin`); `corvis start` does not support it either. Possible new
  issue for both commands.
- `--watch` for `corvis posture`.
- Adding `#[public]` or guards to the controllers produced by `corvis generate resource` (they are generated without
  either. In a new project the application then refuses to start until one is added, and the error explains how).
  Possible follow-up with #20.
- Turning the check on by default for every application (tied to #8).

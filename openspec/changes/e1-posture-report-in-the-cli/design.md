# Design

## Context

The route posture spike (#25) settled the design. This change implements its decision 12 (CLI report mode) and the
`corvis new` part of decision 2, on top of the report model of decision 11, which #26 implements. Review notes of the
spike that apply here: the `corvis new` template turns the check on, and the report environment variable and its
contract (`CORVIS_ROUTE_POSTURE_REPORT=text|json`, exit code 1 when anything is unprotected) are accepted.

Current code, read in this worktree (before #26):

- **`init()`** (`crates/corvis/src/application.rs:361-423`) folds the provider globals (`:368-398`), builds the router
  (`:399`), middleware (`:400`) and gateways (`:401-402`), stores `AppShared` (`:403-417`), discovers handlers
  (`:418-419`) and runs the init hooks (`:420`). #26 adds the posture check between `:402` and `:403` and keeps the
  report available through `app.route_posture()`. `listen()` / `listen_in_background()` call `bind()`, which calls
  `init()` before binding (`:438-491`).
- **Logs go to stdout.** `ConsoleLogger::log` writes JSON lines and non-error text lines to stdout and only error and
  fatal lines to stderr (`crates/corvis/src/logger.rs:128-157`). `bootstrap` logs "Starting Corvis application..."
  (`application.rs:107`) and the instance loader logs one line per module (`crates/corvis/src/di/container.rs:415-417`),
  all before `init()`. A report printed to stdout by the application is therefore mixed with log lines.
- **CLI.** `crates/corvis-cli` is a standalone binary with no dependency on `corvis` (`crates/corvis-cli/Cargo.toml`,
  empty `[dependencies]`). `main` dispatches on the first argument and maps `Result<(), String>` to
  `ExitCode::SUCCESS`/`FAILURE` (`crates/corvis-cli/src/main.rs:29-56`). `corvis start` is
  `cmd_cargo(args, "run")` (`:34`, `:332-349`): `cargo run`, `--release`/`--prod`, arguments after `--` forwarded,
  inherited stdio, failure reported as "cargo run failed". Help text is `print_help` (`:58-77`). The CLI's
  integration tests run the built binary (`crates/corvis-cli/tests/cli_input.rs:14-17`, `CARGO_BIN_EXE_corvis`).
- **Templates.** `templates::MAIN` creates the app with `let app = CorvisFactory::create::<AppModule>().await?;` and
  calls `app.listen(port)` (`crates/corvis-cli/src/templates.rs:32-46`). `APP_CONTROLLER` declares `get_hello` with
  `#[get]` and no guard (`:60-76`). Its generated test builds a testing application and calls `init()` (`:78-95`),
  which does not enable posture. `cmd_new` writes these files (`main.rs:156-183`).
- **Docs.** `docs/cli/usages.md` documents each command (`## corvis start` at `:78`, `## corvis build` at `:93`);
  `docs/cli/overview.md:51-60` has the command table; `docs/first-steps.md:69-91` shows the generated `main.rs`.
- **Parallel work.** #20 (`e13-cli-defaults-that-contradict-the-docs`) edits `cmd_cargo` and `print_help` in the same
  file; whichever lands second rebases.

## Goals / Non-Goals

**Goals:**
- One command that prints the same report the startup check computes, usable in CI (`--json`, meaningful exit codes).
- The command never leaves a server running and never runs init hooks.
- `corvis posture --json | jq` works even though the application logs to stdout.
- New projects have the check on from the first run.

**Non-Goals:**
- Static analysis of sources (rejected in the spike, decision 12).
- Documenting route posture as a whole (#29) or changing the report model (#26).

## Decisions

1. **The report-mode branch sits at the posture check point.** In `init()`, where #26 runs the check, read
   `CORVIS_ROUTE_POSTURE_REPORT` first. If it is `text` or `json` (trimmed, case-sensitive), compute the report
   regardless of the mode, write it, flush stdout and call `std::process::exit(code)`. Enforce or warn handling is
   skipped in that run. Unset or empty: unchanged. Any other value: `init()` returns an error such as
   ``CORVIS_ROUTE_POSTURE_REPORT must be "text" or "json" (got "yaml")``, which `main` turns into a non-zero exit with
   no report, so the CLI reports it as "no report" (decision 4). The pure parts live in `crates/corvis/src/posture.rs`:
   `report_format_from_env(Option<&str>) -> Result<Option<ReportFormat>, ..>` and
   `render_report_mode(&RoutePostureReport, ReportFormat) -> (String, i32)`, so they are unit-tested without
   spawning processes. `init()` only calls them and exits.
   - Alternative: an explicit call in `main` (`app.print_posture_report()`). Rejected in the spike's review (open
     question 4 accepted the variable): it needs every project to add code before the CLI works.
   - Gateway `after_init` hooks have already run at that point (`build_gateways` calls them, see the spike's context).
     They are not lifecycle hooks of the module system, and the process exits right after. Documented.

2. **Marker lines around the report.** The report is written as
   `----- BEGIN CORVIS ROUTE POSTURE REPORT -----`, the table or pretty JSON, `----- END CORVIS ROUTE POSTURE REPORT
   -----`. The markers are public constants in `corvis::posture` (`REPORT_BEGIN`, `REPORT_END`) and duplicated in the
   CLI, with a test in each crate pinning the literal text. The JSON is exactly `serde_json::to_string_pretty` of the
   report type from #26 (one model for the check, the API and the CLI, spike decision 11).
   - Why: logs share stdout (Context). Without markers the CLI cannot separate the report from log lines, and
     `--json` output would not parse.
   - Alternative: write the report to a file named by a second variable. Rejected: a second contract to maintain,
     temporary files to clean up, and the spike accepted stdout.
   - Alternative: silence the logger in report mode. Rejected: the variable is read in `init()`, after `bootstrap` has
     already logged, and user code may print too.

3. **`corvis posture` spawns `cargo run` and filters its stdout.** New `cmd_posture(args) -> ExitCode` in
   `crates/corvis-cli/src/main.rs`, dispatched before the `Res` mapping so it can return 0/1/2. It builds the same
   command as `corvis start` without `--watch` (`cargo run`, `--release`/`--prod`, arguments after `--`), sets
   `CORVIS_ROUTE_POSTURE_REPORT` to `json` with `--json` and `text` otherwise, inherits stderr and pipes stdout. A pure
   function `split_report(lines) -> (report: Option<String>, other: Vec<String>)` (unit-tested) separates the report.
   Other lines are forwarded to stderr as they arrive. The report is printed to stdout when the END marker is read.
   `--watch` is rejected with a message.
   - Alternative: reuse `cmd_cargo`. Rejected: it inherits stdout and collapses exit codes into one failure.

4. **Exit codes.** Report complete and child exit code 0 or 1: forward it. No complete report (no BEGIN or no END),
   whatever the child's code (Cargo build failure is usually 101, a startup `Err` from `main` is 1): print
   `the application ended (exit code N) before printing the route posture report: check the build and startup output
   above` to stderr and exit 2. A report followed by any other exit code is also treated as 2. So CI can tell
   "unprotected routes" (1) from "could not check" (2).
   - Alternative: forward the child's code unchanged. Rejected: a startup failure (exit 1) would look like
     "unprotected routes".

5. **Template.** `MAIN` becomes `let mut app = CorvisFactory::create::<AppModule>().await?;` followed by
   `app.require_route_posture();` and the existing port/listen lines. `APP_CONTROLLER` adds `#[public]` above
   `#[get]` on `get_hello`, with a one-line comment that routes need a guard or `#[public]` because `main` enforces
   route posture. The generated test is unchanged (testing applications do not enable posture).

6. **Tests.**
   - Framework: in `crates/corvis/tests/route_posture.rs` (created by #26), a parent test re-runs the current test
     binary (`std::env::current_exe()`, `--exact <child test> --nocapture`) with `CORVIS_ROUTE_POSTURE_REPORT` and a
     private variable selecting the fixture. The child test returns immediately unless that private variable is set.
     When set, it builds an app (one unguarded route, an `OnModuleInit` provider that prints `HOOK RAN`), calls
     `listen_in_background(0)` and then prints `AFTER LISTEN`. The parent asserts the exit code (1, and 0 for a fixture
     with `#[public]`), that the text between the markers lists the route (and parses as JSON in the JSON case), and
     that neither `HOOK RAN` nor `AFTER LISTEN` appears. No extra binary or example is needed.
   - Unit tests for `report_format_from_env`, `render_report_mode` and the CLI's `split_report` and argument handling.
   - Templates: a unit test in `corvis-cli` asserting `MAIN` contains `require_route_posture()` and `APP_CONTROLLER`
     contains `#[public]`. The end-to-end run of a generated project (`corvis new`, `cargo run`, `corvis posture`) is a
     manual verification step, because building a fresh project in the test suite is too slow for the shared target
     directory.

## Risks / Trade-offs

- [An application that never calls `init()` (or uses a framework version without report mode) starts serving and
  `corvis posture` never returns] → documented. A timeout option is an open question.
- [The process exits from inside `init()`, without running destructors or shutdown hooks] → only with the variable
  set, documented as a development tool (accepted in the spike). Stdout is flushed before exiting.
- [Bootstrap side effects run (DI factories, database connections)] → same environment as `corvis start` is needed;
  documented (spike decision 12).
- [Raw `upgrade` listeners are not counted in report mode] → they are registered in hooks that do not run. The report
  shows them as "not counted" (spike decision 10).
- [Merge conflict with #20 in `main.rs`] → small, mechanical.

## Open questions

1. The spike accepted "prints it to stdout". Are the marker lines an acceptable refinement of that contract?
   (Proposed: yes; without them `--json` cannot be parsed reliably.)
2. Exit code 2 for "no report produced": acceptable, or should the CLI use a different convention?
3. Should `corvis posture` get a `--timeout <seconds>` (time after the build finishes) for applications that never
   reach `init()`? Proposed: not in this change; open an issue if it happens in CI.

## Review notes (2026-11-08)

- Marker lines and exit code 2 for "no report" are accepted.
- Add `--timeout <seconds>` (default 120) so an app that never reaches `init()` does not hang CI.

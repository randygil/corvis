# Tasks

Prerequisite: #26 (`e1-public-metadata-and-startup-check`) is merged (report model, `route_posture()`,
`require_route_posture()`, `#[public]`, `crates/corvis/tests/route_posture.rs`). Preferably #27 too, so the report
already contains SSE labels and gateway handlers.

## 1. Report mode in the framework

- [x] 1.1 In `crates/corvis/src/posture.rs`, add `REPORT_BEGIN` / `REPORT_END` constants, `ReportFormat`,
  `report_format_from_env(Option<&str>)` (unset or empty → `None`; `text`/`json` after trimming; anything else →
  error naming the variable and both values) and `render_report_mode(&RoutePostureReport, ReportFormat) -> (String,
  i32)` (markers + table or pretty JSON; code 0 or 1); verify with unit tests in the same file (all values, both
  formats, code for a report with and without unprotected entries, literal marker text)
- [x] 1.2 In `init()` (`crates/corvis/src/application.rs`, at the posture check point #26 added after
  `build_gateways`), read `CORVIS_ROUTE_POSTURE_REPORT` through `report_format_from_env`; in report mode, write the
  rendered report to a locked stdout, flush, and `std::process::exit(code)`, skipping enforce/warn handling; return the
  error for invalid values; verify with task 1.3
- [x] 1.3 In `crates/corvis/tests/route_posture.rs`, add the child-process tests of design decision 6: text mode with an
  unguarded route (exit 1, route listed, no `HOOK RAN`, no `AFTER LISTEN`), JSON mode with only public/guarded routes
  (exit 0, JSON between the markers parses), invalid value (`init()` returns the error, no markers, process not ended by
  the framework), unset variable (no change); verify `cargo test -p corvis --test route_posture report_mode`

## 2. `corvis posture`

- [x] 2.0 (review note) `--timeout <seconds>`, default 120: see the implementation notes in design.md

- [x] 2.1 Add `split_report` to `crates/corvis-cli/src/main.rs` (or a new `posture.rs` module) with marker constants
  equal to the framework's; verify unit tests: report with surrounding log lines, missing END, missing BEGIN, two
  reports (first one wins), Windows line endings
- [x] 2.2 Add `cmd_posture(args) -> ExitCode`: spawn `cargo run` (`--release`/`--prod`, arguments after `--`) with the
  variable set to `json` with `--json` and `text` otherwise, stderr inherited, stdout piped and filtered by
  `split_report` (other lines to stderr as they arrive), exit codes per design decision 4, `--watch` rejected; dispatch
  `posture` in `main` before the `Res` mapping; verify unit tests of the argument handling and of the exit-code decision
  (pure function from "report found" + child code to the CLI code)
- [x] 2.3 Add `posture [--json] [--release] [-- args]` to `print_help` and the module doc comment at the top of
  `main.rs`; verify `cargo run -p corvis-cli -- --help` shows it and `crates/corvis-cli/tests/cli_input.rs` still passes
- [x] 2.4 Manual verification against `examples/tertulia` (after #29 enables posture there, or with a temporary
  `require_route_posture()` call): `corvis posture` and `corvis posture --json` from `examples/tertulia` (exit 0, report
  on stdout only, `| jq .` works), then with one `#[public]` removed (exit 1), then with a deliberate compile error
  (exit 2 and the explanation); use the shared target directory; record the outputs in the PR description

## 3. `corvis new`

- [x] 3.1 Update `templates::MAIN` (`let mut app`, `app.require_route_posture();`) and `templates::APP_CONTROLLER`
  (`#[public]` on `get_hello` with the one-line comment) in `crates/corvis-cli/src/templates.rs`; verify with a unit
  test asserting both strings
- [x] 3.2 Manual verification: `corvis new posture-demo --skip-git` in a temporary directory (pointing at this checkout
  with `--corvis-path`), then `cargo test`, `cargo run` (starts, `GET /` answers) and `corvis posture` (exit 0, `GET /`
  public); add an unguarded route and check that `cargo run` refuses to start naming it; use the shared target
  directory; record the result in the PR description

## 4. Documentation

- [x] 4.1 `docs/cli/usages.md`: new section `## corvis posture` after `corvis start` (synopsis, what it runs, stdout vs
  stderr, exit codes 0/1/2, `--json` for CI, needs the same environment as `corvis start`, never listens or runs init
  hooks, raw `upgrade` listeners not counted, link to the route posture page once #29 adds it); same section in
  `docs/es/cli/usages.md`; verify `npm run docs:check` in `docs/`
- [x] 4.2 `docs/cli/overview.md` command table: add `corvis posture`; `docs/first-steps.md` `main.rs` block and its
  bullet list: `let mut app`, `require_route_posture()` and one bullet explaining it (with `#[public]` on the generated
  route); same in `docs/es/`; verify `npm run docs:check` (same headings and code fences in both languages)

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes
- [x] 5.4 Regression tests: every scenario of `specs/security/route-posture/spec.md` and `specs/cli/new/spec.md` maps
  to an automated test or to a recorded manual verification (tasks 2.4 and 3.2); list the mapping in the PR description
- [x] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/` passes
- [x] 5.6 Framework runtime code changed (`init()`): run the `corvis-parity` check and verify the three port apps
  (consi, facten, houseplan) keep 0 apidiff differences (not run by the implementer: the maintainer runs it) (maintainer batch run on batch/phase05-h 66ff09a: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)

## 6. Security review fixes (L1 to L6)

- [x] 6.1 L1: reader thread plus deadline in the main thread, drain grace after the application exits, Job Object (Windows) or process group (Unix) killed at timeout and at exit, Ctrl-C forwarded on Unix; verified with the fake-cargo tests (`crates/corvis-cli/tests/cli_posture.rs`)
- [x] 6.2 L2: per-run nonce in the markers, last complete report wins, exit status from the report; unit and fake-cargo tests
- [x] 6.3 L3: report mode fills `unchecked` (static mounts, handshakes with unknown `verify_client`, upgrade listeners not counted); content test
- [x] 6.4 L4: exit codes 40/41, testing-module applications ignore report mode, production and test-job warnings in the docs
- [x] 6.5 L5: limits documented (first `init()` only, default binary only)
- [x] 6.6 L6: `corvis g guard` template denies by default
- [x] 6.7 Informational: exemptions validated in report mode
- [x] 6.8 Definition of done again (fmt, clippy, tests, naming, docs:check, examples smoke)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #28, tick it in epic #24 and move its card to Done (see the `corvis-task` skill).

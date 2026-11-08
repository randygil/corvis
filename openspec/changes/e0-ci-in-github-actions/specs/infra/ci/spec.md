# Spec Delta

## ADDED Requirements

### Requirement: Every pull request runs the definition-of-done checks

The repository SHALL run a CI workflow on every pull request targeting `main`, and a maintainer SHALL be able to start
it by hand. A pull request MUST NOT be considered mergeable unless all of its jobs pass: `fmt` (`cargo fmt --all
--check`), `clippy` (`-D warnings`), `test` (build with warnings denied, then `cargo test --workspace`), `fuzz`, `deny`
(`cargo deny check`), `audit` (`cargo audit`) and `docs` (`npm run docs:check` in `docs/`).

#### Scenario: A clippy warning fails the pull request
- **WHEN** a pull request introduces code that produces a clippy warning
- **THEN** the `clippy` job fails and the pull request shows a failed required check

#### Scenario: A compiler warning fails the pull request
- **WHEN** a pull request introduces an unused import in any workspace crate, example or test
- **THEN** the build step of the `test` job fails because warnings are denied

#### Scenario: A broken docs link fails the pull request
- **WHEN** a pull request adds an English page without its Spanish counterpart under `docs/es/`
- **THEN** the `docs` job fails with the `[pages]` message of `docs/scripts/check.mjs`

#### Scenario: A manual run on main
- **WHEN** a maintainer starts the CI workflow by hand on `main`
- **THEN** every job runs as on a pull request

### Requirement: CI runs only on pull requests and manual dispatch

Because the repository is private and Actions minutes are limited, the workflows SHALL be triggered only by
`pull_request` (targeting `main`) and `workflow_dispatch`. They MUST NOT have a `push` or `schedule` trigger. The long
checks (100,000-case fuzzing and `cargo audit` against the latest advisory database) SHALL run in a separate workflow
that is started by hand.

#### Scenario: Workflow triggers
- **WHEN** a maintainer inspects the files under `.github/workflows/`
- **THEN** the only `on:` keys are `pull_request` and `workflow_dispatch`

#### Scenario: A merge to main
- **WHEN** a pull request is merged into `main`
- **THEN** no workflow run starts

### Requirement: Builds, lints and tests run on Linux and Windows

The `clippy` and `test` jobs SHALL run on both a Linux runner and a Windows runner, because the workspace contains
platform-specific code (`#[cfg(unix)]` in `crates/corvis/src/application.rs`, `#[cfg(windows)]` in
`crates/corvis/tests/transport_security.rs`) and the Windows checkout uses CRLF line endings by default. The Rust
toolchain version SHALL be the same on both runners and set in one place in the workflow.

#### Scenario: A Windows-only failure is caught
- **WHEN** a test passes on Linux but fails on Windows (for example, an embedded asset checked out with CRLF)
- **THEN** the Windows `test` job fails and the pull request is not green

#### Scenario: A toolchain bump is explicit
- **WHEN** a new stable Rust adds lints that the code does not satisfy
- **THEN** CI keeps passing on the pinned toolchain until a pull request changes the toolchain version

### Requirement: Postgres tests cannot pass by skipping in CI

The corvis-orm Postgres test helper SHALL fail the test, instead of skipping it, when the variable
`CORVIS_ORM_PG_REQUIRED` is set to `1` and the server at `CORVIS_ORM_PG` is not reachable. The Linux `test` job MUST
run a PostgreSQL service, point `CORVIS_ORM_PG` at it and set `CORVIS_ORM_PG_REQUIRED=1`. Without the variable, the
current behavior (skip with a message) MUST stay unchanged for local runs.

#### Scenario: The Postgres service is down in CI
- **WHEN** the Linux `test` job runs with `CORVIS_ORM_PG_REQUIRED=1` and the Postgres service is not reachable
- **THEN** the tests of `pg_features`, `pg_oracle`, `pg_json_paths` and the Postgres tests of `security` fail with a
  message naming `CORVIS_ORM_PG`, instead of returning early

#### Scenario: A local run without Postgres
- **WHEN** a developer runs `cargo test -p corvis-orm` without `CORVIS_ORM_PG_REQUIRED` and without a server
- **THEN** the Postgres tests print "skipping: Postgres not reachable" and pass, as today

### Requirement: Fuzzing runs on every pull request and longer on demand

CI SHALL run the property-based fuzzing suites of `corvis` and `corvis-orm` on every pull request with more cases than
the 2,000-case default, within a fixed time limit, and with `PROPTEST_CASES=100000` in the manually started
long-checks workflow. When a property fails, the job MUST upload the crates' `proptest-regressions/` folders as an
artifact, so the shrunk seed can be committed.

#### Scenario: A property fails in CI
- **WHEN** a pull request makes the static-file property serve a dotfile
- **THEN** the `fuzz` job fails and its artifacts contain the new seed under `crates/corvis/proptest-regressions/`

#### Scenario: Manual long run
- **WHEN** a maintainer starts the long-checks workflow on `main`
- **THEN** every fuzzing suite runs with 100,000 cases and a failure is reported on the workflow run

### Requirement: Dependency policy and advisories are enforced

CI SHALL run `cargo deny check` with the repository's `deny.toml` and `cargo audit` with `.cargo/audit.toml`, and
MUST fail on any error those tools report. Exceptions MUST live only in those two files, each with a written reason;
the workflow MUST NOT pass extra ignore flags. `cargo audit` SHALL also run in the manually started long-checks
workflow against the latest advisory database.

#### Scenario: A new advisory affects a dependency
- **WHEN** RustSec publishes an advisory for a crate in `Cargo.lock` that is not listed in `.cargo/audit.toml`
- **THEN** the `audit` job of the next pull request fails, and so does the `audit` job of the next manual long run

#### Scenario: A dependency with a disallowed license
- **WHEN** a pull request adds a dependency whose license is not in `deny.toml` `[licenses] allow`
- **THEN** the `deny` job fails

### Requirement: CI runs with least privilege

The workflows SHALL request only `contents: read` permission, SHALL NOT use repository secrets in jobs triggered by
pull requests, and SHALL pin third-party actions to a full commit SHA. Runs for the same branch SHALL cancel the
previous in-progress run.

#### Scenario: Workflow permissions
- **WHEN** a maintainer inspects `.github/workflows/ci.yml`
- **THEN** the top-level `permissions` block is `contents: read` and every `uses:` of a non-`actions/*` action names a
  40-character commit SHA

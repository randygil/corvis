# infra/ci Specification

## Purpose
What the GitHub Actions CI runs, when it runs, on which platforms, and with which permissions.

## Requirements

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

### Requirement: Every corvis feature builds on its own

CI SHALL check the `corvis` library with clippy and `-D warnings` with no default features and with each of its
features enabled alone, on Linux and Windows.

#### Scenario: Library without default features

- **WHEN** CI runs `cargo clippy -p corvis --no-default-features -- -D warnings`
- **THEN** it passes

#### Scenario: One feature alone

- **WHEN** CI runs the same check with `--features jwt`, `--features websockets`, `--features health-tls` or
  `--features tls`
- **THEN** each passes

### Requirement: Postgres test databases and roles are unique per run and cleaned up

Every scratch database and role that the corvis-orm Postgres tests create on the shared server SHALL carry a per-run
suffix made of a timestamp, the process id and a random component (`orm_test_<name>_<run>` and `orm_<name>_<run>`),
within Postgres' 63-byte identifier limit, so two runs against one server never share a name. Tests MUST get these
names from the shared helper instead of building them by hand. A run SHALL drop the databases and roles it created when
its process exits normally, and the helper MUST drop leftover `orm_test_*` databases and `orm_*` roles of earlier runs
once they are older than 3 hours, judged by the timestamp in the name, never touching other objects.

#### Scenario: Two runs at the same time
- **WHEN** two `cargo test -p corvis-orm` runs use the same Postgres server at the same time
- **THEN** each creates and drops only its own databases and roles, and both pass

#### Scenario: A run ends normally
- **WHEN** a run of the Postgres tests finishes and every test passed
- **THEN** the databases and roles it created no longer exist on the server

#### Scenario: Leftovers of a dead run
- **WHEN** a Postgres test starts and the server holds an `orm_test_*` database or `orm_*` role whose run id is older
  than 3 hours
- **THEN** it is dropped, while objects with a recent run id or without a run id are left alone

### Requirement: TLS tests run in CI

The Linux and Windows test jobs SHALL run the TLS integration tests with the `tls` feature enabled
(`cargo test -p corvis --features tls --test tls`), in addition to `cargo test --workspace`.

#### Scenario: A TLS regression

- **WHEN** a pull request breaks HTTPS serving
- **THEN** the TLS test step fails on both Linux and Windows

### Requirement: CI runs on pull requests, pushes to main and dev, and a nightly schedule

The CI workflow SHALL be triggered by `pull_request` (targeting `main` or `dev`), by `push` to `main` or `dev` and by
`workflow_dispatch`. A newer push to the same pull request SHALL cancel the run in progress; runs on `main` and `dev`
MUST NOT be cancelled by a newer push.

#### Scenario: Workflow triggers
- **WHEN** a maintainer inspects `.github/workflows/ci.yml`
- **THEN** it has the `pull_request` and `push` triggers (branches `main` and `dev`) and `workflow_dispatch`

#### Scenario: A push to dev
- **WHEN** a commit is pushed to `dev` or a pull request is merged into `main`
- **THEN** the CI workflow runs on that commit and is not cancelled by a later push

### Requirement: The long checks run every day on dev

The long checks (100,000-case fuzzing and `cargo audit` against the latest advisory database) SHALL run in a separate
workflow triggered by a daily `schedule` and by `workflow_dispatch`. A scheduled run SHALL check out `dev` and use
100,000 cases per property.

#### Scenario: A scheduled run
- **WHEN** the daily schedule of `nightly.yml` fires
- **THEN** both jobs check out `dev`, and the fuzzing suites run with `PROPTEST_CASES=100000`

### Requirement: Changes reach main only through checked pull requests

Work SHALL land on `dev`, the integration branch, and reach `main` through a pull request. The `main` branch SHALL be
protected so that the CI jobs (`fmt`, `clippy` on Linux and Windows, `test` on Linux and Windows, `fuzz`, `deny`,
`audit`, `docs`, `packaging`) are required status checks, enforced for administrators too, with force pushes and
deletion blocked. A direct push to `main` MUST be rejected. `dev` SHALL block force pushes and deletion.

#### Scenario: A direct push
- **WHEN** a maintainer pushes a commit straight to `main`
- **THEN** GitHub rejects the push because the required status checks have not passed on it

#### Scenario: A pull request with a failing check
- **WHEN** a required job fails on a pull request
- **THEN** the pull request cannot be merged until the job passes

# Proposal

Implements [randygil/corvis#6](https://github.com/randygil/corvis/issues/6) (epic [#1](https://github.com/randygil/corvis/issues/1),
Phase 0.2 of the execution plan [#111](https://github.com/randygil/corvis/issues/111)).

## Why

Every check in the definition of done (no warnings, clippy, tests, fuzzing, dependency policy, docs checks) runs only
when someone remembers to run it, on one Windows machine. Clippy already drifted red without anyone noticing (#108),
and a Windows-only line-ending bug broke three tests (#110). The Postgres tests of corvis-orm skip themselves silently
when no server is reachable, so a green local run says nothing about Postgres. CI makes every pull request prove the
same checks, on Linux and Windows, with a real Postgres.

## What Changes

- New workflow `.github/workflows/ci.yml`, run on every pull request to `main` and every push to `main`:
  - `fmt`: `cargo fmt --all --check` (Linux).
  - `clippy`: `cargo clippy --workspace --all-targets -- -D warnings` on Linux and Windows.
  - `test`: `cargo build --workspace --all-targets` with warnings denied, then `cargo test --workspace`, on Linux
    (with a Postgres service container) and Windows (without Postgres).
  - `fuzz`: a short proptest run of the three fuzzing suites with a raised `PROPTEST_CASES` (Linux), uploading
    `proptest-regressions/` when a property fails.
  - `deny`: `cargo deny check` with the existing `deny.toml`.
  - `audit`: `cargo audit` with the existing `.cargo/audit.toml`.
  - `docs`: `npm ci` and `npm run docs:check` in `docs/`.
- New scheduled workflow `.github/workflows/nightly.yml`: the long fuzzing run (`PROPTEST_CASES=100000`) and
  `cargo audit` against the latest advisory database, so new advisories show up without a pull request.
- `crates/corvis-orm/tests/common/pg.rs`: a new opt-in variable `CORVIS_ORM_PG_REQUIRED=1` turns "Postgres not
  reachable, skipping" into a test failure; CI sets it on Linux so the Postgres suites cannot pass by skipping.
- A `rustfmt.toml` that matches the current code style, plus one formatting-only commit (listed in
  `.git-blame-ignore-revs`), so `cargo fmt --check` can be enforced without rewriting 3,000+ long lines by hand.
- `CONTRIBUTING.md`: what CI runs, how to reproduce each job locally, and the new environment variable.

## Capabilities

### New Capabilities

- `infra/ci`: the checks every pull request must pass, on which platforms, with which services, and how fuzzing and
  dependency policy run in CI.

### Modified Capabilities

None.

## Impact

- New files: `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`, `rustfmt.toml`, `.git-blame-ignore-revs`.
- Test helper only: `crates/corvis-orm/tests/common/pg.rs` (no framework runtime code changes).
- Whole-workspace formatting commit (no behavior change; verified by an unchanged `cargo test --workspace`).
- `CONTRIBUTING.md` updated.
- Depends on #108 (clippy green) and #110 (LF for embedded Swagger assets) being merged first; both are in Phase 0.1.
- GitHub Actions minutes: the repository is private, and Windows minutes count double.

## Out of scope

- Coverage-guided fuzzing with `cargo fuzz` and sanitizers on nightly (no fuzz targets exist yet): new issue.
- Fixing `pg_houseplan.rs`, which also skips when the external `corvis-tests` folder is missing: vendoring its
  migrations is a separate issue.
- Running the Postgres suites on Windows (the runner's preinstalled PostgreSQL): possible follow-up.
- Porting the JSON-filter differential property to Postgres: that is #109, which can rely on this CI.
- An MSRV job, release automation, crate publishing (#10), docs deployment (#5, done through Coolify), Dependabot or
  Renovate for actions and crates, README badges (#3).
- The corvis-parity apidiff suites in CI: they need the Node originals and the external `corvis-tests` folder.
- Turning `deny.toml` `multiple-versions` / `wildcards` warnings into errors (tied to #10).

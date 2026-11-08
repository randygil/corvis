# Tasks

## 1. Preconditions

- [ ] 1.1 Confirm #108 (clippy under Rust 1.99) and #110 (LF for embedded Swagger assets, `.gitattributes`) are merged into `main`; verify locally that `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass on Windows from a fresh checkout
- [ ] 1.2 Re-run `cargo deny check` and `cargo audit` on `main` and record the result (2026-11-08 baseline: both pass, only duplicate/wildcard warnings from `cargo deny`); fix or document any new finding in `deny.toml` / `.cargo/audit.toml` with a reason; verify both exit 0

## 2. Formatting baseline

- [ ] 2.1 Install rustfmt for the pinned toolchain (`rustup component add rustfmt`); for each candidate config (no file; `max_width = 120/140/160` with `use_small_heuristics = "Max"`), count the lines `cargo fmt --all --check` would change; record the numbers in the pull request description and pick the smallest (stable options only); verify the chosen `rustfmt.toml` is committed at the root
- [ ] 2.2 With no other change in flight, run `cargo fmt --all` and commit it alone ("Format the workspace with rustfmt"); add that commit's SHA to a new `.git-blame-ignore-revs`; verify `cargo fmt --all --check` exits 0 and `cargo test --workspace` still passes (no behavior change)

## 3. Postgres tests that cannot skip in CI

- [ ] 3.1 In `crates/corvis-orm/tests/common/pg.rs`, read `CORVIS_ORM_PG_REQUIRED`; when it is `1`, make every early `None` in `fresh_db` (admin connection, `DROP DATABASE`, `CREATE DATABASE`) panic with the error and the server host, never the password; update the module comment; verify that `CORVIS_ORM_PG_REQUIRED=1 CORVIS_ORM_PG=postgres://postgres:postgres@127.0.0.1:5299 cargo test -p corvis-orm --test pg_features` fails with that message, and that without the variable it still passes by skipping
- [ ] 3.2 Add a regression test of the helper itself (unreachable URL + variable set → panic, message has no password); verify with `cargo test -p corvis-orm --test pg_features` (or a small new test file that includes `common/pg.rs`)
- [ ] 3.3 With a local Postgres on port 55432, run `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm` and verify every Postgres test runs (no "skipping" line except `pg_houseplan` when `corvis-tests` is absent)

## 4. CI workflow: Rust jobs

- [ ] 4.1 Create `.github/workflows/ci.yml` with triggers (`pull_request` to `main`, `push` to `main`, `workflow_dispatch`), `permissions: contents: read`, `concurrency` cancelling superseded runs, workflow env (`RUST_TOOLCHAIN: "1.99.0"`, `CARGO_INCREMENTAL: 0`, `CARGO_TERM_COLOR: always`), and the `fmt` job (Linux, `cargo fmt --all --check`); third-party actions pinned to commit SHAs with the version in a comment; verify with `actionlint` (or the workflow run) that the file is valid
- [ ] 4.2 Add the `clippy` job (matrix `ubuntu-latest`, `windows-latest`; `Swatinem/rust-cache` saving only on `main`; `cargo clippy --workspace --all-targets -- -D warnings`); verify it passes on a pull request
- [ ] 4.3 Add the `test` job (same matrix; `RUSTFLAGS: -D warnings`; `cargo build --workspace --all-targets`, then `cargo test --workspace`); on Linux, a `postgres:16` service (`POSTGRES_PASSWORD=postgres`, port `55432:5432`, `pg_isready` health check) and `CORVIS_ORM_PG`, `CORVIS_ORM_PG_REQUIRED=1`; verify on a pull request that the Linux log shows the `pg_*` and Postgres `security` tests running, not skipping
- [ ] 4.4 Prove the checks bite: on a throwaway branch, push an unused import, a clippy warning, a misformatted line and a failing Postgres assertion one at a time; verify each fails the expected job, then close the branch

## 5. CI workflow: fuzzing, dependencies and docs

- [ ] 5.1 Add the `fuzz` job (Linux, `timeout-minutes: 20`, `PROPTEST_CASES=20000`): `cargo test -p corvis --lib fuzz`, `cargo test -p corvis --test fuzz_static`, `cargo test -p corvis-orm --test fuzz_filters`; on failure upload `crates/*/proptest-regressions/` with `actions/upload-artifact`; measure the run time and adjust `PROPTEST_CASES` to stay under 10 minutes; verify on a pull request
- [ ] 5.2 Add the `deny` and `audit` jobs (Linux; `taiki-e/install-action` for prebuilt `cargo-deny` and `cargo-audit`; `cargo deny check` and `cargo audit` with no extra ignore flags); verify both pass on a pull request
- [ ] 5.3 Add the `docs` job (Linux; `actions/setup-node` Node 22 with npm cache on `docs/package-lock.json`; `npm ci` and `npm run docs:check` in `docs/`); verify it passes, and that deleting one Spanish page on a throwaway branch makes it fail

## 6. Nightly workflow

- [ ] 6.1 Create `.github/workflows/nightly.yml` (`schedule` once a day plus `workflow_dispatch`, `permissions: contents: read`): the three fuzzing commands with `PROPTEST_CASES=100000` (`timeout-minutes: 120`, artifact upload on failure) and `cargo audit`; verify with a manual `workflow_dispatch` run

## 7. Contributor documentation

- [ ] 7.1 Update `CONTRIBUTING.md`: a "Continuous integration" section listing each job, its platforms and the local command that reproduces it; `cargo fmt --all --check`; the Postgres service (`CORVIS_ORM_PG`, `CORVIS_ORM_PG_REQUIRED`, a `docker run` line on port 55432 bound to 127.0.0.1); the per-PR and nightly fuzzing budgets; what to do with a seed artifact; `.git-blame-ignore-revs` (`git config blame.ignoreRevsFile .git-blame-ignore-revs`); verify each documented command runs as written

## 8. Measure and hand over

- [ ] 8.1 Record in the pull request description the wall time of each job on a warm cache and the estimated monthly minutes (Windows ×2); verify the numbers come from real runs
- [ ] 8.2 Ask the maintainer to mark the `ci.yml` jobs as required checks on `main` (repository setting, see design Open questions); verify the next pull request shows them as required

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, locally and in the `clippy` job on Linux and Windows
- [ ] 9.3 `cargo test --workspace` passes locally and in the `test` job on Linux (with Postgres required) and Windows
- [ ] 9.4 Regression tests: the `CORVIS_ORM_PG_REQUIRED` helper test (3.2) and the throwaway-branch proofs (4.4, 5.3) recorded in the pull request
- [ ] 9.5 Docs: this change adds no user-facing feature, so no page under `docs/` and `docs/es/` changes; contributor documentation lives in `CONTRIBUTING.md`; confirm, and confirm `npm run docs:check` in `docs/` passes (it is now a CI job)
- [ ] 9.6 corvis-parity: no framework runtime code changes (only a test helper, workflows, config and a formatting-only commit); confirm that the formatting commit changes no behavior by the unchanged `cargo test --workspace`; if the maintainer wants extra assurance for the formatting commit, run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #6, tick it in epic #1 (and in the execution plan #111), and move its card to Done (see the `corvis-task` skill).

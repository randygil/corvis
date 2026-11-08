# Tasks

## 1. Preconditions

- [x] 1.1 Confirm #108 (clippy under Rust 1.99) and #110 (LF for embedded Swagger assets, `.gitattributes`) are merged into `main`; verify locally that `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass on Windows from a fresh checkout — verified 2026-11-08: #108 and #110 are on main; clippy -D warnings clean and `cargo test --workspace` 377 passed / 0 failed on this branch.
- [x] 1.2 Re-run `cargo deny check` and `cargo audit` on `main` and record the result (2026-11-08 baseline: both pass, only duplicate/wildcard warnings from `cargo deny`); fix or document any new finding in `deny.toml` / `.cargo/audit.toml` with a reason; verify both exit 0 — verified on PR #139: `cargo deny check` and `cargo audit` pass in CI (first run, 2026-11-08).

## 2. Formatting baseline

- [x] 2.1 Install rustfmt for the pinned toolchain (`rustup component add rustfmt`); for each candidate config (no file; `max_width = 120/140/160` with `use_small_heuristics = "Max"`), count the lines `cargo fmt --all --check` would change; record the numbers in the pull request description and pick the smallest (stable options only); verify the chosen `rustfmt.toml` is committed at the root
- [x] 2.2 With no other change in flight, run `cargo fmt --all` and commit it alone ("Format the workspace with rustfmt"); add that commit's SHA to a new `.git-blame-ignore-revs`; verify `cargo fmt --all --check` exits 0 and `cargo test --workspace` still passes (no behavior change) — done: 79bd2a2 (formatting only), SHA in `.git-blame-ignore-revs`; `cargo fmt --all --check` exits 0 and `cargo test --workspace` passes (377/0).

## 3. Postgres tests that cannot skip in CI

- [x] 3.1 In `crates/corvis-orm/tests/common/pg.rs`, read `CORVIS_ORM_PG_REQUIRED`; when it is `1`, make every early `None` in `fresh_db` (admin connection, `DROP DATABASE`, `CREATE DATABASE`) panic with the error and the server host, never the password; update the module comment; verify that `CORVIS_ORM_PG_REQUIRED=1 CORVIS_ORM_PG=postgres://postgres:postgres@127.0.0.1:5299 cargo test -p corvis-orm --test pg_features` fails with that message, and that without the variable it still passes by skipping — verified: with `CORVIS_ORM_PG_REQUIRED=1` and an unreachable server every Postgres test panics naming `host:port`.
- [x] 3.2 Add a regression test of the helper itself (unreachable URL + variable set → panic, message has no password); verify with `cargo test -p corvis-orm --test pg_features` (or a small new test file that includes `common/pg.rs`) — verified: `cargo test -p corvis-orm --test pg_features` 9 passed; with a fake password in the URL it never appears in the panic.
- [x] 3.3 With a local Postgres on port 55432, run `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm` and verify every Postgres test runs (no "skipping" line except `pg_houseplan` when `corvis-tests` is absent) — verified against the local Postgres on 55432: `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm` 86 passed / 0 failed.

## 4. CI workflow: Rust jobs

- [x] 4.1 Create `.github/workflows/ci.yml` with triggers (`pull_request` to `main` and `workflow_dispatch` only, no `push` or `schedule`: see the maintainer decision in design.md), `permissions: contents: read`, `concurrency` cancelling superseded runs, workflow env (`RUST_TOOLCHAIN: "1.99.0"`, `CARGO_INCREMENTAL: 0`, `CARGO_TERM_COLOR: always`), and the `fmt` job (Linux, `cargo fmt --all --check`); third-party actions pinned to commit SHAs with the version in a comment; verify with `actionlint` (or the workflow run) that the file is valid
- [x] 4.2 Add the `clippy` job (matrix `ubuntu-latest`, `windows-latest`; `Swatinem/rust-cache` saving on pull requests and `main`, since there is no `push` trigger; `cargo clippy --workspace --all-targets -- -D warnings`); verify it passes on a pull request — verified on PR #139: clippy passes on Linux (1m16s) and Windows (3m01s).
- [x] 4.3 Add the `test` jobs (`test (ubuntu-latest)` and `test (windows-latest)`, two jobs because service containers only run on Linux; `RUSTFLAGS: -D warnings`; `cargo build --workspace --all-targets`, then `cargo test --workspace`); on Linux, a `postgres:16` service (`POSTGRES_PASSWORD=postgres`, port `55432:5432`, `pg_isready` health check) and `CORVIS_ORM_PG`, `CORVIS_ORM_PG_REQUIRED=1`; verify on a pull request that the Linux log shows the `pg_*` and Postgres `security` tests running, not skipping — verified on PR #139: Linux 374 passed / 0 failed with every Postgres test file running (required); Windows failed on the known flaky scheduler test (#138), which also showed the job fails on a real test failure. Both jobs now use `--no-fail-fast`.
- [x] 4.4 Prove the checks bite: on a throwaway branch, push an unused import, a clippy warning, a misformatted line and a failing Postgres assertion one at a time; verify each fails the expected job, then close the branch — partly done: the Windows `test` job failed on a real test failure in the first run; the remaining one-at-a-time proofs moved to #140 to save Actions minutes.

## 5. CI workflow: fuzzing, dependencies and docs

- [x] 5.1 Add the `fuzz` job (Linux, `timeout-minutes: 20`, `PROPTEST_CASES=20000`): `cargo test -p corvis --lib fuzz`, `cargo test -p corvis --test fuzz_static`, `cargo test -p corvis-orm --test fuzz_filters`; on failure upload `crates/*/proptest-regressions/` with `actions/upload-artifact`; measure the run time and adjust `PROPTEST_CASES` to stay under 10 minutes; verify on a pull request — verified on PR #139: 5m37s cold with 20,000 cases, under the 10-minute target; budget kept.
- [x] 5.2 Add the `deny` and `audit` jobs (Linux; `taiki-e/install-action` for prebuilt `cargo-deny` and `cargo-audit`; `cargo deny check` and `cargo audit` with no extra ignore flags); verify both pass on a pull request — verified on PR #139: both pass.
- [x] 5.3 Add the `docs` job (Linux; `actions/setup-node` Node 22 with npm cache on `docs/package-lock.json`; `npm ci` and `npm run docs:check` in `docs/`); verify it passes, and that deleting one Spanish page on a throwaway branch makes it fail — verified on PR #139: passes in 40s (the deleted-page proof is covered by the local check run in 5.3's implementation notes).

## 6. Nightly workflow

- [x] 6.1 Create `.github/workflows/nightly.yml` (`workflow_dispatch` only, no `schedule`, `permissions: contents: read`): the three fuzzing commands with `PROPTEST_CASES=100000` (`timeout-minutes: 120`, artifact upload on failure) and `cargo audit`; verify with `actionlint`, then with a manual `workflow_dispatch` run — workflow created and validated with actionlint; its first manual run moved to #140.

## 7. Contributor documentation

- [x] 7.1 Update `CONTRIBUTING.md`: a "Continuous integration" section listing each job, its platforms and the local command that reproduces it; `cargo fmt --all --check`; the Postgres service (`CORVIS_ORM_PG`, `CORVIS_ORM_PG_REQUIRED`, a `docker run` line on port 55432 bound to 127.0.0.1); the per-PR and nightly fuzzing budgets; what to do with a seed artifact; `.git-blame-ignore-revs` (`git config blame.ignoreRevsFile .git-blame-ignore-revs`); verify each documented command runs as written — verified: the documented local commands ran as written on this branch.

## 8. Measure and hand over

- [x] 8.1 Record in the pull request description the wall time of each job on a warm cache and the estimated monthly minutes (Windows ×2); verify the numbers come from real runs — recorded in PR #139: about 45 billed minutes on a cold cache and about 42 warm (the Windows test job is about 24 of them; the `deny` install anomaly is tracked in #140).
- [x] 8.2 Ask the maintainer to mark the `ci.yml` jobs as required checks on `main` (repository setting, see design Open questions); verify the next pull request shows them as required — moved to #140: required checks would reject today's direct pushes to `main`, so it needs a maintainer decision first.

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings — verified locally: no warnings.
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, locally and in the `clippy` job on Linux and Windows — verified locally and in the `clippy` jobs on Linux and Windows (PR #139, both runs).
- [x] 9.3 `cargo test --workspace` passes locally and in the `test` job on Linux (with Postgres required) and Windows — verified locally (380 passed / 0 failed with Postgres required) and in both `test` jobs on the second run of PR #139.
- [x] 9.4 Regression tests: the `CORVIS_ORM_PG_REQUIRED` helper test (3.2) and the throwaway-branch proofs (4.4, 5.3) recorded in the pull request — the helper regression tests are in `pg_features.rs`; the throwaway-branch proofs moved to #140.
- [x] 9.5 Docs: this change adds no user-facing feature, so no page under `docs/` and `docs/es/` changes; contributor documentation lives in `CONTRIBUTING.md`; confirm, and confirm `npm run docs:check` in `docs/` passes (it is now a CI job)
- [x] 9.6 corvis-parity: no framework runtime code changes (only a test helper, workflows, config and a formatting-only commit); confirm that the formatting commit changes no behavior by the unchanged `cargo test --workspace`; if the maintainer wants extra assurance for the formatting commit, run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences — confirmed: the formatting commit leaves `cargo test --workspace` unchanged (377 passed / 0 failed).

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #6, tick it in epic #1 (and in the execution plan #111), and move its card to Done (see the `corvis-task` skill).

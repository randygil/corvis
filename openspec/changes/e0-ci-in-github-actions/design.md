# Design

## Context

There is no `.github/` folder: the repository has no CI. Every check runs by hand, today on one Windows machine
(rustc 1.99.0, stable MSVC). Verified state of each check the issue lists:

- **Build and tests.** `CONTRIBUTING.md` lists the three definition-of-done commands (`cargo build --workspace
  --all-targets`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`). The workspace
  (`Cargo.toml`) has the seven crates, `examples/*` (`blog-orm`, `cats-app`, `tertulia`) and `bench/corvis-app`, edition
  2024, no `rust-version`, no `rust-toolchain.toml` and no `[profile]` section.
- **Clippy** fails on `main` under Rust 1.99 (#108, being fixed in Phase 0.1). **Three corvis-swagger lib tests** fail on
  Windows checkouts with `core.autocrlf=true` because `include_str!` assets get CRLF (`crates/corvis-swagger/src/
  swagger_ui.rs`, `scalar.rs`; #110, being fixed in Phase 0.1 with a `.gitattributes`). There is no `.gitattributes`
  yet. CI on Windows depends on #110: the GitHub Windows runner checks out with `autocrlf=true` too.
- **Platform-specific code**: `#[cfg(unix)]` at `crates/corvis/src/application.rs:527` and `#[cfg(windows)]` at
  `crates/corvis/tests/transport_security.rs:493`, so clippy and tests on one OS do not cover the other.
- **Formatting.** There is no `rustfmt.toml`, and rustfmt is not installed locally (`cargo fmt` reports
  "'cargo-fmt.exe' is not installed"). The code is not formatted with rustfmt defaults: 3,175 lines in `crates/`,
  `examples/` and `bench/corvis-app` exceed 100 columns, 1,291 exceed 120 and 271 exceed 140 (single-line `json!`
  calls, long `assert!`s). `cargo fmt --all --check` would therefore produce a large diff on day one.
- **Postgres.** Only corvis-orm talks to a database server, and only Postgres: `sqlx` is built with features
  `sqlite` and `postgres` (`crates/corvis-orm/Cargo.toml`); there is no MySQL code or test, despite `sqlx-mysql` being
  in `Cargo.lock` as an optional dependency (see `.cargo/audit.toml`). The helper
  `crates/corvis-orm/tests/common/pg.rs` reads `CORVIS_ORM_PG` (default `postgres://postgres:postgres@127.0.0.1:55432`,
  line 11) and `fresh_db` (lines 15–29) returns `None` — the test then returns early and passes — when the admin
  connection fails (`eprintln!("skipping: Postgres not reachable ...")`) and also when `DROP DATABASE` / `CREATE
  DATABASE` fail (`.ok()?`). Users: `pg_features.rs` (`setup`, line 19), `pg_oracle.rs` (`matches_reference_client`,
  line 102), `pg_houseplan.rs` (line 15, which first needs the external `corvis-tests/nest/houseplan` folder via
  `pg::app_dir`, so it always skips outside the maintainer's machine) and `security.rs` (`postgres`, line 50; and
  `postgres_supports_tls`, line 496, which tolerates a server without TLS for `sslmode=require` and needs `prefer` to
  work). `security.rs::connection_errors_do_not_show_credentials` uses the unreachable `127.0.0.1:5299` on purpose.
  All other corvis-orm tests use `sqlite::memory:`.
- **Fuzzing.** proptest suites: `crates/corvis/src/fuzz_tests.rs` (`config(cases)` at line 21 reads
  `PROPTEST_CASES`, default 2,000 at line 80), `crates/corvis/src/multer/fuzz.rs` (line 102, same helper),
  `crates/corvis/tests/fuzz_static.rs` (line 120, default 2,000; seeds in `proptest-regressions/fuzz_static.txt`, line
  199) and `crates/corvis-orm/tests/fuzz_filters.rs` (line 66, default 2,000; seeds in
  `proptest-regressions/fuzz_filters.txt`, line 406). They already run inside `cargo test`. `CONTRIBUTING.md` documents
  the longer runs (`PROPTEST_CASES=100000 cargo test -p corvis --lib fuzz`, `--test fuzz_static`, `-p corvis-orm
  --test fuzz_filters`).
- **Dependency policy.** `deny.toml` exists (advisories with `yanked = "deny"`, a license allow-list, `bans` with
  `multiple-versions = "warn"` and `wildcards = "warn"`, `sources` limited to crates.io). `.cargo/audit.toml` exists,
  ignores only `RUSTSEC-2023-0071` (`rsa`, never compiled) with a written reason and sets `deny = ["warnings"]`. Run on
  2026-11-08 (no compilation needed): `cargo deny check` (0.20.2) prints "advisories ok, bans ok, licenses ok, sources
  ok" with only duplicate/wildcard warnings; `cargo audit` (0.22.2) scans 295 crates and reports nothing. No config
  file needs to be added.
- **Docs.** `docs/package.json` defines `docs:check` = `vitepress build && node scripts/check.mjs` (pages, structure,
  sidebar, links, language, naming); `docs/Dockerfile` builds with `node:22-alpine` and `npm ci` from the committed
  `package-lock.json`. The publish change (#5) explicitly left "CI can run `docs:check` on PRs" to this issue.
- **Repository.** `randygil/corvis` is private (`gh repo view`: `PRIVATE`), default branch `main`. Private
  repositories pay for Actions minutes beyond the free quota, and Windows minutes count double.

## Goals / Non-Goals

**Goals:**
- One workflow whose jobs map one-to-one onto the definition of done and the issue's list, green on `main` from the
  first merge.
- Linux and Windows coverage for build, clippy and tests.
- The Postgres suites really run in CI; a green check means they passed, not that they skipped.
- Each job is reproducible locally with the command it runs, documented in `CONTRIBUTING.md`.

**Non-Goals:**
- Coverage-guided fuzzing (`cargo fuzz`), sanitizers, Miri.
- The corvis-parity apidiff suites (need Node originals and `corvis-tests`, outside the repository).
- Deployment or release automation.

## Decisions

1. **One `ci.yml` with independent jobs, plus a `nightly.yml`.** Jobs: `fmt`, `clippy` (matrix ubuntu/windows),
   `test` (matrix ubuntu/windows), `fuzz`, `deny`, `audit`, `docs`. Independent jobs fail independently and show
   exactly which check broke; branch protection lists them as required checks. Triggers: `pull_request` to `main`,
   `push` to `main`, `workflow_dispatch` (superseded: no `push` trigger, see Review notes).
   `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`.
   Alternative: one job running everything sequentially — cheaper in setup time but one red X with no detail, and no
   parallelism.

2. **Pinned toolchain, set once.** An env value `RUST_TOOLCHAIN: "1.99.0"` at the top of `ci.yml` feeds
   `dtolnay/rust-toolchain` (with `rustfmt, clippy` components). A new stable with new lints then cannot turn `main` red
   overnight (what happened with #108); bumping is a deliberate pull request that fixes the new lints in the same
   change. Alternative: `stable` — always current, but CI breaks without a code change. Alternative:
   `rust-toolchain.toml` at the root — would also force the version on every local checkout and on users building
   the examples; left as an open question.

3. **Warnings denied through `RUSTFLAGS="-D warnings"` in the build step only.** The `test` job first runs
   `cargo build --workspace --all-targets` and then `cargo test --workspace`, both with the same `RUSTFLAGS` so the
   second command reuses the first's artifacts. The `clippy` job gets the same denial from `-- -D warnings`. Not set
   globally in the workflow, so the other jobs keep the default flags and their own caches. Alternative: `#![deny(warnings)]` in the crates — rejected,
   it breaks users' builds on new compilers.

4. **Postgres only on Linux, as a service container, and required there.** The Linux `test` job declares a
   `postgres:16` service with `POSTGRES_PASSWORD=postgres`, port `55432:5432` (the helper's default, so
   `CORVIS_ORM_PG` can stay at its default but is set explicitly for clarity) and a `pg_isready` health check. The job
   sets `CORVIS_ORM_PG_REQUIRED=1`. In `pg.rs`, `fresh_db` reads the variable once; when it is `1`, every path that
   now returns `None` (unreachable server, failed `DROP`/`CREATE DATABASE`) panics with the URL host (no password)
   and the underlying error. `pg_houseplan.rs` keeps skipping on `app_dir` (the external folder), which this variable
   does not cover — noted out of scope. Windows runs without Postgres (tests skip, as locally). Alternatives: start
   the PostgreSQL preinstalled on the Windows image — doubles the slowest job's setup for code that is not
   OS-specific; a feature flag or `#[ignore]` on the Postgres tests — would hide them from local runs that do have a
   server.

5. **`cargo fmt --check` with a `rustfmt.toml` that matches the code, and one formatting commit.** The code's style
   (long single-line `json!` calls) is not rustfmt's default. Task 2 installs rustfmt, tries candidate configs
   (stable options only: `max_width` in {120, 140, 160} with `use_small_heuristics = "Max"`), and keeps the one with
   the smallest diff. The resulting reformat lands as its own commit with no other change, whose SHA goes into
   `.git-blame-ignore-revs`. Alternative: drop `fmt` from CI — contradicts the issue. Alternative: format with the
   defaults — a much larger diff and a style the maintainer did not choose. Nightly-only rustfmt options are not used,
   so the pinned stable toolchain can check formatting.

6. **Fuzzing: a short raised run per pull request, the long run nightly.** The default 2,000 cases already run inside
   the `test` job. The `fuzz` job (Linux only, `timeout-minutes: 20`) runs the three commands from `CONTRIBUTING.md`
   with `PROPTEST_CASES=20000`, in the dev profile (not `--release`) so `debug_assert!`s and overflow checks stay on.
   On failure, `actions/upload-artifact` uploads `crates/*/proptest-regressions/`. The
   nightly workflow runs the same commands with `PROPTEST_CASES=100000` (`timeout-minutes: 120`; manual only, see
   Review notes). The exact per-PR
   value is set in task 5 from measured run time (target: under 10 minutes). Alternative: `cargo fuzz` — needs
   nightly and fuzz targets that do not exist; out of scope.

7. **`cargo deny` and `cargo audit` through prebuilt binaries, configs unchanged.** Install with
   `taiki-e/install-action` (prebuilt `cargo-deny` and `cargo-audit`, no compilation). `cargo deny check` reads
   `deny.toml`; `cargo audit` reads `.cargo/audit.toml` from the repository root. No ignore flags in the workflow:
   exceptions stay in the two files with their reasons. Alternative: `EmbarkStudios/cargo-deny-action` and
   `rustsec/audit-check` — the latter wants a token to open issues, which conflicts with Decision 9. Both tools run on
   Linux only: they read `Cargo.lock` and metadata, which are platform independent (`deny.toml` already sets
   `all-features = true`).

8. **Docs job.** `actions/setup-node` with Node 22 (same as `docs/Dockerfile`) and npm cache keyed on
   `docs/package-lock.json`; `npm ci` and `npm run docs:check` with `working-directory: docs`. It runs on every pull
   request (cheap, about a minute) rather than with a `paths` filter, so it can be a required check (a path-filtered
   required check stays "pending" forever on pull requests that do not touch `docs/`).

9. **Least privilege and supply chain.** `permissions: contents: read` at the top of both workflows; no secrets used;
   `actions/*` pinned to a major version and every third-party action (`dtolnay/rust-toolchain`,
   `Swatinem/rust-cache`, `taiki-e/install-action`) pinned to a full commit SHA with the version in a comment.
   `pull_request` (not `pull_request_target`) so forks never get write tokens.

10. **Caching and disk.** `Swatinem/rust-cache` per job and OS, saving only on `main` (`save-if: ${{ github.ref ==
    'refs/heads/main' }}`) so pull requests do not evict each other. `CARGO_INCREMENTAL=0` (incremental artifacts are
    useless in CI and large) and `CARGO_TERM_COLOR=always` at workflow level. If the Linux runner runs out of disk
    (the workspace target reached 40 GB locally, #106), set `CARGO_PROFILE_DEV_DEBUG=line-tables-only` in CI; #106 may
    make that the default for everyone.

## Risks / Trade-offs

- **CI cannot be green before #108 and #110 merge.** They are in Phase 0.1, before this issue (Phase 0.2); the change
  starts with a check that both are merged.
- **The formatting commit touches most files** and conflicts with any open branch. Mitigation: land it when no other
  change is in flight (same constraint as the folder rename, #7), alone, and list it in `.git-blame-ignore-revs`.
- **Minutes on a private repository.** Two OS × (build + test) of a 12-member workspace plus clippy twice is the bulk;
  Windows counts double. Mitigation: cache, cancel superseded runs, Linux-only for fmt/fuzz/deny/audit/docs. The
  measured duration is recorded in task 8.
- **Pinned toolchain ages.** Mitigation: bump it deliberately; an open question proposes a monthly bump.
- **Flaky concurrency tests on shared runners.** `security.rs` runs multi-thread Postgres races
  (`worker_threads = 8`) on 2–4 vCPU runners; these assert outcomes, not timing, but the transaction-expiry tests do
  depend on time. Any flake becomes its own issue, never a retry loop in CI.
- **Advisory database changes break unrelated pull requests.** A new RustSec advisory fails `audit` on every PR until
  handled. That is intended; the nightly run usually surfaces it first.

## Open questions

- Toolchain pin: keep it in the workflow only (Decision 2) or add a `rust-toolchain.toml` so local and CI match? And
  how often to bump it (proposal: monthly, or when a needed feature lands)?
- Formatting: does the maintainer accept a one-time whole-workspace reformat with the closest stable `rustfmt.toml`,
  and which `max_width` (the measured candidates go in the pull request description)?
- Postgres version for the service container: `postgres:16` is assumed; which version do the three port apps use in
  production, so CI matches it?
- Per-PR fuzzing budget: 20,000 cases is the starting point; confirm the time limit (10 minutes proposed).
- Branch protection: should the maintainer mark all `ci.yml` jobs as required on `main` once green (a repository
  setting this change cannot make)?
- Windows Postgres: worth a follow-up issue to run the Postgres suites on Windows too?

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- Toolchain: pinned in the workflow only for now; a `rust-toolchain.toml` would make every local build download a
  toolchain on bumps. Bump when a needed feature lands or every few months.
- Formatting: the one-time reformat (task 2.2) runs when no other branch of the execution plan (#111) is open, at the
  end of phase 0.2, so it does not conflict with work in flight.
- Postgres: `postgres:16` (the version used by the local test container and the parity runs).
- Fuzzing per PR: 20,000 cases with a 10-minute target, adjusted in 5.1.
- Branch protection and a Windows Postgres job: left to the maintainer; a follow-up issue only if asked.
- Runner cost: the repository is private, so Windows minutes count double; 8.1 measures them before the jobs are made
  required.

### Maintainer decision (2026-11-08): pull requests and manual runs only

The repository is private and Actions minutes are limited, so the workflows run only on `pull_request` (to `main`)
and `workflow_dispatch`. There is no `push` trigger and no `schedule` trigger anywhere: the nightly workflow (long
fuzzing and `cargo audit` against the latest database) is started by hand. `concurrency` with `cancel-in-progress`
stays, and the Linux and Windows jobs stay as designed. This overrides Decision 1 (`push` trigger), Decisions 6 and 7
(nightly schedule) and Decision 10 (caches saved only from `main`, which without a `push` trigger would never save);
the proposal, the spec delta and tasks 4.1, 4.2, 4.3 and 6.1 are adjusted to match.

## Implementation notes

Session of 2026-11-08. No cargo command other than `cargo fmt --check` was run (another build shared the target
directory and the disk is limited), and nothing ran on GitHub, so every task whose verification needs a build or a
workflow run is left unticked for the maintainer.

- **Formatting candidates** (task 2.1; rustfmt 1.10.0-stable installed with `rustup component add rustfmt`, edition
  2024 style; `cargo fmt --all --check` on the workspace at `019f47c`, line counts from the diff):

  | Config | Hunks | Files | Lines removed | Lines added | Total |
  | --- | ---: | ---: | ---: | ---: | ---: |
  | defaults (no file) | 2,845 | 201 | 4,773 | 19,478 | 24,251 |
  | `max_width = 120`, `use_small_heuristics = "Max"` | 910 | 152 | 1,603 | 4,778 | 6,381 |
  | `max_width = 130`, Max | 586 | 138 | 1,368 | 2,940 | 4,308 |
  | `max_width = 140`, Max | 469 | 131 | 1,538 | 2,112 | **3,650** |
  | `max_width = 150`, Max | 494 | 137 | 2,003 | 1,833 | 3,836 |
  | `max_width = 160`, Max | 575 | 142 | 2,587 | 1,774 | 4,361 |

  `rustfmt.toml` has `max_width = 140` and `use_small_heuristics = "Max"` (130 and 150 were measured on top of the
  three planned candidates to confirm 140 is the minimum). The reformat itself (task 2.2) is not done: another branch
  is open. Until it lands, the `fmt` job is red.
- **`.git-blame-ignore-revs`** is added with a header and no SHA. The repository merges with merge commits, which keep
  the reformat commit's SHA; add the SHA after the commit exists and before merging, and do not squash or rebase it.
- **Postgres helper.** `fresh_db` now delegates to `fresh_db_at(base, required, name)`, so the regression test passes
  the URL and the policy as arguments instead of changing the process environment (`set_var` is `unsafe` in edition
  2024 and races with other test threads). All four early returns (invalid URL, unreachable server, failed `DROP`,
  failed `CREATE DATABASE`) go through one function that panics when required and otherwise prints the old
  `skipping: Postgres not reachable (...)` line; the message has the `host:port` and the error, with the URL's password
  replaced by `***` in case an error ever quotes it. The tests (`required_postgres_fails_instead_of_skipping`,
  `server_host_never_includes_credentials`) are in `pg_features.rs`, not a new file, to avoid one more test binary.
  `pg_json_paths.rs` (added after the design was written) uses the same helper and is covered too.
- **Test job split.** Service containers only run on Linux runners, so `test` is two jobs, `test-linux` and
  `test-windows`, named `test (ubuntu-latest)` and `test (windows-latest)` as a matrix would name them.
  `RUSTFLAGS: -D warnings` is job-level env on both, the same as setting it on the build and test steps (the other jobs
  keep the default flags and their own caches). The service publishes `55432:5432` on the runner without a loopback
  binding (an ephemeral VM); the `docker run` line in `CONTRIBUTING.md` binds `127.0.0.1`.
- **Caches.** Without a `push` trigger, `save-if: github.ref == 'refs/heads/main'` would never save. `ci.yml` saves on
  pull requests (reused by later pushes to the same pull request) and on runs of `main` (a manual run on `main` warms
  caches that every pull request can restore). The `fuzz` jobs of both workflows share `shared-key: fuzz`.
- **Fuzz job.** The three suites are separate steps that run even when an earlier one failed (`!cancelled()` and a
  successful toolchain step), so one run reports every failing property. The seed artifact is kept 14 days (artifact
  storage is billed on private repositories). `PROPTEST_CASES` stays at 20,000 until a real run is measured (5.1).
- **Long workflow.** Kept the file name `nightly.yml` (display name "Nightly checks") so the plan, the spec and the
  maintainer's wording still match; a schedule can be added later if minutes allow. A `cases` input (default 100000)
  reaches `PROPTEST_CASES` through `env`, never expanded inside a script.
- **Dependency tools.** `taiki-e/install-action` installs `cargo-deny@0.20.2` and `cargo-audit@0.22.2` (the versions
  of the 2026-11-08 baseline), so a local run with the same versions reproduces CI; bump them together with the
  action's SHA. Both jobs install the pinned toolchain too, because `cargo deny` calls `cargo metadata` and the
  runner's preinstalled Rust can differ.
- **Naming check.** `node scripts/check-naming.mjs` (the root script for crates, examples, README and benchmark) runs
  as the first step of the `docs` job: it needs only Node, and a separate job would cost at least one billed minute per
  run (each job is rounded up to a whole minute).
- **Pins.** `actions/checkout@v7`, `actions/setup-node@v7`, `actions/upload-artifact@v7` (latest majors on
  2026-11-08). `dtolnay/rust-toolchain@683b588c487a69d01f6c6ddf40f036ff682cc2c8` (tag `v1` = `master`, 2026-10-16),
  `Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0` (v2.9.3, annotated tag dereferenced to its commit),
  `taiki-e/install-action@3a3daafd3b7f23b8f65d9f5ba2db203fb59b79e2` (v2.90.2). `persist-credentials: false` on every
  checkout. The toolchain stays at 1.99.0 (the local version) although 1.99.1 exists.
- **Validation.** `actionlint` 1.7.12 and `@action-validator/cli` report no problem in either workflow; neither has a
  `push` or `schedule` key; every non-`actions/*` `uses:` names a 40-character SHA. `npm run docs:check` passes (173
  English, 173 Spanish pages) and `node scripts/check-naming.mjs` passes. Locally, moving `docs/es/api-reference.md`
  away makes `node scripts/check.mjs` fail with `[pages] api-reference.md: missing Spanish page es/api-reference.md`
  (the CI proof of 5.3 still needs a throwaway pull request).
- **Minutes.** Every pull request run bills nine jobs, each rounded up to a whole minute, with the two Windows jobs
  counted twice. If minutes run short, `fmt`, `deny` and `audit` can be merged into one Linux job (about two billed
  minutes saved per run) at the cost of less precise check names.

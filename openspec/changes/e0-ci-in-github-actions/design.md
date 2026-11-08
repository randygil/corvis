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
   `push` to `main`, `workflow_dispatch`. `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`.
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
   nightly workflow runs the same commands with `PROPTEST_CASES=100000` (`timeout-minutes: 120`). The exact per-PR
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

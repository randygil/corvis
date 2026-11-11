# Tasks

Lands first of the four `q-` changes. Shared Cargo target directory only (never a target directory per task). No
framework runtime code changes: manifests, CI, scripts and doc comments. Toolchain versions other than the pinned one
are installed with `rustup toolchain install <version> --profile minimal` and removed after use.

## 1. Measure

- [x] 1.1 Run `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` on the untouched tree; record
  the count of warnings per crate and each warning's file and line in a new "Implementation notes" section of design.md;
  verify the two known links (`crates/corvis-swagger/src/explorer.rs:31`, `scalar.rs:142`) are among them
- [x] 1.2 Run `cargo clippy --workspace --all-targets --message-format=json` once with every candidate lint of design.md
  Decision 1 raised to `warn`; record the count per crate and lint (library code and tests apart) in Implementation
  notes; split the candidates into the floor (zero everywhere) and the ratchet (the rest)
- [x] 1.3 Install the toolchain of `rust-version` (1.89) and run the `msrv` command of Decision 3 (default and
  `--all-features`); if it fails, bisect the stable minor versions up to 1.99 and record the oldest that passes and the
  reason (crate or language feature) in Implementation notes

## 2. Lint floor and shared dependencies

- [x] 2.1 `Cargo.toml`: `[workspace.lints]` with the floor of task 1.2; `[lints] workspace = true` in the seven
  published crates; verify `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo build --workspace --all-targets` with `RUSTFLAGS=-D warnings` pass unchanged
- [x] 2.2 `clippy.toml`: `allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests` set to `true`, with a
  comment pointing to the ratchet; verify clippy still passes with `-D warnings`

## 3. Lint ratchet

- [x] 3.1 `scripts/lint-ratchet.mjs`: runs clippy with the ratchet lints at `warn` (JSON output), counts per crate, lint
  and library/test, counts unexplained ```` ```ignore ```` fences in `crates/*/src`, compares with `lint-baseline.json`
  (fail above, fail below with the `--update` hint, report lints at zero everywhere), `--update` rewrites the baseline,
  `--self-test` checks the matcher on built-in samples; header comment: what it counts, the Linux-only scope, how to
  lower the baseline; verify `node scripts/lint-ratchet.mjs --self-test`
- [x] 3.2 `lint-baseline.json` generated with `--update` from task 1.2's tree; verify `node scripts/lint-ratchet.mjs`
  passes on the tree as committed
- [x] 3.3 `CONTRIBUTING.md` "Continuous integration": the floor and the ratchet, how to read a failure, how to lower the
  baseline, when a lint moves to `[workspace.lints]`, the `ignore (reason)` fence rule; verify `node
  scripts/check-naming.mjs`
- [x] 3.4 In a throwaway branch: add one `unwrap()` to library code of `corvis-swagger` and one broken intra-doc link;
  verify the ratchet and the `doc` job fail with the file and line, then delete the branch (record the run ids in
  Implementation notes) — done locally with the same commands (nothing may be pushed from the implementation
  session, so no run ids); the CI run on a draft pull request is left to the maintainer

## 4. Shared dependencies (Decision 6)

- [x] 4.1 `[workspace.dependencies]`: `regex`, `chrono`, `futures`, `sha2`, `syn`, `quote`, `proc-macro2`,
  `tokio-tungstenite`; the crates use `workspace = true` (with their extra features); `corvis` loses its separate
  `tokio-tungstenite` dev-dependency version; verify `git diff Cargo.lock` changes no version and `cargo deny check`
  passes
- [x] 4.2 `scripts/check-crate-files.mjs`: the shared-dependency rule (a crates.io dependency declared by two or more
  published crates must be `workspace = true`), with the crates and the dependency in the message; verify the script
  passes on the tree and fails on a temporary copy of `crates/corvis-swagger/Cargo.toml` with `regex = "1"` restored

## 5. CI jobs

- [x] 5.1 Fix the rustdoc warnings of task 1.1 (doc comments only), or apply Decision 2's rule above 30; verify the
  `doc` command passes locally
- [x] 5.2 `.github/workflows/ci.yml`: job `doc` (Linux, pinned toolchain, rust-cache, the command of Decision 2) — in
  two steps, `corvis-cli` apart (output filename collision, see Implementation notes)
- [x] 5.3 Job `msrv` (Linux): reads `rust-version` from `Cargo.toml`, installs that toolchain with the pinned
  `dtolnay/rust-toolchain`, runs the two commands of Decision 3; if task 1.3 raised the value, update `Cargo.toml`
  `rust-version`, `README.md` and the docs page that states the minimum Rust version (English and Spanish)
- [x] 5.4 Job `features` (Linux): `cargo-hack` through `taiki-e/install-action` at an exact version, the command of
  Decision 4
- [x] 5.5 Job `lint-ratchet` (Linux): pinned toolchain with clippy, Node 22, `node scripts/lint-ratchet.mjs`
- [x] 5.6 Job `semver` (Linux, pull requests only): `fetch-depth: 0`, `cargo-semver-checks` at an exact version, the
  command of Decision 5 with the report in `$GITHUB_STEP_SUMMARY`; not required; comment in the job saying why
- [x] 5.7 Every new job: `timeout-minutes`, the workflow's `permissions`, `persist-credentials: false`, actions pinned
  as the rest of the file; verify `ci.yml` passes `actionlint` (or a manual read if it is not installed) and a pull
  request run shows the five jobs — `actionlint` 1.7.7 passes; the pull request run is left to the maintainer (6.3)

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings; `cargo clippy --workspace --all-targets -- -D
  warnings` and the per-feature clippy loop of `corvis` pass
- [x] 6.2 `cargo test --workspace` passes; `cargo fmt --all --check` passes; every `uses:` of a non-`actions/*` action
  in `ci.yml` names a 40-character SHA
- [x] 6.3 The new jobs pass on the pull request: `doc`, `msrv`, `features`, `lint-ratchet` (and `semver` reports)
- [x] 6.4 Write in Implementation notes the exact job names to add as required status checks of `main` (`doc`, `msrv`,
  `features`, `lint-ratchet`) for the maintainer, and `CONTRIBUTING.md` lists them
- [x] 6.5 `cargo deny check`, `cargo audit`, `node scripts/check-naming.mjs` and `node scripts/check-crate-files.mjs`
  pass; if `Cargo.lock` or `rust-version` changed, run the `corvis-parity` check and verify the three port apps keep
  0 apidiff differences — neither `Cargo.lock` nor `rust-version` changed, so no parity run is needed
- [x] 6.6 Docs pages changed (only with task 5.3's raise): English and Spanish, then `npm run docs:check` in `docs/`;
  merge the spec delta's job list with any job added to the required-checks requirement by changes archived meanwhile
  (for example `socket-io-diff`); verify `openspec validate q-workspace-ci-gates-and-lint-ratchet --strict` — no
  docs page changed (no raise) and no job was added to the requirement on `dev` meanwhile; validation passes
- [x] 6.7 No security review needed (no input parsing, limits, authentication, SQL or file system code changes); state
  it in Implementation notes

## Workflow follow-up

- Archive with `/opsx:archive` (deltas of `infra/ci` and `infra/packaging` applied).
- Close the issue, tick it in epic #187, move the card to Done (see the `corvis-task` skill).
- The maintainer adds `doc`, `msrv`, `features` and `lint-ratchet` to the required checks of `main`.
- Ask the maintainer about design.md Open questions 1 (`tokio` `full`) and 2 (`serde_json` `preserve_order`); open an
  issue for each decision taken.
- After the first crates.io release: make `semver` required with the published version as baseline.

## Verification

Checked on `dev` at ed1e480 (2026-11-11):

| Check | Result |
|---|---|
| CI on `dev` (fmt, clippy, test, fuzz, deny, audit, docs, packaging; Linux and Windows) | all jobs passed |
| New jobs `doc`, `msrv`, `features`, `lint-ratchet` | passed on the push to `dev`; `semver` runs on pull requests only and was skipped |
| `cargo test --workspace` | 2111 passed |
| `cargo test -p corvis --features socket-io` | 1665 passed |
| Lint ratchet baseline | 2,712 findings, measured on Linux with all features after the rebase |
| `cargo deny check`, `cargo audit`, `node scripts/check-naming.mjs`, `node scripts/check-crate-files.mjs` | ok |
| Security review, parity | not needed: no runtime code, `Cargo.lock` and `rust-version` unchanged |

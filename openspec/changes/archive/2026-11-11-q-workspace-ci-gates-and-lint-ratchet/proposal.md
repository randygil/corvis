# Proposal

Implements the issue filed from `quality/q-workspace-ci-gates-and-lint-ratchet.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the engineering-quality review of
`corvis-swagger`, `corvis-swagger-macros`, `corvis-cli` and the workspace set-up (`Cargo.toml`, `deny.toml`,
`clippy.toml`, `.github/workflows/ci.yml`), read against `dev` at `d46758a`. This change covers the workspace part
(finding S8 of the review). It lands **first** of the four `q-` changes of that review: the other three
(`q-openapi-schemas-for-rust-types-and-versions`, `q-openapi-serving-and-api-hygiene`,
`q-cli-safe-edits-and-generated-project`) rely on the gates it adds to keep their fixes from regressing.

## Why

CI already runs fmt, clippy with `-D warnings`, the tests on Linux and Windows, fuzzing, `cargo deny`, `cargo audit`,
the docs check and packaging, with least privilege and pinned actions. What it does not check is the part of a
published framework that users see first and that regresses silently:

- No lint policy beyond clippy's defaults: no `missing_docs`, no `unreachable_pub`, nothing against `unwrap()` or
  `panic!` in library code. The only trace of an intention is `#![allow(missing_docs)]` in
  `crates/corvis/src/__private.rs:7`, which allows a lint nobody enables.
- Rustdoc is never built with warnings denied. Two broken links are in the tree today
  (`crates/corvis-swagger/src/explorer.rs:31` links `crate::model`, which does not exist;
  `crates/corvis-swagger/src/scalar.rs:142` links the private `escape_inline_json`).
- `rust-version = "1.89"` (`Cargo.toml:23`) is declared and never built: CI pins 1.99.0 (`ci.yml:29`).
- Only `corvis` has its features built one by one (the clippy loop, `ci.yml:72-76`); `corvis-orm` never builds without
  its default features, and `corvis-swagger` gains features with `q-openapi-schemas-for-rust-types-and-versions`.
- Nothing reports a change of public API, so the API hygiene work of the other changes can be undone by accident.
- Shared dependencies are declared per crate (`regex = "1"` in `corvis` and `corvis-swagger`, `tokio-tungstenite` twice
  in `crates/corvis/Cargo.toml`, `chrono`, `futures` and `sha2` in `corvis` and `corvis-orm`).

The constraint: this change must land without fixing the whole code base at once. Every CI job builds with
`-D warnings`, so a lint added at `warn` in `[workspace.lints]` would fail every job on the first run. New lints
therefore go through a ratchet that only lets the count of findings go down.

## What Changes

- **Lint floor and lint ratchet.** `[workspace.lints]` holds the lints every crate already satisfies (measured in task
  1.2); every crate inherits them. The lints with findings (`missing_docs`, `unreachable_pub`, `unwrap_used`,
  `expect_used`, `panic`, `print_stdout` in libraries and the others of design.md Decision 1) are counted per crate and
  per lint by `scripts/lint-ratchet.mjs` against a committed `lint-baseline.json`: a pull request that raises a count
  fails; one that lowers it must lower the baseline in the same commit. A lint whose count reaches zero in every crate
  moves to `[workspace.lints]`. The same script counts the `ignore` doctests per crate.
- **New CI jobs.** `doc` (`cargo doc --workspace --no-deps --all-features` with `RUSTDOCFLAGS=-D warnings`), `msrv`
  (the published crates checked with the toolchain of `rust-version`, `--locked`), `features` (`cargo hack` builds each
  feature of `corvis-orm` and `corvis-swagger` on its own), `lint-ratchet`, and `semver` (`cargo semver-checks` against
  the merge base with `main`, informational). The first four become required checks of `main`.
- **The two broken doc links** are fixed, with any other rustdoc warning task 1.1 finds.
- **`rust-version` made true.** If 1.89 does not build the published crates, `rust-version` becomes the oldest toolchain
  that does (design.md Decision 3); README and docs state it.
- **Shared dependencies in `[workspace.dependencies]`**, and `scripts/check-crate-files.mjs` reports a dependency that
  two published crates declare on their own.
- **Two decisions are written up for the maintainer, not taken:** whether the framework keeps forcing `tokio`'s `full`
  feature and `serde_json`'s `preserve_order` on applications (design.md Open questions 1 and 2, with where
  `preserve_order` is load-bearing).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `infra/ci`: the definition-of-done job list and the required checks of `main` gain `doc`, `msrv`, `features` and
  `lint-ratchet`; added requirements for each new job and for the informational `semver` job.
- `infra/packaging`: added requirement that a dependency shared by published crates is declared once.

## Impact

- New: `scripts/lint-ratchet.mjs`, `lint-baseline.json`.
- Edited: `Cargo.toml` (`[workspace.lints]`, `[workspace.dependencies]`, possibly `rust-version`), the seven published
  crates' `Cargo.toml` (`[lints] workspace = true`, `workspace = true` dependencies), `clippy.toml`
  (`allow-unwrap-in-tests`, `allow-expect-in-tests`, `allow-panic-in-tests`), `.github/workflows/ci.yml`,
  `scripts/check-crate-files.mjs`, `crates/corvis-swagger/src/explorer.rs` and `scalar.rs` (doc links), any file task
  1.1 finds with a rustdoc warning, `CONTRIBUTING.md` (lint ratchet, new jobs), `README.md` and `docs/` pages that state
  the minimum Rust version (only if it changes).
- Public API: none. Behavior: none. If `rust-version` rises, applications on an older toolchain get Cargo's
  `rust-version` error instead of a compile error inside the framework.
- Dependencies: no new crate in `Cargo.lock`. CI installs `cargo-hack` and `cargo-semver-checks` with
  `taiki-e/install-action` (already pinned in `ci.yml`), at exact versions.
- No framework runtime code changes (doc comments and manifests only), so the corvis-parity check is not required;
  task 6.5 runs it anyway if `rust-version` or a dependency declaration changed what `Cargo.lock` resolves.

## Out of scope

- Fixing the findings the ratchet counts: each crate's count goes down in the changes that touch it (the other `q-`
  changes lower the baseline for `corvis-swagger`, `corvis-swagger-macros` and `corvis-cli`).
- A coverage report: it does not stop regressions; a later issue if wanted.
- Running the benchmark in CI: timings on shared runners cannot gate a merge (house rule: no test compares timings);
  the benchmark crates already compile in `cargo build --workspace --all-targets`.
- Changing `tokio` or `serde_json` features (maintainer decisions, design.md Open questions 1 and 2).

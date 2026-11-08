# Tasks

## 1. Versioned intra-workspace dependencies

- [x] 1.0 Translate `THIRD_PARTY_NOTICES.md` to English (same content and license texts verbatim); verify no Spanish
  prose is left and the license texts are unchanged (`git diff` touches only the surrounding prose)
- [x] 1.1 In the root `Cargo.toml`, give `corvis` and `corvis-macros` in `[workspace.dependencies]` a `version`
  (`"0.1.0"` and `"=0.1.0"`), and add `corvis-orm-macros = { path = "crates/corvis-orm-macros", version = "=0.1.0" }`
  and `corvis-swagger-macros = { path = "crates/corvis-swagger-macros", version = "=0.1.0" }`; verify with
  `cargo metadata --no-deps --format-version 1` that every dependency between workspace crates has a `req` other
  than `*`
- [x] 1.2 Switch `crates/corvis-orm/Cargo.toml` (`corvis-orm-macros`) and `crates/corvis-swagger/Cargo.toml`
  (`corvis-swagger-macros`) to `{ workspace = true }`; verify `Cargo.lock` is unchanged (`git diff Cargo.lock` empty)
- [x] 1.3 Set `wildcards = "deny"` in `deny.toml` and update the comment above it (lines 29-32); verify
  `cargo deny check bans` passes, and that removing one `version` on a scratch edit makes it fail

## 2. Metadata

- [x] 2.1 Add `rust-version` (start at `"1.88"`) and `homepage = "https://corvis.randygil.com"` to
  `[workspace.package]`, and `rust-version.workspace = true` / `homepage.workspace = true` to the seven crates; verify
  with `cargo metadata --no-deps` that each crate reports both
- [x] 2.2 Measure the real minimum: install the candidate toolchain (`rustup toolchain install 1.88 --profile
  minimal`) and run `cargo +1.88 check --workspace` with the shared target dir; raise the version until it passes;
  write the final value in `[workspace.package]` and in design.md; uninstall the extra toolchains afterwards; verify
  the recorded version passes
- [x] 2.3 Complete `crates/corvis-cli/Cargo.toml` (`description`, `authors.workspace`, `repository.workspace`,
  `keywords`, `categories`, `readme`) and add `keywords` / `categories` to the other crates as listed in design
  decision 8; verify `cargo package --list -p corvis-cli --allow-dirty` prints no manifest warning
- [x] 2.4 Write a short English `README.md` for `corvis-macros`, `corvis-orm-macros`, `corvis-swagger-macros`,
  `corvis-swagger` and `corvis-cli` (what the crate is, that macro crates are not used directly, links to the main
  crate, the docs site and the repository) and point `readme` at it; verify each appears in its `cargo package --list`

## 3. License and notices in every crate

- [x] 3.1 Copy the root `LICENSE` and `THIRD_PARTY_NOTICES.md` into each of the seven crate folders (keep LF, matching
  `.gitattributes` if it covers them); verify `cargo package --list -p <crate> --allow-dirty` lists both files for all
  seven crates
- [x] 3.2 Write `scripts/check-crate-files.mjs`: for every package without `publish = false` (from `cargo metadata
  --no-deps --format-version 1`), fail when `LICENSE` or `THIRD_PARTY_NOTICES.md` is missing or differs from the root,
  when `description`, `license`, `repository`, `homepage`, `rust_version`, `readme`, `keywords` (1-5) or `categories`
  is missing, or when a dependency on a workspace crate has `req` `*`; print one line per problem; verify it passes,
  then fails with clear messages after editing one copy and removing one category on a scratch edit

## 4. Package the workspace

- [x] 4.1 Run `cargo package --allow-dirty` with `-p` for each of the seven crates (`--workspace` fails on the `publish = false` examples with Cargo 1.99) (with the shared `CARGO_TARGET_DIR`); fix any error or
  warning; check that seven `.crate` files are produced and none for the examples or `bench/corvis-app`; record the
  size of each package in design.md; delete `target/package/` afterwards; verify the run is clean
- [x] 4.2 Inspect `target/package/corvis-0.1.0/Cargo.toml` (before deleting it) and confirm the macro dependencies
  are `=0.1.0` without `path`; verify by reading the normalized manifest

## 5. Contributor documentation and CI hook

- [x] 5.1 Add a "Packaging" section to `CONTRIBUTING.md`: the root `LICENSE` / `THIRD_PARTY_NOTICES.md` are the
  sources and the crate copies are refreshed from them, `node scripts/check-crate-files.mjs`, `cargo package
  --workspace --allow-dirty`, cleaning `target/package/`, and the exact pin of macro crates; verify every command runs
  as written
- [x] 5.2 If the CI workflow of #6 is already in `main`, add the check script (and `cargo package --workspace` if its
  run time is acceptable) to it; otherwise note it in #6's change or as a follow-up comment for the maintainer; verify
  the job passes on a pull request, or that the note exists

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes
- [x] 6.4 Regression checks: `node scripts/check-crate-files.mjs` and `cargo deny check bans` pass, and the
  scratch-edit failures of 1.3 and 3.2 were observed (record them in the pull request)
- [x] 6.5 Docs: no user-facing behavior changes, so no page under `docs/` or `docs/es/` changes (the "not on crates.io
  yet" sentences stay true until #11); confirm, and confirm `npm run docs:check` in `docs/` still passes
- [x] 6.6 corvis-parity: no framework runtime code changes (manifests, copies, script and config only); confirm that
  `Cargo.lock` is unchanged, so the three port apps keep 0 apidiff differences without a rerun

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #10, tick it in epic #1 (and in the execution plan #111), and move its card to Done (see the
  `corvis-task` skill).

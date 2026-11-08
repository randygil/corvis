# Tasks

## 1. Dependency selection

- [x] 1.1 Change `corvis_dependency` in `crates/corvis-cli/src/main.rs` to
  `corvis_dependency(args: &[String], env_path: Option<String>) -> Result<String, String>` with the precedence of
  design decision 3 (conflicting flags -> error; `--corvis-version`; `--corvis-path`; non-empty `CORVIS_PATH`;
  otherwise `version = "<CARGO_PKG_VERSION without +metadata>"`), keeping the existing path normalization and
  validation and naming `CORVIS_PATH` in the error when the path came from it; remove the `CARGO_MANIFEST_DIR`
  fallback; `cmd_new` passes `std::env::var("CORVIS_PATH").ok()`; verify with unit tests in the `tests` module of
  `main.rs`: default, flag path, environment path, flag beats environment (both kinds), empty `CORVIS_PATH` ignored,
  both flags rejected, unsafe `CORVIS_PATH` rejected with its name in the message
- [x] 1.2 Add the `NOTE` lines of design decision 6 (build checkout exists while the default version is used; path
  taken from `CORVIS_PATH`), with the text built by a function; verify with a unit test of that function and by
  running the workspace build of the CLI (`$CARGO_TARGET_DIR/debug/corvis new note-check --skip-git`) in a temporary
  directory outside the repository (the note names `<repo>/crates/corvis`), then deleting `note-check`
- [x] 1.3 Update the help text (`print_help`, design decision 7) and the module doc comment if it mentions the
  dependency; verify `cargo run -q -p corvis-cli -- --help` shows the default and `CORVIS_PATH`

## 2. Integration tests

- [x] 2.1 In `crates/corvis-cli/tests/cli_input.rs`, make the `corvis()` helper remove `CORVIS_PATH` from the child
  environment (`Command::env_remove`), so a developer's setting cannot change the results; add a variant that sets it;
  verify the existing tests still pass with `CORVIS_PATH` exported in the shell
- [x] 2.2 Add `crates/corvis-cli/tests/cli_new.rs` (or tests in `cli_input.rs`): the default writes
  `corvis = { version = "<env!("CARGO_PKG_VERSION")>" }` and the file contains no `path =`; `--corvis-path` writes the
  path; `CORVIS_PATH` writes the path and prints the `NOTE`; `--corvis-version` beats `CORVIS_PATH`; both flags fail
  and create no folder; verify each test fails against the current code (default case) before the change and passes
  after it
- [x] 2.3 End-to-end check, once, by hand, with the workspace build of the CLI in a temporary directory outside the
  repository: `corvis new e2e --skip-git --corvis-path <repo>/crates/corvis`, then `cargo build` in `e2e` with the
  shared `CARGO_TARGET_DIR` (never a new target directory); then `corvis new e2e-default --skip-git` and
  `cargo build` there, and record the resolution error (expected before the first release, design decision 5); delete
  both folders; verify the first build succeeds and the second fails only on resolving `corvis`

## 3. Documentation (English and Spanish)

- [x] 3.1 `docs/cli/overview.md`, "Framework dependency": the default is the published version that matches the CLI;
  `--corvis-path` / `CORVIS_PATH` for a local checkout (precedence, flag beats variable, both flags rejected); a
  callout that until the first release on crates.io you set `CORVIS_PATH=<clone>/crates/corvis` (shell example for
  Bash and PowerShell); update the TOML sample to the version form and keep a path sample; same in
  `docs/es/cli/overview.md`; verify both pages say the same thing section by section
- [x] 3.2 `docs/cli/usages.md`, `corvis new`: synopsis `[--corvis-path <path> | --corvis-version <requirement>]`,
  options table (default row, `CORVIS_PATH`, the conflict error, the `NOTE` lines); same in `docs/es/cli/usages.md`;
  verify against `corvis --help`
- [x] 3.3 `docs/first-steps.md`: dependency table (none -> version requirement equal to the CLI version) and replace
  the tip with the pre-release instruction (`CORVIS_PATH`); same in `docs/es/first-steps.md`; verify the commands in
  the page run as written from a clone
- [x] 3.4 `README.md:98-99`: say that `corvis new` depends on the published version by default and that
  `--corvis-path` / `CORVIS_PATH` select a local checkout; verify the README has no other sentence that describes the
  old default (`grep -n "built from" README.md`)
- [ ] 3.5 `npm run docs:check` in `docs/` passes

## 4. Definition of done

- [x] 4.1 `cargo fmt --all --check` passes
- [ ] 4.2 `cargo build --workspace --all-targets` with no warnings
- [x] 4.3 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 4.4 `cargo test --workspace` passes, including the new unit and integration tests of 1.1, 1.2, 2.1 and 2.2
- [ ] 4.5 Docs in English and Spanish updated (3.1-3.4) and `npm run docs:check` passes
- [x] 4.6 `node scripts/check-naming.mjs` passes
- [x] 4.7 corvis-parity: only `crates/corvis-cli` changes (no framework runtime crate), so no parity run is required;
  confirm with `git diff --stat` that no file under `crates/corvis/`, `crates/corvis-macros/`, `crates/corvis-orm*` or
  `crates/corvis-swagger*` changed and that `Cargo.lock` is unchanged
- [ ] 4.8 File the follow-up issue for the VS Code extension (design, open question 3) if the maintainer agrees, and
  link it from the pull request

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive` after review
- [ ] Close issue [#121](https://github.com/randygil/corvis/issues/121)
- [ ] Tick #121 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [ ] Move the card to Done in the Corvis Roadmap project (see the `corvis-task` skill)

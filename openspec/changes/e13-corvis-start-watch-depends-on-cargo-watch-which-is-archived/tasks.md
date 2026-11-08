# Tasks

## 1. Confirm watchexec behavior

- [ ] 1.1 Install the current watchexec release (`cargo binstall watchexec-cli` or `cargo install --locked
  watchexec-cli`, using the shared target dir) and record its version in design.md. In a project created with
  `corvis new demo --skip-git --corvis-path <clone>/crates/corvis`, run
  `watchexec --restart --shell=none --project-origin <root> --watch <root> --ignore /target/** -- cargo run -- --name "my app"`
  with a `main` that prints its arguments, and check: (a) `my app` arrives as one argument (Windows and one Unix
  platform if available); (b) saving `src/main.rs` once gives exactly one rebuild, with no loop from `target/`;
  (c) Ctrl+C stops watchexec and the server. Adjust decisions 2 and 4 if any flag behaves differently; verify by
  writing the observed results in design.md under "Implementation notes"

## 2. Command builder

- [ ] 2.1 In `crates/corvis-cli/src/main.rs`, rename `find_cargo_subcommand` to `find_executable(file_stem)` (same
  directories, `EXE_SUFFIX` appended) and update `finds_cargo_watch_in_cargo_home` into
  `finds_watchexec_in_cargo_home`. Verify: `cargo test -p corvis-cli finds_`
- [ ] 2.2 Add `watch_invocation(args, root) -> Vec<String>` (decision 2) and make `cargo_invocation` handle only the
  non-watch forms; remove `watch_safe`. Unit tests replacing `builds_watch_commands` and
  `watch_rejects_shell_sensitive_arguments`: `--watch`; `--watch --release`; `--watch --prod -- --port 4000`;
  `--watch -- --name "my app"`; `--watch -- "a;b" "$HOME" "%PATH%" "a&b" "a|b" "\"q\"" ""` (each kept as one element,
  in order, after `-- cargo run --`); `-- --watch` still not watch mode. Verify: `cargo test -p corvis-cli`
- [ ] 2.3 Add `workspace_root()` running `cargo locate-project --workspace --message-format plain` (honoring
  `CORVIS_CARGO` like `posture.rs:254-256` if that helper is shared) and returning the manifest's parent folder, or
  the Cargo error text. Verify: unit test of the output parsing (trailing newline, Windows path) without running Cargo

## 3. Command and messages

- [ ] 3.1 `cmd_cargo`: in watch mode, look for `watchexec` first (decision 3); if missing, return the message of
  decision 5; otherwise locate the root, build the arguments, and spawn the found `watchexec` path. Verify:
  `corvis start --watch` in a scaffolded project restarts on save (manual, after 1.1)
- [ ] 3.2 `print_help`: `start [--watch] [--release] [-- args]` -> `Run the application (cargo run; with --watch,
  watchexec restarts it when files change).` Update the doc comment at the top of `main.rs` if needed. Verify:
  `corvis --help`
- [ ] 3.3 `crates/corvis-cli/tests/cli_input.rs`: rename the missing-tool test to
  `start_watch_without_watchexec_explains_how_to_install_it` and assert `watchexec is not installed`,
  `cargo install --locked watchexec-cli` and `without --watch`; add a case with a fake `cargo-watch` file in the
  empty `CARGO_HOME/bin` that still fails the same way; delete `start_watch_rejects_shell_arguments`. Verify:
  `cargo test -p corvis-cli --test cli_input`

## 4. Docs

- [ ] 4.1 `docs/cli/usages.md` (`corvis start`, lines 85-101): the three watch rows show the watchexec command (short
  form `watchexec -r -n ... -- cargo run`, with the full flags in the paragraph); replace the cargo-watch paragraph
  with: install options, workspace root and `target` ignore, arguments passed unchanged, the one-line note for users
  of cargo-watch (open question 3). `docs/cli/scripts.md` (lines 12 and 17), `docs/cli/overview.md` (line 57),
  `docs/recipes/hot-reload.md` (lines 8-14, plus the Windows restart note from Risks), root `README.md` line 91.
  Verify: every command shown matches the unit tests of 2.2
- [ ] 4.2 Same changes in `docs/es/cli/usages.md`, `docs/es/cli/scripts.md`, `docs/es/cli/overview.md`,
  `docs/es/recipes/hot-reload.md`. Verify: `npm run docs:check` in `docs/`; `grep -rn "cargo-watch\|cargo watch" docs
  README.md crates/corvis-cli` finds only the migration note

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes
- [ ] 5.4 Regression tests: the command-builder unit tests (2.2, 2.3), `finds_watchexec_in_cargo_home` (2.1) and the
  missing-tool integration test (3.3)
- [ ] 5.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [ ] 5.6 corvis-parity: not required, this change only touches `crates/corvis-cli` (no framework runtime code);
  state this in the commit message

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#143](https://github.com/randygil/corvis/issues/143)
- [ ] Tick #143 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project

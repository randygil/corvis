# Tasks

## 1. Command builder

- [ ] (NOT DONE: cargo-watch is not installed on this machine, so how it runs -x was not verified; the design rests on its documentation) 1.1 Check how the installed cargo-watch runs `-x` (`cargo watch --help`, the `--use-shell`/`--shell` option and
  its default on Windows and Unix) and record the version in design.md; adjust the error text of Decision 3 if `-x` is
  not run through a shell; verify by running `cargo watch -x "run -- --port 4000"` in a scaffolded project and
  checking that the program receives `--port 4000`
- [x] 1.2 Add `cargo_invocation(args, sub) -> Result<Vec<String>, String>` in `crates/corvis-cli/src/main.rs`, split
  at the first `--` (Decision 2), with the watch form and allow-list of Decision 3; make `cmd_cargo` spawn `cargo`
  with its result; verify with unit tests in a new `#[cfg(test)] mod tests` in `main.rs` for: `start`,
  `start --release`, `start --prod`, `start -- a b`, `start -- --release`, `start -- --watch`, `start --watch`,
  `start --watch --release`, `start --watch --prod -- --port 4000`, `build`, `build --release`, `build -- x`
- [x] 1.3 Unit tests for the allow-list: `--config=./dev.toml`, `127.0.0.1` and `http://localhost:3000/x` accepted;
  `my app`, the empty string, `a;b`, `$HOME`, `%PATH%`, `a&b`, `a|b` and `"q"` rejected with the argument named in
  the error; `start -- "my app"` (no watch) accepted; verify `cargo test -p corvis-cli`

## 2. Help and integration test

- [x] 2.1 Update `print_help` to `start [--watch] [--release] [-- args]` with the description of Decision 5; verify
  with `corvis --help`
- [x] 2.2 Add `start_watch_rejects_shell_arguments` to `crates/corvis-cli/tests/cli_input.rs`: in a sandbox, run
  `corvis start --watch -- "my app"` and assert a failing exit and that the output names `my app` and
  `without --watch`; verify `cargo test -p corvis-cli` (the test starts no Cargo process)

## 3. Docs

- [x] 3.1 `docs/cli/usages.md` (`corvis start` section, lines 79-91): add the rows `corvis start --watch --release` →
  `cargo watch -x "run --release"` and `corvis start --watch -- --port 4000` → `cargo watch -x "run -- --port 4000"`;
  replace "In that mode, `--release` and the arguments after `--` are ignored" with the `--` rule (everything after it
  goes to the program) and the watch-mode character restriction; check `docs/cli/scripts.md` and
  `docs/recipes/hot-reload.md` for consistency; verify each example against the unit tests of group 1
- [x] 3.2 Same changes in `docs/es/cli/usages.md`, `docs/es/cli/scripts.md` and `docs/es/recipes/hot-reload.md`;
  verify `npm run docs:check` in `docs/`

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes
- [x] 4.4 Regression tests: the command-builder unit tests (1.2, 1.3) and the integration test (2.2)
- [x] 4.5 Docs updated in English and Spanish (`docs/` and `docs/es/`); `npm run docs:check` in `docs/` passes
- [x] 4.6 corvis-parity: not required (CLI-only change), this change only touches `crates/corvis-cli`, not framework runtime code; state
  this in the commit message

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#127](https://github.com/randygil/corvis/issues/127)
- [ ] Tick #127 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project

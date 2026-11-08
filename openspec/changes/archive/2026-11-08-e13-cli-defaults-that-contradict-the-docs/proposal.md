# CLI defaults that contradict the docs

Implements [randygil/corvis#20](https://github.com/randygil/corvis/issues/20) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The docs review found three places where the `corvis` CLI and what users read disagree, or where the CLI fails in a
way that does not explain itself:

- The README says `corvis build` runs `cargo build --release`; the CLI (and the docs site) build in debug unless
  `--release` is passed. A user following the README ships a debug binary.
- `corvis g controller profile --path users` looks for `src/users/profile_module.rs`, does not find it, and registers
  the controller in `AppModule` even when `src/users/users_module.rs` exists. The docs page currently documents this
  as a quirk and tells users to move the registration by hand.
- `corvis start --watch` runs `cargo watch -x run`; without `cargo-watch` installed the user gets Cargo's
  `no such command: watch` instead of an explanation.

## What Changes

- `corvis build` keeps Cargo's convention (debug by default, `--release`/`--prod` for release), which the docs site
  already describes; the README line that says otherwise is corrected, and `corvis build` prints which profile it
  builds.
- `corvis generate` with `--path <dir>` registers controllers, services and gateways in the module that owns the folder:
  `src/<dir>/<name>_module.rs` if it exists, otherwise `src/<dir>/<dir>_module.rs`, otherwise `src/app_module.rs`. The
  CLI prints which module file received the registration.
- `corvis start --watch` checks that `cargo-watch` is installed before running it, the same way Cargo finds external
  subcommands (`PATH` and `$CARGO_HOME/bin`). If it is missing, the command fails with a message that explains how to
  install it (`cargo install cargo-watch`) and how to run without watch mode.
- Docs (`cli/usages`, `cli/overview`, `cli/scripts`, `recipes/hot-reload`) in English and Spanish describe the new
  registration rule and the missing-`cargo-watch` message; the "move it by hand" paragraph goes away.

## Capabilities

### New Capabilities
- `cli/generate`: where `corvis generate` writes files and which module receives the registration.
- `cli/start-build`: what `corvis start` and `corvis build` run, their defaults and their errors.

## Impact

- Code: `crates/corvis-cli/src/main.rs` (`cmd_generate`'s `simple` closure at lines 248-259, `cmd_cargo` at lines
  332-349, `print_help`).
- Tests: `crates/corvis-cli/tests/cli_input.rs` gains cases (or a new `tests/cli_defaults.rs`).
- Docs: `README.md` line 110; `docs/cli/usages.md`, `docs/cli/overview.md`, `docs/cli/scripts.md`,
  `docs/recipes/hot-reload.md` and their `docs/es/` translations.
- No framework runtime code changes.

## Out of scope

- `--release` and arguments after `--` being ignored in `--watch` mode (documented today); passing them through to
  `cargo watch -x "run ..."` can be a separate issue.
- Alternatives to `cargo-watch` (`bacon`, `watchexec`) or a built-in watcher.
- Registering guards, interceptors, pipes, filters or middleware (they are not registered by design).
- Translating the rest of the README to English (issue #3).

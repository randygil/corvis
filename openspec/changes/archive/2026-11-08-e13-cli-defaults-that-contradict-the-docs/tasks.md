# Tasks

## 1. Build profile

- [x] 1.1 In `cmd_cargo` (`crates/corvis-cli/src/main.rs`), print `Building the debug profile (use --release for an
  optimized build)` or `Building the release profile` for `build` (and the equivalent for `start`) before running
  Cargo; verify manually with `corvis build` in a scaffolded project that the line appears and that `--release` is
  passed only when requested
- [x] 1.2 Fix `README.md:110` to `corvis build  # cargo build (debug); --release for release` (in the README's current
  language); verify by grepping the README and `docs/` for any other `corvis build` description that says release

## 2. Registration with `--path`

- [x] 2.1 Extract the target-module choice from the `simple` closure into a function
  `registration_target(src, dir, name_snake, flat) -> PathBuf` implementing the order
  `<dir>/<name>_module.rs` → `<dir>/<dir>_module.rs` → `app_module.rs`; verify with unit tests in `main.rs` over a temp
  `src/` tree for the four scenarios of `specs/cli/generate/spec.md`
- [x] 2.2 Print the note when the fallback is `app_module.rs` and the folder has other `*_module.rs` files; verify with
  an integration test that scaffolds a project (`corvis new app --skip-git --corvis-version 0.1`), runs
  `g module users`, then `g controller profile --path users`, and asserts `UsersModule` contains `ProfileController`
  and `app_module.rs` does not
- [x] 2.3 Add an integration test for `g service users` after `g module users` (default folder, unchanged behavior);
  verify `cargo test -p corvis-cli`

## 3. cargo-watch detection

- [x] 3.1 Add `find_cargo_subcommand("watch") -> Option<PathBuf>` searching `PATH` entries and `$CARGO_HOME/bin`
  (default `~/.cargo/bin`) for `cargo-watch` + `EXE_SUFFIX`; call it in `cmd_cargo` before spawning `cargo watch` and
  return the error text from design.md when it is missing; verify with an integration test that runs
  `corvis start --watch` with `PATH` and `CARGO_HOME` set to empty temp dirs and asserts a failing exit and
  `cargo install cargo-watch` in the output
- [x] 3.2 Add a test where a dummy `cargo-watch` file exists in the temp `CARGO_HOME/bin`, asserting the detection
  function finds it (unit test, so no Cargo process is spawned); verify `cargo test -p corvis-cli`

## 4. corvis-orm schema search

- [x] 4.1 Append `db/schema.prisma` and `db/schema` to the corvis-orm CLI's default schema search (after the existing
  paths, order unchanged) and list the search order in its help text; verify with a test or a recorded manual run in a
  temporary folder that only has `db/schema.prisma` and `db/migrations/`, and that a folder with both `prisma/` and
  `db/` still uses `prisma/schema.prisma`

## 5. Docs

- [x] 5.1 `docs/cli/usages.md`: replace the "exact rule" paragraph (line 66) with the new order and an example with
  `--path users`; mention the profile line printed by `start`/`build`; describe the missing-`cargo-watch` message
  (line 91); check `docs/cli/overview.md`, `docs/cli/scripts.md` and `docs/recipes/hot-reload.md` for consistency;
  verify the examples against the CLI behavior from groups 1-3
- [x] 5.2 Same changes in `docs/es/cli/usages.md`, `docs/es/cli/overview.md`, `docs/es/cli/scripts.md` and
  `docs/es/recipes/hot-reload.md`; verify `npm run docs:check` in `docs/`

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the new regression tests in `crates/corvis-cli`
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`, plus `README.md`); `npm run docs:check` in
  `docs/` passes
- [x] 6.5 corvis-parity: not required, this change does not touch framework runtime code (only `crates/corvis-cli`);
  state this in the commit message

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#20](https://github.com/randygil/corvis/issues/20)
- [x] Tick #20 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [x] Move the card to Done in the Corvis Roadmap project

# Design

## Context

The CLI is a single binary, `crates/corvis-cli/src/main.rs` (370 lines), with hand-written argument parsing (`flag`,
`opt`, `positional` at lines 107-132) and text editors in `src/edit.rs` (`add_mod`, `add_use`, `add_to_module`).

- **Build.** `main` maps `start` to `cmd_cargo(args, "run")` and `build` to `cmd_cargo(args, "build")`
  (lines 34-35). `cmd_cargo` (lines 332-349) adds `--release` only when `--release` or `--prod` is present, so the
  default is debug. The help text says `build [--release]` (line 68). The docs site agrees
  (`docs/cli/usages.md:93-99`, `docs/cli/scripts.md:13-14`, `docs/cli/overview.md:58`); only `README.md:110`
  says `corvis build  # cargo build --release`.
- **Registration with `--path`.** `cmd_generate` (lines 216-279) sets `dir = --path or n.snake` (line 222). The
  `simple` closure registers controllers/services/gateways (lines 248-259) in
  `src/<dir>/<name>_module.rs` if it exists and the element is not `--flat`, otherwise in `src/app_module.rs`. With
  the default `dir == n.snake` that is the folder's module; with `--path users` and name `profile` it looks for
  `src/users/profile_module.rs` and falls back to `AppModule` even if `src/users/users_module.rs` exists.
  `docs/cli/usages.md:66` (and `docs/es/cli/usages.md:66`) documents this as the "exact rule" and tells users to move
  the registration by hand. The `use` path is `crate::<dir>::<Type>` (line 250), which works for any module file in
  the folder because `mod.rs` gets a `pub use` (lines 243-246). Modules themselves (`g module`) and resources are
  registered in `AppModule` imports by `register_in_app_module` (lines 303-313); that is not affected.
- **Watch.** `cmd_cargo` with `run` and `--watch` builds `cargo watch -x run` (lines 334-336) and maps any spawn error
  to `failed to run cargo`; when `cargo-watch` is missing, Cargo itself prints `error: no such command: watch` and the
  CLI adds `cargo run failed`. `docs/cli/usages.md:91`, `docs/cli/scripts.md:17` and `docs/recipes/hot-reload.md:10`
  already say it needs `cargo install cargo-watch`.
- **Tests.** `crates/corvis-cli/tests/cli_input.rs` runs the real binary (`env!("CARGO_BIN_EXE_corvis")`) in a temp
  sandbox (`sandbox`, `corvis` helpers at lines 7-18); `generate_rejects_unsafe_paths` already scaffolds a project with
  `corvis new app --skip-git --corvis-version 0.1` and runs `g service cats --path animals`. `src/edit.rs` has unit
  tests for the editors.

## Goals / Non-Goals

**Goals:**
- What the README, the docs site and `corvis --help` say about `build` is what the CLI does.
- `--path` registers in the module that owns the folder without manual edits.
- A missing `cargo-watch` produces an actionable message.

**Non-Goals:**
- A new argument parser, passing `--release`/`--` through in watch mode, other watchers (see proposal).

## Decisions

1. **Keep debug as the default build and fix the README.** Debug-by-default is Cargo's convention, the help text and
   every docs page already describe it, and `--release` is one flag away. Printing the profile before building
   removes the ambiguity in the terminal. Alternative: make `corvis build` default to release (as the README claims and
   as the original framework's `build` produces production output); rejected because it would contradict five docs
   pages and the help text, and silently slow down every local build. See Open questions.
2. **Registration order `<dir>/<name>_module.rs` → `<dir>/<dir>_module.rs` → `app_module.rs`.** The first entry keeps
   today's behavior when a module named after the element exists (including the default folder case, where both are
   the same file), so nothing that works today changes. The second entry is the folder's own module, which is what
   the docs table promises ("providers of the folder's module, or of `AppModule`"). Alternatives: scan the folder for
   any `*_module.rs` and pick it when there is exactly one (more magic, and ambiguous with two); add a `--module <file>`
   option (more surface than the issue asks; can come later).
3. **Report the target.** `update_file` already prints `UPDATE <path>`; when the registration falls back to
   `app_module.rs` while the folder has other `*_module.rs` files, the CLI also prints a one-line note naming them, so
   the user can move it if that was not intended. When no module file exists at all, behavior is unchanged (no
   registration, as today when `app_module.rs` is missing).
4. **Detect `cargo-watch` the way Cargo does, without spawning Cargo.** Look for `cargo-watch` (plus
   `std::env::consts::EXE_SUFFIX`) in each `PATH` entry and in `$CARGO_HOME/bin` (default `~/.cargo/bin`, using
   `USERPROFILE` on Windows and `HOME` elsewhere). Alternative: run `cargo watch --version` and inspect the result;
   rejected because it spawns Cargo twice, is slower, and is hard to test without a toolchain on `PATH`. The file
   lookup can be tested by running the binary with `PATH` and `CARGO_HOME` pointing to empty temp dirs.
5. **Error text.** `cargo-watch is not installed: corvis start --watch runs "cargo watch -x run". Install it with
   "cargo install cargo-watch", or run "corvis start" without --watch.` Exit code 1 through the existing
   `Err(String)` path in `main` (lines 49-55).

## Risks / Trade-offs

- [A project where users relied on the `AppModule` fallback with `--path`] → only changes when `<dir>/<dir>_module.rs`
  exists, which is the case the docs call wrong; the printed `UPDATE` path shows where it went.
- [`cargo-watch` installed somewhere Cargo finds but the lookup does not (custom Cargo config `[install] root`)] →
  rare; the message tells the user what is missing, and the check covers the two locations Cargo documents for
  external subcommands. If it becomes a problem, fall back to `cargo watch --version` when the lookup fails.

## Open questions

- Confirm that `corvis build` should stay debug by default (recommended) rather than switch to release. If the
  maintainer prefers release, only `cmd_cargo`, the help text and the five docs pages change; the README would then be
  right as is.

## Review notes (2026-11-08)

- `corvis build` stays debug by default.
- Added from the review of #22: the corvis-orm CLI's default schema search does not include `db/schema.prisma`,
  although `docs/data/corvis-orm.md` uses that layout. Fix it here (task 4.1): append `db/schema.prisma` and
  `db/schema` after the existing paths, keep the existing order, list the search order in the help text.

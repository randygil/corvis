# Proposal

Implements [randygil/corvis#143](https://github.com/randygil/corvis/issues/143) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found while implementing #127 (archived change
`2026-11-08-e13-corvis-start-watch-drops-release-and-extra-arguments`).

## Why

`corvis start --watch` runs `cargo watch -x "<run command>"` (`crates/corvis-cli/src/main.rs:450-486`), and the docs
tell users to `cargo install cargo-watch`. cargo-watch's repository was archived on 2025-01-18; its README says it
will not receive further updates and points to Bacon or Watchexec. Recommending an unmaintained tool to every new
user is a bad first impression and a supply-chain liability.

cargo-watch also runs the `-x` value through a shell (`cmd` by default on Windows), so #127 had to restrict watch-mode
program arguments to a character allow-list (`watch_safe`, `main.rs:488-491`): `corvis start --watch -- --name "my
app"` is refused today.

## What Changes

- **Backend: watchexec.** `corvis start --watch` runs
  `watchexec --restart --shell=none --project-origin <root> --watch <root> --ignore /target/** -- cargo run
  [--release] [-- <program arguments>]`, where `<root>` is the Cargo workspace root of the current directory
  (`cargo locate-project --workspace --message-format plain`). The explicit `target` ignore matters for projects
  created with `--skip-git`, where watchexec does not read `.gitignore` (design.md, decision 4). The user-facing
  command and its flags do not change.
- **No shell, no allow-list.** With `--shell=none`, watchexec runs `cargo` directly with each argument as a separate
  `argv` entry, so watch mode accepts any program argument, like plain `corvis start`. `watch_safe` and its error are
  removed.
- **Missing tool message.** Before starting any process, the CLI looks for the `watchexec` executable in the
  directories of `PATH` and in `$CARGO_HOME/bin` (default `~/.cargo/bin`). If it is missing, the command fails with
  `watchexec is not installed`, the install command `cargo install --locked watchexec-cli`, a pointer to prebuilt
  binaries and package managers, and `corvis start` without `--watch` as the alternative. cargo-watch is no longer
  looked for or used.
- **Help text** of `start` names watchexec instead of `cargo watch -x`.
- **Tests:** unit tests of the new command builder (with and without `--release`, with program arguments containing
  spaces, `;`, `$`, `%`, `&`, `|` and quotes, passed unchanged), the missing-tool integration test updated, the
  allow-list tests removed.
- **Docs (English and Spanish):** `docs/cli/usages.md` (`corvis start` table and paragraph), `docs/cli/scripts.md`,
  `docs/cli/overview.md` (command table), `docs/recipes/hot-reload.md`, the root `README.md` line on
  `corvis start --watch`, and the matching pages under `docs/es/`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli/start-build`: watch mode runs watchexec instead of cargo-watch, passes program arguments unchanged, and explains
  a missing watchexec; the cargo-watch requirements and the watch-mode allow-list are removed.

## Impact

- Code: `crates/corvis-cli/src/main.rs` (`cargo_invocation`, `watch_safe` removed, `find_cargo_subcommand`
  generalized, `cmd_cargo`, `print_help`, unit tests) and `crates/corvis-cli/tests/cli_input.rs`.
- `corvis-cli` keeps zero dependencies (`crates/corvis-cli/Cargo.toml` has an empty `[dependencies]`).
- No framework runtime change: the corvis-parity check does not apply.
- Users who installed cargo-watch must install watchexec. The error message tells them how; the docs say it once.
- The VS Code extension's **Start Application (watch)** runs `corvis start --watch` (`editors/vscode/src/generate.ts:178`)
  and needs no change.

## Out of scope

- A watcher built into `corvis-cli` so `--watch` needs no extra install (design.md, decision 1 explains why not now;
  possible follow-up issue).
- Watching local path dependencies outside the workspace (for example a framework checkout used through
  `CORVIS_PATH`); cargo-watch did this through `cargo metadata`. Possible follow-up with an extra `--watch` per path.
- Rebuilding before stopping the running server (keep serving until the new build succeeds); watchexec restarts the
  whole `cargo run`, as cargo-watch did.
- Graceful shutdown on Windows: watchexec can only force-terminate there (as cargo-watch did); see Risks.

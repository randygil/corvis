# Design

## Context

Current implementation (verified in `crates/corvis-cli/src/main.rs`):

- `main` dispatches `start` to `cmd_cargo(&args[1..], "run")` (line 39).
- `find_cargo_subcommand(name)` (lines 434-445) looks for `cargo-<name><EXE_SUFFIX>` in the directories of `PATH` and
  in `$CARGO_HOME/bin` (falling back to `$USERPROFILE/.cargo` or `$HOME/.cargo`).
- `cargo_invocation(args, sub)` (lines 447-486) splits at the first `--`; in watch mode it builds one string
  `run[ --release][ -- <args joined by spaces>]`, rejects any argument that fails `watch_safe`, and returns
  `["watch", "-x", line]`.
- `watch_safe` (lines 488-491) accepts non-empty arguments of ASCII letters, digits and `-_./:=,+@`.
- `cmd_cargo` (lines 493-512) prints the profile when not watching; in watch mode it fails with
  `cargo-watch is not installed: ... Install it with "cargo install cargo-watch", or run "corvis start" without
  --watch.` when `find_cargo_subcommand("watch")` is `None`; then it runs `cargo <invocation>` and waits for it. The
  two watch error strings (lines 467-469 and 505-507) contain long runs of spaces because the string literals continue
  on a new line without a trailing `\`; they disappear with this change.
- `print_help` (lines 63-89) documents `start [--watch] [--release] [-- args]` as `cargo run, or cargo watch -x "run
  ..." with --watch`.
- Tests: `finds_cargo_watch_in_cargo_home` (lines 583-595), `builds_watch_commands` (618-631),
  `watch_rejects_shell_sensitive_arguments` (633-640) in `main.rs`; `start_watch_without_cargo_watch_explains_how_to_install_it`
  (`crates/corvis-cli/tests/cli_input.rs:114-131`, runs the binary with `PATH` and `CARGO_HOME` set to an empty
  folder) and `start_watch_rejects_shell_arguments` (`cli_input.rs:133-142`).
- `corvis-cli` has no dependencies (`crates/corvis-cli/Cargo.toml`, empty `[dependencies]`), and
  `crates/corvis-cli/src/posture/tree.rs:1-6` declares Windows Job Object and Unix process-group calls by hand "so the
  CLI keeps no dependencies".
- The generated project's `.gitignore` is `/target\n.env\n` (`crates/corvis-cli/src/templates.rs:28`); `corvis new
  --skip-git` skips `git init` (`main.rs:237-239`).
- Spec `openspec/specs/cli/start-build/spec.md`: requirements "Watch mode explains a missing cargo-watch", "Watch mode
  runs the same command as start", "Watch mode rejects arguments the shell would alter", "Plain start accepts any
  argument".
- Docs that name cargo-watch: `docs/cli/usages.md:94-101`, `docs/cli/scripts.md:12,17`, `docs/cli/overview.md:57`,
  `docs/recipes/hot-reload.md:10,14`, the same lines under `docs/es/`, and `README.md:91` (comment only).

Upstream facts used below (from the projects' own documentation, checked 2026-11-08):

- cargo-watch: archived 2025-01-18, last release 8.5.3 (2024-10-02); runs `-x` through a shell; by default watches the
  workspace and local path dependencies, minus `target/` and `.git/`.
- watchexec (CLI crate `watchexec-cli`, binary `watchexec`): `-r/--restart` restarts the command when an event arrives
  while it runs; `--shell=none` (`-n`) runs the command directly, "each distinct element of the command is passed as
  per the execvp(3) convention"; `-w/--watch` (default: current directory), `-i/--ignore` glob patterns,
  `--project-origin` (where ignore files are read and what a leading `/` in a pattern means); built-in ignores cover
  `.git`, `.hg`, `.svn` and similar but not `target`; VCS ignore files are honored only when that VCS is detected;
  `--wrap-process` defaults to a process group on Unix, a session on macOS and a Job Object on Windows; on restart it
  sends SIGTERM and force-kills after `--stop-timeout` (10 s) on Unix, and can only force-terminate on Windows.
  Install: `cargo install --locked watchexec-cli`, `cargo binstall watchexec-cli`, GitHub release binaries, Homebrew,
  Scoop, Chocolatey, Arch, Debian, Nix.
- Bacon: an interactive terminal UI built around jobs (check, clippy, test, run) configured in an optional
  `bacon.toml`; long-running programs use its run jobs, whose output is shown inside the TUI.

## Goals / Non-Goals

**Goals:**

- `corvis start --watch` stops depending on an archived tool, with the same flags and the same restart-on-save
  behavior.
- Watch mode passes any program argument, like plain `corvis start`.
- `corvis-cli` keeps zero dependencies.

**Non-Goals:**

- A built-in file watcher.
- Build-before-restart (keeping the old server up while compiling).
- Watching path dependencies outside the workspace.

## Decisions

1. **Backend: watchexec.** Options considered:

   | Option | Install for the user | CLI dependencies | Shell / allow-list | Process tree on restart | Fit for a server | Work |
   |---|---|---|---|---|---|---|
   | **watchexec** (chosen) | one tool (`cargo install --locked watchexec-cli`, binstall, or a package manager) | none | none (`--shell=none`) | process group / Job Object | good: restarts a long-running command, plain output | small: change the command builder |
   | Bacon | one tool | none | job config | its own | poor: TUI owns the terminal, server logs live inside it, a job in `bacon.toml` or its run job conventions decide how arguments reach the program; awkward in a VS Code task | small, but the UX changes |
   | Built-in on `notify` | none | first dependencies of the CLI (`notify`, a debouncer, platform backends; `ignore` for `.gitignore` support) | none | reuse `posture/tree.rs` | best possible (can build before restart) | large: debounce, ignore rules, restart loop, Ctrl+C on Windows, tests of file events |
   | Built-in polling with `std` only | none | none | none | reuse `posture/tree.rs` | good, 0.5-1 s latency | medium-large: same loop as above, approximate ignore rules |

   watchexec is maintained, is the engine cargo-watch itself was built on (so behavior stays close: restart on
   change, ignore files, debounce), runs commands without a shell, and keeps the CLI dependency-free. The issue keeps
   `corvis start --watch` as the command and asks only to swap the backend, which watchexec does with the smallest
   change.
   - Bacon rejected: it is a TUI for check/test loops; a server under it is usable interactively but not from the VS
     Code task (`editors/vscode/src/generate.ts:178`) or scripts.
   - Built-in rejected for now: the zero-install experience is the best one, but it is a new subsystem with its own
     flaky-test surface and, with `notify`, the CLI's first dependencies, against the stated "no dependencies" rule.
     It can replace watchexec later without changing the user-facing command (Out of scope, possible issue).

2. **Exact command.** `watchexec --restart --shell=none --project-origin <root> --watch <root> --ignore /target/**
   -- cargo run [--release] [-- <args>]`, passed to `Command` as separate arguments. `cargo_invocation` keeps its
   signature for `run`/`build` without watch and gains a watch-mode builder
   `watch_invocation(args, root) -> Vec<String>` (pure, unit-tested; no error case left since the allow-list goes).
   - `--restart`: the server never exits on its own; without it watchexec would ignore changes while it runs
     (`--on-busy-update=do-nothing` is the default).
   - `--shell=none`: removes the shell and with it `watch_safe`.
   - No `--clear`, `--quiet` or `--stop-timeout`: keep watchexec's defaults, which are what a user running it by hand
     gets; `NO_COLOR` is honored by watchexec itself.
   - Alternative: keep a shell and quote per platform. Rejected: quoting for `cmd`, PowerShell and POSIX shells is
     exactly the fragile part #127 avoided.

3. **Lookup and execution use the same path.** Generalize `find_cargo_subcommand` into `find_executable(file_stem)`
   (same directories: `PATH`, then `$CARGO_HOME/bin`), call it with `watchexec`, and spawn the returned path, so a
   `watchexec` found only in `~/.cargo/bin` (not on `PATH`) still runs. The lookup happens first, before the workspace
   root is located, so a missing tool starts no process (the integration test runs with an empty `PATH`, where `cargo`
   is not available either).

4. **Watch root and ignores.** The root is the directory of the manifest printed by
   `cargo locate-project --workspace --message-format plain` (plain text, no JSON parser needed). If that command
   fails, `corvis start --watch` fails with its message (`could not find Cargo.toml ...`) instead of starting
   watchexec. `--project-origin <root>` makes watchexec read ignore files there and makes `/target/**` mean
   `<root>/target/**`. The explicit `--ignore /target/**` is needed because watchexec honors `.gitignore` only when a
   git repository is detected, and `corvis new --skip-git` creates none: without it every build would trigger another
   restart. A `CARGO_TARGET_DIR` outside the root is not watched anyway; one inside the root other than `target` is
   rare and documented as a known limitation.
   - `.env` is listed in the generated `.gitignore`, so in a git repository saving `.env` does not restart, as with
     cargo-watch today. Kept unchanged.
   - Alternative: watch the current directory (watchexec's default). Rejected: started from `src/`, saving
     `Cargo.toml` would be missed.

5. **Message when watchexec is missing.** Exact text:
   `watchexec is not installed: corvis start --watch runs "watchexec --restart --shell=none ... -- cargo <run
   command>". Install it with "cargo install --locked watchexec-cli" (prebuilt binaries and package managers:
   https://github.com/watchexec/watchexec#install), or run "corvis start" without --watch.` Written on one logical
   line with `\` continuations so no stray spaces end up in it. No mention of cargo-watch: the docs' migration note
   covers existing users.

6. **No fallback to cargo-watch.** Keeping it when watchexec is missing would keep the shell path, the allow-list and
   two behaviors to document. A single backend is simpler; the migration cost is one install.

7. **No version check.** The flags used (`--restart`, `--shell=none`, `--watch`, `--ignore`, `--project-origin`)
   exist in watchexec 2.x; the docs say "watchexec 2.0 or later". An older binary fails with its own clap error,
   which names the unknown flag. Task 1.1 confirms the flags against the current release.

## Risks / Trade-offs

- [Users still install a tool] -> same friction as today, with a maintained tool and binary installs available; the
  built-in watcher remains a possible follow-up.
- [Windows: no graceful shutdown on restart] -> watchexec force-terminates the Job Object, as cargo-watch did; shutdown
  hooks do not run on restart in development. Documented in `hot-reload.md`.
- [`cargo locate-project` adds one Cargo call per start] -> it is fast (no build) and only in watch mode.
- [watchexec's command parsing] -> its docs call the `--shell=none` parsing "rudimentary" while also stating that each
  distinct argument is passed per execvp(3); task 1.1 verifies that an argument with a space reaches the program as
  one argument on Windows and on Linux/macOS before the allow-list is removed.
- [Path dependencies outside the workspace are not watched] -> regression compared with cargo-watch for framework
  contributors who use `CORVIS_PATH`; Out of scope, with a workaround (`watchexec` by hand with an extra `--watch`).

## Open questions

1. watchexec (recommended) or a built-in watcher? If the maintainer prefers zero install, the polling `std`-only
   variant keeps the no-dependency rule and would be a separate, larger change.
2. Should watch mode also pass `--stop-timeout 2s` so an application that ignores SIGTERM restarts faster on
   Unix? Recommendation: keep watchexec's default (10 s) to let shutdown hooks finish.
3. Should the docs keep a one-line note for users who still have cargo-watch installed ("no longer used, you can
   uninstall it")? Recommendation: yes, in `docs/cli/usages.md` only.

## Review notes (2026-11-08)

1. watchexec replaces cargo-watch, run without a shell and with an explicit `--ignore /target/**`.
2. Keep watchexec's default `--stop-timeout`.
3. Add a migration note for cargo-watch users in `docs/cli/usages.md` and its Spanish copy only.

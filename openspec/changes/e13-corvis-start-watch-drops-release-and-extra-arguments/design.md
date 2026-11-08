# Design

## Context

- **Dispatch.** `main` (`crates/corvis-cli/src/main.rs:29-56`) maps `start` to `cmd_cargo(&args[1..], "run")` and
  `build` to `cmd_cargo(&args[1..], "build")` (lines 34-35). Errors returned as `Err(String)` are printed as
  `Error: ...` and exit with `ExitCode::FAILURE` (lines 49-55).
- **`cmd_cargo`** (lines 331-348):
  - With `sub == "run"` and `flag(args, "--watch")` (line 333) it builds `cargo watch -x run` (lines 334-335) and
    nothing else: `--release`, `--prod` and the arguments after `--` are dropped.
  - Otherwise it runs `cargo <sub>`, adds `--release` when `flag(args, "--release") || flag(args, "--prod")`
    (lines 339-341) and appends `--` plus everything after the first `--` (lines 342-344).
  - `flag` (lines 107-109) is `args.iter().any(|a| a == name)`: it scans the whole slice, including the program
    arguments after `--`. So `corvis start -- --release` runs `cargo run --release -- --release`, and
    `corvis start -- --watch` starts `cargo watch -x run`.
  - The command is spawned directly with `cmd.status()` (line 346); no function returns the command line, so nothing
    can be tested without running Cargo.
- **Help.** `print_help` documents `start [--watch] [-- args]` (line 67), without `--release`, although the non-watch
  path accepts it.
- **Docs.** `docs/cli/usages.md:81-91` lists `corvis start [--watch] [--release] [-- <program arguments>]` and says
  "In that mode, `--release` and the arguments after `--` are ignored" (same in `docs/es/cli/usages.md:91`).
  `docs/cli/scripts.md:12,17` and `docs/recipes/hot-reload.md:14` describe `--watch` as `cargo watch -x run`.
- **Tests.** There is no `#[cfg(test)]` module in `main.rs` (unit tests live in `edit.rs:156`, `names.rs:106` and
  `templates.rs`). `crates/corvis-cli/tests/cli_input.rs` runs the real binary with the `sandbox` and `corvis` helpers
  (lines 7-18); none of its tests covers `start` or `build`.
- **cargo-watch.** `cargo watch -x <cmd>` prefixes `cargo ` to `<cmd>` and runs the result through a shell; that is
  why a single `-x` string with spaces works. This comes from cargo-watch's documentation, not from code in this
  repository (cargo-watch is not installed on the machine where this change was drafted); task 1.1 confirms it with
  the installed version.
- **Related change.** `e13-cli-defaults-that-contradict-the-docs` (#20, not implemented yet) also edits `cmd_cargo`:
  it adds a `cargo-watch` presence check before spawning and a profile line for `build`. Its proposal lists the
  watch pass-through as out of scope; that is this change.

## Goals / Non-Goals

**Goals:**
- `corvis start --watch [--release] [-- args]` runs the same program, with the same profile and arguments, as
  `corvis start [--release] [-- args]`, restarted on save.
- Arguments after `--` never change what `corvis` itself does.
- The generated command line is unit-tested without spawning Cargo.

**Non-Goals:**
- Detecting a missing `cargo-watch` or printing the build profile (#20).
- Quoting arbitrary arguments for every shell; passing other Cargo options; a new argument parser.

## Decisions

1. **A pure command builder.** Replace the inline construction with
   `fn cargo_invocation(args: &[String], sub: &str) -> Result<Vec<String>, String>`, returning the arguments given to
   `cargo` (for example `["watch", "-x", "run --release -- --port 4000"]` or `["run", "--release", "--", "a b"]`).
   `cmd_cargo` calls it and spawns `Command::new("cargo").args(..)`. Unit tests assert the returned vector for every
   form in `specs/cli/start-build/spec.md`. Alternative: integration tests with a fake `cargo` on `PATH` that echoes
   its arguments; rejected because it needs a platform-specific script (`.cmd` on Windows) and is slower, while the
   builder gives the same coverage of the "generated command line" the issue asks for.
2. **Split at the first `--`.** The builder splits `args` into `own` (before the first `--`) and `program` (after it),
   and searches only `own` for `--watch`, `--release` and `--prod`. A faithful pass-through needs this: otherwise a
   program argument named `--release` or `--watch` changes the build. It also changes the non-watch path
   (`corvis start -- --release` no longer builds in release), which today is a bug, not documented behavior.
   Alternative: keep scanning the whole slice; rejected because the new tests would have to encode the bug.
3. **Watch mode: one `-x` string, with a conservative character allow-list.** The `-x` value is `run` or
   `run --release`, followed, when there are program arguments, by ` -- ` and the arguments joined with single spaces.
   Since cargo-watch hands that string to a shell, an argument with whitespace, quotes, `$`, `%`, `&`, `|`, `;`, `<`,
   `>`, `^`, `!`, `*`, `?`, backslashes, brackets or parentheses would be split or interpreted, and differently on `sh`
   and on Windows. The builder accepts only non-empty arguments made of `[A-Za-z0-9]` and `- _ . / : = , + @`, and
   otherwise returns:
   `argument "<arg>" cannot be passed through cargo watch (it runs the command through a shell). Run "corvis start"
   without --watch, or run cargo watch directly with your shell's quoting.`
   This covers the common cases (`--port 4000`, `--config=./dev.toml`, `--host 127.0.0.1`, URLs without query
   strings). Alternatives: (a) quote per platform (`'...'` for `sh`, `"..."` with `^` escapes for `cmd`); rejected
   because the shell cargo-watch uses depends on its version and options (`--use-shell`), and a wrong guess silently
   changes the arguments; (b) cargo-watch's trailing-command form `cargo watch -- cargo run ...`; rejected because it
   is also run through a shell unless extra options are passed, and those options differ between cargo-watch
   versions; (c) pass everything unchecked; rejected because it silently splits arguments or runs parts of them.
4. **Validate before anything is spawned.** The builder runs before the `cargo-watch` presence check that #20 adds,
   so an invalid argument is reported the same way whether or not `cargo-watch` is installed, and the integration
   test does not depend on the machine having `cargo-watch`.
5. **Help text.** `start [--watch] [--release] [-- args]` with the description
   `Run the application (cargo run, or cargo watch -x "run ..." with --watch).`

## Risks / Trade-offs

- [A user relied on `corvis start -- --release` building in release] → unlikely and undocumented; the docs table puts
  `--release` before `--`. The docs state the `--` rule explicitly.
- [The allow-list rejects an argument a user needs, such as a path with spaces] → the error names the argument and
  gives two working alternatives; plain `corvis start` still accepts it. Quoting can be added later (see Out of scope
  in proposal.md).
- [Conflict with #20 in `cmd_cargo`] → both changes edit the same function; whichever lands second rebases. The builder
  is pure and the #20 check sits between building and spawning, so the merge is mechanical.
- [cargo-watch behaves differently from what Context states] → task 1.1 checks it before the code is written; if `-x`
  is not run through a shell in the installed version, the allow-list is still safe, and only the error text changes.

## Open questions

- Confirm that fixing the `--` handling of the non-watch path (Decision 2) belongs in this change. A correct
  pass-through needs it and it is small, but it changes what `corvis start -- --release` and `corvis build -- ...` do.
- Confirm rejecting (instead of quoting) watch-mode arguments with spaces or shell characters (Decision 3).

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- The `--` split also applies without `--watch`: it is the same bug (flags after `--` belong to the program), and
  fixing it only in watch mode would leave `corvis start -- --release` behaving differently in each mode. Mention the
  behavior change in the release notes.
- Arguments that are unsafe for the shell `cargo watch -x` uses are rejected with an error, not escaped: escaping rules
  differ between `cmd.exe`, PowerShell and POSIX shells, and a wrong escape is a command-injection bug. Escaping can be
  a follow-up issue if users ask for it.

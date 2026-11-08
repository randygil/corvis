# corvis start --watch drops --release and extra arguments

Implements [randygil/corvis#127](https://github.com/randygil/corvis/issues/127) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

`corvis start --watch` always runs `cargo watch -x run`. It ignores `--release` (and its alias `--prod`) and every
argument after `--`, so watch mode builds and runs a different program than `corvis start` with the same flags: a user
who runs `corvis start --watch --release -- --port 4000` gets a debug build started without `--port 4000`, with no
warning. The docs currently describe this as a limitation (`docs/cli/usages.md:91`).

## What Changes

- `corvis start --watch` builds the watched Cargo command from the same options as `corvis start`:
  `--release`/`--prod` become `run --release` and the arguments after `--` are appended after `run ... --`, all inside
  the single `-x` argument of `cargo watch` (for example `cargo watch -x "run --release -- --port 4000"`).
- `corvis start` and `corvis build` only read their own flags (`--watch`, `--release`, `--prod`) before the first `--`;
  everything after `--` belongs to the program, so `corvis start -- --release` passes `--release` to the application
  instead of switching the build profile.
- A trailing argument that cannot be passed safely through the shell that `cargo watch` uses (whitespace, quotes or
  shell metacharacters) makes `corvis start --watch` fail with an explanation instead of being split or interpreted
  by the shell. Plain `corvis start` is unaffected (it does not go through a shell).
- The command-line construction is extracted into a pure function and covered by unit tests of the generated command.
- `corvis --help` lists `--release` for `start`; the docs (`cli/usages`, `cli/scripts`, `recipes/hot-reload`, in
  English and Spanish) describe the watch forms and drop the "ignored in that mode" sentence.

## Capabilities

### New Capabilities
- `cli/start-build`: what `corvis start` and `corvis build` run. This capability is also introduced by the pending
  change `e13-cli-defaults-that-contradict-the-docs` (#20); this change adds separate requirements to it (watch mode
  command line, flag parsing around `--`) and does not touch the requirements #20 defines (build profile, missing
  `cargo-watch`).

## Impact

- Code: `crates/corvis-cli/src/main.rs` (`cmd_cargo` at lines 331-348, `print_help` at line 67).
- Tests: a `#[cfg(test)] mod tests` in `crates/corvis-cli/src/main.rs` for the command builder (no Cargo process is
  spawned); one integration test in `crates/corvis-cli/tests/cli_input.rs` for the rejected-argument error.
- Docs: `docs/cli/usages.md`, `docs/cli/scripts.md`, `docs/recipes/hot-reload.md` and their `docs/es/` translations.
- No framework runtime code changes (only `crates/corvis-cli`).

## Out of scope

- Detecting a missing `cargo-watch` and the build-profile message: change `e13-cli-defaults-that-contradict-the-docs`
  (#20).
- Shell quoting of arbitrary trailing arguments in watch mode (rejected here, see design.md Decision 3); could become
  an issue if users need it.
- Passing other Cargo options (`--bin`, `--features`, `-p`) through `corvis start`/`corvis build`; the CLI has no
  syntax for them today. Possible new issue.
- `--watch` for `corvis build` (silently ignored today) and other watchers (`bacon`, `watchexec`).
- A real argument parser for the CLI (the hand-written `flag`/`opt`/`positional` helpers stay).

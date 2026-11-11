# Proposal

Implements the issue filed from `quality/q-cli-safe-edits-and-generated-project.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: the engineering-quality review of
`corvis-swagger`, `corvis-swagger-macros`, `corvis-cli` and the workspace, read against `dev` at `d46758a`. This change
covers the command-line tool (findings M4, M5, S6, S7 and the CLI nice-to-have items). It lands after
`q-workspace-ci-gates-and-lint-ratchet` (its new CI job joins the required checks that change sets up) and does not
depend on the two OpenAPI changes.

## Why

- **`corvis generate` can leave the user's source broken, in place.** `add_use` inserts after the last line that
  starts with `use ` (`crates/corvis-cli/src/edit.rs:38`); with a `use crate::{` group split over lines, as rustfmt
  writes long imports, the new line lands inside the group. `add_to_module` and `find_key` count brackets without
  knowing comments or strings (`edit.rs:48-155`; `source.find("#[module")` also matches a comment). A type already
  imported inside a group is imported again (`edit.rs:33` compares whole lines), which does not compile. The result is
  written over the file with `fs::write` (`main.rs:106-117`), with no check and no way back.
- **Nothing compiles what the templates generate.** The only template tests look for substrings
  (`templates.rs:426-447`). Generated controllers, resources and gateways have routes without a guard or `#[public]`
  (`templates.rs:124-148`, `266-303`, `237-253`), so a project from `corvis new` (which enforces route posture,
  `templates.rs:43`) refuses to start after `corvis g res cats`. That refusal was decided when route posture was
  designed (archived `e1-route-posture-design`, "corvis generate resource produces controllers without guards… The
  error text says exactly that"), but the CLI says nothing when it generates such routes, the generated `crud` test
  passes because it does not enable posture, and any drift between the templates and the framework API goes unseen.
- **The CLI accepts what it does not understand.** An unknown command prints the help and exits 0 (`main.rs:49-52`);
  unknown flags are ignored (`main.rs:119-144`), so `corvis g co users --dry-run` writes the files; `--path` without a
  value silently uses the default folder. Files are checked and then written (`main.rs:93-104`), a failure halfway
  through `new` or `g res` leaves half a project, and colors are always on (`main.rs:26-29`).
- **`corvis posture` can signal a process group it no longer owns.** On Unix the loop reaps `cargo run` with
  `try_wait` (`posture.rs:307-311`) and later sends `SIGKILL` to its process group (`posture/tree.rs:177-181`,
  `posture.rs:357`); once the leader is reaped and the group is empty, its id can be reused.
- Smaller: `Names::singular` turns `status` into `Statu`, `address` into `Addres`, `houses` into `Hous`
  (`names.rs:85-98`); a unit test writes into the crate folder and leaves files there when it fails
  (`main.rs:635-661`); edits turn a CRLF file into LF (`edit.rs` joins lines with `\n`).

## What Changes

- **Edits located with `syn`.** The registration editors parse the file, find the last top-level `use` / `mod` item
  and the `#[module(...)]` attribute's lists by their spans, insert text at those positions (comments, formatting and
  line endings kept), skip an item already imported or registered, and refuse (`SKIP`, nothing written) when the file
  does not parse before or after the edit.
- **Safe writes.** New files with `create_new`; edited files through a temporary file and a rename; a failed `new` or
  `generate` removes what it created and restores what it edited.
- **Generated routes and posture.** Default unchanged (routes without a guard, as decided for route posture); in a
  project whose `main` enforces route posture, the CLI prints a `NOTE` listing the generated routes and the two ways to
  accept them. New flag `--public` for `controller`, `resource` and `gateway` marks the generated routes `#[public]`
  with a `TODO` comment.
- **A CI job that builds a generated project.** `generated-project`: `corvis new` with the workspace's crates by path,
  every schematic, `g res`, a rustfmt-split import, then `cargo build --all-targets` and `cargo clippy` with warnings
  denied, `cargo test`, and `corvis posture` (expected to list exactly the generated routes; with `--public`, clean).
  Driven by an ignored Rust test so it runs the same way locally.
- **Strict arguments.** One table of flags per command; unknown commands and flags, and flags missing their value, are
  usage errors (exit status 2, message on stderr); help only for `corvis`, `help`, `-h`, `--help`.
- **Colors** only on a terminal and never with `NO_COLOR`.
- **Posture reaping.** On Unix the CLI waits for the application's exit without reaping it (`waitid` with `WNOWAIT`),
  kills the group while the leader still holds its id, then reaps.
- **Singular names** by ordered rules plus a short exception list, and `--entity <Name>` for `g res`; the unit test
  that wrote into the crate folder cleans up on drop.

## Capabilities

### New Capabilities

- `cli/arguments`: how the CLI treats commands and options it does not know, exit statuses for usage errors, colors.

### Modified Capabilities

- `cli/generate`: added requirements for edits that always parse, safe writes and rollback, generated routes under
  route posture (`NOTE`, `--public`), singular entity names.
- `security/route-posture`: added requirement that `corvis posture` never signals a process group after giving up its
  leader.
- `infra/ci`: added requirement for the `generated-project` job, a required check of `main`.

## Impact

- Edited: `crates/corvis-cli/src/{main,edit,templates,names,posture}.rs`, `crates/corvis-cli/src/posture/tree.rs`,
  `crates/corvis-cli/Cargo.toml` (dependencies `syn` with `full`, `proc-macro2` with `span-locations`, `libc` on Unix;
  all already in `Cargo.lock`, `syn` and `proc-macro2` through `[workspace.dependencies]` after the workspace change),
  `crates/corvis-cli/tests/{cli_input,cli_new}.rs`, `.github/workflows/ci.yml`, `crates/corvis-cli/README.md`,
  `docs/cli/overview.md`, `docs/cli/usages.md` and their Spanish pages, `CONTRIBUTING.md` (the new job).
- New: `crates/corvis-cli/src/args.rs` (the flag tables), `crates/corvis-cli/src/write.rs` (create-new, atomic replace,
  rollback), `crates/corvis-cli/tests/cli_edit.rs` (edit corpus), `crates/corvis-cli/tests/generated_project.rs`
  (ignored, run by the CI job).
- Behavior: an unknown command or flag now fails with status 2 instead of being ignored; edits that would not parse are
  skipped; colors depend on the terminal; new `NOTE` lines; new flags `--public` and `--entity`. The generated code is
  unchanged except with `--public`, and except entity names fixed by the singular rules (`status` → `Status`).
- The CLI is not framework runtime code: the corvis-parity check is not affected (it is run only if a template change
  touched `crates/corvis`, which this change does not plan).

## Out of scope

- Generating a guard with each controller or resource by default: route posture's decision stands (the refusal names
  the routes); `--public` is the opt-in.
- Adopting `clap`: a new dependency for a parser of a dozen flags; the tables of `args.rs` give the same checks.
- Changing `corvis start`'s pass-through of arguments after `--` (`cli/start-build` requirements unchanged).

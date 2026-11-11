# Design

## Context

Source: findings M4, M5, S6, S7 and the CLI nice-to-have items of the engineering-quality review recorded in the issue
(file `quality/q-cli-safe-edits-and-generated-project.issue.md`). Code read on `dev` at `d46758a` (2026-11-11):
`crates/corvis-cli/src/{main,edit,templates,names,posture}.rs`, `posture/tree.rs`, `tests/{cli_input,cli_new,cli_posture}.rs`;
specs `cli/new`, `cli/generate`, `cli/start-build`, `security/route-posture`, `infra/ci`; the archived
`e1-route-posture-design` and `e1-posture-report-in-the-cli`. `corvis-cli` has no dependencies today
(`crates/corvis-cli/Cargo.toml`).

### The findings, checked against the code

| Finding | Where now | Verdict |
|---|---|---|
| M4 `add_use` breaks a multi-line `use` group | `edit.rs:32-45` (`rposition(starts_with("use "))`) | Confirmed: with `use crate::{\n    a::A,\n    b::B,\n};` the last matching line is `use crate::{`; the new line is inserted inside the braces |
| M4 brackets counted without comments or strings | `edit.rs:48-135` (`add_to_module`), `137-155` (`find_key`); `source.find("#[module")` (`:49`) | Confirmed. Siblings: `#[module` in a comment or doc string is taken as the attribute; a `]` or `)` inside a string in the attribute (`ConfigModuleOptions { env_file: "x)" }`) ends the list early |
| M4 empty format argument | `edit.rs:114` (`format!("{}{}\n{}{item},", .., "", ..)`) | Confirmed (harmless, dead argument) |
| M4 written in place without a check | `main.rs:106-117` (`update_file` → `fs::write`) | Confirmed |
| New: duplicate import | `edit.rs:33` compares whole trimmed lines | A type already imported inside a group (`use crate::users::{UsersService, UsersController};`) gets a second `use` line: error E0252, the file no longer compiles |
| New: CRLF files become LF | `edit.rs:9,26` (`lines()` then `join("\n")`), `add_use` the same | Every edited CRLF file is rewritten with LF line endings |
| M5 generated routes refused by the generated app | `templates.rs:43` (`require_route_posture`), `templates.rs:131-134`, `279-302`, `248-251` (no guard, no `#[public]`) | Confirmed by reading; the gateway case depends on how posture treats gateway handlers (`security/route-posture` "Gateway message handlers are checked") and is verified by running (task 2.2). Already decided: the archived `e1-route-posture-design` (Risks) accepted that `corvis generate resource` produces routes the enforcing app refuses, with an error naming them. What remains confirmed: the CLI does not say so at generation time, the generated `crud` test (`templates.rs:310-325`) passes because a testing module does not enforce posture, and no test compiles a generated project (`templates.rs:426-447` check substrings), so template drift is invisible |
| S6 unknown command exits 0 | `main.rs:49-52` | Confirmed |
| S6 unknown flags ignored | `main.rs:119-144` (`flag`, `opt`, `positional` skip anything starting with `--`) | Confirmed. Siblings: `--path` with no value falls back to the default folder (`opt` returns `None`); a value starting with `-` is taken as a positional name and then rejected by `validate_name` with an unrelated message. `posture`'s parser already rejects unknown options (`posture.rs:85-88`) |
| S6 check-then-write | `main.rs:93-104` | Confirmed. The race needs another writer in the same folder during the command: low risk, trivial fix (`create_new`) |
| S6 half-made project on failure | `main.rs:215-250` (`cmd_new`), `417-431` (`generate_resource`) | Confirmed |
| S6 colors always on | `main.rs:26-29`, also `posture.rs:250` | Confirmed |
| S7 group killed after the leader was reaped | `posture.rs:305-311` and `:334-335` (`try_wait` reaps `cargo run`), `posture.rs:351-357` and `tree.rs:177-187` (`kill(-group, SIGKILL)` on drop) | Confirmed. While any member of a group lives its id cannot be reused; after the leader is reaped and the group is empty it can. The window is short (`DRAIN_GRACE`, 500 ms) and reuse needs the same number to come round, so the risk is low, but the outcome is killing unrelated processes |
| `Names::singular` | `names.rs:85-98` | Confirmed: `status` → `statu`, `address` → `addres`. Siblings found: `houses` → `hous`, `responses` → `respons`, `courses` → `cours`, `movies` → `movy` |
| Test writes into the crate folder | `main.rs:635-661` (`ignores_relative_and_empty_search_directories`) | Confirmed: files under the crate folder, removed only at the end of a passing run |

## Goals / Non-Goals

**Goals:**

- `corvis generate` never leaves a file that does not parse, and never loses the user's comments or line endings.
- What the templates generate is compiled, linted, tested and posture-checked in CI against the workspace's crates.
- A command the CLI does not understand fails, with a status a script can test.
- `corvis posture` signals only its own processes.

**Non-Goals:**

- Changing route posture's default for generated routes (decided in `e1-route-posture-design`).
- A full English inflector; `clap`.

## Decisions

### 1. Edits located by `syn`, applied as text

- `corvis-cli` depends on `syn` (`full`, `parsing`) and `proc-macro2` with `span-locations` (both already in
  `Cargo.lock`; through `[workspace.dependencies]` once the workspace change lands). Span locations give line and column
  of every item; a helper turns them into byte offsets of the original text.
- `add_use(source, path)`: parse with `syn::parse_file`; if the name the path brings in is already imported by any
  top-level `use` tree (same path: nothing to do; another path with the same name: `SKIP` with a message naming the
  clash), return unchanged; otherwise insert the line after the end of the last top-level `use` item, or after the inner
  attributes and inner doc comments when there is none.
- `add_mod(source, decl)`: the same with the last top-level `mod name;` item; nothing if a `mod` of that name exists.
- `add_to_module(source, key, item)`: find the item with an outer `#[module]` / `#[module(...)]` attribute (by the
  attribute's path, so comments and strings never match); parse the attribute's tokens as a comma list of `key = expr`;
  for the key's array, compare the last path segment of each element with `item` (present: unchanged), else insert
  before the closing bracket using its span, with the current single-line / multi-line formatting rules; for a missing
  key, insert `key = [item]` before the attribute's closing delimiter; `#[module]` without a list becomes
  `#[module(key = [item])]`.
- **Guard.** Every editor returns `Edit::Unchanged`, `Edit::Changed(String)` or `Edit::Skip(reason)`. A source that does
  not parse is `Skip` before any edit; a result that does not parse is `Skip` after it (nothing written; the message says
  which registration to add by hand).
- **Line endings.** The inserted text uses the file's newline (`\r\n` when the first line ending is one).

Alternatives considered: regenerate the file with a pretty printer (loses comments and the user's formatting); keep the
textual editors and only add the parse check (stops the damage but keeps the misplaced inserts, the duplicate imports
and the comment matches as `SKIP` noise). Rejected.

Verified by: `tests/cli_edit.rs`, a corpus of module files (rustfmt-split `use` group — fails before the fix; `#[module(`
inside a comment and a doc string; a `)` inside a string literal in the attribute; nested `for_root(...)` calls; a type
already imported in a group; a name clash; `#[module]` without a list; a multi-line list with and without a trailing
comma; CRLF; a file that does not parse), each asserting the result parses with `syn::parse_file`, contains the item
once, keeps every comment byte for byte, and keeps the newline style; the existing `edit.rs` unit tests pass unchanged.

### 2. Safe writes and rollback

`crates/corvis-cli/src/write.rs`: a `Changes` value records each file created (with `OpenOptions::create_new`, so an
existing file is an error, never overwritten) and the original bytes of each file edited (replaced through a temporary
file in the same folder and `fs::rename`). `cmd_new` and every `generate` path run their steps through one `Changes`;
on the first error it removes the files it created (and the folders it created, if empty), restores the edited files,
and returns the error. On success it prints the same `CREATE` / `UPDATE` / `SKIP` lines as today.

Verified by: a test that makes the third write of `g res` fail (a file planted where `dto/mod.rs` goes) and asserts the
tree is byte for byte what it was before; a test that a planted file is not overwritten by `corvis new`.

### 3. Generated routes under route posture

- The default stays: generated routes have no guard and no `#[public]`, so an enforcing application names them in its
  refusal. That is route posture's decision (secure by default; a generated in-memory CRUD that becomes public silently
  is the failure it was designed against).
- New: when the project's crate root (`src/main.rs`) calls `require_route_posture()` or `set_route_posture(` and the
  schematic generates routes (`controller`, `resource`, `gateway`), the CLI prints after the `UPDATE` lines:
  `NOTE route posture is enforced: GET /cats, POST /cats, … have no guard; add a guard or #[public] before running, or
  run corvis posture`. The list is computed from the template.
- New flag `--public` for those three schematics: each generated route (and the gateway's message handler) gets
  `#[public]` and a comment `// TODO(posture): public until you add a guard`. Then the project starts and
  `corvis posture` exits 0.

Verified by: the generated-project job (Decision 4) in both modes; a unit test that the `NOTE` lists exactly the routes of
the template; the template tests check `--public` adds the marker to every route.

Alternative considered: generate a denying guard with every controller (posture passes, every request answers 403).
Rejected for now: more files than asked for, and the generated tests would need to override it; listed as a possible
later flag.

### 4. The `generated-project` job

An ignored integration test, `crates/corvis-cli/tests/generated_project.rs`, does the work, so the job and a developer
run the same thing (`cargo test -p corvis-cli --test generated_project -- --ignored`):

1. Builds nothing itself: uses `CARGO_BIN_EXE_corvis`.
2. In a fresh folder under `std::env::temp_dir()` (outside the workspace, so Cargo does not take the project for a
   member), runs `corvis new gen-app --skip-git --corvis-path <workspace>/crates/corvis`.
3. Copies the workspace `Cargo.lock` into `gen-app/` (Cargo keeps the locked versions it can and drops the rest, so the
   generated build uses the versions CI already tests and downloads nothing new).
4. Writes a long grouped import into `src/app_module.rs` and runs `cargo fmt` (so the file has a rustfmt-split `use`
   group before the first registration), then runs every schematic: `g mo users`, `g co users`, `g s users`,
   `g co profile --path users`, `g s audit --flat`, `g gu auth`, `g itc logging`, `g pi trim`, `g f http`,
   `g mi request-log`, `g ga events`, `g res cats`.
5. With `RUSTFLAGS=-D warnings`: `cargo build --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
6. `corvis posture --json`: exit status 1; the unprotected entries are exactly the routes of `UsersController`,
   `ProfileController`, `CatsController` and the `events` gateway handler; `GET /` is public.
7. A second project with the same steps and `--public` on `co`, `res` and `ga`: `corvis posture` exits 0.

Target directory: `CORVIS_GENERATED_TARGET` when set (CI points it inside the workspace `target/`, which rust-cache
keeps), otherwise `<workspace target>/generated-project`, one fixed folder reused by every run (never a folder per run).
Network: what `cargo build` already uses in CI (crates.io), and with the copied lock file no new versions.

Job `generated-project` in `ci.yml`: Linux, the pinned toolchain with clippy and rustfmt, rust-cache, `timeout-minutes:
45`, the workflow's permissions; runs the ignored test. Required check of `main`.

Alternative considered: a shell script in the workflow. Rejected: it would not run on a developer's Windows machine and
would duplicate assertions the Rust test makes once.

### 5. Strict arguments

`crates/corvis-cli/src/args.rs`: per command, the flags it takes and whether each takes a value (`new`: `--corvis-path
<dir>`, `--corvis-version <req>`, `--skip-git`; `generate`: `--path <dir>`, `--flat`, `--no-spec`, `--public`,
`--entity <Name>`; `start`: `--watch`, `--release`, `--prod`; `build`: `--release`, `--prod`; `info`: none; `posture`
keeps its parser). Parsing returns positional arguments and flags, or a usage error for: an unknown flag, a flag missing
its value, a repeated flag, an unexpected extra positional. For `start` and `build`, only the arguments before the first
`--` are parsed (the `cli/start-build` requirements stay as they are). Unknown commands are usage errors with the closest
command suggested. A usage error prints `Error: ...` and `see corvis --help` on stderr and exits 2 (the status `posture`
already uses when it cannot produce a report). `corvis`, `corvis help`, `-h`, `--help` print the help on stdout and
exit 0.

Verified by: `tests/cli_input.rs` cases: `corvis nwe app` (status 2, suggestion `new`, nothing created),
`corvis g co users --dry-run` (status 2, nothing written), `--path` without value (status 2), `corvis start --wtach`
(status 2), `corvis start -- --anything` (passes through, as the spec requires).

### 6. Colors

Colors are used when the stream is a terminal (`std::io::IsTerminal`) and `NO_COLOR` is unset or empty; `posture.rs`'s
error line uses the same helper. Verified by: integration tests (captured output is not a terminal) assert no escape
code, and a unit test of the helper with `NO_COLOR=1`.

### 7. Posture: kill the group before reaping the leader

On Unix, the loop stops using `Child::try_wait` to learn that `cargo run` ended. A `Leader` helper calls
`libc::waitid(P_PID, pid, WEXITED | WNOHANG | WNOWAIT)` (the child stays a zombie, so its process group id stays
reserved), the loop proceeds as today, and at the end the CLI sends `SIGKILL` to the group (if anything is left) and only
then reaps the child with `Child::wait`, which gives the exit status. Windows keeps the Job Object (no id reuse issue:
the job handle is the CLI's). `libc` is added as a Unix-only dependency (already in `Cargo.lock`).

Verified by: a unit test of the sequence with a fake `Leader` that records calls (`exited?` → `kill group` → `reap`, never
`reap` before `kill group`); the existing `tests/cli_posture.rs` scenarios (exit codes, timeout, tree cleanup through the
pid file) pass unchanged on Linux and Windows.

Alternative considered: skip the group kill once the leader was reaped. Rejected: the application's helpers would be
orphaned, which the tree cleanup exists to prevent.

### 8. Smaller items

- `Names::singular`: rules in order — unchanged if it ends in `ss`, `us`, `is` or is in a short list of uncountables
  (`news`, `series`, `species`, `data`); a short exception table (`movies` → `movie`, `houses` → `house`, `courses` →
  `course`, `responses` → `response`, `people` → `person`); `ies` → `y`; `ses` / `xes` / `zes` / `ches` / `shes` → drop
  `es`; otherwise drop a final `s`. `--entity <Name>` on `g res` overrides it. Table-driven test with every case above
  and the existing ones.
- The test that writes into the crate folder uses a guard that removes its files on drop and file names unique to the
  process, so a failure leaves nothing.
- `edit.rs:114`'s empty argument disappears with the rewrite of Decision 1.

## Security invariants this change comes near

| Invariant | Source | Pinned by |
|---|---|---|
| Names, `--path`, `--corvis-path`, `--corvis-version` are validated before any file is created | `cli/new` "Dependency values are validated", archived `e13-cli-defaults-that-contradict-the-docs` | Existing `tests/cli_input.rs` and `cli_new.rs` cases unchanged; the new parser runs before validation, never instead of it |
| `PATH` lookups ignore relative entries | `main.rs:447-452` (`find_in`) | The rewritten test keeps every assertion |
| `corvis posture` accepts only reports carrying its nonce; stdout carries only the report; exit statuses 0/1/2 | `security/route-posture` "The report is delimited on stdout", "CLI output and exit status" | `tests/cli_posture.rs` unchanged |
| `corvis posture` stops the application and its helpers | `security/route-posture` "The timeout bounds the whole run" | `tests/cli_posture.rs` pid-file tests; Decision 7's order test |
| Generated code enforces posture from `corvis new` | `cli/new` "New projects enforce route posture" | The generated-project job (step 6) |

The change touches file writes in the user's project and process signals: the security review is required (task 8.5).

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| Unknown command | Help, status 0 | Error with a suggestion, status 2 | Scripts that relied on status 0 for a typo now fail, as they should |
| Unknown flag, flag without its value | Ignored | Error, status 2, nothing written | `--dry-run` no longer writes files by accident |
| A registration that cannot be placed safely | Possibly broken file | `SKIP` with the line to add by hand | The user's file stays as it was |
| Failure halfway through `new` / `generate` | Half-made project | Created files removed, edited files restored | — |
| CRLF files | Rewritten as LF | Line endings kept | Smaller diffs on Windows |
| Colors | Always | Terminal only, never with `NO_COLOR` | Clean logs in CI |
| Generated routes in an enforcing project | Silent | `NOTE` listing them; `--public` available | — |
| Entity names of `g res` | `Statu`, `Addres`, `Hous` | `Status`, `Address`, `House`; `--entity` | Generated type names change for those words |
| `corvis posture` on Unix | Group killed after reaping | Group killed before reaping | No signal to a reused group id |

## Risks / Trade-offs

- `syn` and `proc-macro2` with `span-locations` make the CLI slower to compile → a one-time cost for `cargo install`;
  both are already compiled for the macro crates in a workspace build (features differ, so `proc-macro2` builds once
  more for the CLI).
- The generated-project job builds the framework a second time (different features and lock root) → its own cache key,
  45-minute limit; it is the only check that the templates compile.
- `waitid` constants differ between Unix systems → taken from `libc`, not written by hand; other Unix targets keep the
  current behavior behind `cfg` (Linux and macOS covered).
- Stricter parsing breaks invocations with stray flags → that is the fix; the error names the flag.

## Open questions

1. Generated routes: keep the refusal by default, mark them `#[public]` by default, or generate a denying guard?
   Recommendation: keep the refusal (route posture's decision), add the `NOTE` and `--public`.
2. Exit status for usage errors: 2 (as `posture` and most CLIs) or 1? Recommendation: 2.
3. Run the generated-project job on Windows too? Recommendation: not as a required job (doubles the cost); the edit corpus
   includes CRLF cases and runs on both runners in the normal `test` jobs.
4. `--entity` or a full inflector? Recommendation: rules plus the flag.

## Review notes

These settle the open questions and override the design where they differ.

1. Generated routes keep no guard and no `#[public]` by default; the `NOTE` and `--public` are added.
2. Usage errors exit with status 2.
3. The generated-project job runs on Linux only and is required; the edit corpus runs on Linux and Windows.
4. Rules plus `--entity`; no inflector crate.
5. The security review is mandatory before the merge (file writes, rollback, temporary files and renames in the user's
   folder, process-group signals).

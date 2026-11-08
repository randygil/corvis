# Design

## Context

Verified on 2026-11-08 by reading `crates/corvis-cli/`, its tests, the docs pages and the CI workflow.

**How the dependency is chosen today** (`crates/corvis-cli/src/main.rs:145-161`, `corvis_dependency`):

1. `--corvis-version <req>`: validated by `names::validate_version_req` (digits or `*`, only ` .,^~=<>*+-` and
   alphanumerics) and written as `version = "<req>"`.
2. Otherwise `--corvis-path <dir>`, then the `CORVIS_PATH` environment variable, then
   `PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("corvis")` canonicalized at run time. The `\\?\` prefix
   is stripped, backslashes become `/`, and a path with `"` or control characters is rejected. Written as
   `path = "<dir>"`.

`cmd_new` replaces `__CORVIS_DEP__` in `templates::CARGO_TOML` (`templates.rs:18-26`), which yields
`corvis = { <dependency> }` plus `serde`. No other framework crate appears in the generated manifest, and
`corvis generate` never edits `Cargo.toml`. `positional()` skips the values of `--corvis-path` and
`--corvis-version`. The help text (`main.rs:68`) lists both flags but not `CORVIS_PATH` and not the default.

**Why the default breaks outside a clone.** `CARGO_MANIFEST_DIR` is fixed at compile time. `cargo install corvis-cli`
compiles in `~/.cargo/registry/src/<index>/corvis-cli-<version>/`; the registry folders carry the version in their
names (`corvis-0.1.0`), so `../corvis` never exists there. `canonicalize` fails and the raw, non-existent path is
written. The same happens if a clone is moved or deleted after `cargo install --path crates/corvis-cli`.

**Tests.** `crates/corvis-cli/tests/cli_input.rs` runs the real binary (`env!("CARGO_BIN_EXE_corvis")`) in a temporary
directory. Every `corvis new` call there passes `--corvis-version` (`^0.1` or `0.1`), and no test compiles a generated
project (`cli_posture.rs` uses a fake `cargo`). The helper `corvis()` inherits the caller's environment, so a
developer with `CORVIS_PATH` set would change what a default-run test sees. `scripts/smoke-examples.mjs` starts the
example binaries built from the workspace and never calls `corvis new`; CI (`.github/workflows/ci.yml`) runs fmt,
clippy, build, test, fuzz, deny, audit, the naming check and the docs check, none of which generates a project.

**Docs today.** `docs/first-steps.md:34-44` (table: none -> path to the checkout; tip: "the simplest option today is
the path dependency"), `docs/cli/overview.md:65-77` ("Framework dependency": "by default, to the `crates/corvis`
folder the CLI was built from"), `docs/cli/usages.md:12-23` (options table), the Spanish translations of the three,
and `README.md:98-99` (lists the flags).

**Publication state.** Nothing is on crates.io yet. #11 reserves the names by publishing `0.0.x` versions; #10 (in
progress) pins the macro crates with `=` and releases all crates together with one version (`0.1.0` in the
workspace).

## Goals / Non-Goals

**Goals:**
- A CLI installed from crates.io generates a project whose dependency resolves to the framework release it was made
  for.
- Contributors can still generate projects against a local checkout with one flag or one environment variable.
- The choice is deterministic: it depends on the arguments and the environment, never on what exists on the disk of
  the machine that compiled the CLI.

**Non-Goals:**
- Publishing, or changing the workspace version.
- Adding framework crates other than `corvis` to the generated project.
- Changing the VS Code extension.

## Decisions

1. **Default requirement = the CLI's own version, caret semantics.** The generated line is
   `corvis = { version = "0.1.0" }` (the value of `env!("CARGO_PKG_VERSION")`, any `+build` metadata removed because
   Cargo ignores it in requirements and warns). A bare version is a caret requirement (`>=0.1.0, <0.2.0`), so the
   project picks up compatible fixes and never gets a framework older than the templates.
   - Alternative: exact `=0.1.0`. Blocks patch releases for every generated project until its owner edits the
     manifest; the exact pin is right between the framework and its own macro crates (#10), not for applications.
   - Alternative: `0.1` (major.minor). Allows `0.1.0` even if the templates rely on something added in `0.1.3`.
   - Alternative: keep the path default until the first release, then flip it. Two changes instead of one, and a CLI
     released before the flip would reproduce this bug. Since the CLI and the framework are released together, every
     CLI anyone can install from crates.io has its matching framework version on crates.io, so the version default is
     correct for all of them from day one.
2. **Local checkout through the existing `--corvis-path` and `CORVIS_PATH`; no new `--local` flag.** Both are already
   in the help, the README and the docs in two languages, and `--corvis-path` already has injection tests. The
   environment variable is the recommended development setting: a contributor sets it once in a shell profile, and a
   test harness or editor integration sets it per process without changing the command line. A new `--local [dir]`
   flag would be a third spelling of the same thing and would have to define what "local" means for a CLI installed
   from crates.io.
3. **Deterministic precedence, conflicts rejected.**
   1. `--corvis-version <req>` and `--corvis-path <dir>` together: error
      (`--corvis-path and --corvis-version cannot be used together`), nothing is created.
   2. `--corvis-version <req>`: version requirement (validated as today).
   3. `--corvis-path <dir>`: path (validated as today).
   4. `CORVIS_PATH` set and not empty: path (same validation; the error names `CORVIS_PATH`).
   5. Otherwise: `version = "<CLI version>"`.
   A flag beats the environment variable, so `--corvis-version 0.2` works for a contributor who has `CORVIS_PATH`
   set. The compile-time fallback to `CARGO_MANIFEST_DIR` is removed.
   - Alternative: keep "version wins" when both flags are given. Silent; the user asked for two contradictory things.
4. **One selection function, pure.**
   `corvis_dependency(args: &[String], env_path: Option<String>) -> Result<String, String>` returns the inside of the
   `{ ... }` table; `cmd_new` passes `std::env::var("CORVIS_PATH").ok()`. Unit tests then cover every branch without
   touching the process environment (which is shared between test threads). When a template later adds another
   framework crate, it uses the same source: the same version requirement, or, for a path, the sibling folder of the
   `corvis` path (`<dir>/../corvis-orm`). No such template exists yet, so no code is written for it now.
5. **Before the first release: version by default, docs and tests use the local option.** Until a framework release
   with the CLI's version is on crates.io, a project generated with the defaults fails at `cargo build` with
   `failed to select a version for the requirement corvis = "^0.1.0"` (the `0.0.x` name reservations of #11 do not
   match it). This affects only people who build the CLI from a clone, who are the people with a checkout to point
   at. To keep them on track:
   - the docs say, in the "Framework dependency" section and in the getting-started page, that until the first
     release you set `CORVIS_PATH` to `<clone>/crates/corvis` or pass `--corvis-path`;
   - the CLI tests pass `--corvis-version` or `--corvis-path` explicitly, or remove `CORVIS_PATH` when they test the
     default;
   - decision 6 prints a hint.
   - Alternative: detect the checkout at run time and use it silently when it exists. Rejected: the output would
     depend on the machine that compiled the CLI, which is the behavior that caused this bug, and a contributor
     would get different manifests from a CLI built from a clone and one installed from crates.io.
6. **A `NOTE` hint, never an automatic switch.** When the default version is used and
   `<CARGO_MANIFEST_DIR>/../corvis/Cargo.toml` exists at run time, `cmd_new` prints after the `CREATE` lines:
   `NOTE corvis = "0.1.0" comes from crates.io; to use the checkout this CLI was built from, pass
   --corvis-path <dir> or set CORVIS_PATH=<dir>` with `<dir>` normalized as today. For a crates.io install that file
   does not exist (see Context), so the note does not appear. When the path comes from `CORVIS_PATH` (not from the
   flag), `cmd_new` prints `NOTE using CORVIS_PATH=<dir>`, so a variable left in the environment is visible. The texts
   are built by a small function so a unit test can check them.
7. **Help text.** `new|n <name> [--corvis-path <dir> | --corvis-version <req>] [--skip-git]` with a second line:
   `Depends on corvis <version> from crates.io unless --corvis-path or CORVIS_PATH gives a local checkout.`

## Risks / Trade-offs

- [Contributors lose the zero-option path default] -> the `NOTE` line and the docs give the exact command; setting
  `CORVIS_PATH` once restores the old behavior.
- [Default projects do not build until the first release] -> stated in the docs and in open question 1; the
  maintainer can choose to delay merging until the release if that is preferred.
- [VS Code "New Project" gets a version requirement] -> before the first release its projects need the path edited or
  `CORVIS_PATH` set for VS Code; follow-up issue to have the extension pass `CORVIS_PATH` when it runs the CLI from a
  checkout.
- [A user with `CORVIS_PATH` set in the environment forgets it] -> generated projects keep pointing at a local folder;
  the `CREATE Cargo.toml` line does not show it. Mitigation: the `NOTE using CORVIS_PATH=<dir>` line of decision 6.

## Open questions

1. Default before the first release: version requirement now (recommended, this design), or keep the path default
   until the framework is released and flip it in the release change? Recommendation: version now, because the bug is
   in the released artifact and the release itself must not depend on remembering a flip; the docs and the `NOTE`
   cover the pre-release period.
2. Reject `--corvis-path` together with `--corvis-version` (recommended), or keep "version wins"?
3. Should the VS Code extension set `CORVIS_PATH` when it resolved a framework checkout (setting
   `corvis.frameworkPath` or a workspace folder)? Recommendation: yes, in a follow-up issue for the extension.
4. Requirement form: `0.1.0` (caret, recommended) or `=0.1.0`?

## Review notes (2026-11-08)

1. Open question 1: the version requirement is the default now. Until the first release the docs say to use
   `CORVIS_PATH` (or `--corvis-path`), and the CLI's `NOTE` line points to the local copy when it exists.
2. Open question 2: `--corvis-path` together with `--corvis-version` is an error.
3. Open question 3: the VS Code extension passing `CORVIS_PATH` is a separate issue.
4. Open question 4: a caret requirement (`0.1.0`), not `=0.1.0`.

## Implementation notes

- `corvis_dependency` detects the conflict with `flag()` (the flag is present), so `--corvis-path --corvis-version x` also fails.
- The "default used" state for the build-checkout `NOTE` is decided in `cmd_new` (no dependency flag and no `CORVIS_PATH`); the checkout exists when `<CARGO_MANIFEST_DIR>/../corvis/Cargo.toml` is a file.
- `docs:check` was skipped (no `docs/node_modules`); tasks 3.5 and 4.5 stay open for that reason. Tasks 4.2 and 4.4 cover the whole workspace, and only `cargo clippy --workspace --all-targets` and `cargo test -p corvis-cli` were run.
- Task 4.8 (follow-up issue for the extension) is tracked as #149 by the maintainer and not filed here.

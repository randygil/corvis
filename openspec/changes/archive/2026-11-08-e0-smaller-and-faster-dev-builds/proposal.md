# Proposal

Implements [randygil/corvis#106](https://github.com/randygil/corvis/issues/106) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

The shared Cargo target directory reached 40 GB. The workspace builds dozens of test and example executables, and on
the `x86_64-pc-windows-msvc` target every one of them carries a full `.pdb` with the debug info of all its
dependencies. The root `Cargo.toml` has no `[profile]` section, so the dev and test profiles use Cargo's defaults
(`debug = true`). That costs disk, link time and build time, and fills the disk of the machines that build several
worktrees.

## What Changes

- Measure, on this machine, the effect on full build time, link-dominated rebuild time, target size and test runs of:
  `[profile.dev] debug = "line-tables-only"`, no debug info for dependencies
  (`[profile.dev.package."*"] debug = false`), and `rust-lld` as the linker of the msvc target.
- Keep only the settings that measurably help and do not make debugging unusable (panics must still show `file:line`
  in `RUST_BACKTRACE=1` backtraces). Record the results table in design.md.
- Check that `cargo test` (the test profile inherits dev) gets the same benefit.
- Document the build settings for contributors in `CONTRIBUTING.md`.

Out of scope:
- The release profile, CI caching and any `sccache` or shared-cache setup (CI work is issue #6).
- Per-developer settings that do not belong in the repository (a different linker on other platforms, `mold`).
- Changes to framework runtime code.

## Capabilities

### New Capabilities

None (tooling only).

### Modified Capabilities

None (tooling only; no spec delta).

## Impact

- Root `Cargo.toml` (a `[profile]` section) and possibly a new `.cargo/config.toml` (the linker).
- `CONTRIBUTING.md`.
- No runtime code changes, so no parity run is needed.

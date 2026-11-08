# Design

## Context

The shared target dir reached 40 GB. Most of it is debug info: on `x86_64-pc-windows-msvc` every test and example
executable has a `.pdb`, and with `debug = true` each one carries the debug info of all its dependencies.

## Method

Machine: Windows 11, x86_64-pc-windows-msvc, rustc 1.99.0, 32 logical cores. For each configuration, in a temporary
empty target dir (`CARGO_TARGET_DIR`, deleted after each run), with `CARGO_INCREMENTAL=0`:

1. `cargo build --workspace --all-targets` from scratch (time and target size).
2. `touch crates/corvis/src/lib.rs`, then the same build again (the rebuild recompiles `corvis` and its dependents and
   relinks every test and example executable, so it is link-heavy).
3. A throwaway integration test that indexes out of bounds, run with `RUST_BACKTRACE=1`: the backtrace must show
   `file:line` of the test.
4. `cargo test --workspace --no-fail-fast` (time, target size afterwards, pass/fail counts).

Sizes are `du` of the target dir. Incremental compilation was not measured (it stays off, as in the repository
settings). A single run per configuration: differences under about 10% are noise.

## Results

| # | Configuration | Full build | Rebuild after touch | Target after build | `.pdb` total | Target after `cargo test` | `cargo test --workspace` | Backtrace |
|---|---|---|---|---|---|---|---|---|
| 1 | Baseline (no `[profile]`, `debug = true`) | 68 s | 54 s | 5,819 MB | 4,127 MB | 10,870 MB | 111 s, 364 passed | `zz_bt.rs:4` |
| 2 | `[profile.dev] debug = "line-tables-only"` | 33 s | 18 s | 2,815 MB | 1,569 MB | 4,851 MB | 90 s, 364 passed | `zz_bt.rs:4` |
| 3 | 2 + `[profile.dev.package."*"] debug = false` | 30 s | 17 s | 2,135 MB | 941 MB | 3,638 MB | 86 s, 363 passed, 1 flaky | `zz_bt.rs:4` |
| 4 | 3 + `linker = "rust-lld"` | 29 s | 17 s | 2,164 MB | 972 MB | 3,686 MB | 88 s, 363 passed, 1 flaky | `zz_bt.rs:4` |

Failures in 3 and 4: `transport_security::oversized_head_is_rejected` (the known flaky Windows test, #130) and
`corvis-orm` `security::delete_and_upsert_apply_their_filter_atomically_postgres` (a Postgres test that depends on the
server). The second one passed when rerun alone under configuration 4 (22 passed). The final profile passes
`cargo test --workspace` with 364 passed, 0 failed.

Executables of all test and example targets weigh the same in every row (about 480 to 520 MB), so the savings are the
`.pdb` files and the `.rlib` metadata (872 MB baseline, 422 MB final).

### Test profile

`cargo build --all-targets` and `cargo test` build the test profile, which inherits dev. The `.pdb` totals above
include all test executables, and the numbers after `cargo test` include the test builds; both drop by the same ratio,
so tests get the full benefit without a `[profile.test]` section.

### Backtraces

A panic in workspace code still reports `file:line` in every configuration (`crates\corvis\tests\zz_bt.rs:4:14`, and
the frame `at .\tests\zz_bt.rs:4`). The standard library frames keep `file:line` too, because the toolchain ships
its own debug info. Frames inside third-party dependencies are expected to lose `file:line` with row 3 (not measured; the check panics in
workspace code); the function names stay.
What is lost with line tables only is local variable inspection in a debugger.

## Decision

Keep 2 and 3 in the root `Cargo.toml`:

- Row 2 halves build time, rebuild time and target size, with no loss in the backtraces that matter.
- Row 3 trims another 24% of the target after build (2,815 to 2,135 MB) and 25% after tests, with a small speedup.
  Dependencies are rarely stepped into, and the override is a one-line change.
- Rows 1 to 3 together: 63% less after the build (5.8 to 2.1 GB), 66% less after `cargo test` (10.9 to 3.6 GB), full
  build 2.3 times faster, link-heavy rebuild 3.2 times faster.

Drop 4 (`rust-lld`): no gain in build time (29 against 30 s) or rebuild time (17 s both), and slightly larger
`.pdb` files (972 against 941 MB). Rustc did use it (`-C linker=rust-lld`). It also makes the build depend on the
bundled `rust-lld` of the installed toolchain, which is a cost without a benefit here, so no `.cargo/config.toml` is
added.

Both settings are in the `dev` profile, so release builds and published crates are not affected. To get full debug
info in a session, `CARGO_PROFILE_DEV_DEBUG=true` overrides the workspace crates (documented in `CONTRIBUTING.md`).

## Risks

- Debugging with a native debugger loses local variables in workspace crates and all symbols of dependencies. The
  documented override restores them.
- Timings come from one machine and one run each; the size numbers are stable, the second-level timings are not.

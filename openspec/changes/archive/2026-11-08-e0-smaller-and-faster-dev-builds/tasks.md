# Tasks

## 1. Measurement

- [x] 1.1 Measure the baseline (no `[profile]` section) in a temporary target dir: full `cargo build --workspace --all-targets`, rebuild after touching `crates/corvis/src/lib.rs`, target size, `cargo test --workspace`, panic backtrace with `RUST_BACKTRACE=1`; verify the numbers are in design.md
- [x] 1.2 Repeat with `[profile.dev] debug = "line-tables-only"`
- [x] 1.3 Repeat with that plus `[profile.dev.package."*"] debug = false`
- [x] 1.4 Repeat with `rust-lld` as the linker (`.cargo/config.toml`) on top of the best of 1.2/1.3
- [x] 1.5 Check that the test profile gets the same benefit (sizes of the test executables and their `.pdb` files)

## 2. Apply

- [x] 2.1 Keep in `Cargo.toml` / `.cargo/config.toml` only the settings that helped; verify backtraces still show `file:line` for workspace code
- [x] 2.2 Document the settings and how to override them (full debug info when stepping into a dependency) in `CONTRIBUTING.md`

## 3. Definition of done

- [x] 3.1 `cargo build --workspace --all-targets` with no warnings
- [x] 3.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 3.3 `cargo test --workspace` passes (`transport_security` is known flaky on Windows, #130)
- [x] 3.4 Docs: contributor-only change, covered by `CONTRIBUTING.md`; `npm run docs:check` in `docs/` passes
- [x] 3.5 No runtime code changed: no parity run needed; confirm
- [x] 3.6 Temporary measurement target dir deleted; shared target cleaned and sizes recorded

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #106, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

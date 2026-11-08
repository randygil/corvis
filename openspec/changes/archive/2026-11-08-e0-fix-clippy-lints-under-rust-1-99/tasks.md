# Tasks

## 1. Inventory

- [x] 1.1 Run `cargo clippy --workspace --all-targets --keep-going -- -D warnings` on Rust 1.99 and record the lints by crate and name in design.md; verify the counts match the run

## 2. Mechanical fixes

- [x] 2.1 Apply `cargo clippy --fix` for the auto-fixable lints (mostly `collapsible_if`) across the workspace; verify the diff is mechanical and `cargo build --workspace --all-targets` is warning-free
- [x] 2.2 Fix by hand: type aliases for `type_complexity`, struct-update initializers (`websockets.rs`, `ws.rs`), unused generic of `override_enhancer`, no-op `replace` in `engine/mod.rs`; verify clippy no longer reports them

## 3. Policy items

- [x] 3.1 Add `clippy.toml` with `large-error-threshold = 256` and a comment with the reason; verify `result_large_err` only remains for the two internal functions
- [x] 3.2 Add the narrow `#[allow]` attributes listed in design.md, each with a one-line reason; verify there is no crate-level or group allow (`grep -rn "allow(clippy::all" crates` finds nothing)

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, except the three CRLF failures of `corvis-swagger` tracked in #110 (`swagger_ui::tests::html`, `swagger_ui::tests::init_js`, `scalar::tests::page`)
- [x] 4.4 Non-test runtime code changed (mechanically): run the `corvis-parity` check and verify the three ports keep 0 differences (for the maintainer's parity-verifier, not the implementer)
  - Result: consi 832/832, facten 727/727, houseplan 598/598 (quiet-hours count, same as main), routes identical, databases equivalent. Re-verified serially after the shared-target issue: build 0 warnings, clippy clean, tests pass except the pre-existing flaky `transport_security` (#130).
- [x] 4.5 Docs: tooling only, no user-facing change, no `docs/` page needed; confirm

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #108, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

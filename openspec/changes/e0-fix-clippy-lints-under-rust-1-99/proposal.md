# Proposal

Implements [randygil/corvis#108](https://github.com/randygil/corvis/issues/108) (epic
[#1](https://github.com/randygil/corvis/issues/1), phase 0.1 of the execution plan
[#111](https://github.com/randygil/corvis/issues/111)).

## Why

`cargo clippy --workspace --all-targets -- -D warnings` is part of the definition of done, but it fails on the
installed toolchain (Rust 1.99) with lints that predate the current changes. Without a green clippy step, no change
can meet its definition of done and the CI work (#6) cannot gate on it.

## What Changes

- Fix every lint the workspace reports under Rust 1.99, preferring real, mechanical fixes (collapsed `if`s, type
  aliases, derived impls, `is_multiple_of`, byte strings, struct-update initializers).
- Where a fix would change the public API or hurt readability, add a narrow `#[allow(clippy::<lint>)]` on the single
  item with a one-line reason. No blanket `#![allow(clippy::all)]`, no allowed lint groups.
- Add a `clippy.toml` at the repository root that raises `large-error-threshold` (for `result_large_err`) so the
  public error types (`HttpException`, the ORM error) stay unboxed, instead of allowing the lint.
- Runtime behavior stays identical: only mechanical refactors.

Out of scope:
- Lints introduced by toolchains newer than 1.99 (they get their own change when the toolchain moves).
- Behavior changes, API changes (for example boxing `HttpException`) and refactors beyond what a lint requires.
- CI wiring of clippy (issue #6) and the CRLF-dependent `corvis-swagger` test failures (issue #110).

## Capabilities

### New Capabilities

None (tooling only).

### Modified Capabilities

None (tooling only; no spec delta).

## Impact

- Source files of every crate (`corvis`, `corvis-macros`, `corvis-cli`, `corvis-orm`, `corvis-swagger`) and a few
  integration tests; no dependency changes.
- New `clippy.toml` at the repository root.
- Because non-test runtime code is touched (mechanically), the three ports must be re-checked for parity.

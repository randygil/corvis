# Tasks

## 1. Fix

- [x] 1.1 Annotate the empty gateway list in `application.rs`; verify `cargo check -p corvis --no-default-features`
- [x] 1.2 Allow `ErasedHooks::gateway` to be unread without `websockets`; verify clippy `-D warnings` on the four sets

## 2. CI

- [x] 2.1 Add the per-feature clippy step to the clippy job; verify with a manual CI dispatch

## 3. Release

- [ ] 3.1 Apply the fix to the `release/0.0.1` branch before #11 publishes; verify `cargo publish --dry-run` there

## 4. Definition of done

- [x] 4.1 `cargo clippy --workspace --all-targets -- -D warnings`
- [x] 4.2 `cargo test --workspace` (CI)
- [x] 4.3 No docs change: the features are listed in the crate metadata and the docs do not promise per-feature builds
- [x] 4.4 corvis-parity: not required (no behavior change for the default feature set)

## Workflow follow-up

- [ ] Archive with /opsx:archive, close #153, tick it in epic #1, move the card to Done

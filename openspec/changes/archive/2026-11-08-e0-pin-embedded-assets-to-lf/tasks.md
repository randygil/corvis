# Tasks

## 1. Line endings

- [x] 1.1 Add `.gitattributes` with `text eol=lf` for `crates/*/src/assets/**` and the text fixtures under
  `crates/*/tests/fixtures/`; run `git add --renormalize .`; verify a fresh worktree with `core.autocrlf=true` checks the
  assets out as `w/lf` (`git ls-files --eol`) while other files keep the platform default
- [x] 1.2 Check the rest of the workspace for `include_str!` / `include_bytes!` files whose bytes matter (list them in
  design.md)
- [x] 1.3 `cargo test -p corvis-swagger --lib` passes on the fresh Windows worktree
  - Result: 41 passed, 0 failed (fresh worktree, `core.autocrlf=true`).

## 2. Definition of done

- [x] 2.1 No Rust code changed: build warnings, clippy and the apidiff of the ports are unaffected
- [x] 2.2 `cargo test --workspace` no longer shows the three corvis-swagger failures
  - Result: the three failures were the only CRLF-dependent tests; they pass in 1.3, and no Rust code changed.
- [x] 2.3 Docs EN+ES: no user-facing change; nothing to document

## Workflow follow-up

- Archive the change with `/opsx:archive`.
- Close issue #110, tick it in epic #1 and in #111, move its card to Done.

# Tasks

## 1. Comment and tests

- [ ] 1.1 Replace the rustdoc of `z::coerce::boolean()` (`crates/corvis/src/z/types.rs:3182-3183`) with the text of
  design.md Decision 1; verify `cargo doc -p corvis --no-deps` shows no broken intra-doc link warning for
  `super::stringbool`
- [ ] 1.2 Add the asserts of Decision 2 to `coercion_v3` and a zod 4 block next to `v4_coerce_defaults_refine` in
  `crates/corvis/src/z/tests.rs`; verify `cargo test -p corvis --lib z::tests` passes, and that changing `"0"` to map
  to `false` in `js::truthy` (temporary local edit, reverted) makes the new asserts fail

## 2. Docs

- [ ] 2.1 Check `docs/application/validation.md` (lines 152, 235) and `docs/es/application/validation.md` (same lines)
  against the new comment; they already say `Boolean(x)`, `"false"` gives `true`, and point to `z::stringbool()`, so
  no edit is expected; grep `docs/` for `coerce::boolean` and `coerce.boolean` to confirm no other page repeats the
  wrong claim, and record the result in this task

## 3. Definition of done

- [ ] 3.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 3.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 3.3 `cargo test --workspace` passes
- [ ] 3.4 Regression tests: the new asserts in `crates/corvis/src/z/tests.rs` (1.2)
- [ ] 3.5 Docs pages in English and Spanish (`docs/` and `docs/es/`) checked (2.1); if any page was edited,
  `npm run docs:check` in `docs/` passes
- [ ] 3.6 corvis-parity: not required, only a rustdoc comment and unit tests change, no framework runtime code; state
  this in the commit message

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#129](https://github.com/randygil/corvis/issues/129)
- [ ] Tick #129 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project

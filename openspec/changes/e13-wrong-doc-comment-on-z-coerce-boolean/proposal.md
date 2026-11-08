# Wrong doc comment on z::coerce::boolean

Implements [randygil/corvis#129](https://github.com/randygil/corvis/issues/129) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The rustdoc of `z::coerce::boolean()` (`crates/corvis/src/z/types.rs:3182-3183`) says that in zod 3 mode `"false"`,
`"0"`, `"no"`, `"off"` and `""` become `false`. The code does not do that in any mode: both the zod 3 and the zod 4
paths convert with JavaScript's `Boolean(x)`, so every non-empty string, `"false"` included, becomes `true`. That is
also what real zod 3 and zod 4 do, as the differential fixtures in `crates/corvis/tests/fixtures/zod4/` show. A user
who trusts the rustdoc and validates `?active=false` with `z::coerce::boolean()` gets `true`.

## What Changes

- Rewrite the rustdoc of `z::coerce::boolean()` to describe the real behavior: `Boolean(x)` in both zod 3 and zod 4
  modes, the values that become `false` (`""`, `0`, `null`, a missing value), that `"false"`, `"0"`, `"no"` and
  `"off"` become `true`, and a pointer to `z::stringbool()` for text flags.
- Add regression asserts for those values in both modes to the existing unit tests, so the documented behavior is
  pinned.
- No code behavior changes. The docs site already describes the correct behavior
  (`docs/application/validation.md:152,235` and `docs/es/application/validation.md:152,235`); they are checked and
  left as they are.

## Capabilities

### New Capabilities
- `validation/zod`: the `corvis::z` schema API; this change adds the requirement for `z::coerce::boolean()` and its
  documentation.

## Impact

- Code: one rustdoc comment in `crates/corvis/src/z/types.rs` (lines 3182-3183).
- Tests: extra asserts in `crates/corvis/src/z/tests.rs` (`coercion_v3`, lines 57-68, and `v4_coerce_defaults_refine`,
  starting at line 311, or a new test next to them).
- Docs: none expected (verified, see design.md).
- No runtime behavior changes.

## Out of scope

- The lenient deserializer for route params, query strings and headers (`crates/corvis/src/common/lenient.rs:46-55`),
  which maps `"false"`, `"0"`, `"off"`, `"no"` and `""` to `false` on purpose; it is a different feature and is
  correct as documented there. It is the likely source of the wrong comment.
- `examples/blog-orm` using `z::coerce::boolean()` for `?published=` (`examples/blog-orm/src/posts/dto.rs:44`):
  handled by change `e13-fix-the-examples` (#21).
- Extending the zod fixtures (`crates/corvis/tests/fixtures/zod4/cases.mjs:146,350`) with more inputs; would need
  regenerating `parse.json` with Node and zod, not required for a comment fix.

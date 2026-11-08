# Design

## Context

- **The comment.** `crates/corvis/src/z/types.rs:3182-3183`, in `pub mod coerce` (lines 3171-3195):
  `` /// `z.coerce.boolean()`. zod 3 mode: `"false"`, `"0"`, `"no"`, `"off"` and `""` become `false`; ``
  `` /// zod 4: `Boolean(x)` (any non-empty string is `true`). ``
  The function returns `ZBoolean::new().coerce()` (line 3185).
- **zod 3 path.** `ZBoolean::check` (lines 2016-2027): a JSON boolean passes through; with `coerce`, any other present
  value becomes `Value::Bool(js::truthy(Some(other)))` (line 2020) and a missing value becomes `false` (line 2021).
  The comment at line 2019 already says "`Boolean(input)`: only `""`, `0`, `null` and `undefined` are false
  (`"false"` is true)". `js::truthy` (`crates/corvis/src/z/js.rs:214-225`) is `false` for a missing value, `null`,
  `false`, a number equal to `0` or NaN, and the empty string; `true` for everything else.
- **zod 4 path.** `ZBoolean::run_v4` (lines 2031-2039): with `coerce`, `p.value = Val::Json(Value::Bool(js_truthy(..)))`,
  where `js_truthy` (lines 88-96) delegates JSON values to `js::truthy` and treats `undefined`, `NaN`, `0`, `0n` as
  `false` and dates as `true`. So both modes apply the same rule; nothing in the code reads `"no"` or `"off"`.
- **Tests that already pin it.**
  - `crates/corvis/src/z/tests.rs:62-64` (`coercion_v3`, zod 3 mode): `"false"` → `true`, `""` → `false`,
    `"true"` → `true`.
  - `crates/corvis/src/z/tests.rs:311-319` (`v4_coerce_defaults_refine`, zod 4 mode): `""` → `false`, missing value
    with `.default(true)` → `true`.
  - Differential fixtures against real zod: `crates/corvis/tests/fixtures/zod4/cases.mjs:146` (zod 4.4.3, inputs
    `"false"`, `""`, `0`, `1`, `null`, `undefined`, `"x"`) and `:350` (`zod/v3`, inputs `"false"`, `""`); the recorded
    results in `parse.json` give `true` for `"false"` in both versions, and `crates/corvis/tests/zod4_fixtures.rs`
    replays both lists (line 537 runs `casesV3`).
  So the code and the original library agree; only the rustdoc is wrong.
- **Where the wrong text likely came from.** The lenient deserializer for params, query and headers
  (`crates/corvis/src/common/lenient.rs:46-55`) maps `"false" | "0" | "off" | "no" | ""` to `false`. That is a
  different feature (typed handler arguments, not `z`).
- **Docs.** `docs/application/validation.md:152` and `:235` already say `z::coerce::boolean()` is `Boolean(x)`, that
  `"false"` gives `true`, and recommend `z::stringbool()`; `docs/es/application/validation.md:152,235` say the same in
  Spanish. No other page under `docs/` mentions `coerce::boolean` or `coerce.boolean`.
- **`z::stringbool()`** (`types.rs:3093`) is the documented alternative: its default falsy list is `"false"`, `"0"`,
  `"no"`, `"off"`, `"n"`, `"disabled"` (`StringBoolOptions::default`, lines 2654-2663).

## Goals / Non-Goals

**Goals:**
- The rustdoc of `z::coerce::boolean()` matches the code, the zod fixtures and the docs site.
- A test fails if the coercion stops being `Boolean(x)` for the strings the old comment listed.

**Non-Goals:**
- Changing the coercion (it matches zod 3 and zod 4).
- Changing the lenient deserializer or the examples.

## Decisions

1. **New comment text.**
   ```rust
   /// `z.coerce.boolean()`: converts the input with JavaScript's `Boolean(x)` before validating, in both zod 3 and
   /// zod 4 modes. Only `false`, `""`, `0`, `NaN`, `null` and a missing value become `false`; every other value is
   /// `true`, including the strings `"false"`, `"0"`, `"no"` and `"off"`. To read text flags such as
   /// `?active=false`, use [`z::stringbool()`](super::stringbool).
   ```
   The intra-doc link resolves because `coerce` is a child module of `types`, where `stringbool` is defined, and the
   crate already uses `super::` links in this file (line 2054). Alternative: only delete the zod 3 sentence; rejected
   because the user still would not learn which strings are `false` or what to use instead.
2. **Regression asserts in the existing unit tests, not a doctest.** Add `"0"`, `"no"`, `"off"` → `true` and `0`,
   `null` → `false` to `coercion_v3`, and a zod 4 block (`z::with_version(z::ZodVersion::V4, ..)`) with `"false"`,
   `"0"`, `"no"`, `"off"` → `true` and `""`, `0`, `null` → `false`. Alternatives: a runnable doctest in the comment
   (the `z` module only uses `ignore`/`text` code blocks, e.g. `z/mod.rs:259`, and each doctest is a separate
   compilation); extending the zod fixtures (needs regenerating `parse.json` with Node and zod; listed as out of
   scope).
3. **Docs pages unchanged.** They already state the correct behavior (Context); the task records the check.

## Risks / Trade-offs

- [None for runtime] → only a comment and test asserts change.
- [The comment drifts again] → the new asserts cover exactly the strings the comment names.

## Open questions

- None.

## Review notes (2026-11-08)

Reviewed: no open questions; the new comment text and the regression asserts are approved as proposed.

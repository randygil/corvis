# Design

## Context

Toolchain: rustc/clippy 1.99.0. The lints were counted from real `cargo clippy --workspace --all-targets
--keep-going -- -D warnings` runs on `main` (7d2a83f). Clippy does not compile a crate whose dependency failed, so the
inventory appeared in stages: first `corvis-macros` and `corvis-cli`, then `corvis`, then the crates and tests that
depend on `corvis`. Counts are unique source locations (the library and its test target report the same location
twice).

## Lint inventory (before)

| Crate | Lint | Count |
|---|---|---|
| corvis-macros | `collapsible_if` | 2 |
| corvis-cli | `collapsible_if` | 2 |
| corvis | `collapsible_if` | 60 |
| corvis | `result_large_err` | 35 |
| corvis | `type_complexity` | 12 |
| corvis | `byte_char_slices`, `manual_is_multiple_of`, `field_reassign_with_default`, `result_unit_err` | 2 each |
| corvis | `chunks_exact_to_as_chunks`, `default_constructed_unit_structs`, `derivable_impls`, `extra_unused_type_parameters`, `items_after_test_module`, `large_enum_variant`, `manual_is_ascii_check`, `manual_range_contains`, `needless_late_init`, `new_without_default`, `useless_format` | 1 each |
| corvis-orm, corvis-swagger, `corvis` integration tests | `collapsible_if` (fixed automatically, same pattern as above) | not counted separately |
| corvis-orm, corvis-swagger, `corvis` integration tests, `cats-app` | `result_large_err` | about 190 (same two error types) |
| corvis-orm | `should_implement_trait` (`Decimal::cmp`, `Decimal::eq`) | 2 |
| corvis-orm | `type_complexity` | 2 |
| corvis-orm | `replace_with_identity` | 1 |
| corvis-swagger | `type_complexity` | 1 |
| corvis-orm tests | `doc_lazy_continuation` (generated file) | 4 lines, 1 module |

Total in `corvis` alone: 126; the dependents add the rest. After this change: 0.

## Policy

1. **Fix** when the fix is mechanical and behavior-neutral: `cargo clippy --fix` for `collapsible_if`,
   `manual_is_multiple_of`, `byte_char_slices`, `manual_range_contains`, `manual_is_ascii_check`,
   `useless_format`, `needless_late_init`, `derivable_impls`, `new_without_default`,
   `default_constructed_unit_structs`, `chunks_exact_to_as_chunks`; by hand for the rest: type aliases for
   `type_complexity`, struct-update initializers for `field_reassign_with_default`, dropping the unused generic of
   `TestingModule::override_enhancer`, removing a no-op `replace("varbit", "varbit")`.
2. **Configure** `result_large_err` in `clippy.toml` (`large-error-threshold = 256`). `HttpException` (128+ bytes)
   and the ORM error are returned by value across the whole public API and the macro-generated code; boxing them is
   an API change outside this issue. A threshold, unlike an allow, still reports an error type that grows well past
   today's size. Two internal functions return larger error values on purpose (the request handed back to the
   caller, the ready-made rejection response) and carry a local allow.
3. **Allow narrowly** (single item, one-line reason) when the fix changes the public API or hurts readability:
   `result_unit_err` on `ZodType::check` and `Schema::check`, `large_enum_variant` on a per-request value,
   `should_implement_trait` on `Decimal::cmp`/`eq` (decimal.js signatures), `doc_lazy_continuation` on the module that
   includes generated code.
4. Never a crate-level `allow(clippy::all)` or an allowed group. New allows need a reason comment.

## Risks

- Runtime code changes mechanically; `cargo test --workspace` plus the parity check (three ports, 0 apidiff
  differences) cover it.
- `crates/corvis-orm/src/engine/filter.rs` is being edited by #109; clippy reported only `result_large_err` there,
  which the threshold resolves, so this change does not touch that file.

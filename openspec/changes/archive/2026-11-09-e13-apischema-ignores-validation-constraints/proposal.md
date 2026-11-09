# Proposal

Implements [randygil/corvis#128](https://github.com/randygil/corvis/issues/128) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found while drafting the change for #21 (fix the examples).

## Why

`#[derive(ApiSchema)]` turns every named field into an OpenAPI property, but it ignores the constraints declared on
the same field for `#[derive(Validate)]` (`#[max_length(50)]`, `#[min(0)]`, `#[is_email]`, `#[is_in(...)]`...). The
original framework's OpenAPI compiler plugin reads the class-validator decorators of a DTO (its `classValidatorShim`
option, on by default) and adds `minLength`, `maximum`, `pattern`, `format`, `enum`, `minItems`... to each property. A
DTO migrated from the original therefore produces a less precise document in Corvis, and the docs tell users to repeat
every constraint by hand in `#[api_property(...)]` (`docs/openapi/types-and-parameters.md:117-119`).

## What Changes

- `#[derive(ApiSchema)]` reads the `#[derive(Validate)]` attributes of each field and adds the OpenAPI keys the
  original plugin adds for the equivalent class-validator decorator, with the same values and the same precedence
  (design.md, Decision 2 has the table): `min` → `minimum`, `max` → `maximum`, `min_length` / `max_length` / `length`
  → `minLength` / `maxLength`, `matches` and `contains` → `pattern`, `is_email` / `is_url` / `is_uuid` → `format`,
  `is_hex_color` → `pattern`, `is_positive` / `is_negative` → `minimum: 1` / `maximum: -1`, `array_min_size` /
  `array_max_size` / `array_not_empty` → `minItems` / `maxItems`, `array_unique` → `uniqueItems`, `is_in` → `enum`
  (plus `isArray` with `each`).
- `#[is_enum]` adds `enum` with the enum's variant names. The original takes `enum` from the TypeScript type, not from
  `@IsEnum`; this stands in for that (Decision 4).
- Rules the original plugin does not map (`is_int`, `is_date_string`, `is_not_empty`, `is_optional`...) add nothing.
- Explicit `#[api_property(...)]` options always win over derived keys, as explicit `@ApiProperty()` options win over
  the plugin's metadata in the original.
- Array fields keep the existing behavior of the document generator: value constraints (`minimum`, `maxLength`,
  `pattern`, `format`...) move into `items`, as `transform_to_array_schema_property` already does.
- A differential fixture compiles the same DTOs with the original plugin and records `components.schemas`; a Rust test
  derives the equivalent structs and compares.
- `examples/cats-app/tests/openapi.rs` asserts the derived constraints of `CreateCatDto`.
- Docs (EN + ES): `openapi/types-and-parameters` (the warning becomes the mapping table) and `openapi/cli-plugin`.

## Capabilities

### New Capabilities

- `openapi/api-schema`: which `#[derive(Validate)]` constraints `#[derive(ApiSchema)]` turns into OpenAPI keywords,
  with what values and precedence.

### Modified Capabilities

None.

## Impact

- `crates/corvis-swagger-macros/src/derive.rs` (read validation attributes per field) and a new module
  `crates/corvis-swagger-macros/src/constraints.rs` (the mapping table and its argument parsing).
- `crates/corvis-swagger/src/lib.rs` (`__private` gains the hex color pattern constant if it is not inlined).
- Tests: a new `crates/corvis-swagger/tests/api_schema_constraints.rs` and a fixture under
  `crates/corvis-swagger/tests/fixtures/class_validator_shim/`; unit tests in `corvis-swagger-macros`.
- `examples/cats-app/tests/openapi.rs`.
- Docs: `docs/openapi/types-and-parameters.md`, `docs/openapi/cli-plugin.md` and their translations under `docs/es/`.
- Users: documents of `ApiSchema` + `Validate` DTOs gain keywords. Clients generated from the document become stricter
  (for example a generator that turns `maxLength` into a client-side check). No runtime validation changes.
- Port apps: none of consi, facten or houseplan derives `ApiSchema` or `Validate` (design.md, Context), so apidiff
  should not move; the corvis-parity check confirms it.

## Out of scope

- Zod DTOs (`zod_dto!`): they already document their constraints through the zod schema.
- class-validator decorators the plugin maps but Corvis has no rule for (`IsBase64`, `IsCreditCard`, `IsCurrency`,
  `IsJSON`, `IsMobilePhone`, `IsAscii`, `IsHexadecimal`, `IsDivisibleBy`): adding those rules is a validation feature;
  possible new issue.
- `#[is_optional]` on a non-`Option` field making the property optional: the original takes `required` from the `?`
  token, not from `@IsOptional`, so parity keeps it required; possible new issue if users want it.
- `validate_nested` and nested DTO schemas: references already come from the field type.
- Descriptions from `///` comments (`introspectComments`): separate feature.
- Request parameters (`#[query]`, `#[param]`) described from a `Validate` DTO: they go through the explorer, not the
  derive; possible new issue if the plugin output differs there.

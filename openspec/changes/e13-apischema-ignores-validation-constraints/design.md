# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta `openapi/api-schema`. Current state, verified in
the code on 2026-11-08:

**`#[derive(ApiSchema)]`**

- Declared in `crates/corvis-swagger-macros/src/lib.rs:372-385` with the helper attributes `api_property`,
  `api_property_optional`, `api_response_property`, `api_hide_property` and `api_schema`; the expansion is
  `derive::expand` (`crates/corvis-swagger-macros/src/derive.rs:137-239`).
- Per field (`derive.rs:174-217`): skips `#[serde(skip)]` / `skip_serializing` and `#[api_hide_property]`, computes the
  JSON name from `#[serde(rename)]` / `rename_all`, then builds an options map `__o` from the `api_*` attributes only
  (`derive.rs:191-206`: `readOnly` first for `api_response_property`, then the explicit options, then
  `required: false` for `api_property_optional`). Every other attribute of the field is ignored, including the
  `Validate` ones.
- The map is passed to `property_metadata(__o, design, optional)` (`derive.rs:215`;
  `crates/corvis-swagger/src/schema_factory.rs:149-171`), which runs `create_api_property` (`schema_factory.rs:104-144`,
  a port of `createApiPropertyDecorator`: `enum` + `isArray` become `type: array` with `items.enum`) and marks
  `Vec<T>`-like fields with `isArray`.
- The document generator is a port of `SchemaObjectFactory`. `SCHEMA_OPTIONS_KEYS` (`schema_factory.rs:242-268`)
  already carries `minimum`, `maximum`, `minLength`, `maxLength`, `minItems`, `maxItems`, `uniqueItems`, `format`,
  `pattern`... into the schema, and `transform_to_array_schema_property` (`schema_factory.rs:693-715`) moves `format`,
  `maximum`, `maxLength`, `minimum`, `minLength` and `pattern` into `items` for array properties. So keys placed in the
  options map already reach the document in the original's shape; only the derive does not produce them.
- `docs/openapi/types-and-parameters.md:117-119` warns that the derive does not read `Validate` constraints;
  `docs/openapi/cli-plugin.md` lists what the macros do instead of the plugin, with no row for class-validator.
- No test derives `ApiSchema` together with validation attributes and checks keywords;
  `examples/cats-app/tests/openapi.rs:35-40` checks that `CreateCatDto`'s properties exist and their `description` /
  `example`, not constraints. `CreateCatDto` (`examples/cats-app/src/cats/dto/mod.rs:7-24`) has `#[max_length(50)]`,
  `#[min(0)]`, `#[max(40)]` and `#[is_in(...)]`.

**`#[derive(Validate)]`**

- Declared in `crates/corvis-macros/src/lib.rs:110` with one helper attribute per rule; the rule names are in
  `RULES` (`crates/corvis-macros/src/validate.rs:9-57`).
- `rule_args` (`validate.rs:205-225`) splits the arguments into positional expressions, `message = ...` and `each`
  (bare `each` or `each = ...`).
- Arguments are Rust expressions: `#[min(0)]`, `#[length(2, 20)]`, `#[matches("^[a-z]+$")]` (a Rust regex string,
  compiled with `regex::Regex`, `validate.rs:369-381`), `#[is_in("a", "b")]` or `#[is_in(["a", "b"])]` (flattened,
  `validate.rs:394-411`), `#[is_enum]` / `#[is_enum(T)]` (`validate.rs:441-456`, element type with `each`).
- `corvis::validation::rules::enum_variants::<T>()` (`crates/corvis/src/validation/rules.rs:223-228`) returns the
  variant names serde accepts for `T`; `Validate`'s `is_enum` message uses it.
- Helper attributes of one derive are visible to every other derive on the same item, so `ApiSchema` can read
  `#[max_length]` without declaring it, and a struct that does not derive `Validate` cannot carry these attributes at
  all (unknown attribute error).

**Original framework (read from `@nestjs/swagger` 11.4.6 in the facten original's `node_modules`)**

- `classValidatorShim` defaults to `true` (`dist/plugin/merge-options.js:9`).
  `ModelClassVisitor.createDecoratorObjectLiteralExpr` (`dist/plugin/visitors/model-class.visitor.js:187-202`) builds
  each property's metadata as `required` (from `?`), `type`, description, `default`, `enum` (from the TypeScript type),
  then `createValidationPropertyAssignments` (`model-class.visitor.js:447-544`).
- `createValidationPropertyAssignments` pushes, in this order: `IsIn` → `enum` (+ `isArray: true` when `{ each: true }`);
  the table `decoratorsProperties` (`dist/services/decorators-properties.js`): `Min` → `minimum`, `Max` → `maximum`,
  `MinLength` → `minLength`, `MaxLength` → `maxLength`, `ArrayNotEmpty` → `minItems: 1`, `IsPositive` → `minimum: 1`,
  `IsNegative` → `maximum: -1`, `ArrayUnique` → `uniqueItems: true`, `IsBase64` / `IsCreditCard` / `IsCurrency` /
  `IsEmail` / `IsJSON` / `IsUrl` / `IsUUID` / `IsMobilePhone` → `format` (`base64`, `credit-card`, `currency`, `email`,
  `json`, `uri`, `uuid`, `mobile-phone`), `IsAscii` / `IsHexColor` / `IsHexadecimal` → a fixed `pattern`,
  `ArrayMinSize` → `minItems`, `ArrayMaxSize` → `maxItems`, `IsDivisibleBy` → `multipleOf`, `Contains` → `pattern`
  (the raw argument); then `Length` → `minLength` (+ `maxLength`); then `Matches` → `pattern` (the regex source
  without flags, or the string argument; nothing for a non-literal argument).
- The assignments form one object literal, so a later key replaces an earlier one (`IsPositive` after `Min` wins).
  For each decorator name only the first decorator found counts (`getDecoratorOrUndefinedByNames`).
- `each` is ignored except for `IsIn`. `IsEnum`, `IsInt`, `IsDateString`, `IsNotEmpty`, `IsOptional` and the rest add
  nothing through the shim (`IsInt` does not turn `number` into `integer`).
- The plugin's metadata is applied with `createApiPropertyDecorator(metadata, false)`
  (`dist/services/model-properties-accessor.js:27-31`), and `createPropertyDecorator` with `overrideExisting = false`
  spreads the existing explicit metadata over it (`dist/decorators/helpers.js:53-55`): explicit `@ApiProperty()`
  options win.

**Port apps**

- None of the three ports derives `ApiSchema` or `Validate` (no `derive(... ApiSchema` or `derive(... Validate` under
  `C:/Users/Usuario/Devel/corvis-tests/corvis/*/src`). consi serves the original's `openapi.json` with
  `corvis_swagger::scalar::mount` (`consi-backend/src/main.rs:89-90`). So apidiff should not move.

## Goals / Non-Goals

**Goals:**
- A `Validate` + `ApiSchema` DTO produces the same `components.schemas` entry as the equivalent class-validator DTO in
  the original with its compiler plugin.
- No new attribute to learn: constraints written once for validation also document the API.

**Non-Goals:**
- New validation rules for the decorators the plugin maps but Corvis lacks (proposal, Out of scope).
- Changing `required` from `#[is_optional]`, or `type: integer` from `#[is_int]` (the original does neither).
- Zod DTOs and request parameters.

## Decisions

1. **Read the validation attributes in the `ApiSchema` derive, at macro time.** A new module
   `crates/corvis-swagger-macros/src/constraints.rs` recognizes the rule attributes by name, parses their arguments
   like `rule_args` (positional, `message = ...` ignored, `each`), and returns `__o.insert(...)` statements that
   `derive.rs` places **before** the explicit `api_*` options of the field, so explicit options overwrite them
   (Decision 3). Argument expressions are emitted as `::corvis_swagger::__private::to_value(&(expr))`, so constants
   work as well as literals. `ApiSchema` does not declare the rule names as its own helper attributes.
   Alternatives considered: (a) a runtime hook on the `Validate` trait (`fn constraints() -> Vec<..>`) found by the
   probe in `corvis-swagger/src/lib.rs`: needs a new public trait method and a second probe, for information already
   visible to the macro; rejected. (b) a shared non-proc-macro crate holding the rule table for both derives: a new
   workspace crate for one list; rejected. The two lists can drift; a test (task 2.2) derives both on a DTO that uses
   every mapped rule, so a renamed rule breaks compilation of the test.
2. **Mapping table and precedence copied from the plugin.** The table is the spec delta's. Emission order follows
   `createValidationPropertyAssignments`: `is_in`; then `min`, `max`, `min_length`, `max_length`, `array_not_empty`,
   `is_positive`, `is_negative`, `array_unique`, `is_email`, `is_url`, `is_uuid`, `is_hex_color`, `array_min_size`,
   `array_max_size`, `contains` (the plugin's table order restricted to rules Corvis has); then `length`; then
   `matches`. Because they are successive inserts into one map, a later key replaces an earlier one, as in the
   object literal. For a rule repeated on a field only the upper attribute counts. The fixed patterns (`is_hex_color`)
   are copied verbatim from `decorators-properties.js`.
   Alternative considered: "most restrictive wins" (for example `max(min(5), is_positive)`). Rejected: diverges from
   the original for no user-visible gain, and the fixture would have to list it as a difference.
3. **Explicit options win; derived keys never touch `required`, `readOnly` or `type`.** Inserting derived keys before
   the explicit ones reproduces `{ ...plugin, ...explicit }`. None of the mapped rules produces those three keys.
4. **`is_enum` maps to `enum` with the variant names (Corvis addition).** The original's `enum` comes from the field's
   TypeScript type (`createEnumPropertyAssignment`), which runs before the validation keys; `@IsEnum` itself adds
   nothing. Rust DTOs validate enums with `#[is_enum(T)]` over a `String` field or `#[is_enum]` over an enum-typed field,
   and a Rust enum without `ApiSchema` or `SwaggerType` is an opaque type for the derive today
   (`crates/corvis-swagger/src/lib.rs` probe fallback). So the derive emits
   `enum: ::corvis::validation::rules::enum_variants::<T>()` first (the position of the type-based `enum`), and
   `is_in` and explicit options can replace it. With `each` it also sets `isArray: true`, as `is_in` does.
   Alternative considered: no mapping, for strict parity with `@IsEnum`. Rejected: the TypeScript DTO the user
   migrates usually has `role: Role`, which does produce `enum`; the Rust analog is `#[is_enum]`.
5. **`matches` and `contains` emit their argument as written.** `matches` emits the Rust regex string, even when it
   is a constant (the plugin skips non-literal patterns; Rust has no such limitation and the value is known).
   `contains` emits the raw substring as `pattern`, like the plugin, even though a substring with regex
   metacharacters is not the same pattern. Both are documented. Rust-only regex syntax (inline flags such as `(?i)`,
   `\p{..}` classes) is emitted unchanged; the docs say `pattern` uses the validator's syntax.
   Alternative considered: escape `contains` values (`regex::escape`). Rejected for parity; Open question 2.
6. **Differential fixture with the real compiler plugin.** `crates/corvis-swagger/tests/fixtures/class_validator_shim/`:
   `package.json` pinning `@nestjs/common` and `@nestjs/core` 11.2.1, `@nestjs/swagger` 11.4.6 (the facten
   original's versions), `@nestjs/platform-express` 11.2.1, `class-validator` (exact 0.14.x version),
   `typescript` 5.9.3, `reflect-metadata` 0.2.2 and `rxjs` 7.8.2. `dtos.ts` declares the fixture DTOs;
   `generate.mjs` compiles it with `ts.createProgram` and `program.emit(..., { before: [plugin.before({
   classValidatorShim: true, introspectComments: false }, program)] })` from `@nestjs/swagger/plugin`, loads the
   output, registers each DTO as a `@Body()` of a controller, runs `SwaggerModule.createDocument` without listening on
   a port, and writes `components.schemas` to `class_validator_shim.json`. DTOs cover: every mapped rule once;
   precedence pairs (`Min` + `IsPositive`, `Contains` + `Matches`, `MinLength` + `Length`); `each` on an array field
   (`MaxLength(10, { each: true })`, `IsIn([...], { each: true })`); optional fields; explicit `@ApiProperty` overriding
   a derived key; an enum field (`@IsEnum(Role) role: Role`); unmapped rules (`IsInt`, `IsDateString`, `IsNotEmpty`).
   The Rust test (`crates/corvis-swagger/tests/api_schema_constraints.rs`) declares the equivalent structs, builds the
   document and compares each schema as a `serde_json::Value`. Known differences (if the fixture shows any, for
   example around numeric types) are listed in the JSON next to the scenario with a reason and asserted explicitly.
   Alternative considered: hand-written expected JSON. Rejected: the issue asks for comparison with the original's
   output, and the plugin's precedence quirks are easy to get wrong by hand.

## Risks / Trade-offs

- [Documents of existing `ApiSchema` + `Validate` DTOs change] → additive keywords only; clients generated from the
  document may start checking lengths and ranges the server already enforces. Noted in the docs.
- [The plugin's quirks are reproduced (`is_positive` → `minimum: 1` also for floats, `contains` → unescaped
  `pattern`)] → parity first; documented in the mapping table; Open question 2.
- [Rule names duplicated in two proc-macro crates] → Decision 1's test; a comment in `validate.rs`'s `RULES` points to
  `constraints.rs`.
- [Rust regex syntax in `pattern`] → documented; OpenAPI tools that compile `pattern` as ECMA-262 may reject Rust-only
  constructs. Rare in DTO patterns.
- [Fixture needs Node and network once to install] → like the other fixtures, the JSON is committed and the Rust test
  only reads it.

## Open questions

1. **`is_enum`.** Decision 4 maps it although `@IsEnum` maps nothing in the original. Confirm, or drop it for strict
   parity (users would add `#[api_property(enum = [...])]`).
2. **Quirks.** Keep `is_positive` → `minimum: 1` and the unescaped `contains` pattern for parity (this proposal), or
   diverge (`exclusiveMinimum: 0` / escaped pattern) and list them as differences in the fixture.
3. **`class-validator` version.** The originals do not depend on class-validator (none of consi, facten or houseplan
   uses it), so the fixture picks the current 0.14.x release; the plugin's mapping does not depend on it. Confirm.

## Review notes (2026-11-08)

1. `is_enum` maps to `enum`.
2. The two quirks of the original plugin are fixed, not copied, and documented as differences: `is_positive` / `is_negative` on floats produce `exclusiveMinimum: 0` / `exclusiveMaximum: 0` (integers keep `minimum: 1` / `maximum: -1`), and `contains` produces a `pattern` with the text escaped.
3. The fixture pins an exact class-validator 0.14.x version.

## Implementation notes

- The review notes were applied: `is_enum` maps to `enum`; floats get `exclusiveMinimum: 0` / `exclusiveMaximum: 0` for
  `is_positive` / `is_negative` (a field whose type, through `Option` / `Vec` / `Box`, is `f32` or `f64`; integers and
  any other type keep `minimum: 1` / `maximum: -1`; a type alias to a float is seen as an integer); `contains` emits an
  escaped `pattern` (`__private::escape_pattern` escapes `\ ^ $ . * + ? ( ) [ ] { } |`, valid in ECMAScript and Rust
  regex syntax); the fixture pins class-validator 0.14.4.
- For a float, `is_positive` does not replace an earlier `minimum` (it adds `exclusiveMinimum`), so `min(5)` with
  `is_positive` on an `f64` gives both keys.
- Deviation: the intended differences are listed in the Rust test (`known_differences()` in
  `api_schema_constraints.rs`, asserted against the recorded value) and not in the generated JSON, because the JSON is
  produced by the original and a hand-edited entry would be lost on regeneration. The fixture matched the spec delta
  otherwise (no change to the mapping or the precedence).
- Fixture details: the plugin refuses `dtoFileNameSuffix: ['.ts']`, so the DTOs are in `cases.dto.ts`; the plugin emits
  `require()`, so `generate.mjs` compiles to CommonJS in `out/` (ignored by git, like `node_modules`). The committed
  files are `package.json`, `package-lock.json`, `cases.dto.ts`, `generate.mjs` and `class_validator_shim.json`.
- `scripts/check-naming.mjs` gained an allowlist entry for `crates/corvis-swagger/tests/fixtures/**` (the fixture names
  the original's packages).
- Port apps (task 6.1): no `derive(... ApiSchema` or `derive(... Validate` under
  `C:/Users/Usuario/Devel/corvis-tests/corvis/*/src` (grep on 2026-11-09 found none).- `cargo doc -p corvis-swagger --no-deps` prints two warnings that were already there and are not in code of this change: an unresolved link to `crate::model` and a public doc of `serialize_config_to_js` that links to the private item `escape_inline_json`.

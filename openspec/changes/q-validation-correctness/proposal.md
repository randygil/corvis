# Proposal

Implements the issue filed from `quality/q-validation-correctness.issue.md` (epic
[#187](https://github.com/randygil/corvis/issues/187), code quality audit). Source: an engineering-quality review (not a
security review) of `crates/corvis/src/z/`, `crates/corvis/src/validation/` and the validation pipes of
`crates/corvis/src/common/`, read against `dev` at `d46758a`. It is the first of three changes cut from that review, in
dependency order: this one (correctness), then `q-validation-hot-path` (copies on the success path, measured), then
`q-validation-api-hygiene` (typed options and consistency). Each is valid on its own.

## Why

The review found bugs a client can reach and divergences from the libraries the module ports:

- `z.coerce.date()` overflows `i64` on a year of 18 digits (`"999999999999999999-01-01"`): a panic in debug builds, a
  wrapped value in release builds (the workspace and the port apps do not enable `overflow-checks`), which is sometimes
  accepted as a valid date. The same parser overflows on a large offset and turns an over-long numeral into `0`.
- zod 3 mode (`ZodVersion::V3`) is a second engine that re-implements, with Rust primitives, what zod 4 mode already does
  with JavaScript ones: string length in code points instead of UTF-16 units, a `multipleOf` tolerance that grows with the
  value, Rust `trim`, Rust `parse::<f64>` for `z.coerce.number()`, Rust float formatting in messages, and email, cuid, url
  and ip patterns that are not zod 3's. The `casesV3` fixtures only use ASCII and small numbers, so none of this is caught.
- `.overwrite()` and `.check()` do not expose the structure of the schema they wrap: an object with `.overwrite()` as a
  discriminated union option panics when the union is built, and a template literal part with either panics too.
- `schema.parse()` panics where the original throws (an intersection whose results cannot be merged, a transform met while
  encoding); only the pipe and the serializer catch it, and every such panic prints the panic hook's message.
- `ParseArrayPipe` turns `"NaN"` into `null` and `"1e20"` into `i64::MAX`; `isNumberString` accepts `"NaN"`, `"inf"` and
  `"1e5"`; `z.coerce.bigint()` saturates; `ZDate` parses its bounds twice with two different parsers.
- `z::literal(json!([]))` panics at the first parse instead of when it is built; `ParseEnumPipe::of::<T>()` for an
  untagged enum silently rejects every value; three tests bound wall-clock time.

Every claimed difference from an original library is settled by fixture cases recorded from that library, designed here
and recorded by the implementer before any fix; a claim the fixture contradicts is dropped.

## What Changes

- **Fixtures first.** New cases in `tests/fixtures/zod4/cases.mjs` for zod 3 mode and zod 4 mode (lengths with astral
  characters, `multipleOf` with large and decimal values, the format edges, coercion edges, `trim` edges, numbers in
  messages, discriminated union and template literal parts behind wrappers, dates with huge years), recorded from zod
  4.4.3 and its `zod/v3` build; the recorder also records construction errors and non-finite numbers. A new fixture
  package `tests/fixtures/builtin_pipes/` records the original framework's built-in pipes and class-validator 0.14.4
  rules (`isNumberString`, `isIP`, `minLength`, `maxLength`).
- **Date parser.** Numerals read with checked arithmetic, years outside the ECMAScript range refused before any
  arithmetic, day and time arithmetic in `i128`, offsets in `f64`: invalid date for every input that does not fit, the
  same in debug and release. A proptest property for the totality of `js::parse_date`, `string_to_number` and
  `string_to_bigint`.
- **zod 3 mode reuses the JavaScript primitives** (`js_len`, `js_trim`, `string_to_number`, `to_js_string`,
  `num_to_string`) and gets zod 3's own patterns as `JsRegex`, each fix landing with its fixture case. The v3 engine stays
  a separate engine (design.md Decision 4).
- **Check-like wrappers** (`refine`, `superRefine`, `check`, `overwrite`) forward the structural methods of the wrapped
  schema through one macro; their zod 3 behavior follows the zod 3 fixture.
- **Thrown errors are returned.** The two places that throw in the original unwind with a private payload that does not
  call the panic hook, and every parse entry point turns it into `Err(ZodError)` with `thrown()`. The pipe and the
  serializer answer as today. A panic in user code keeps being caught by the pipe and is now logged, rate-limited, without
  its message.
- **Coercion.** `ZDate` keeps one bound parsed once with the JavaScript parser; `z.coerce.bigint()` refuses what does not
  fit in `i128`; the built-in pipes and the class-validator rules follow their fixture.
- **Construction.** `z::literal(json!([]))` panics when built, with the original's message; `z::native_enum::<T>()` and
  `ParseEnumPipe::of::<T>()` panic when built for a type whose variant names cannot be read.
- **Tests without timing bounds** for the linear traversal, the linear regex engine and `array_unique`.

## Capabilities

### New Capabilities

- `core/builtin-pipes`: the built-in pipes follow a fixture recorded from the original framework.

### Modified Capabilities

- `validation/zod`: added requirements for a total date parser, zod 3 mode against its fixture, thrown errors returned as
  errors, structure-preserving check wrappers, one date bound for both versions, BigInt coercion without saturation, and
  construction failures at build time.
- `core/validation-pipe`: added requirement that the number, address and length rules follow the class-validator fixture.

## Impact

- New: `crates/corvis/tests/fixtures/zod4/package.json` and `package-lock.json` (zod 4.4.3 pinned),
  `crates/corvis/tests/fixtures/builtin_pipes/{package.json,package-lock.json,generate.mjs,builtin_pipes.json}`,
  `crates/corvis/tests/builtin_pipes_fixtures.rs`, `crates/corvis/tests/zod_thrown.rs`,
  `crates/corvis/tests/zod_linear_traversal.rs`.
- Edited: `crates/corvis/src/z/{js,types,compound,wrappers,core,mod,pipe,error}.rs`,
  `crates/corvis/src/common/pipe.rs`, `crates/corvis/src/validation/rules.rs`, `crates/corvis/src/fuzz_tests.rs`,
  `crates/corvis/tests/{zod4_fixtures,zod_security}.rs`, `crates/corvis/tests/fixtures/zod4/{cases.mjs,parse.json}`,
  `docs/application/validation.md`, `docs/es/application/validation.md`, `docs/pipes.md`, `docs/es/pipes.md`.
- Public API: `ZodError` gains a private field and `thrown()` (code that builds it with a struct literal must call
  `ZodError::new`); `parse` and `safe_parse` return `Err` where they panicked. This changes now, before 1.0.
- Behavior visible to applications: listed in design.md "Behavior visible to applications". zod 4 mode changes only for
  dates with years outside the ECMAScript range and for check-like wrappers that panicked; zod 3 mode changes where the
  fixture shows it differed.
- No new crate dependency. Framework runtime code changes, so the corvis-parity check runs; the three port apps use zod 4
  mode only (design.md Decision 4).

## Out of scope

- Copies on the success path, the global configuration lock, per-key costs of strict objects (`q-validation-hot-path`).
- Typed JSON Schema options, `normalize` and `hash` arguments, the pipes' option names, comments and dead code
  (`q-validation-api-hygiene`).
- A differential fixture for the remaining class-validator rules (`isEmail`, `isUrl`, `isDateString`, ...) and for the
  implicit conversion of query and route values (`common/lenient.rs`, `rules::coerce`): possible follow-ups.
- The OpenAPI `enum: []` that `corvis-swagger-macros` emits for a type without readable variant names
  (`constraints.rs:164`): a follow-up for the OpenAPI module.
- zod 3 mode date messages, whose original text depends on the server's time zone name (design.md Review note 3).

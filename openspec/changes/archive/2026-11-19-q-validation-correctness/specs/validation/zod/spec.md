## ADDED Requirements

### Requirement: The date parser is total in every build
`z.coerce.date()`, `z.date()` with date strings and the bounds of `z::date().min/max` SHALL give an invalid date, never
a panic and never a wrapped value, for any input string, in debug and release builds alike. A year outside the
ECMAScript range (±275,760), an offset or a numeral too large to compute SHALL give an invalid date before any
arithmetic; inside the range the result SHALL equal the one recorded from zod 4.4.3.

#### Scenario: Huge year
- **WHEN** `z::coerce::date()` parses `"999999999999999999-01-01"` in a debug build and in a release build
- **THEN** both return an `invalid_type` issue with `received: "Invalid Date"` and nothing panics

#### Scenario: A year whose wrapped value would look valid
- **WHEN** a release build parses `"<y>-01-01"` for a year `y` above 10^17 whose wrapping arithmetic would land inside
  ±8.64e15 milliseconds
- **THEN** the result is an invalid date

#### Scenario: Edges of the range
- **WHEN** `z::coerce::date()` parses `"+275760-09-13T00:00:00.000Z"` and `"+275760-09-13T00:00:00.001Z"`
- **THEN** the first gives the date and the second an invalid date, as recorded in `parse.json`

### Requirement: zod 3 mode follows the recorded zod 3 build
With `ZodVersion::V3`, string lengths, `multipleOf`, the `email`, `cuid`, `url` and `ip` formats, `z.coerce.number()`,
`z.coerce.string()`, `.trim()` and the numbers written in messages SHALL give, for every case of `casesV3` in
`tests/fixtures/zod4/parse.json`, the result recorded from the `zod/v3` build of zod 4.4.3, computed with the same
JavaScript primitives as zod 4 mode (UTF-16 length, `Number()`, `String()`, `String.prototype.trim`).

#### Scenario: Astral character
- **WHEN** `z::string().min(2)` parses `"😀"` with `ZodVersion::V3`
- **THEN** the result is the one recorded for that input in `casesV3` (the string is two UTF-16 units long)

#### Scenario: Large value and multipleOf
- **WHEN** `z::number().multiple_of(1.0)` parses `1000000000000.5` with `ZodVersion::V3`
- **THEN** the result is the one recorded for that input in `casesV3`

#### Scenario: Coercion of number text
- **WHEN** `z::coerce::number()` parses `"inf"` and `"0x10"` with `ZodVersion::V3`
- **THEN** each result is the one recorded in `casesV3`, and neither is a successful `null`

### Requirement: What the original throws is returned as an error
A parse that the original aborts by throwing (an intersection whose results cannot be merged, a unidirectional
transform met while encoding) SHALL return `Err(ZodError)` with empty issues and `thrown()` equal to the recorded
message, and MUST NOT panic or call the panic hook. `ZodValidationPipe` SHALL answer it with 400 without `errors`, the
serializer with 500.

#### Scenario: Unmergeable intersection
- **WHEN** `schema.parse` runs an intersection of two transforms that return different values
- **THEN** it returns `Err(e)` with `e.issues` empty and `e.thrown()` equal to the recorded message, and a counting
  panic hook was not called

#### Scenario: Through the pipe
- **WHEN** `ZodValidationPipe` validates the same input with that schema
- **THEN** the answer equals the recorded `pipeThrown` response

#### Scenario: A user callback panics
- **WHEN** a transform closure panics while `ZodValidationPipe` validates a request, twice within a minute
- **THEN** both requests get the 400 of a thrown error and one `error` line is logged, without the panic's message

### Requirement: Check-like wrappers keep the structure of what they wrap
In zod 4 mode, `.refine()`, `.superRefine()`, `.check()` and `.overwrite()` SHALL expose the wrapped schema's values,
property values, pattern, optionality and type, so the wrapped schema can be a discriminated union option or a template
literal part wherever the unwrapped one can. In zod 3 mode they SHALL behave as the recorded zod 3 build.

#### Scenario: Overwrite on a discriminated union option
- **WHEN** `z::discriminated_union("kind", [z::object! { kind: z::literal("a") }.overwrite(|v| v)])` is built and parses
  `{"kind": "a"}`
- **THEN** building does not panic and the parse gives the result recorded for `du.overwrite`

#### Scenario: Check on a template literal part
- **WHEN** a template literal is built with `z::number().check_fn(|_| {})` as a part and parses `"id-5"`
- **THEN** building does not panic and the parse gives the result recorded for `templateLiteral.check`

### Requirement: A date bound is one instant in both versions
`z::date().min(s)` and `.max(s)` SHALL parse `s` once with the JavaScript date parser and use that instant in zod 3
and zod 4 mode, so a bound without a time zone means the same local instant whichever version runs.

#### Scenario: Bound without a time zone
- **WHEN** `z::date().min("2024-01-01T10:00:00")` checks the instant one millisecond before that local time, with
  `ZodVersion::V3` and with `ZodVersion::V4`
- **THEN** both report a `too_small` issue, and both accept the bound's own instant

### Requirement: BigInt coercion does not saturate
`z.coerce.bigint()` SHALL report an issue for a number whose integer value does not fit in `i128`, and MUST NOT return a
saturated value.

#### Scenario: Number beyond i128
- **WHEN** `z::coerce::bigint()` parses `1e39`
- **THEN** the result is an `invalid_type` issue, not `i128::MAX`

### Requirement: Schemas the original rejects fail when built
A schema the original rejects when it is created (`z.literal([])`, a discriminated union option without a
discriminator) SHALL panic when it is built, with the original's message, never at the first parse;
`z::native_enum::<T>()` SHALL panic when built for a type whose variant names cannot be read.

#### Scenario: Empty literal
- **WHEN** `z::literal(json!([]))` is called
- **THEN** it panics with the message recorded for `literal.empty`

#### Scenario: Untagged enum
- **WHEN** `z::native_enum::<T>()` is called for a `#[serde(untagged)]` enum
- **THEN** it panics with a message naming the type

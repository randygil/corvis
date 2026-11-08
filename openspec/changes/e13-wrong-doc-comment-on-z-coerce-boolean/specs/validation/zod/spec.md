# Spec Delta

## ADDED Requirements

### Requirement: z::coerce::boolean follows Boolean(x) and documents it
`z::coerce::boolean()` SHALL convert its input with JavaScript's `Boolean(x)` rule in both zod 3 and zod 4 modes: the
result MUST be `false` for `false`, `""`, `0`, `null` and a missing value, and `true` for every other input, including
the strings `"false"`, `"0"`, `"no"` and `"off"`. Its rustdoc MUST state this rule, MUST NOT claim that any non-empty
string becomes `false`, and MUST point to `z::stringbool()` for parsing text flags.

#### Scenario: The string "false" in zod 3 mode
- **WHEN** `z::coerce::boolean()` parses `"false"` with `ZodVersion::V3`
- **THEN** the result is `true`

#### Scenario: The string "false" in zod 4 mode
- **WHEN** `z::coerce::boolean()` parses `"false"` with `ZodVersion::V4`
- **THEN** the result is `true`

#### Scenario: Falsy values
- **WHEN** `z::coerce::boolean()` parses `""`, `0` or `null`, in either mode
- **THEN** the result is `false`

#### Scenario: The rustdoc matches the behavior
- **WHEN** a user reads the rustdoc of `z::coerce::boolean()`
- **THEN** it says the conversion is `Boolean(x)` in both modes, that `"false"` gives `true`, and refers to
  `z::stringbool()` for text values

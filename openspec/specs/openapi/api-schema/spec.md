# openapi/api-schema Specification

## Purpose
How `#[derive(ApiSchema)]` turns a DTO's fields and validation attributes into OpenAPI schema properties and
constraints (lengths, ranges, patterns, formats), compatible with the original Swagger module's output.

## Requirements

### Requirement: Validation constraints become OpenAPI keywords
For each field of a struct that derives both `ApiSchema` and `Validate`, `#[derive(ApiSchema)]` SHALL add to the
property the OpenAPI keywords that the original framework's OpenAPI compiler plugin (with `classValidatorShim`
enabled, its default) adds for the equivalent class-validator decorator:

| Field attribute | Keywords |
|---|---|
| `#[min(n)]` | `minimum: n` |
| `#[max(n)]` | `maximum: n` |
| `#[min_length(n)]` | `minLength: n` |
| `#[max_length(n)]` | `maxLength: n` |
| `#[length(a)]`, `#[length(a, b)]` | `minLength: a`, and `maxLength: b` when given |
| `#[matches(re)]` | `pattern: re` |
| `#[contains(s)]` | `pattern: s` |
| `#[is_email]` | `format: "email"` |
| `#[is_url]` | `format: "uri"` |
| `#[is_uuid]` | `format: "uuid"` |
| `#[is_hex_color]` | `pattern: "^#?([0-9A-F]{3}\|[0-9A-F]{4}\|[0-9A-F]{6}\|[0-9A-F]{8})$"` |
| `#[is_positive]` | `minimum: 1` |
| `#[is_negative]` | `maximum: -1` |
| `#[array_not_empty]` | `minItems: 1` |
| `#[array_unique]` | `uniqueItems: true` |
| `#[array_min_size(n)]` | `minItems: n` |
| `#[array_max_size(n)]` | `maxItems: n` |
| `#[is_in(v1, v2, ...)]` | `enum: [v1, v2, ...]`, plus `isArray: true` with `each` |

Values MUST be the attribute's argument values serialized as JSON (an integer stays an integer). Other validation
attributes MUST NOT add keywords.

#### Scenario: Length and range
- **WHEN** a DTO field `name: String` has `#[max_length(50)]` and a field `age: u32` has `#[min(0)]` and `#[max(40)]`
- **THEN** the schema has `name: { type: "string", maxLength: 50 }` and `age: { type: "number", minimum: 0, maximum: 40 }`

#### Scenario: Email format
- **WHEN** a DTO field `email: String` has `#[is_email]`
- **THEN** its property has `format: "email"`

#### Scenario: Allowed values
- **WHEN** a DTO field `breed: String` has `#[is_in("persian", "siamese")]`
- **THEN** its property has `enum: ["persian", "siamese"]`

#### Scenario: Rule without a mapping
- **WHEN** a DTO field has only `#[is_int]` and `#[is_not_empty]`
- **THEN** its property has the same keywords as without those attributes

#### Scenario: Constraint on array elements
- **WHEN** a DTO field `tags: Vec<String>` has `#[max_length(10, each)]` and `#[array_max_size(5)]`
- **THEN** the property has `type: "array"`, `maxItems: 5` and `items: { type: "string", maxLength: 10 }`, as the
  original produces for `@MaxLength(10, { each: true }) @ArrayMaxSize(5) tags: string[]`

### Requirement: Derived keywords follow the original's precedence
When several attributes of one field produce the same keyword, the value MUST be the one the original plugin keeps: the
keywords are applied in the order `is_in`; `min`, `max`, `min_length`, `max_length`, `array_not_empty`, `is_positive`,
`is_negative`, `array_unique`, `is_email`, `is_url`, `is_uuid`, `is_hex_color`, `array_min_size`, `array_max_size`,
`contains`; `length`; `matches`, and a later one replaces an earlier one. When the same attribute appears twice on a
field, only the upper one MUST count.

#### Scenario: Positive with a minimum
- **WHEN** a field has `#[min(5)]` and `#[is_positive]`
- **THEN** its property has `minimum: 1`, as in the original

#### Scenario: Pattern from matches wins
- **WHEN** a field has `#[contains("x")]` and `#[matches("^[a-z]+$")]`
- **THEN** its property has `pattern: "^[a-z]+$"`

### Requirement: Explicit property options win
A keyword given in `#[api_property(...)]`, `#[api_property_optional(...)]` or `#[api_response_property(...)]` MUST
replace the derived value of the same keyword, and the derived keywords MUST NOT change `required`, `readOnly` or
`type`.

#### Scenario: Explicit maximum
- **WHEN** a field has `#[max(40)]` and `#[api_property(maximum = 30)]`
- **THEN** its property has `maximum: 30`

### Requirement: Enum constraints list the variants
A field with `#[is_enum(T)]`, or `#[is_enum]` on a field whose type is the enum, SHALL get `enum` with the serialized
variant names of `T` (the names `Validate` accepts), plus `isArray: true` with `each`. An `enum` from `#[is_in]` or
from explicit property options MUST take precedence over it.

#### Scenario: Enum over a string field
- **WHEN** a field `role: String` has `#[is_enum(Role)]` and `Role` has the variants `Admin` and `User` serialized as
  `"admin"` and `"user"`
- **THEN** its property has `enum: ["admin", "user"]`

### Requirement: The derived schema matches the original for the same DTO
For every DTO of the differential fixture, the `components.schemas` entry produced for the Rust struct SHALL equal the
one produced by the original framework for the equivalent TypeScript class compiled with the OpenAPI compiler plugin,
except for differences listed in the fixture with a reason.

#### Scenario: Fixture replay
- **WHEN** the Rust test builds the document for the fixture's DTOs
- **THEN** each schema equals the recorded one as a JSON value

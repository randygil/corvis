# Spec Delta

## ADDED Requirements

### Requirement: Rust scalar types are described
A field or handler parameter of a Rust scalar type SHALL be documented from one table: strings as `string`; numbers
and `NonZero*` as `number`; `bool` as `boolean`; `Vec`, `VecDeque`, sets and arrays as `array`; `DateTime`,
`NaiveDateTime`, `OffsetDateTime`, `SystemTime` as `string` `date-time`; `NaiveDate` as `string` `date`; `NaiveTime`
as `string` `time`; `Uuid` as `string` `uuid`; maps and `Value` as `object`. An explicit or constraint `format` MUST
win.

#### Scenario: A timestamp field
- **WHEN** a DTO deriving `ApiSchema` has `created_at: chrono::DateTime<chrono::Utc>`
- **THEN** the property is `{ type: "string", format: "date-time" }` and no component is created for `DateTime`

#### Scenario: A date parameter
- **WHEN** a handler has `#[query("day")] day: chrono::NaiveDate`
- **THEN** the parameter's schema is `{ type: "string", format: "date" }`

#### Scenario: Field and parameter agree
- **WHEN** the same Rust scalar type is used as a DTO field and as a handler parameter
- **THEN** both are documented with the same type and format

### Requirement: Wrapper types and scalar features
`Box<T>`, `Arc<T>` and `Rc<T>` SHALL be documented as `T`. The `chrono` and `uuid` types MUST be described through
default features of `corvis-swagger` (`chrono`, `uuid`), and the `time` types through its `time` feature; with a
feature off, its types fall back as any undescribed type does.

#### Scenario: Boxed DTO
- **WHEN** a field has type `Box<UserDto>` and `UserDto` derives `ApiSchema`
- **THEN** the property is a reference to the `UserDto` component

#### Scenario: Feature off
- **WHEN** `corvis-swagger` is built with `default-features = false` and a DTO has a `uuid::Uuid` field
- **THEN** the field refers to an empty object component and the undescribed-type warning names `uuid::Uuid`

### Requirement: Unit enums describe their values
`#[derive(ApiSchema)]` on an enum whose variants are all unit variants SHALL make a field or parameter of that type
`type: "string"` with `enum` listing the names serde writes for the variants, following `rename_all` and per-variant
`rename`. An enum with a data-carrying variant, or with `untagged`, `tag` or `content`, MUST be a compile error that
names the variant or attribute. An `enum` given explicitly or by `#[is_in]` MUST take precedence.

#### Scenario: Renamed variants
- **WHEN** `enum Role { Admin, PowerUser }` derives `ApiSchema` and `Serialize` with `#[serde(rename_all = "snake_case")]`
  and a DTO has `role: Role`
- **THEN** the property is `{ type: "string", enum: ["admin", "power_user"] }`

#### Scenario: Data variant
- **WHEN** an enum with a variant `Other(String)` derives `ApiSchema`
- **THEN** compilation fails with an error naming `Other`

### Requirement: Optional fields follow what serde writes
For a struct deriving `ApiSchema`, a field of type `Option<T>` without `skip_serializing_if` SHALL be not required and
nullable; with `skip_serializing_if` it SHALL be not required and not nullable. A field with `#[serde(default)]`, a
struct with `#[serde(default)]`, or a non-`Option` field with `skip_serializing_if` SHALL be not required. The name
written by `rename(serialize = ..)` or `rename_all(serialize = ..)` SHALL be the documented name.

#### Scenario: Plain option
- **WHEN** a DTO has `nick: Option<String>` with no serde attribute, in a document that declares `3.0.0`
- **THEN** `nick` is not in `required` and its property is `{ type: "string", nullable: true }`

#### Scenario: Option skipped when empty
- **WHEN** the field has `#[serde(skip_serializing_if = "Option::is_none")]`
- **THEN** `nick` is not in `required` and its property has no nullability keyword

#### Scenario: Defaulted field
- **WHEN** a DTO has `#[serde(default)] page: u32`
- **THEN** `page` is not in `required`

### Requirement: Flattened fields contribute their properties
A field with `#[serde(flatten)]` SHALL contribute the properties of its type, which MUST derive `ApiSchema`, at the
field's position and with their own required flags, instead of a property of its own. A flattened type that does not
implement `ApiSchema` MUST be a compile error that says so.

#### Scenario: Flattened pagination
- **WHEN** `ListDto` has `#[serde(flatten)] page: PageParams` and `PageParams` has the fields `page` and `size`
- **THEN** the `ListDto` schema has the properties `page` and `size` and no property named after the field

## MODIFIED Requirements

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
| `#[is_positive]` | `minimum: 1`; on a float field, an exclusive lower bound of 0 |
| `#[is_negative]` | `maximum: -1`; on a float field, an exclusive upper bound of 0 |
| `#[array_not_empty]` | `minItems: 1` |
| `#[array_unique]` | `uniqueItems: true` |
| `#[array_min_size(n)]` | `minItems: n` |
| `#[array_max_size(n)]` | `maxItems: n` |
| `#[is_in(v1, v2, ...)]` | `enum: [v1, v2, ...]`, plus `isArray: true` with `each` |

Values MUST be the attribute's argument values serialized as JSON (an integer stays an integer). Other validation
attributes MUST NOT add keywords.

An exclusive bound of a float field (typed `f32` / `f64` directly or inside `Option` / `Vec`) MUST be written in the
form of the document's declared version: in 3.0 as `minimum: 0` with `exclusiveMinimum: true` (or the `maximum`
pair), dropped when the field already has a tighter inclusive bound; in 3.1 as `exclusiveMinimum: 0` (or
`exclusiveMaximum: 0`).

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

#### Scenario: Positive float in a 3.0 document
- **WHEN** a DTO field `ratio: f64` has `#[is_positive]` and the document declares `3.0.0`
- **THEN** its property is `{ type: "number", minimum: 0, exclusiveMinimum: true }`

#### Scenario: Positive float in a 3.1 document
- **WHEN** the same field is documented in a document that declares `3.1.0`
- **THEN** its property is `{ type: "number", exclusiveMinimum: 0 }`

#### Scenario: Positive float with a tighter minimum
- **WHEN** a field `ratio: f64` has `#[min(5)]` and `#[is_positive]` in a `3.0.0` document
- **THEN** its property has `minimum: 5` and no `exclusiveMinimum`

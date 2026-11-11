## ADDED Requirements

### Requirement: JSON Schema options are typed
`JsonSchemaOptions` SHALL take its `target`, `io`, `unrepresentable`, `cycles` and `reused` options as enums, each with
`FromStr` accepting the original's option strings and returning an error that names the accepted values for any other
string. For every recorded option, the generated JSON Schema SHALL be the one recorded from zod 4.4.3.

#### Scenario: An option written as text
- **WHEN** `"input".parse::<JsonSchemaIo>()` and `"Input".parse::<JsonSchemaIo>()` run
- **THEN** the first gives `JsonSchemaIo::Input` and the second an error that lists `input` and `output`

#### Scenario: Recorded options
- **WHEN** `zod4_json_schema_fixtures` runs every `jsonSchemas` case with its options parsed from the recorded strings
- **THEN** every generated schema equals the recorded one

### Requirement: String transforms and formats take typed arguments
`ZString::normalize` SHALL take a `NormalizationForm` and `z::hash` SHALL take a `HashAlgorithm` and a `HashEncoding`,
so neither can receive an unknown form or algorithm, and neither SHALL panic or fall back silently.

#### Scenario: Hash format
- **WHEN** `z::hash(HashAlgorithm::Sha256, HashEncoding::Base64Url)` is built
- **THEN** its format is `sha256_base64url` and it accepts what the recorded `sha256_base64url` pattern accepts

### Requirement: to_json_schema reports why a schema cannot be represented
`ZodSchema::to_json_schema()` SHALL return `Result<Value, String>`, with the error of `z::to_json_schema` when the schema
cannot be represented, instead of `null`.

#### Scenario: Unrepresentable schema
- **WHEN** `z::bigint().to_json_schema()` is called
- **THEN** it returns `Err` with the message that BigInt cannot be represented in JSON Schema

### Requirement: The backtracking warning is rate limited
When a `JsRegex` test exhausts the backtracking limit, the warning SHALL be written at most once per minute per pattern,
with the number of warnings left out since the last one and the input's length, never the input itself.

#### Scenario: Many inputs hit the limit
- **WHEN** one pattern exhausts the backtracking limit on 100 inputs within a minute
- **THEN** one warning line is written for that pattern, and each test still answers no match

# Spec Delta

## ADDED Requirements

### Requirement: Attribute options are checked at compile time
Every OpenAPI attribute that takes `key = value` options (route, response, parameter, body, property and schema
attributes, inside `#[derive(ApiSchema)]` too) SHALL accept only the keys of the original library's options for that
attribute plus Corvis's documented additions. An unknown key MUST be a compile error at the key, with a suggestion when
a valid key is within two edits, followed by the valid keys.

#### Scenario: Misspelled response option
- **WHEN** a route has `#[api_response(status = 200, desciption = "ok")]`
- **THEN** compilation fails at `desciption` with ``did you mean `description`?``

#### Scenario: Misspelled property option
- **WHEN** a DTO field has `#[api_property(min_lenght = 3)]`
- **THEN** compilation fails at `min_lenght` with ``did you mean `minLength`?``

### Requirement: The option lists follow the original's declarations
The key list of each attribute SHALL equal the keys of the corresponding option interface of the original library, as
extracted from its type declarations into a committed fixture, plus Corvis's additions, each documented. A test MUST
compare them.

#### Scenario: Fixture comparison
- **WHEN** `cargo test -p corvis-swagger --test attribute_keys` runs
- **THEN** every attribute's key list equals the recorded keys plus its listed additions

### Requirement: Extension keys are checked before run time
`#[api_extension(key, value)]` SHALL require a string literal key that starts with `x-`, checked at compile time, and
`decorators::api_extension` SHALL return `Err(SwaggerError::ExtensionKey)` for any other key instead of panicking.

#### Scenario: Missing prefix in the attribute
- **WHEN** a route has `#[api_extension("rate-limit", 10)]`
- **THEN** compilation fails with an error that the key must start with `x-`

#### Scenario: Missing prefix in the function
- **WHEN** application code calls `decorators::api_extension("rate-limit".into(), json!(10))`
- **THEN** it returns `Err(SwaggerError::ExtensionKey)` and nothing panics

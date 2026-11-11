# Spec Delta

## ADDED Requirements

### Requirement: The public surface of corvis-swagger is its API
`corvis-swagger` SHALL publish as modules only `decorators`, `document`, `scalar` and `swagger_ui`, and re-export at
its root every other public item users need; its implementation modules (`js`, `metadata`, `schema_factory`,
`explorer`, `yaml`, `types`, `zod`) MUST NOT be public. Support code for the macros MUST live under the hidden
`__private` module.

#### Scenario: Internal module
- **WHEN** application code names `corvis_swagger::schema_factory::SchemaObjectFactory`
- **THEN** compilation fails because the module is private

#### Scenario: Public item through the root
- **WHEN** application code implements `corvis_swagger::SwaggerType` for its own type
- **THEN** it compiles

### Requirement: Library errors are typed and do not panic
Fallible functions of `corvis-swagger` SHALL return `SwaggerError`, an extensible enum whose messages use Corvis terms
only. `SwaggerModule::try_create_document` SHALL return the errors `create_document` documents as panics (a duplicate
`$defs` id of a zod schema, a model that is not registered). No other public function MUST panic on input a caller can
pass at run time.

#### Scenario: Duplicate id in a zod schema
- **WHEN** a document is created with `try_create_document` and a zod DTO's output schema holds the same `$defs` id
  twice
- **THEN** it returns `Err(SwaggerError::DuplicateDefsId)` naming the id, and nothing panics

#### Scenario: Error text
- **WHEN** any `SwaggerError` is displayed
- **THEN** the message names Rust items or attributes and no TypeScript decorator or syntax

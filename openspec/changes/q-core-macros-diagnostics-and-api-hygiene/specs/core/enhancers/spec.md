# Spec Delta

## ADDED Requirements

### Requirement: Malformed route, module and injectable attributes are spanned compile errors
`#[http_code]` with other than one argument, `#[redirect]` with other than one or two, `#[set_metadata]` with other than
two, `#[module(global = ...)]` with a value that is not `true` or `false`, and `#[init(...)]` with other than one
expression SHALL each be a compile error that points at the attribute (or its value) and says what it expects. The macro
MUST NOT panic and MUST NOT expand with a default in their place.

#### Scenario: http_code without a status
- **WHEN** a route is annotated with `#[http_code]`
- **THEN** compilation fails with an error on that attribute saying that it expects one argument

#### Scenario: global that is not a literal
- **WHEN** a module is declared with `#[module(global = IS_GLOBAL)]`
- **THEN** compilation fails with an error on `IS_GLOBAL` saying that `global` expects `true` or `false`

#### Scenario: set_metadata with one argument
- **WHEN** a route is annotated with `#[set_metadata("roles")]`
- **THEN** compilation fails with an error on that attribute, not on the whole impl block

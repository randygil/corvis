# Spec Delta

## Purpose

How the built-in pipes (`ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `ParseArrayPipe`, `ParseEnumPipe`,
`DefaultValuePipe`) answer, checked against a fixture recorded from the original framework's pipes. Introduced by
`q-validation-correctness`.

## ADDED Requirements

### Requirement: Built-in pipes follow the recorded original
`ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `ParseArrayPipe` and `DefaultValuePipe` SHALL return, for every case
of `tests/fixtures/builtin_pipes/builtin_pipes.json`, the value, or the status and body, that the original framework's
pipe returns. A value Corvis cannot represent SHALL be refused with the pipe's own error and be marked as a divergence
in the fixture.

#### Scenario: A NaN item
- **WHEN** `ParseArrayPipe::new().numbers()` transforms `"1,NaN"`
- **THEN** the answer equals the recorded one, and it is never an array that contains `null`

#### Scenario: A large item
- **WHEN** `ParseArrayPipe::new().numbers()` transforms `"1e20"`
- **THEN** the answer equals the recorded one, and it never contains `9223372036854775807`

#### Scenario: Replay of the fixture
- **WHEN** `cargo test -p corvis --test builtin_pipes_fixtures` runs
- **THEN** every `pipes` record matches, or is a recorded divergence answered with the pipe's 400

### Requirement: Enum pipes need readable variant names
`ParseEnumPipe::of::<T>()` SHALL panic when it is built for a type whose variant names cannot be read (an untagged or
internally tagged enum, a type that is not an enum), with a message naming the type, instead of building a pipe that
refuses every value.

#### Scenario: Untagged enum
- **WHEN** `ParseEnumPipe::of::<T>()` is called for a `#[serde(untagged)]` enum
- **THEN** it panics with a message that names `T`

#### Scenario: Plain enum
- **WHEN** `ParseEnumPipe::of::<Role>()` is called for an enum of unit variants with `rename_all = "lowercase"`
- **THEN** the pipe accepts `"admin"` and refuses `"root"` with the enum error

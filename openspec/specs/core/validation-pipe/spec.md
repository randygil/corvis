# core/validation-pipe Specification

## Purpose
Options of the built-in `ValidationPipe` either change its behavior or tell the user, at compile time, that they do
not.

## Requirements

### Requirement: Options without effect are deprecated
Every public option of `ValidationPipe` / `ValidationPipeOptions` SHALL either change the pipe's behavior or be marked
`#[deprecated]` with a note explaining why it has no effect. `ValidationPipe::transform(bool)` and the
`ValidationPipeOptions::transform` field MUST be deprecated, with a note saying that values are always converted to
the declared type.

#### Scenario: Using transform produces a warning
- **WHEN** application code calls `ValidationPipe::new().transform(false)`
- **THEN** the build emits a deprecation warning whose note says the conversion to the declared type always happens

#### Scenario: Deprecated option does not change behavior
- **WHEN** a route with `#[query] dto: PageDto` (field `page: u32` with `#[is_int]`) is served with
  `ValidationPipe::new().transform(false)` and with `ValidationPipe::new()`, and both receive `?page=2`
- **THEN** both answer the same way, with `page` equal to `2`

#### Scenario: The framework itself builds without deprecation warnings
- **WHEN** the workspace is built with `cargo build --workspace --all-targets`
- **THEN** no deprecation warning comes from the framework's own use of the `transform` field (its `Default`)

### Requirement: Number, address and length rules follow the recorded original
The rules `validation::rules::is_number_string`, `is_ip`, `min_length` and `max_length`, used by `#[derive(Validate)]`
and `ValidationPipe`, SHALL return, for every `rules` case of `tests/fixtures/builtin_pipes/builtin_pipes.json`, the
result recorded from class-validator 0.14.4.

#### Scenario: Number text that is not a decimal numeral
- **WHEN** `is_number_string` checks `"NaN"`, `"inf"` and `"1e5"`
- **THEN** each result equals the one recorded for class-validator's `isNumberString`

#### Scenario: Length of text with an emoji
- **WHEN** `min_length` with 2 checks `"❤️"` and `"😀"`
- **THEN** each result equals the one recorded for class-validator's `minLength`

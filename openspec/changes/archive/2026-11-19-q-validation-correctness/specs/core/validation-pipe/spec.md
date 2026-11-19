## ADDED Requirements

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

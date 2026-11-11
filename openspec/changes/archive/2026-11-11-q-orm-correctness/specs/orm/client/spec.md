# Spec Delta

## ADDED Requirements

### Requirement: Typed helpers never change a value into null
A value given to a generated helper (`equals`, `not`, `in_vec`, comparisons, `set`, arithmetic, list operations) that
cannot be serialized MUST make the query fail with a validation error before any SQL runs. A non-finite float MUST NOT
become `null`: it SHALL be sent as the original client sends it when that is a value, and be refused with a validation
error otherwise.

#### Scenario: NaN in a filter
- **WHEN** `score::equals(f64::NAN)` or `score::not(f64::NAN)` is used in a `find_many`, `update_many` or `delete_many`
- **THEN** the filter is not `IS NULL` or `IS NOT NULL`, and the rows read or changed are those recorded from the original
  client for the same filter, or the query fails with a validation error and changes nothing

#### Scenario: Value that fails to serialize
- **WHEN** a helper receives a value whose `Serialize` implementation returns an error
- **THEN** the query fails with a validation error naming the type, and no statement is sent

### Requirement: Conversions of caller input do not panic
Text SHALL become a `Decimal` only through `FromStr`, `TryFrom` or `Decimal::parse`, which return an error for invalid
text; no `From` conversion from text SHALL exist. A literal decimal SHALL be written with `decimal!`, checked when the
program is compiled. `RawSql::join` of an empty list MUST NOT panic: running the statement fails with a validation error.

#### Scenario: Invalid text from a request
- **WHEN** `"12,5".parse::<Decimal>()` or `Decimal::try_from("12,5")` runs
- **THEN** it returns an error and nothing panics

#### Scenario: Invalid literal
- **WHEN** a program contains `decimal!("1.2.3")`
- **THEN** it does not compile, and the compiler error names the invalid literal

#### Scenario: Empty join
- **WHEN** `_query_raw` runs a statement built with `RawSql::join` of an empty list
- **THEN** it fails with a validation error carrying the original's message, and no statement is sent

### Requirement: Arithmetic updates follow the recorded original on each dialect
`divide` by zero and an `increment` that leaves the range of an `Int` field SHALL give, on each dialect, the result, or
the error class, code and message, recorded from the original client on that dialect. The documentation SHALL list the
differences between the two dialects that remain.

#### Scenario: Divide by zero
- **WHEN** `update` sets `value: { divide: 0 }` on an `Int`, a `Float` and a `Decimal` field, on Postgres and on SQLite
- **THEN** each outcome equals the recording for that field type and dialect

#### Scenario: Integer overflow
- **WHEN** `update` sets `views: { increment: 1 }` on an `Int` field holding `2147483647`, on each dialect
- **THEN** the outcome equals the recording for that dialect

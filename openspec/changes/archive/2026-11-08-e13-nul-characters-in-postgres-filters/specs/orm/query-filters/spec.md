# Spec Delta

## ADDED Requirements

### Requirement: NUL characters in filters are validation errors
A `where` that contains the NUL character (U+0000) in a string value, a JSON path segment or an object key SHALL fail
with a validation error before any SQL is built, on every dialect. It MUST NOT reach the database, where PostgreSQL
would answer with a raw database error. The check SHALL cover nested `AND`/`OR`/`NOT`, relation filters and the
`where` of `include` arguments.

#### Scenario: String filters on both dialects
- **WHEN** a filter uses `equals`, `not`, `contains`, `startsWith`, `endsWith`, `in`, `notIn` or a comparison with a
  string that contains NUL
- **THEN** the query fails with a validation error on SQLite and on PostgreSQL, and no row is read or changed

#### Scenario: JSON filters
- **WHEN** a JSON filter has a NUL in a `path` segment (list or string form), in a string compared with `equals`,
  `string_contains` or `array_contains`, or in a key or string of a compared document
- **THEN** the query fails with a validation error on both dialects

#### Scenario: Other control characters are values
- **WHEN** a filter holds `\u0001`, ` ` or non-ASCII text
- **THEN** it is compiled and matched as an ordinary value

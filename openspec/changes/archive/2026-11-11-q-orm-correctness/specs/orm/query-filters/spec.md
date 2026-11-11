# Spec Delta

## ADDED Requirements

### Requirement: Decimal comparisons are numeric on SQLite
On SQLite, a filter, `orderBy`, cursor, `_min`, `_max`, `_sum`, `_avg` or `having` on a `Decimal` field SHALL compare and
aggregate the values as numbers, whether the column was created as `DECIMAL` or as `TEXT`, with the results recorded from
the original client on SQLite. The SQL generated for Postgres MUST NOT change.

#### Scenario: Greater-or-equal on a pushed table
- **WHEN** a SQLite table created by `db push` holds the amounts `99.5`, `100`, `1000` and `-5`, and a `findMany` filters
  `amount: { gte: 100 }`
- **THEN** it returns exactly the rows `100` and `1000`

#### Scenario: Order and maximum
- **WHEN** the same rows are sorted by `amount` ascending and aggregated with `_max: { amount: true }`
- **THEN** the order is `-5`, `99.5`, `100`, `1000` and the maximum is `"1000"`

#### Scenario: Postgres SQL unchanged
- **WHEN** the Postgres SQL of every oracle case is generated after the change
- **THEN** it equals, text and parameter types, the SQL generated before the change

### Requirement: Engine-built key lists stay within the parameter limit
Statements the engine builds from a list of keys (the read-back of `createManyAndReturn` and `updateManyAndReturn`,
includes and relation counts, nested `set`, `disconnect` and `delete` lists, `in` and `notIn` with `mode: insensitive`)
SHALL succeed whatever the number of keys: a single-field list as one parameter, compound keys split into statements
within the dialect's parameter limit.

#### Scenario: Create many and return on SQLite
- **WHEN** `createManyAndReturn` creates 40,000 rows on SQLite
- **THEN** it returns the 40,000 rows in input order

#### Scenario: Include over a compound key
- **WHEN** a `findMany` of 20,000 parents includes a relation whose key has two fields
- **THEN** every parent carries its own children, as with a small number of parents

#### Scenario: A chunked update keeps its filter
- **WHEN** an update by a list of compound keys is split into several statements
- **THEN** each statement repeats the caller's filter, and the returned count is the sum of the rows each one changed

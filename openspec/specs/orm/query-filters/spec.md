# orm/query-filters Specification

## Purpose
Guarantees that corvis-orm query filters (`where` arguments) are never interpreted as SQL: any filter a client can
influence becomes parameters, a validation error or a result, never an injected statement or a crash.

## Requirements

### Requirement: Filters never produce SQL syntax errors
For any `where` argument built from field names, operators and values (including quotes, comment markers, statement
separators and unknown keys), a query SHALL return results, a validation or known-request error, or the value-out-of-range error the original
client also raises for integers that do not fit the column. It MUST NOT fail with a database syntax error or crash.

#### Scenario: Injection-shaped values
- **WHEN** a `findMany` runs with filters whose values contain `'`, `"`, `;`, `--`, `/*` or `DROP TABLE`
- **THEN** the query returns results or an error that is not a database syntax error

#### Scenario: Unknown fields and operators
- **WHEN** a filter names a field or operator the model does not have
- **THEN** the query fails with a validation error

### Requirement: String equality filters match exactly
A string equality filter on a column SHALL return exactly the rows whose column value equals the given string, whatever
characters the string contains.

#### Scenario: Hostile string in equals
- **WHEN** a `findMany` filters `email` with `equals` set to a generated string
- **THEN** every returned row has exactly that email, and rows with other emails are not returned

### Requirement: Read queries do not change data
Read queries with any filter MUST NOT change the stored rows.

#### Scenario: Table intact after hostile reads
- **WHEN** many read queries with generated filters have run against a seeded table
- **THEN** the table still holds the same rows with the same values

### Requirement: JSON paths are bound parameters
The `path` of a JSON filter SHALL be sent to the database as a bound parameter on every dialect. No path segment MUST
appear in the generated SQL text, whatever characters it contains, and the filter SHALL match exactly the rows whose
JSON holds the value at the path made of those literal segments.

#### Scenario: Hostile path segments on Postgres
- **WHEN** a `findMany` filters a JSON column with a `path` whose segments contain `'`, `"`, `\`, `{`, `}`, `,`,
  non-ASCII characters or statement-shaped text
- **THEN** the generated SQL does not contain the segments, and the query returns exactly the rows that have a key
  equal to each segment

#### Scenario: Path size is bounded
- **WHEN** a JSON filter has a `path` of more than 128 segments, in list or string form
- **THEN** the query fails with a validation error before any SQL is built, and a path of 128 segments is accepted

#### Scenario: Segments are literal keys
- **WHEN** a path segment looks like an array literal (`{a,b}`) or a JSONPath (`$.a`)
- **THEN** it is matched as the single key with that text, not parsed

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

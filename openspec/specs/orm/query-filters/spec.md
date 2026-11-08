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

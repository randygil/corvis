# Spec Delta

## ADDED Requirements

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

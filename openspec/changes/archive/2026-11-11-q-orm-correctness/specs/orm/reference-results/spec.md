# Spec Delta

## Purpose

How corvis-orm's results are checked against results recorded from the original client, on Postgres and on SQLite, and
how those recordings are made. Introduced by the ORM quality change `q-orm-correctness` (epic
[#187](https://github.com/randygil/corvis/issues/187)).

## ADDED Requirements

### Requirement: Postgres results equal the recorded reference
`tests/pg_oracle.rs` SHALL run every case of `tests/fixtures/oracle/results.json`, in order, through corvis-orm on a fresh
Postgres database built from the recorded `migration.sql`. Every result MUST equal the recorded one as JSON text, key
order included, and every error MUST equal its recorded class, code, meta and core message (the class only, for
validation errors).

#### Scenario: Full run
- **WHEN** `cargo test -p corvis-orm --test pg_oracle` runs with Postgres available
- **THEN** every recorded case gives the recorded result or error, and the test fails naming each case that differs

#### Scenario: Cases appended to the recording
- **WHEN** new cases are appended to the Postgres recorder and the fixture is regenerated
- **THEN** the regenerated `results.json` differs from the previous one only by the appended cases

### Requirement: SQLite results equal the recorded reference
`tests/sqlite_oracle.rs` SHALL run every case of `tests/fixtures/oracle_sqlite/results.json` twice: on a SQLite database
built from the recorded `migration.sql`, and on one built by `db_push` of the models generated from the recorded schema.
Both runs MUST give the recorded results and errors, compared as for Postgres, except the cases listed in the runner's
table of known differences.

#### Scenario: Database built from the original's migration
- **WHEN** the runner applies the recorded `migration.sql` to a new database file and runs the cases
- **THEN** every case equals the recording

#### Scenario: Database built by db push
- **WHEN** the runner builds the database with `db_push` of the generated models and runs the cases
- **THEN** every case equals the recording

#### Scenario: A known difference
- **WHEN** a case is listed in the table of known differences with its database and reason, and it no longer differs
- **THEN** the test fails and asks for the entry to be removed

### Requirement: Reference results are recorded from the original client
Every fixture compared with the original SHALL be produced by a recorder script run against the original client, and
MUST name the client version and the script, or be written by a documented regeneration script that does. Fixtures MUST
NOT be edited by hand. A migration fixture SHALL be the SQL produced by the original's own tooling for the recorded schema.

#### Scenario: Regenerating the SQLite reference
- **WHEN** the SQLite recorder's regeneration script runs against the original client
- **THEN** it writes the schema, the migration SQL produced by the original's tooling and the results, and the results
  file names the client version and the recorder script

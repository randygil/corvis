# Proposal

Implements [randygil/corvis#109](https://github.com/randygil/corvis/issues/109) (epic
[#1](https://github.com/randygil/corvis/issues/1), phase 0.1 of the plan in #111).

## Why

The `orm/query-filters` spec says filters are never interpreted as SQL. On SQLite the JSON `path` of a filter is a
bound parameter, but on Postgres `json_at` in `crates/corvis-orm/src/engine/filter.rs` writes it into the SQL text as an
escaped string literal (`col #> '{"a''b"}'`). The escaping is correct today, with `standard_conforming_strings`
on and off, so no injection is known, but the guarantee then depends on hand-written escaping instead of on the
database driver, and Postgres is not covered by the filter fuzzing. The security review of #2 reported it.

## What Changes

- Postgres JSON `path` filters bind the path as one `text[]` parameter (`col #> $1::text[]`, `#>>` for text
  comparisons); no segment is ever part of the SQL text.
- Regression tests against a real Postgres: paths with quotes, backslashes, braces, commas, double quotes, unicode and
  statement-shaped text; the generated SQL is asserted not to contain the path.
- A Postgres variant of the JSON-path differential property (generated hostile paths against a Rust evaluation),
  skipped when no server is reachable.
- Cap JSON filter paths at 128 segments (validation error) and bind the path once per filter (review follow-up).
- Runtime code changes (`engine/filter.rs`): the Postgres SQL text of JSON path filters changes; results do not.

Out of scope:
- SQLite JSON paths (fixed and tested in #2; unchanged here).
- MySQL: corvis-orm has no MySQL dialect (`Dialect` is `Sqlite | Postgres`), so there is nothing to change.
- Postgres variants of the other filter properties (`filters_never_break_the_query`, `string_equals_matches_exactly`):
  the rest of the SQL builder is shared with SQLite and already binds every value.
- CI wiring of the Postgres server (#6).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `orm/query-filters`: adds the requirement that JSON paths are bound parameters.

## Impact

- `crates/corvis-orm/src/engine/filter.rs` (runtime): `json_at`.
- `crates/corvis-orm/tests/pg_json_paths.rs` (new), `crates/corvis-orm/proptest-regressions/` (seeds, if any).
- No public API, dependency or documentation change.

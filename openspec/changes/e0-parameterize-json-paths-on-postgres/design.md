# Design

## Context

`Engine::json_at` builds the expression for "the JSON value at `path`" for every JSON filter (`equals`, `not`,
`string_*`, `array_*`, comparisons). On SQLite it already pushes `json_extract(col, ?)` with the path as a bound text
(see `sqlite_json_path`). On Postgres it did this:

```rust
let lit = format!("{{{}}}", p.iter().map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))).collect::<Vec<_>>().join(","));
q.push(&format!("'{}'", lit.replace('\'', "''")));
```

so the filter for `path: ["a'b"]` compiled to `("accounts"."data"::jsonb #> '{"a''b"}') = $1::jsonb`.

## Decisions

1. **One `text[]` parameter.** The path becomes `SqlValue::Cast(Array(segments as Text), "text[]")`, which `Sql::bind`
   renders as `$n::text[]` and the Postgres binder sends as a `text[]` array. The result is
   `("accounts"."data"::jsonb #> $1::text[]) = $2::jsonb`. `#>` and `#>>` accept a `text[]` right operand, and the
   explicit cast keeps the operator unambiguous. Alternative considered: `jsonb_extract_path(col, VARIADIC $1)`;
   rejected because it changes the expression shape for no gain.
2. **Same results.** An empty path is an empty array (`#> '{}'` before): the whole value. A segment is a key, or an
   array index when it is a number and the value is an array, exactly as before. Segments with `NUL` cannot occur in
   `jsonb` and still fail at the database as before.
3. **Prepared statement cache.** `pg_type_signature` tags statements with the type of each parameter; the array is
   tagged `text[]`, so a statement with an empty path array and one with segments share an entry, with the same
   parameter type.
4. **MySQL.** corvis-orm has no MySQL dialect (`db::Dialect` is `Sqlite | Postgres`; the word only appears in comments
   about the PSL provider and the `?` placeholder style), so there is no MySQL branch to fix.
5. **Tests.** The SQL text is asserted with `corvis_orm::debug_sql` on a client that is never connected, so that
   check runs everywhere. Behavior is checked on a real Postgres (the `CORVIS_ORM_PG` server of the other Postgres
   tests, databases `orm_test_json_paths_*`, skipped when unreachable): every hostile key finds exactly its own row,
   keys are not parsed as array literals or JSONPath, and a statement-shaped key leaves the table intact. A property
   compares generated paths and `equals`/`not` filters (optionally negated) against a Rust evaluation of `#>` on the
   same documents, 256 cases by default and `PROPTEST_CASES` to run longer, failing seeds in
   `proptest-regressions/pg_json_paths.txt`.

## Risks / Trade-offs

- The SQL text of Postgres JSON path filters changes (a parameter instead of a literal). Anything that compared SQL
  text or parameter positions would see it; nothing in the repository does. The three ports are checked with the
  parity suite, since runtime code changed.

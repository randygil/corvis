# Design

## Context

`Engine::where_sql` compiles a `where` into SQL with every value bound as a parameter. A string holding U+0000 is a
valid Rust `String` and valid JSON (`"\u0000"`), so it reaches the driver. PostgreSQL rejects it in `text` (`22021`)
and in `jsonb` (`22P05`); SQLite accepts it as a value but a JSON path built from it is cut short at the NUL and
fails with `bad JSON path`. Since #109 binds the path as `$n::text[]`, the Postgres failure surfaces for paths too.
The proposal has the measured behavior of each case.

## Decisions

1. **Validate, do not map the error.** The original returns the raw database error (`PrismaClientUnknownRequestError`,
   no `P` code), so there is no known request error code to match. Mapping `22021`/`22P05` to a code would invent one
   and still reach the database; a validation error before the query is the safer behavior and the same on both
   dialects. This diverges from the original's error kind (a validation error, HTTP 400 through `OrmExceptionFilter`,
   instead of an unknown request error, HTTP 500) and is documented in `docs/data/corvis-orm.md`.
2. **One scan per `where`, at the entry.** `where_sql` becomes a wrapper that runs `find_nul` over the whole value
   (string values, array items and object keys, so JSON paths, JSON documents, `in` lists, nested `AND`/`OR`/`NOT`
   and relation filters are all covered) and then calls the recursive `where_node`. Recursive calls (nested logical
   operators, `exists`) use `where_node`, so the scan runs once and not once per level. The cost is linear in the size
   of the filter, which the depth limit already bounds. Alternative considered: checking at each leaf that binds a
   string; rejected because there are many leaves (scalar, list, JSON, `in`, relation) and a missed one would reach the
   database again.
3. **Both dialects.** SQLite filters with NUL were harmless (no match) except in paths, but accepting a filter on one
   dialect and rejecting it on the other would make an app behave differently by database; the check does not depend
   on the dialect.
4. **Keys count.** A NUL in an object key of a JSON document (`equals: { "a\u0000": 1 }`) fails on Postgres with
   `22P05`, so keys are checked like values. A NUL in a field name is already an unknown-argument validation error.
5. **Message.** "Argument \`where\`: Invalid value provided. A string contains the NUL character (\u0000), which the
   database cannot store or compare." ("A key" for object keys). It does not echo the value.
6. **Fuzz.** `ALLOWED_CODES` is unchanged. `filters_never_break_the_query` now fails when a filter with NUL is accepted
   or fails with anything but a validation error; `string_equals_matches_exactly` expects a validation error for
   strings with NUL. The failing seed found with the previous code (`AND: { data: { lt: "'", path: "\0" } }`,
   `findMany`) is in `proptest-regressions/fuzz_filters.txt`.
7. **Original check.** Run with the `@prisma/client` found in the port originals against a scratch PostgreSQL
   database: `findMany` with `where` on a `String` and a `Json` column, and a `create`; all fail the same raw way.

## Risks

- A caller that filters by a value that really contains NUL on SQLite (where `create` can store it) now gets a
  validation error instead of an empty or matching result. Such data cannot exist on PostgreSQL, and the original has
  no portable behavior for it.

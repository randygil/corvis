# Proposal

Implements [randygil/corvis#141](https://github.com/randygil/corvis/issues/141) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The property test `filters_never_break_the_query` found a JSON filter `path` containing a NUL character (U+0000) that
ends in a raw database error instead of a validation error or a result, so CI can fail at random depending on the
generated cases. The same character in any string of a `where` fails on PostgreSQL, which cannot hold NUL in `text`
or `jsonb`. Measured before the change (`UnknownRequestError` for every failure, none a validation error), with
`a\u0000b`:

| Filter                                                                           | SQLite                     | PostgreSQL                    |
| -------------------------------------------------------------------------------- | -------------------------- | ----------------------------- |
| JSON `path` segment (list or string form)                                        | `bad JSON path` (code `1`) | `22021` invalid byte sequence |
| `equals`, `not`, `contains`, `startsWith`, `endsWith`, `in`, `notIn` on `String` | empty result, no error     | `22021` invalid byte sequence |
| JSON `equals` with a string or an object holding it                              | empty result, no error     | `22P05` unsupported escape    |
| JSON `string_contains`                                                           | empty result, no error     | `22021` invalid byte sequence |

The original ORM (`@prisma/client` 6.19.3, PostgreSQL) was run with the same cases: every one fails with
`PrismaClientUnknownRequestError` carrying the same raw `22021` / `22P05` database error, with no request error code.
Matching it would mean returning a raw database error, so this change prefers a validation error (security over
parity) and documents the divergence.

## What Changes

- A `where` that contains a NUL character in any string value, JSON path segment or object key is rejected with a
  validation error before any SQL is built, on SQLite and PostgreSQL alike. This covers nested `AND`/`OR`/`NOT`,
  relation filters (`some`/`none`/`every`/`is`/`isNot`) and the `where` of `include` arguments.
- Regression tests: every filter shape above on both dialects (Postgres against a real server), the SQL-compilation
  path without a server, relation and `include` filters, the failing proptest seed, and the fuzz properties now assert
  that NUL is a validation error.

Out of scope:
- Writes (`create`/`update` data with NUL): they fail on PostgreSQL with the database error, as in the original;
  `where` filters are the subject of the issue.
- Raw SQL (`query_raw`/`execute_raw`) parameters.
- Widening `ALLOWED_CODES` of the fuzz test (not needed: the error is a validation error).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `orm/query-filters`: adds the requirement that a NUL character in a filter is a validation error.

## Impact

- `crates/corvis-orm/src/engine/filter.rs` (runtime): `where_sql` checks the whole `where` once.
- `crates/corvis-orm/tests/` (`nul_characters.rs` new, `client.rs`, `fuzz_filters.rs`),
  `crates/corvis-orm/proptest-regressions/fuzz_filters.txt` (seed).
- `docs/data/corvis-orm.md` and `docs/es/data/corvis-orm.md`: one line in the Security section.
- Runtime code changes: the corvis-parity check is needed (the ports do not send NUL, so no difference is expected).

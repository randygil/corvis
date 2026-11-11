# Proposal

Implements the issue filed from the engineering-quality review of `crates/corvis-orm` and `crates/corvis-orm-macros`
(epic [#187](https://github.com/randygil/corvis/issues/187), code quality audit), first of three changes in dependency
order: this one (data correctness and panics), then `q-orm-hot-path`, then `q-orm-api-hygiene`. Source: a read-only
review of `dev` at `d46758a`; every finding was re-read against the code before it was kept (design.md "The findings,
confirmed against the code").

## Why

The ORM is a port whose promise is verified parity with the original client: same arguments, same results, same errors.
The review found places where the results are wrong without any error:

- On SQLite, a `Decimal` column created by `db push` is `TEXT` and every comparison, `orderBy`, `_min` and `_max` on it is
  a text comparison: `amount >= 100` matches `99.5`, and `_max` of `99.5, 100, 1000` is `99.5`.
- `db push` builds a schema that differs from the original's migration for the same models: a compound primary key
  becomes a key on its first column, implicit many-to-many join tables are not created (`P2021` on the first nested
  write), Postgres enums, scalar lists and `@db.*` native types become `TEXT`/`INTEGER` (queries that cast to the enum or
  `uuid` type then fail), `@@unique`, `@@index`, `@@schema` and `map:` names are ignored. It also runs statement by
  statement, so a failure leaves half a schema.
- The typed helpers serialize their argument with `unwrap_or(Value::Null)`: `score::equals(f64::NAN)` becomes
  `score IS NULL` and `score::not(f64::NAN)` becomes `score IS NOT NULL`, on reads, updates and deletes.
- `From<&str> for Decimal` panics on invalid text, and every typed helper takes `impl Into<Decimal>`, so a string from a
  request can crash the handler task; `RawSql::join` panics on an empty list.
- Lists of keys built by the engine (the read-back of `createManyAndReturn` / `updateManyAndReturn`, includes and counts
  over compound keys, nested `set`/`disconnect`, `in` with `mode: insensitive`) bind one parameter per key and fail past
  the database's parameter limit.
- `divide` by zero and `Int` overflow in `increment` behave differently on the two dialects, without a test or a
  document saying which behavior is the original's.

## What Changes

- **SQLite reference results.** A second differential oracle: the original client recorded on SQLite (schema, the
  original's own migration SQL and results), run against corvis-orm on a database built from that migration and on one
  built by `db push`. New Postgres oracle cases are appended for non-finite floats, `divide` by zero and overflow.
- **Decimal on SQLite follows the recorded original.** `db push` declares `DECIMAL` (the reference migration's type) and
  every `Decimal` column that is compared, sorted, aggregated or used in arithmetic on SQLite is read through
  `CAST(.. AS NUMERIC)`, so columns created as `TEXT` by earlier versions compare numerically too. Postgres SQL is
  unchanged, byte for byte.
- **`db push` builds the reference schema, or refuses.** Compound keys, join tables, `@@unique`, `@@index`, enum types,
  scalar lists, native types, defaults, `@@schema` and `map:` names, compared catalog by catalog with the original's
  migration; constructs it cannot express fail before any statement; the whole push runs in one transaction.
- **Typed helpers never turn a value into `null`.** Serialization errors travel with the parameter and fail the query with
  a validation error; non-finite floats are sent the way the recorded original sends them, or refused.
- **No panics on caller input.** `From<&str>`, `From<String>` and `From<&String>` for `Decimal` are removed (breaking, now,
  before 1.0) in favour of `FromStr`/`TryFrom` and a compile-time checked `decimal!("1.50")` literal; `RawSql::join` of an
  empty list makes the query fail with a validation error.
- **Key lists within the parameter limit.** One helper writes every engine-built key list: single-column keys through the
  existing one-parameter list, compound keys in chunks sized from the dialect's limit.
- **Dialect differences settled.** `divide` by zero and `Int` overflow follow the recorded original on each dialect; the
  remaining differences (case folding, JSON path segments, `search`, decimal precision) are documented in a table.

## Capabilities

### New Capabilities

- `orm/reference-results`: how corvis-orm results are checked against results recorded from the original client on
  Postgres and SQLite, and how those results are recorded.
- `orm/db-push`: what `db push` creates, what it refuses, and that it is atomic.

### Modified Capabilities

- `orm/query-filters`: added requirements for numeric `Decimal` comparison on SQLite and for key lists within the
  parameter limit.
- `orm/client`: added requirements for typed helpers that never change a value into `null`, conversions that do not
  panic, and arithmetic updates that follow the recorded original on each dialect.

## Impact

- Edited: `crates/corvis-orm/src/{schema,query,decimal,raw,client,lib}.rs`,
  `crates/corvis-orm/src/engine/{mod,filter,read,write,aggregate}.rs`, `crates/corvis-orm-macros/src/lib.rs`,
  `crates/corvis-orm/README.md`, `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`, `scripts/check-naming.mjs`
  (one allowlist entry for the SQLite oracle runner, like `pg_oracle.rs`).
- New: `crates/corvis-orm/tests/{sqlite_oracle,sqlite_decimal,db_push,typed_params,parameter_limits}.rs`,
  `crates/corvis-orm/tests/fixtures/oracle_sqlite/{schema.prisma,migration.sql,results.json}`,
  `crates/corvis-orm/tests/generated/oracle_sqlite.rs`, a Postgres SQL snapshot fixture, appended cases in
  `tests/fixtures/oracle/results.json`. Recorder scripts live outside the repository, next to the existing ones
  (`corvis-tests/tmp/orm/oracle/`).
- Public API (breaking, stated plainly): `Decimal` loses `From<&str>`, `From<String>`, `From<&String>` and gains
  `TryFrom<&str>`, `TryFrom<String>` and the `decimal!` macro; `corvis_orm::to_json` returns `Result`; `WhereParam` and
  `SetParam` carry an optional error (their constructors keep their signatures). Downstream code that passes `&str` to a
  `Decimal` helper stops compiling; the port apps have at least 31 explicit call sites (29 literals, one test helper, and
  one that converts a JSON string at run time and would panic on bad data), updated by task 7.3.
- Behavior visible to applications: listed in design.md "Behavior visible to applications"; the large ones are numeric
  `Decimal` comparison on SQLite, the new schema `db push` creates, and `db push` refusing what it cannot build.
- No new dependency. The parity check is run (framework code changes; the three apps run on Postgres, whose SQL must not
  change).

## Out of scope

- Exact (non-floating) decimal arithmetic on SQLite: the original does not have it (design.md Decision 2); a follow-up only
  if the maintainer wants a Corvis extension.
- `db push` altering existing tables (it creates what is missing, as documented); a diffing push is a separate feature.
- Performance of `createMany`, includes and the connection path (`q-orm-hot-path`); error types, migrations locking and the
  public-surface cleanup (`q-orm-api-hygiene`).
- Renaming the `_transaction` / `_query_raw` family (the port's names for the original's `$` methods): a project-wide
  naming decision, not a quality finding.

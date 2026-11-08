# Proposal

Implements [randygil/corvis#137](https://github.com/randygil/corvis/issues/137) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The original client reads the Postgres schema from the database URL: with `postgresql://.../db?schema=tenant_a`,
queries, `migrate deploy` and the `_prisma_migrations` table all live in `tenant_a`, and a missing schema is created by
the migration command. Teams use it to share one database between environments or tenants, through `DATABASE_URL`
alone.

corvis-orm ignores it. `OrmClient::with_options` (`crates/corvis-orm/src/client.rs:144-147`) stores the URL as given,
and the Postgres pool connects with that URL unchanged (`client.rs:234-240`). sqlx logs
`ignoring unrecognized connect parameter` for `schema` and connects with the server's default `search_path`
(normally `"$user", public`). The consequences:

- `?schema=other`: tables that exist only in `other` are not found (`P2021`), or, worse, same-named tables in `public`
  are read and written instead.
- `migrate deploy` with `?schema=other` creates `_prisma_migrations` and applies every migration in `public`
  (`crates/corvis-orm/src/migrate.rs:179-180`, `:239`).
- `sqlite://dev.db?schema=main`: sqlx rejects the URL (`unknown query parameter`), so the client cannot connect.

Two of the three port apps (consi and facten) pass `?schema=public`; they only work because it matches the default.
houseplan passes no `schema` parameter (`houseplan-api/scripts/env.sh:6`).

## What Changes

- corvis-orm reads the `schema` query parameter of the database URL (percent-decoded) and removes it before the URL
  reaches the driver, at the same single parsing point in `OrmClient::with_options` that
  `e13-connection-limit-and-pool-timeout-in-the-database-url` (#133) introduces for the pool parameters.
- Postgres: every pooled connection runs `SET search_path TO "<schema>"` (identifier quoted, embedded `"` doubled)
  right after connecting, before any other statement. Generated queries, raw queries, interactive transactions,
  `db push` and migrations therefore resolve unqualified names in that schema. Models and enums with an explicit
  `@@schema` keep their qualified names.
- `migrate deploy`, `migrate status` and `db push` create the schema when it does not exist (`CREATE SCHEMA`, only
  after a catalog lookup shows it is missing), so `_prisma_migrations` and the migration SQL apply inside it.
- Without the parameter nothing changes: no `SET`, the server's `search_path` applies, as today.
- An invalid value (empty, containing a NUL character, or longer than 63 bytes) fails when the client is created with
  an initialization error (`P1013`) that names the parameter and never echoes the URL.
- SQLite: the parameter is removed and ignored, so `sqlite://dev.db?schema=main` connects (today it fails).
- Docs (EN + ES): the corvis-orm page documents `schema` in the URL, the exact `search_path` it sets, and its effect on
  migrations.

## Capabilities

### New Capabilities

- `orm/database-schema`: the `schema` parameter of the database URL, its effect on queries and migrations, schema
  creation, validation and identifier quoting.

### Modified Capabilities

None (no main spec covers the ORM connection yet; `orm/connection-pool` is still a pending change, see design).

## Impact

- `crates/corvis-orm/src/client.rs`: the URL parameter helper (shared with #133) gains `schema`; `ClientInner` stores
  the parsed schema; `OrmClient::pool` installs an `after_connect` hook on the Postgres pool; `db_push_models` ensures
  the schema; `OrmClientOptions::url` doc comment.
- `crates/corvis-orm/src/migrate.rs`: `migration_rows` ensures the schema before creating `_prisma_migrations`;
  `ensure_database` uses the URL without the parameter.
- Tests: unit tests for parsing and quoting, a SQLite integration test, a Postgres integration test with two schemas
  holding same-named tables (skipped without a server), and a differential fixture recorded with the original 6.x
  client.
- Docs: `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`.
- Port apps: consi and facten now run `SET search_path TO "public"` on each new connection. Their database user is
  `postgres`, which has no schema of its own, so the effective path was already `public`; `public` exists, so
  migrations issue no `CREATE SCHEMA`. houseplan is unaffected. The parity check confirms 0 differences.

## Out of scope

- Qualifying every generated table name with the URL schema (as the original's generated SQL does): search_path gives
  the same resolution without changing every query text; possible follow-up if a case needs it.
- Defaulting to `public` when `schema` is absent: kept as today (server `search_path`); see the open question in
  design.md.
- `db push` ignores `@@schema` on models (`crates/corvis-orm/src/schema.rs:139-153` and `:156-168` use only
  `quote_ident(m.table)`): pre-existing, unrelated to the URL; possible new issue.
- An `OrmClientOptions::schema(..)` builder (the 7.x line's adapter `schema` option is code, not URL); possible new
  issue.
- `pgbouncer=true` (transaction pooling drops session `SET`s) and the other URL parameters listed as out of scope in
  #133.
- `migrate status` creating `_prisma_migrations` (and now the schema) on a read-only command: pre-existing behavior of
  `migration_rows` (`migrate.rs:179-180`).
- The pool parameters `connection_limit` / `pool_timeout`: #133.

# Design

## Context

See proposal.md for motivation; the behavior is in the `orm/database-schema` spec delta. Current state, verified in
the code (corvis-orm at this branch, sqlx 0.8.6 from `Cargo.lock`):

**corvis-orm: connection**

- `OrmClient::with_options` (`crates/corvis-orm/src/client.rs:144-147`) only validates the protocol through
  `dialect_of` (`:120-132`) and stores `OrmClientOptions` unchanged in `ClientInner` (`:92-96`).
- `OrmClient::pool` (`client.rs:197-244`) creates the pool lazily. Postgres (`:234-240`):
  `PgPoolOptions::new().max_connections(..)`, optional `acquire_timeout`, then `connect(url)` with the URL as given; no
  hook runs on new connections. SQLite (`:208-233`): `file:` URLs drop their query string except `mode=memory`
  (`:209-215`); other URLs go to `SqliteConnectOptions::from_str` (`:217`).
- Every statement goes through that pool or through an interactive transaction taken from it: generated queries,
  `query_raw` / `execute_raw`, and scripts (`execute_script_raw`, `client.rs:328-345`).
- `init_error` / `server_of` (`client.rs:593-615`) print only `host:port`, never the query string.
- Identifier quoting already exists: `quote_ident` doubles embedded `"` (`crates/corvis-orm/src/db.rs:92-94`);
  `qualified` (`db.rs:97-102`) prefixes `@@schema` models, and `enum_type_sql`
  (`crates/corvis-orm/src/engine/mod.rs:210-216`) does the same for enums. `ModelMeta::schema` / `EnumMeta::schema`
  hold `@@schema` (`crates/corvis-orm/src/meta.rs:166-167`, `:290`, `:436`). Unqualified names resolve through the
  connection's `search_path`.
- Nothing in `crates/corvis-orm/src` reads `schema` from the URL or sets `search_path` (searched for `search_path`,
  `current_schema`, `schema=`).

**corvis-orm: migrations and db push**

- `migrate_deploy` (`crates/corvis-orm/src/migrate.rs:206-272`) and `migrate_status` (`:275-306`) call
  `ensure_database` (`:152-177`), which parses `options().url` with `PgConnectOptions::from_str` (`:158-159`) only to
  create a missing database, then `migration_rows` (`:179-199`), which runs `CREATE TABLE IF NOT EXISTS
  "_prisma_migrations"` unqualified (`:106-118`, `:180`). Migration SQL runs through `execute_script_raw` (`:239`) on
  the client's pool. Everything therefore lands in whatever `search_path` the pooled connection has.
- `db_push_models` (`client.rs:566-574`) runs `push_statements` (`crates/corvis-orm/src/schema.rs:139-153`), also
  unqualified.
- The CLI (`crates/corvis-orm/src/bin/corvis-orm.rs:147-180`) builds an `OrmClient::new(url)` from `--url` or
  `DATABASE_URL`, so it inherits whatever the client does.

**sqlx 0.8.6**

- Postgres URL parsing logs `ignoring unrecognized connect parameter` for unknown keys such as `schema`
  (`sqlx-postgres-0.8.6/src/options/parse.rs:50-108`); SQLite parsing rejects them with `unknown query parameter`
  (`sqlx-sqlite-0.8.6/src/options/parse.rs:37-110`).
- `PgConnectOptions::options` (`sqlx-postgres-0.8.6/src/options/mod.rs:426-442`) writes `-c {k}={v}` into the startup
  `options` string with no escaping: a value with a space adds further `-c` settings.
- `PoolOptions::after_connect` (`sqlx-core-0.8.6/src/pool/options.rs:350-380`) runs a callback on every newly opened
  connection before it is handed out; its own doc example sets `search_path` there.

**Original client**

- 6.x line (facten's original uses `@prisma/client` / `prisma` 6.19.3, `corvis-tests/nest/facten-api/package.json:34`,
  `:59`): the engine reads `schema` from the datasource URL. Its query logs show table names qualified with that schema
  (default `public`); whether it also sets the connection's `search_path` (which decides where unqualified raw SQL
  goes), what it does with an empty value, and whether `migrate deploy` creates a missing schema, are recorded by the
  differential fixture (decision 7) rather than assumed.
- 7.x line (consi's original uses 7.9.1, houseplan's 7.10): the runtime client connects through
  `@prisma/adapter-pg`, whose option `schema` is "The name of the schema to use in generated queries"
  (`node_modules/@prisma/adapter-pg/dist/index.d.ts:59-60`, surfaced as `schemaName` at `index.js:755`). Both originals
  build `new PrismaPg({ connectionString })` without it (`consi-backend/src/prisma/prisma.service.ts:18`,
  `houseplan/api/src/db/prisma.service.ts:8`). Migrations still go through the schema engine, which reads the URL from
  `prisma.config.ts` (`houseplan/api/prisma.config.ts`).

**Port apps** (`corvis-tests/corvis/`, paths from the corvis-parity skill)

- consi: `?schema=public` in `.env.example:2`, `docker-compose.yml:32` and the apidiff `.env` files under
  `tmp/consi/*/.env`; facten: `?schema=public` in `.env.example:2`, `scripts/apidiff.sh:56`, `scripts/run-nest.sh:9`
  and `e2e-external/*.sh`. houseplan: no `schema` (`scripts/env.sh:6`, `.env.example`, `docker-compose.yml:24`), so
  the issue's "all three" is two.
- All three run `migrate_deploy` through `OrmClient::new(url)` in `src/bin/migrate.rs`. None of their migration SQL
  qualifies names with `"public".` (searched `*/prisma/migrations`), so it follows the `search_path`. Two migrations
  create extensions unqualified (`facten-api/prisma/migrations/20260815053307_fase2_catalogos/migration.sql:140`
  `btree_gist`, `houseplan-api/prisma/migrations/0000_init/migration.sql:5` `vector`); `CREATE EXTENSION` puts them in
  the first schema of the path, which stays `public` for both ports.
- The apidiff database comparison lists tables with `table_schema='public'` (`corvis-tests/tools/apidiff/apidiff.mjs:213`).

**Related change**

`e13-connection-limit-and-pool-timeout-in-the-database-url` (#133, not implemented yet) adds the single URL parsing
point: a private helper `split_pool_params(url) -> Result<(String, PoolUrlParams)>` called in `with_options`, a
`driver_url` field in `ClientInner` used by `pool()` and `ensure_database`, `P1013` initialization errors for invalid
values, `OrmClientOptions::url` kept as given (its design decisions 1 and 5).

## Goals / Non-Goals

**Goals:**
- `?schema=x` makes queries, raw SQL, transactions, migrations and `db push` use `x`, on every pooled connection.
- A missing schema is created by the migration and push commands; an existing one is never re-created.
- No value of `schema` can inject SQL or other server settings.
- No `schema` parameter, no behavior change.

**Non-Goals:**
- Qualifying generated SQL with the URL schema; defaulting to `public` (see Out of scope and Open questions).
- Any URL parameter other than `schema` (pool parameters are #133).

## Decisions

1. **One URL parsing point, shared with #133.** The helper becomes `split_url_params(url) -> Result<(String,
   UrlParams)>` with `UrlParams { connection_limit, pool_timeout, schema: Option<String> }`; it removes every
   recognized key and copies the other pairs byte for byte, as #133 decision 1 describes (split on the first `?` and on
   `&`, percent-decode values, last occurrence of a repeated key wins). `with_options` calls it once and stores
   `driver_url` and the params in `ClientInner`.
   - **If #133 lands first:** add the `schema` key to its helper, rename `split_pool_params` / `PoolUrlParams` to
     `split_url_params` / `UrlParams` (mechanical), and add the `schema` field.
   - **If this change lands first:** introduce the helper with the neutral names and only `schema`, plus `driver_url`;
     #133 then adds its two keys and pool fields to the same helper and struct. Its tasks 1.1-2.3 shrink to "add the
     keys", and `ensure_database` already uses `driver_url`.
   - Either way there is one parse, one `driver_url`, and one validation style (`P1013`, decision 5). The two changes
     touch the same lines of `with_options`, `ClientInner` and `pool()`; the second to land rebases onto the first.
   - Alternative: a separate `split_schema_param` pass. Rejected: two passes over the query string, two places that
     must agree on encoding and order.

2. **`search_path` through `after_connect`, with a quoted `SET`.** When `schema` is set on a Postgres URL, `pool()`
   adds `PgPoolOptions::after_connect` running `sqlx::raw_sql("SET search_path TO <quote_ident(schema)>")` (simple
   protocol, nothing prepared or cached). The statement text is built once in `pool()` and moved into the closure as an
   `Arc<str>`. The hook runs before any statement on the connection, so statements prepared later (sqlx caches them
   per connection) are planned against the right schema. The path is exactly the one schema, without `public`, matching
   what the original resolves generated queries against; the fixture confirms the raw-SQL side.
   - Alternative: `PgConnectOptions::options([("search_path", schema)])`, a startup parameter with no extra round trip.
     Rejected: sqlx does not escape the value (`options/mod.rs:439`), so `?schema=a%20-c%20statement_timeout%3D0` would
     set another server setting; correct escaping needs both libpq backslash rules and GUC quoting, and the startup
     `options` parameter is refused by PgBouncer.
   - Alternative: qualify every generated table and enum with the URL schema, like the original's SQL. Rejected for
     this change: every query text, `log_queries` output and SQL-text test changes, and raw SQL would still use the
     server's path; listed as Out of scope.
   - Alternative: `SET search_path` before each statement. Rejected: an extra round trip per query, and it would not
     cover statements prepared earlier.

3. **Ensure the schema with a lookup first, then `CREATE SCHEMA IF NOT EXISTS`.** A private
   `OrmClient::ensure_schema()` (Postgres only, no-op without `schema`) runs
   `SELECT 1 FROM pg_namespace WHERE nspname = $1` with the name bound, and only when no row comes back runs
   `CREATE SCHEMA IF NOT EXISTS <quote_ident(schema)>`. It is called at the start of `migration_rows`
   (`migrate.rs:179`, so both `migrate deploy` and `migrate status`) and of `db_push_models` (`client.rs:566`, which
   `db_push_force_reset` also reaches). Because `SET search_path` accepts a missing schema, the pool works before the
   schema exists and `ensure_schema` can run through it.
   - The lookup matters: Postgres checks the database `CREATE` privilege before the `IF NOT EXISTS` existence check, so
     an unconditional `CREATE SCHEMA IF NOT EXISTS "public"` would fail for an application role without that privilege
     (a common production setup), breaking `migrate deploy` for apps that worked before.
   - Alternative: create the schema in `pool()` on connect. Rejected: queries must not do DDL; a misspelled schema
     would be created silently on the first `find_many`.
   - Alternative: create it in `ensure_database`. Rejected: that function uses a separate, throwaway connection to the
     server and only exists to create the database.

4. **No parameter, no `SET`.** Without `schema`, `pool()` installs no hook and `ensure_schema` does nothing, so apps
   that rely on a role's `search_path` (for example `"$user", public, extensions` on managed services) keep it.
   - Alternative: default to `public`, as the original's generated SQL does. Rejected for now: it would change every
     app without the parameter and drop extension schemas from their path; listed as an open question.

5. **Validation and error.** After percent-decoding, `schema` must be non-empty, contain no NUL character (the
   protocol cannot carry it), and be at most 63 bytes (`NAMEDATALEN - 1` on a default server; a longer name would be
   truncated by `SET` and `CREATE SCHEMA` but not by the bound `pg_namespace` lookup, so the lookup would never match).
   Otherwise `with_options` returns `OrmError::Initialization` with `error_code: Some("P1013")` and
   ``The provided database string is invalid. `schema` must be a non-empty schema name of at most 63 bytes.``. The value
   is not echoed (unlike #133's numeric values, a schema name can carry arbitrary text). Any other character is
   accepted: quoting (decision 2) and binding (decision 3) make it safe.

6. **SQLite strips and ignores `schema`.** The helper removes it from SQLite URLs too, so `sqlite://dev.db?schema=main`
   stops failing in sqlx; it has no effect on the connection. The fixture records whether the original accepts the
   parameter on SQLite.

7. **Differential fixture against the original 6.x client.** A Node script in `corvis-tests/tmp/orm/schema/` (next to
   #133's `tmp/orm/pool/`) with `@prisma/client` and `prisma` 6.19.3 records, on a fresh database with `"User"` tables
   in `public` and `other` holding different rows: the rows of `findMany` with `?schema=other`; `current_schema()` and
   `SHOW search_path` through `$queryRaw` with `?schema=other`, `?schema=public` and no parameter; `migrate deploy` with
   `?schema=tenant_a` on a missing schema (schema created or error, location of `_prisma_migrations`); the error code
   and message for `schema=` (empty) and for a hostile name; and whether `sqlite:` / `file:` URLs accept `schema`. The
   results go to `crates/corvis-orm/tests/fixtures/schema_url.json`, replayed by `tests/pg_schema_url.rs`.

## Risks / Trade-offs

- [The path is exactly one schema] → with `?schema=public` on a managed service whose role path also lists an
  `extensions` schema, unqualified extension functions stop resolving. That matches what the original resolves
  against; documented, with "qualify extension objects or drop the parameter" as the advice.
- [Unqualified `CREATE EXTENSION` in migrations] → with `?schema=tenant_a` the extension is installed in `tenant_a`
  (or, if it already exists in another schema, its objects are not on the path). Expected to match the original; the
  docs mention it.
- [One extra round trip per new connection when `schema` is set] → only when a connection is opened, not per query.
- [Concurrent first migrations on a missing schema] → two processes can both see it missing; `IF NOT EXISTS` absorbs
  the race except for a rare unique-violation on the catalog, which fails one `migrate deploy` with a clear error; a
  retry succeeds.
- [User SQL that runs `SET search_path` outside a transaction] → it persists on that pooled connection, as with any
  session setting; unchanged by this change, mentioned in the docs.
- [Port apps now send a `SET` per connection] → effective path is already `public` for their `postgres` user; parity
  run confirms.
- [Merge conflicts with #133] → both edit `with_options`, `ClientInner` and `pool()`; decision 1 fixes the shared
  shape so the second to land only adds fields and keys.

## Open questions

- Absent `schema`: keep the server's `search_path` (this change) or default to `public` like the original's generated
  SQL? The fixture shows what the original does for raw SQL; a maintainer should confirm.
- Does the original set the connection's `search_path` at all, or only qualify generated SQL? If it only qualifies,
  unqualified raw SQL in the original goes to the server's path, and decision 2 diverges for raw queries; the fixture
  settles it, and the divergence (if any) is documented or the decision revisited.
- Does the original's `migrate deploy` create a missing schema (the issue says so) and does `migrate status` do it too?
  Confirm with the fixture.
- Empty `schema=`: error (this change) or treated as absent? Align with the fixture.
- Names over 63 bytes: rejected here; the original may truncate silently. Align with the fixture if it accepts them.

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- The recorded fixture (task 1.x) settles what the original does — default schema without the parameter, `search_path`
  versus qualified names, schema creation by `migrate deploy`/`migrate status`, empty `schema=` — before any code is
  written; the spec delta is adjusted to match it, except where security over parity applies.
- Names longer than 63 bytes are rejected with `P1013`, not truncated: a silent truncation could point two
  applications at the same schema.
- Without the parameter nothing changes (no `SET`), as in Decision 3, unless the fixture shows the original forces
  `public`.
- Correction to the issue text: only consi and facten pass `?schema=public`; houseplan does not.

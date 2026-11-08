# Tasks

## 1. Record the original behavior

- [x] 1.1 Write `corvis-tests/tmp/orm/schema/cases.js` with `@prisma/client` and `prisma` 6.19.3 (design decision 7)
  and record `crates/corvis-orm/tests/fixtures/schema_url.json`: `findMany` rows with two same-named `"User"` tables
  and `?schema=other`; `current_schema()` / `SHOW search_path` via `$queryRaw` for `?schema=other`, `?schema=public`
  and no parameter; `migrate deploy` with `?schema=tenant_a` on a missing schema (created or not, location of
  `_prisma_migrations`); code and message for `schema=` and for `x"; DROP TABLE canary; --`; whether `sqlite:` and
  `file:` URLs accept `schema`; verify the file is produced and its header names the script's path
- [x] 1.2 Compare the fixture with the spec delta and design decisions 2, 4, 5 and 6; write the outcome under the open
  questions of design.md and adjust the spec delta if the maintainer accepts a different rule; verify each open
  question has a recorded answer

## 2. Parse the parameter (shared helper with #133)

- [x] 2.1 Create or extend the private URL helper per design decision 1: `split_url_params(url) -> Result<(String,
  UrlParams)>` with `UrlParams::schema: Option<String>` (if #133 already landed, rename `split_pool_params` /
  `PoolUrlParams` and add the field). Unit tests in `crates/corvis-orm/src/client.rs` (or the helper's module): only
  `schema` (the `?` disappears); `schema` mixed with `sslmode` / `application_name` / `options=-c%20x%3Dy` (order and
  encoding of the others unchanged); `My%20Schema` → `My Schema`; `%22` decoded; repeated key (last wins);
  `sqlite://dev.db?schema=main` and `file:./dev.db?schema=main` stripped; verify with
  `cargo test -p corvis-orm --lib url_params`
- [x] 2.2 Validation per design decision 5: empty, `a%00b` and a 64-byte name each return an initialization error with
  code `P1013` that mentions `schema`; a 63-byte name is accepted; a test with `postgresql://app:secret@host/db?schema=`
  asserts the message contains neither `app` nor `secret`; verify the unit tests pass
- [x] 2.3 Call the helper in `OrmClient::with_options` (`client.rs:144-147`), store `driver_url` and the params in
  `ClientInner` (`:92-96`), keep `OrmClientOptions::url` as given, and use `driver_url` in `OrmClient::pool`
  (`:205-206`) and `OrmClient::ensure_database` (`crates/corvis-orm/src/migrate.rs:158`); verify that
  `OrmClient::new("postgresql://u:p@127.0.0.1:1/db?schema=")` fails at once without connecting, and that
  `options().url` still returns the original URL

## 3. Apply it

- [x] 3.1 In `OrmClient::pool` Postgres branch (`client.rs:234-240`), when `schema` is set build
  `SET search_path TO <quote_ident(schema)>` once (`quote_ident` from `crates/corvis-orm/src/db.rs:92-94`) and install
  `PgPoolOptions::after_connect` running it with `sqlx::raw_sql` (design decision 2); no hook without `schema`; add a
  unit test for the statement text with `a"b` → `SET search_path TO "a""b"`; verify `cargo build -p corvis-orm` has no
  warnings
- [x] 3.2 Add the private `OrmClient::ensure_schema()` (design decision 3): Postgres only, no-op without `schema`,
  `SELECT 1 FROM pg_namespace WHERE nspname = $1` with the name bound, then
  `CREATE SCHEMA IF NOT EXISTS <quote_ident(schema)>` only when missing; call it at the start of `migration_rows`
  (`migrate.rs:179`) and `db_push_models` (`client.rs:566`); verify `cargo build -p corvis-orm` has no warnings
- [x] 3.3 Doc comments: `OrmClientOptions::url` (`client.rs:24-25`) mentions `?schema=` and that it sets the
  `search_path` of every connection; `migrate_deploy` (`migrate.rs:201-205`) says it creates the URL schema when
  missing; verify `cargo doc -p corvis-orm --no-deps` has no warnings

## 4. Tests

- [x] 4.1 SQLite: in `crates/corvis-orm/tests/client.rs`, a client on `sqlite://<tmp>/schema.db?schema=main` connects
  and runs `db_push` + a query (today it fails with `unknown query parameter`); verify
  `cargo test -p corvis-orm --test client schema` passes
- [x] 4.2 Postgres (skipped without a server, `crates/corvis-orm/tests/common/pg.rs`): new
  `crates/corvis-orm/tests/pg_schema_url.rs` with the spec scenarios "Same-named tables in two schemas", "Raw queries
  and transactions", "Every pooled connection", "Explicit model schema wins", "Server default path" and "TLS parameter
  kept" (`sslmode=disable`); verify they pass against the local Postgres (`docker start corvis-pg`)
- [x] 4.3 Migrations in the same file: "Deploy to a missing schema" and "Status reads the schema's history" with a
  two-migration fixture folder under `crates/corvis-orm/tests/fixtures/`; "Hostile schema name" (create
  `public.canary` first, assert it survives and the schema name round-trips through `current_schema()`); "Mixed case
  and spaces"; verify they pass
- [x] 4.4 Privilege case: create a role `orm_test_schema_role` (unique name, dropped at the end) with `USAGE, CREATE`
  on an existing schema `app` and no database `CREATE`, run `migrate_deploy` with its URL and `?schema=app`, and assert
  success (scenario "Existing schema without database CREATE privilege"); verify it passes
- [x] 4.5 Replay `tests/fixtures/schema_url.json` (task 1.1) in `pg_schema_url.rs`: rows, `current_schema()`, migrate
  outcome and error codes; messages compared after removing the original's version-specific suffix, if any; verify
  it passes or the divergence is documented in design.md

## 5. Docs

- [x] 5.1 `docs/data/corvis-orm.md`: a "Database schema" subsection near the connection / `OrmModuleOptions` table
  (next to #133's "Connection pool" section if present): `?schema=name` sets `search_path` to exactly that schema on
  every connection, without it the server's path applies, `@@schema` models keep their schema, invalid values fail at
  creation with `P1013`, extensions and functions in other schemas must be qualified, and the 7.x adapter's `schema`
  option maps to the URL parameter; in "Migrations" (around line 248-256) state that `migrate deploy` / `status` and
  `push` create a missing schema and keep `_prisma_migrations` in it; same in `docs/es/data/corvis-orm.md`
- [x] 5.2 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of groups 2-4
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 6.5 Framework runtime code changed (corvis-orm connection and migrations): run the `corvis-parity` check and (not run by the implementer: the maintainer runs it)
  verify the three port apps keep 0 apidiff differences (and equivalent databases); consi and facten exercise
  `?schema=public`, houseplan the no-parameter path (maintainer batch run on batch/phase03-d 2780936: consi 832/832, facten 727/727, houseplan 636/636; routes identical, databases equivalent)
- [x] 6.6 Security review of the parsing and quoting (`security-reviewer` agent): no injection through `schema`, no URL
  parts in error messages. Note: maintainer security review 2026-11-08: no critical/high/medium findings; low/informational
  fixes applied in the commit "Harden the database URL parameters after review"; connect timeout filed as #142

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #137, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

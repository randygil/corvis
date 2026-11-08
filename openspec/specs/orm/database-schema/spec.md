# orm/database-schema Specification

## Purpose
The `schema` parameter of a Postgres database URL selects the schema used by queries and migrations, the same way the
original client does it, so a migrated app that keeps its `DATABASE_URL` reads and writes the same tables.

## Requirements

### Requirement: Queries use the schema from the URL
For Postgres URLs with a `schema` query parameter, every pooled connection of the client SHALL have its `search_path`
set to exactly that schema before it runs any other statement. Generated queries, raw queries and interactive
transactions MUST therefore resolve unqualified table and type names in that schema.

#### Scenario: Same-named tables in two schemas
- **WHEN** the database has a table `"User"` in `public` holding `a@x` and a table `"User"` in `other` holding `b@x`,
  and a client created with `postgresql://.../db?schema=other` runs `user().find_many({})`
- **THEN** the result contains only `b@x`, and `public."User"` is not read

#### Scenario: Raw queries and transactions
- **WHEN** a client created with `...?schema=other` runs `SELECT current_schema()` as a raw query, and again inside an
  interactive transaction
- **THEN** both return `other`

#### Scenario: Every pooled connection
- **WHEN** a client created with `...?schema=other` and `max_connections(3)` runs 6 concurrent
  `SELECT current_schema(), pg_sleep(0.2)` raw queries
- **THEN** all 6 return `other`

#### Scenario: Explicit model schema wins
- **WHEN** a model declares `@@schema("audit")` and the client URL has `?schema=other`
- **THEN** queries on that model target `"audit"."<table>"`

### Requirement: No parameter, no change
Without a `schema` query parameter, the client MUST NOT change the `search_path` of its connections.

#### Scenario: Server default path
- **WHEN** a client is created with a Postgres URL without `schema` and runs `SHOW search_path`
- **THEN** the result is the server's default for that role (`"$user", public` on a default server)

### Requirement: The parameter does not reach the driver
The client SHALL remove `schema` from the URL before it is given to the driver, for Postgres and SQLite URLs, and MUST
leave every other query parameter unchanged (same order, same encoding). On SQLite the parameter MUST be ignored.

#### Scenario: TLS parameter kept
- **WHEN** a client is created with `postgresql://host/db?sslmode=disable&schema=other`
- **THEN** the driver receives `postgresql://host/db?sslmode=disable`, no `ignoring unrecognized connect parameter`
  warning is logged for `schema`, and queries run in `other`

#### Scenario: SQLite URL with schema
- **WHEN** a client is created with `sqlite://<dir>/dev.db?schema=main`
- **THEN** the client connects to `<dir>/dev.db` and runs queries normally

### Requirement: Migrations apply inside the schema
`migrate deploy`, `migrate status`, `db push` and `db push --force-reset` SHALL create the URL schema when it does not
exist, then create `_prisma_migrations` (migration commands) and apply the migration SQL or the pushed DDL inside it.
When the schema already exists they MUST NOT issue `CREATE SCHEMA`.

#### Scenario: Deploy to a missing schema
- **WHEN** `migrate_deploy(dir)` runs with `...?schema=tenant_a` on a database without a `tenant_a` schema
- **THEN** the schema `tenant_a` is created, `tenant_a._prisma_migrations` records every migration of `dir`, the
  migration tables exist in `tenant_a`, and no table is created in `public`

#### Scenario: Status reads the schema's history
- **WHEN** `migrate_status(dir)` runs with the same URL after that deploy
- **THEN** it reports the database as up to date, and with `...?schema=public` it reports every migration as pending

#### Scenario: Existing schema without database CREATE privilege
- **WHEN** the role in the URL has `USAGE` and `CREATE` on an existing schema `app` but no `CREATE` privilege on the
  database, and `migrate_deploy(dir)` runs with `...?schema=app`
- **THEN** the migrations are applied in `app` and no `CREATE SCHEMA` statement is sent

### Requirement: The schema name is always quoted
The schema name SHALL reach the server only as a double-quoted identifier with every embedded `"` doubled, or as a
bound parameter. Case, spaces and any other character MUST be preserved, and no value of `schema` MUST be able to run
another statement or change another setting.

#### Scenario: Hostile schema name
- **WHEN** a client is created with `...?schema=x%22%3B%20DROP%20TABLE%20canary%3B%20--` (decoded:
  `x"; DROP TABLE canary; --`), runs `migrate_deploy(dir)` and `SELECT current_schema()`
- **THEN** a schema named exactly `x"; DROP TABLE canary; --` holds the migrated tables, `current_schema()` returns
  that name, and the table `public.canary` still exists

#### Scenario: Mixed case and spaces
- **WHEN** a client is created with `...?schema=My%20Schema` and runs `migrate_deploy(dir)`
- **THEN** the tables are created in the schema `My Schema` (not `my schema`)

### Requirement: Invalid schema values fail at creation
An empty `schema` value, `$user`, a value starting with `pg_`, a value containing a NUL character, or a value longer
than 63 bytes after decoding
SHALL make `OrmClient::with_options` (and `OrmClient::new`) return an initialization error with code `P1013` that names
the `schema` parameter. The message MUST NOT contain the user, the password or any other part of the URL.

#### Scenario: Empty schema
- **WHEN** a client is created with `postgresql://app:secret@host/db?schema=`
- **THEN** creation fails with a `P1013` initialization error mentioning `schema`, without connecting, and the message
  contains neither `app` nor `secret`

#### Scenario: NUL character
- **WHEN** a client is created with `postgresql://host/db?schema=a%00b`
- **THEN** creation fails with a `P1013` initialization error mentioning `schema`

#### Scenario: Reserved names
- **WHEN** a client is created with `?schema=%24user`, `?schema=pg_temp` or `?schema=pg_x`
- **THEN** creation fails with a `P1013` initialization error mentioning `schema` (`"$user"` is the server's special
  path token and `pg_temp` would send every object to temporary tables)

### Requirement: Scripts do not change the pooled sessions
Migration scripts and `db_execute` scripts SHALL run on a connection that is closed afterwards and never returns to
the pool, and `_prisma_migrations` SHALL be referenced by its name qualified with the URL schema, so that a script that
changes the session (`set_config('search_path', ...)`, `RESET ALL`, `SET search_path`) cannot send later statements
to another schema.

#### Scenario: Migration that resets the search path
- **WHEN** `migrate_deploy(dir)` runs with `...?schema=sticky&connection_limit=1` and a migration runs
  `SELECT set_config('search_path', 'public', false)` and another `RESET ALL`
- **THEN** both migrations are recorded as finished in `sticky._prisma_migrations`, and afterwards `current_schema()`
  and model queries still resolve to `sticky`

### Requirement: Parameters are read as form data
The query string SHALL end at the first `#` (the fragment is never read as parameters), keys SHALL be percent-decoded
before they are matched (`schem%61=t` is `schema=t`), values SHALL be decoded with `+` as a space and `%` followed by
two ASCII hex digits only, and the pairs that are not ours MUST be copied unchanged.

#### Scenario: Encoded key and plus sign
- **WHEN** a client is created with `...?schem%61=My+Schema&sslmode=disable`
- **THEN** the schema is `My Schema` and the driver receives `...?sslmode=disable`

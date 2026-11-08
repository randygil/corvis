# Proposal

Implements [randygil/corvis#133](https://github.com/randygil/corvis/issues/133) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The original client (the Rust query engine of the 6.x line, which the port originals run) reads the pool size and the
pool wait time from the database URL: `postgresql://.../db?connection_limit=5&pool_timeout=20`. Teams size their pool
per environment that way, through `DATABASE_URL`, without touching code.

corvis-orm never reads them. `OrmClient::pool` (`crates/corvis-orm/src/client.rs:197-244`) builds the pool only from
`OrmClientOptions::max_connections` / `acquire_timeout`, with its own defaults (10 for Postgres, 5 for SQLite files,
sqlx's 30 s wait). The parameters are not just ignored, they behave inconsistently:

- Postgres: sqlx logs `ignoring unrecognized connect parameter` and connects with the default pool.
- `sqlite://dev.db?connection_limit=1`: sqlx rejects the URL (`unknown query parameter`), so the client cannot connect.
- `file:./dev.db?connection_limit=1`: the query string is dropped.

A migrated app keeps its URL and silently gets a different pool size and a different wait before `P2024`.

## What Changes

- corvis-orm reads `connection_limit` and `pool_timeout` from the URL query string for Postgres and for file-backed
  SQLite (`file:` and `sqlite:`), removes them before the URL reaches the driver, and applies them to the pool.
- Same defaults as the original's 6.x engine: pool size `num_physical_cpus * 2 + 1`, pool timeout 10 seconds,
  `pool_timeout=0` waits without limit. **Behavior change:** the default Postgres pool goes from 10 to
  `2 * physical cores + 1`, the default SQLite file pool from 5 to the same formula, and the default wait for a free
  connection from 30 s to 10 s.
- Explicit options still win: `OrmClientOptions::max_connections(n)` / `acquire_timeout` and
  `OrmModuleOptions::max_connections(n)` override the URL; the URL overrides the defaults.
- An invalid value (`connection_limit=0`, `connection_limit=abc`, `pool_timeout=-1`) fails when the client is created
  with an initialization error that names the parameter and never echoes the URL.
- In-memory SQLite keeps exactly one connection; `connection_limit` is accepted and ignored there.
- Docs (EN + ES): a "Connection pool" section in the corvis-orm page.

## Capabilities

### New Capabilities

- `orm/connection-pool`: pool size and pool timeout taken from the database URL, their defaults, precedence over code
  options and validation.

### Modified Capabilities

None (no spec covers the ORM connection yet).

## Impact

- `crates/corvis-orm/src/client.rs`: URL parameter parsing (new private helper), `OrmClient::with_options`,
  `OrmClient::pool`, `OrmClientOptions` doc comments.
- `crates/corvis-orm/src/migrate.rs` (`ensure_database` parses the URL with sqlx; it must use the URL without the
  pool parameters).
- `crates/corvis-orm/src/module.rs` (doc comment of `OrmModuleOptions::max_connections`).
- `crates/corvis-orm/Cargo.toml`: new dependency `num_cpus` (physical core count).
- Tests: unit tests in `client.rs`, a SQLite integration test and a Postgres test (skipped without a server) with a
  differential fixture recorded with the original client.
- Docs: `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`.
- Port apps: their URLs carry `?schema=public` and no pool parameter, and they do not set `max_connections`, so only
  the defaults change for them (pool size and 10 s wait). Responses are not affected; the parity check confirms it.

## Out of scope

- Other URL parameters of the original: `connect_timeout`, `socket_timeout`, `max_connection_lifetime`,
  `max_idle_connection_lifetime`, `pgbouncer`, `statement_cache_size`, `schema` (sqlx logs and ignores it today, so
  `?schema=other` does not set the search path), `sslaccept`, `sslidentity`/`sslpassword`; possible new issues,
  `schema` first because every port URL carries it.
- Driver-adapter pool settings of the 7.x line (`max`, `connectionTimeoutMillis` on the `pg` adapter): those are code,
  not URL, and map to the existing `max_connections` / `acquire_timeout` builders.
- An `acquire_timeout` builder on `OrmModuleOptions` (only `max_connections` exists there).
- A separate connection-establishment timeout: sqlx bounds connecting with the same acquire timeout (see design).
- The `More info: http://pris.ly/...` suffix of the `P2024` message (`crates/corvis-orm/src/error.rs:282-286`).

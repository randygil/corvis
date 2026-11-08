# Proposal

Implements [randygil/corvis#142](https://github.com/randygil/corvis/issues/142) (epic
[#13](https://github.com/randygil/corvis/issues/13)). Found by the security review of #133 and #137, and builds on
both: `e13-connection-limit-and-pool-timeout-in-the-database-url` (#133) and
`e13-schema-parameter-in-the-database-url` (#137), implemented but not merged when this change was written.

## Why

corvis-orm has no time limit for opening a database connection apart from the pool's acquire timeout. In sqlx 0.8.6
that one deadline covers waiting for a pool slot, the TCP/TLS connect, the startup and authentication exchange, the
retries on "connection refused" and on SQLSTATE 53300/57P03, and every failure of the `after_connect` hook, which
sqlx always retries (`sqlx-core-0.8.6/src/pool/inner.rs:242-293`, `:321-392`). Since #133, `pool_timeout` sets that
deadline, and `pool_timeout=0` means "about 136 years". So:

- `postgres://u:p@127.0.0.1:1/x?pool_timeout=0`: `connect()` never returns; sqlx retries the refused connection until
  the deadline.
- A database that is down, full (53300) or silent parks every incoming request forever, holding its socket and memory.
- When the `SET search_path` that #137 runs on every new connection fails (for example 22P05 on a LATIN1 database
  with `?schema=%E2%82%AC`), `connect()` hangs. With a finite `pool_timeout` it fails with a pool timeout that hides the
  real cause; the database error only appears in a `tracing::error!` line of sqlx.

The original client (6.x query engine) keeps a separate `connect_timeout` URL parameter: whole seconds, 5 by default,
`0` for no limit, failing with `P1001` "Can't reach database server" when it expires.

## What Changes

- corvis-orm reads `connect_timeout` from Postgres URLs in the single URL parser of #133 (`split_url_params` in
  `crates/corvis-orm/src/url_params.rs`): whole seconds, default 5, `0` = no limit. The key is removed before the URL
  reaches the driver. An invalid value fails at creation with the same `P1013` error shape as `pool_timeout`.
- Waiting for a pool slot and opening a connection are timed separately. `pool_timeout` (and the code option
  `OrmClientOptions::acquire_timeout`) bound only the wait for a free slot; `connect_timeout` bounds getting a usable
  connection once the request has a slot: the TCP/TLS connect, startup and authentication, the `schema` setup
  statement, the driver's own retries, and the liveness check of an idle connection. The first connection of
  `connect()` and the throwaway connections of `migrate deploy` / `migrate status` (`ensure_database`) are bounded
  the same way.
- When `connect_timeout` expires, the query (or `connect()`) fails with `P1001` "Can't reach database server at
  `host:port`" instead of hanging or reporting `P2024`.
- When the per-connection setup statement fails with a database error, the request fails at once with `P1001` and the
  database's message, instead of retrying until a timeout and reporting a generic pool timeout.
- SQLite URLs accept any `connect_timeout` value, valid or not, and ignore it (the parameter is removed so sqlx does
  not reject the URL).
- A differential fixture recorded with the original 6.x client, like #133's.
- Docs (EN + ES): `connect_timeout` in the "Connection pool" section of the corvis-orm page.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `orm/connection-pool` (created by #133): new requirements for `connect_timeout`, separate timing of the slot wait and
  the connection setup, the reported error on setup failure, validation, and SQLite handling; the `pool_timeout`
  requirement is narrowed to the slot wait.

## Impact

- `crates/corvis-orm/src/url_params.rs`: new key `connect_timeout`, new `UrlParams` field, tests.
- `crates/corvis-orm/src/client.rs`: `OrmClient::pool` (pool construction, first connection), `ClientInner`,
  callers of `acquire_pg`, `init_error`, doc comments of `OrmClientOptions::acquire_timeout` and `url`.
- `crates/corvis-orm/src/db.rs`: `DbPool::Postgres`, `acquire_pg`, `DbPool::begin`, `DbTx::Postgres` (holds the slot
  for the life of a transaction).
- `crates/corvis-orm/src/migrate.rs`: `ensure_database` bounds its direct connections.
- Tests: unit tests in `url_params.rs` and `db.rs`, a loopback "silent server" test that needs no database, a Postgres
  test (skipped without a server) replaying a new fixture `crates/corvis-orm/tests/fixtures/connect_timeout.json`
  recorded by `corvis-tests/tmp/orm/connect/cases.js`.
- Docs: `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`.
- Port apps: their URLs carry no `connect_timeout`, so they get the 5 s default. A healthy local server connects in
  milliseconds; responses are not affected. The parity check confirms it.
- Behavior change: an unreachable server now fails after at most about 5 s by default, whatever `pool_timeout` or
  `acquire_timeout` say. A program that set `acquire_timeout` to 2 s to fail fast on an unreachable server now waits up
  to `connect_timeout` (5 s) instead; `?connect_timeout=2` restores it.

## Out of scope

- A code option (`OrmClientOptions::connect_timeout`): the original only reads the URL; `OrmClientOptions` has public
  fields, so a new one breaks struct-literal construction. Listed as an open question in design.md.
- `socket_timeout` (the original's per-query socket timeout) and `max_connection_lifetime` /
  `max_idle_connection_lifetime`: other URL parameters of the original, possibly a new issue.
- Mapping connection failures to the original's specific codes (`P1000` authentication, `P1003` missing database,
  `P2037` too many connections); corvis-orm keeps `P1001` for all of them, as today.
- Making "connection refused" fail at once like the original instead of being retried within `connect_timeout`
  (design decision 6).
- The text of `P2024` (the original appends the current pool timeout and limit; noted by #133).
- SQLite busy waits (`busy_timeout`): unrelated to opening a connection.
- The 7.x line (`@prisma/adapter-pg`), which ignores `connect_timeout` in the URL; this change follows the 6.x engine,
  as #133 did.

# Spec Delta

## Purpose

The connection pool of corvis-orm is sized and timed from the database URL the same way the original client (6.x
query engine) does it, so a migrated app that keeps its `DATABASE_URL` gets the same pool.

## ADDED Requirements

### Requirement: Pool size from `connection_limit`
For Postgres URLs and file-backed SQLite URLs (`file:` and `sqlite:` that are not in-memory), the client SHALL use the
`connection_limit` query parameter as the maximum number of pooled connections. Without the parameter, the maximum
MUST be `2 * physical CPU cores + 1`.

#### Scenario: Postgres URL with a limit
- **WHEN** a client is created with `postgresql://u:p@host/db?connection_limit=2` and runs 6 concurrent
  `SELECT pg_sleep(0.3)` raw queries
- **THEN** all 6 queries succeed and at most 2 server connections are open for that client at any time

#### Scenario: Default pool size
- **WHEN** a client is created with a Postgres URL without `connection_limit` and without `max_connections(..)`
- **THEN** the pool allows at most `2 * physical cores + 1` connections

#### Scenario: SQLite file URL with a limit
- **WHEN** a client is created with `file:./dev.db?connection_limit=1` or `sqlite://dev.db?connection_limit=1`
- **THEN** the client connects to `./dev.db` (respectively `dev.db`) with a pool of 1 connection, and the parameter
  does not reach the driver

#### Scenario: In-memory SQLite ignores the limit
- **WHEN** a client is created with `file::memory:?connection_limit=4`
- **THEN** the pool keeps exactly one connection, as without the parameter

### Requirement: Pool wait from `pool_timeout`
The client SHALL use the `pool_timeout` query parameter, in whole seconds, as the maximum time a query waits for a free
pooled connection. Without the parameter it MUST be 10 seconds. `pool_timeout=0` MUST mean no limit. When the wait
expires the query MUST fail with the known-request error `P2024`.

#### Scenario: Short pool timeout
- **WHEN** a client is created with `...?connection_limit=1&pool_timeout=1`, one interactive transaction holds the only
  connection, and another query runs outside it
- **THEN** that query fails after about 1 second with error code `P2024`

#### Scenario: No limit
- **WHEN** a client is created with `...?connection_limit=1&pool_timeout=0`, a transaction holds the only connection
  for 2 seconds, and another query runs outside it
- **THEN** that query waits and succeeds once the transaction ends

### Requirement: Code options take precedence over the URL
An explicit `OrmClientOptions::max_connections(n)` (also set through `OrmModuleOptions::max_connections(n)`) SHALL
override `connection_limit`, and an explicit `OrmClientOptions::acquire_timeout` SHALL override `pool_timeout`. The URL
parameters MUST override the defaults.

#### Scenario: Builder wins over the URL
- **WHEN** a client is created with `OrmClientOptions::new("postgresql://.../db?connection_limit=2").max_connections(4)`
- **THEN** the pool allows at most 4 connections

### Requirement: Invalid pool parameters fail at creation
`connection_limit` MUST be an integer of at least 1 and `pool_timeout` a non-negative integer. Any other value SHALL
make `OrmClient::with_options` (and `OrmClient::new`) return an initialization error that names the parameter and, when
the value is at most 40 characters of `[0-9A-Za-z+._-]`, the rejected value (otherwise "got an invalid value": an
unencoded password can contain a parameter key). The error message MUST NOT contain the user, the password or any other part of the URL.

#### Scenario: Non-numeric limit
- **WHEN** a client is created with `postgresql://app:secret@host/db?connection_limit=abc`
- **THEN** creation fails with an initialization error mentioning `connection_limit` and `abc`, and the message does
  not contain `app` or `secret`

#### Scenario: Pool timeout on SQLite
- **WHEN** a client is created with `file:./dev.db?pool_timeout=abc`
- **THEN** creation succeeds and the parameter is ignored (the original client accepts any `pool_timeout` on SQLite);
  Postgres URLs reject the same value

#### Scenario: Zero limit
- **WHEN** a client is created with `file:./dev.db?connection_limit=0`
- **THEN** creation fails with an initialization error mentioning `connection_limit` (the original treats `0` as no
  limit; corvis-orm requires at least 1)

### Requirement: The pool size is bounded
The pool size, from `connection_limit` or from `OrmClientOptions::max_connections`, MUST be between 1 and 262143 (the
largest value Postgres accepts for its own `max_connections`), because the driver preallocates one slot per connection.
A value outside that range SHALL make `OrmClient::with_options` return an initialization error that names the
parameter or field; for `connection_limit` the error has code `P1013` and the same shape as the other invalid values.

#### Scenario: Limit at the bound
- **WHEN** a client is created with `connection_limit=262143`
- **THEN** creation succeeds

#### Scenario: Limit above the bound
- **WHEN** a client is created with `connection_limit=262144` or `connection_limit=4294967295`
- **THEN** creation fails with a `P1013` initialization error saying the value must be a positive integer no greater
  than 262143, without allocating a pool

#### Scenario: Size set in code
- **WHEN** a client is created with `OrmClientOptions::max_connections(0)` or `max_connections(u32::MAX)`
- **THEN** creation fails with an initialization error that names `max_connections`

### Requirement: Other URL parameters are preserved
Removing the pool parameters SHALL leave every other query parameter of the URL unchanged (same order, same encoding),
so driver parameters such as `sslmode`, `application_name` or `options` keep working.

#### Scenario: TLS parameter kept
- **WHEN** a client is created with `postgresql://host/db?sslmode=require&connection_limit=3`
- **THEN** the driver receives `postgresql://host/db?sslmode=require` and the pool allows at most 3 connections

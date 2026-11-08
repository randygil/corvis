# orm/connection-pool Specification

## Purpose
The connection pool of corvis-orm is sized and timed from the database URL the same way the original client (6.x
query engine) does it, so a migrated app that keeps its `DATABASE_URL` gets the same pool.

## Requirements

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
pooled connection slot. Without the parameter it MUST be 10 seconds. `pool_timeout=0` MUST mean no limit. When the wait
expires the query MUST fail with the known-request error `P2024`. On Postgres the wait MUST NOT include opening a new
connection or checking an idle one, which `connect_timeout` bounds instead; `OrmClientOptions::acquire_timeout`, when
set, replaces `pool_timeout` with the same meaning.

#### Scenario: Short pool timeout
- **WHEN** a client is created with `...?connection_limit=1&pool_timeout=1`, one interactive transaction holds the only
  connection, and another query runs outside it
- **THEN** that query fails after about 1 second with error code `P2024`

#### Scenario: No limit
- **WHEN** a client is created with `...?connection_limit=1&pool_timeout=0`, a transaction holds the only connection
  for 2 seconds, and another query runs outside it
- **THEN** that query waits and succeeds once the transaction ends

#### Scenario: Waiting for a slot is not cut by the connect timeout
- **WHEN** a Postgres client is created with `...?connection_limit=1&pool_timeout=0&connect_timeout=1`, a transaction
  holds the only connection for 3 seconds, and another query runs outside it
- **THEN** that query waits about 3 seconds and succeeds, without a `P1001` error

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

### Requirement: Connect timeout from `connect_timeout`
For Postgres URLs the client SHALL read the `connect_timeout` query parameter, in whole seconds, as the maximum time to
obtain a usable connection once a request holds a pool slot, 5 seconds by default, `0` meaning no limit, and applied
even when `pool_timeout` is `0`. On expiry the request MUST fail with an initialization error `P1001` whose message
starts with "Can't reach database server at `host:port`" and contains no credentials or query parameters.

#### Scenario: Refused port with no pool limit
- **WHEN** a client is created with `postgres://u:p@127.0.0.1:1/x?pool_timeout=0` and `connect()` is called
- **THEN** it fails within about 5 seconds with `P1001` and a message starting "Can't reach database server at
  `127.0.0.1:1`"

#### Scenario: Default connect timeout
- **WHEN** a TCP listener on `127.0.0.1` accepts connections and never sends a byte, and a client with
  `postgres://u:p@127.0.0.1:<port>/x?pool_timeout=0` calls `connect()`
- **THEN** it fails after about 5 seconds with `P1001`

#### Scenario: No connect limit
- **WHEN** the same silent listener is used with `?connect_timeout=0&pool_timeout=0`
- **THEN** `connect()` is still pending after 3 seconds

#### Scenario: Parameter removed from the driver URL
- **WHEN** a client is created with `postgresql://h/db?sslmode=disable&connect_timeout=3&schema=s`
- **THEN** the driver receives `postgresql://h/db?sslmode=disable` and the connect timeout is 3 seconds

### Requirement: The connect timeout covers every step of opening a connection
The `connect_timeout` bound SHALL cover the TCP and TLS connect, the startup and authentication exchange, the
per-connection setup statement of the `schema` parameter, the driver's retries of refused connections and of SQLSTATE
53300 / 57P03, and the liveness check of an idle connection taken from the pool.

#### Scenario: Server that accepts the socket and never answers
- **WHEN** the silent listener is used with `postgres://u:p@127.0.0.1:<port>/x?connect_timeout=1&pool_timeout=0` and
  `connect()` is called
- **THEN** it fails after about 1 second (less than 3) with `P1001`

#### Scenario: Role at its connection limit
- **WHEN** the role of the URL has `CONNECTION LIMIT 0` and a client with `?connect_timeout=1&pool_timeout=0` calls
  `connect()`
- **THEN** it fails after about 1 second with `P1001` instead of retrying forever

### Requirement: Every connection the client opens is bounded
The `connect_timeout` bound SHALL apply to the first connection opened by `OrmClient::connect` (or by the first query),
to every later connection the pool opens, and to the direct connections that `migrate deploy` and `migrate status` open
to check that the database exists.

#### Scenario: Server lost after start
- **WHEN** a client with `?pool_timeout=0&connect_timeout=1` has connected, new connections to its server then stop
  succeeding, and a query needs a new connection
- **THEN** the query fails after about 1 second with `P1001` instead of waiting forever

#### Scenario: Migration commands are bounded
- **WHEN** `migrate deploy` runs with a URL whose server accepts the socket and never answers, with `connect_timeout=1`
- **THEN** it fails after about 1 second (less than 3) with `P1001`

### Requirement: Connection setup failures report the database error
When the per-connection setup statement (the `SET search_path` of the `schema` parameter) fails with an error returned
by the database, the request that opened the connection SHALL fail at once, without waiting for `connect_timeout` or
`pool_timeout`, with an initialization error `P1001` whose message starts with "Can't reach database server at
`host:port`" and contains the database's message. It MUST NOT be reported as `P2024`, `P2010` or a timeout.

The original client does not run a per-connection statement for `schema` and connects to such a database; this is a
documented difference (the statement is part of #137's design).

#### Scenario: Schema name the database cannot encode
- **WHEN** a client is created for a database with `LATIN1` encoding with `?schema=%E2%82%AC&pool_timeout=0` and
  `connect()` is called
- **THEN** it fails in less than 1 second with `P1001` and a message that contains the database's
  "has no equivalent in encoding" text

#### Scenario: Raw query on a failing setup
- **WHEN** the same client runs `query_raw("SELECT 1", vec![])` instead of `connect()`
- **THEN** it fails the same way with `P1001`, not `P2010`

### Requirement: Socket errors during setup are retried within the connect timeout
When the per-connection setup statement fails with a socket error, the client SHALL let the connection be retried
within `connect_timeout`. When the bound then expires, the `P1001` message MUST include the last setup error.

#### Scenario: Setup keeps failing on the socket
- **WHEN** every setup attempt fails with a socket error and `connect_timeout=1`
- **THEN** the request fails after about 1 second with `P1001` and the message contains that socket error

### Requirement: Invalid `connect_timeout` fails at creation
On Postgres URLs, a `connect_timeout` that is not a non-negative integer of decimal digits after percent-decoding SHALL
make `OrmClient::with_options` (and `OrmClient::new`) return an initialization error with code `P1013` whose message
starts "The provided database string is invalid." and names `connect_timeout`. The value MUST be shown only when it is
at most 40 characters of `[0-9A-Za-z+._-]`, and no other part of the URL MUST appear.

#### Scenario: Non-numeric value
- **WHEN** a client is created with `postgresql://app:secret@host/db?connect_timeout=abc`
- **THEN** creation fails with `P1013`, the message mentions `connect_timeout` and `abc`, and contains neither `app` nor
  `secret`

### Requirement: `connect_timeout` is parsed like the other URL parameters
The key SHALL be matched after percent-decoding and the last occurrence of a repeated key MUST win. A value that does not
fit 64 bits is invalid, as in the original client; a value that fits but is larger than the clock can add (more than
`u32::MAX` seconds) MUST mean no limit.

#### Scenario: Repeated key
- **WHEN** a client is created with `postgresql://h/db?connect_timeout=1&connect_timeout=3`
- **THEN** the connect timeout is 3 seconds

#### Scenario: Huge value
- **WHEN** a client is created with `postgresql://h/db?connect_timeout=18446744073709551615`
- **THEN** creation succeeds and connecting has no time limit

#### Scenario: Value beyond 64 bits
- **WHEN** a client is created with `postgresql://h/db?connect_timeout=99999999999999999999`
- **THEN** creation fails with `P1013` naming `connect_timeout`

### Requirement: SQLite ignores `connect_timeout`
For SQLite URLs (`file:` and `sqlite:`) the client SHALL remove `connect_timeout` from the URL and accept any value,
valid or not, without using it.

#### Scenario: SQLite URL with the parameter
- **WHEN** a client is created with `file:./dev.db?connect_timeout=abc` or `sqlite://dev.db?connect_timeout=1`
- **THEN** creation succeeds, the client connects to the file, and the parameter does not reach the driver

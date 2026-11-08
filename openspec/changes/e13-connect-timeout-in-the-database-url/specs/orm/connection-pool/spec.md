# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

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
The key SHALL be matched after percent-decoding, the last occurrence of a repeated key MUST win, and a value too large
for 64 bits MUST mean no limit.

#### Scenario: Repeated key
- **WHEN** a client is created with `postgresql://h/db?connect_timeout=1&connect_timeout=3`
- **THEN** the connect timeout is 3 seconds

#### Scenario: Huge value
- **WHEN** a client is created with `postgresql://h/db?connect_timeout=99999999999999999999`
- **THEN** creation succeeds and connecting has no time limit

### Requirement: SQLite ignores `connect_timeout`
For SQLite URLs (`file:` and `sqlite:`) the client SHALL remove `connect_timeout` from the URL and accept any value,
valid or not, without using it.

#### Scenario: SQLite URL with the parameter
- **WHEN** a client is created with `file:./dev.db?connect_timeout=abc` or `sqlite://dev.db?connect_timeout=1`
- **THEN** creation succeeds, the client connects to the file, and the parameter does not reach the driver

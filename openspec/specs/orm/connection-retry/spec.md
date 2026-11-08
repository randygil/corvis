# orm/connection-retry Specification

## Purpose
Guarantees that a transient socket error while corvis-orm opens a PostgreSQL connection does not reach the
application as a failed query, and that real failures still do.

## Requirements

### Requirement: Transient socket errors while opening a connection are retried
When opening a PostgreSQL connection (the first connection of the pool, a connection taken from the pool for a query,
a script or a transaction) fails with a socket-level I/O error other than "connection refused", the client SHALL try
again, up to four more times with a short backoff, before reporting an error. A statement MUST NOT be sent more than
once because of this retry.

#### Scenario: One attempt fails with a socket error
- **WHEN** opening a connection fails once with an I/O error such as `WSAEINVAL` (os error 10022) and the next attempt succeeds
- **THEN** the query runs once and returns its result, with no error

#### Scenario: Every attempt fails
- **WHEN** opening a connection fails with an I/O error on all five attempts
- **THEN** the last error is returned, as the "can't reach database server" error, unchanged

#### Scenario: Other errors are not retried here
- **WHEN** opening a connection fails with "connection refused", a pool timeout, a TLS error or a database error
- **THEN** the client does not repeat the attempt on its own (the pool applies its own rules for refused connections)

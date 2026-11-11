# Spec Delta

## Purpose

What the `timeout` of an interactive transaction and the cancellation of the request that runs it do to the running
statement and to the connection. Introduced by the ORM quality change `q-orm-hot-path` (epic
[#187](https://github.com/randygil/corvis/issues/187)).

## ADDED Requirements

### Requirement: The transaction timeout ends the running statement
On Postgres, when an interactive transaction's `timeout` expires while one of its statements runs, the client SHALL cancel
that statement on the server and roll the transaction back. The statement's caller MUST receive the error recorded from
the original client for the same case, and the connection SHALL return to the pool clean.

#### Scenario: Long statement past the timeout
- **WHEN** an interactive transaction with `timeout: 1000` runs `SELECT pg_sleep(30)`
- **THEN** the call fails with the recorded `P2028` error, the backend is neither `active` nor `idle in transaction`
  before the statement could have ended, and the next query on a one-connection pool succeeds

#### Scenario: Statement that ends in time
- **WHEN** 1,000 interactive transactions run statements that end before their timeout
- **THEN** no statement is cancelled

### Requirement: A cancelled transaction releases its connection without waiting for its statement
On Postgres, when the future of an interactive transaction is dropped while one of its statements runs, the client SHALL
cancel the statement on the server, roll back, and release the pool slot once the connection is back in the pool. SQLite
and statements outside interactive transactions keep running until they end, and the documentation SHALL say so.

#### Scenario: Request dropped mid-statement
- **WHEN** a request running `SELECT pg_sleep(30)` inside an interactive transaction on a one-connection pool is dropped
- **THEN** the long statement disappears from `pg_stat_activity`, and the next query obtains the connection and finds no
  transaction left open

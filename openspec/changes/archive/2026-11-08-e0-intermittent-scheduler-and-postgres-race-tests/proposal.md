# Proposal

Implements [randygil/corvis#138](https://github.com/randygil/corvis/issues/138) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

Two tests fail now and then on a full workspace run, which makes a red run ambiguous:

- `crates/corvis-orm/tests/security.rs::delete_and_upsert_apply_their_filter_atomically_postgres` failed on the
  assertion at `security.rs:401` with `Can't reach database server: error communicating with database: ... (os error
  10022)`. The error is a connection failure that the test counted as the result of a race. It also fails when the
  test runs alone (2 of 60 runs), so other tests running in parallel are not the cause.
- `crates/corvis/tests/schedule.rs::wait_for_completion_skips_overlapping_ticks_and_default_overlaps` failed once
  and fails about 1 run in 17 alone (3 of 50 runs): its assertions depend on where in
  the second the cron job starts.

The investigation of the first one found a real bug in the framework: `OrmClient` on Windows can hand
`WSAEINVAL` (os error 10022) to the application when a pool opens a connection, because sqlx calls `set_nodelay`
right after `connect` and fails when the peer has already reset the socket; sqlx's pool retries only
`ConnectionRefused`.

## What Changes

- `corvis-orm`: opening a PostgreSQL connection (the initial pool connection, a connection taken from the pool for a
  query, a script or a transaction) is repeated up to 4 more times, with a short backoff, when it fails with a
  socket-level I/O error other than `ConnectionRefused`. If every attempt fails, the last error is returned
  unchanged, as before. Queries are never repeated: only the acquisition of a connection, before anything is sent.
- `corvis-orm` tests: unit tests of the retry helper, and the Postgres test helper (`tests/common/pg.rs`) retries the
  same way before it decides to skip a test, so a transient error cannot silently skip a test either.
- `corvis` tests: the scheduler test no longer depends on the wall-clock phase of the first tick; it keeps asserting
  the same two behaviors (`wait_for_completion` skips overlapping ticks, the default lets them overlap).

## Capabilities

### New Capabilities
- `orm/connection-retry`: transient socket errors while opening a database connection are retried before they reach
  the application.

## Impact

- Runtime code changes in `crates/corvis-orm/src/db.rs` and `client.rs` (Postgres only; SQLite is untouched). Parity
  with the Node originals must be checked with the corvis-parity skill before archiving.
- Behavior change visible to applications: a transient connect error that used to fail the query (P1001) is now
  absorbed; a persistent one still fails with the same error after about 150 ms of retries. No documented behavior
  changes, so no docs change.

## Out of scope

- Retrying queries or transactions after a failure on an open connection: unsafe for non-idempotent statements.
- Retrying SQLite file opens, `ConnectionRefused` (sqlx already retries it until the acquire timeout) or TLS errors.
- Making the scheduler testable with paused tokio time: the cron library computes ticks from the real clock.

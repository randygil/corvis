# Spec Delta

## ADDED Requirements

### Requirement: Pool rules are verified without timing races
The Postgres tests that verify the pool size (`connection_limit`, `max_connections(..)`, the default size) and the
pool and connect waits (`pool_timeout`, `connect_timeout`) SHALL keep the queries under test in flight until the test
has observed the state it checks, using a server-side gate (an advisory lock held by a separate transaction) or an
explicit handshake, and MUST NOT depend on concurrent sleeps overlapping or on a fixed delay before a holder is in
place. Duration assertions MUST use a tight lower bound and an upper bound placed below the nearest value an incorrect
implementation would produce, not a symmetric tolerance around the original client's timing. The tests MUST still
compare the pool size, the wait bound, the error codes and the error messages with the fixtures recorded from the
original client.

#### Scenario: Pool size under load
- **WHEN** `pg_pool_url` runs while the machine is fully loaded by other builds and test runs
- **THEN** each pool size case observes exactly the expected number of queries blocked at the server at the same
  time, and the tests pass

#### Scenario: A pool that exceeds its limit is still caught
- **WHEN** the pool would open more connections than `connection_limit` allows
- **THEN** the observer sees more blocked queries or backends than the limit during the gate's settle window, and the
  test fails naming the URL case

#### Scenario: Timeout case under load
- **WHEN** the query behind a held connection uses `connection_limit=1&pool_timeout=1` and the machine is loaded
- **THEN** the holder is released only after the query returned, the query fails with `P2024` and the original's
  message, and the measured wait is at least 1 s and below 3 s

#### Scenario: Success case under load
- **WHEN** the query behind a held connection uses `connection_limit=1&pool_timeout=1&pool_timeout=5` and the recorded
  hold is 3 s
- **THEN** the query is still pending when the test releases the holder after 3 s, and it then succeeds

#### Scenario: Holder in place before the query
- **WHEN** a connect-timeout test needs the only pooled connection held by a transaction
- **THEN** the query under test starts only after the holder has signalled that it holds the connection

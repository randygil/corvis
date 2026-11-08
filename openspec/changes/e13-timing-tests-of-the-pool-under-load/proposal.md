# Proposal

Implements [randygil/corvis#147](https://github.com/randygil/corvis/issues/147) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

Postgres pool tests in corvis-orm fail now and then when the machine is busy (several workspace test runs or port
builds at once) and pass when rerun alone. Latest failure: `pg_pool_url::builder_wins_over_the_url`
(`crates/corvis-orm/tests/pg_pool_url.rs:101`, peak below the expected `(4, 4)`). The cause is in the tests, not in
the pool:

- `peak` (`pg_pool_url.rs:25-60`) runs `n` queries of `pg_sleep(0.3)` and samples `pg_stat_activity` every 30 ms. It
  only sees the limit if all pooled connections are inside their 300 ms sleep at the same instant *and* a sample lands
  then. Under load, connections open late, the sleeps do not overlap, or the sampler's own query is slow, and the peak
  reads lower than the real limit.
- `recorded_waits_match` (`pg_pool_url.rs:142-160`) compares the wait behind a held transaction with the original's
  recording within ±500 ms, in both directions. Load can only make waits longer, so the upper side fails.
- Several `pg_connect_timeout.rs` tests start a holder task and then `sleep(300 ms)` instead of waiting for it to hold
  the connection, or cancel a transaction after a fixed 200 ms, or bound a duration far tighter than what they need to
  distinguish (`< 1 s`). The failing one was not captured, so all of them are reviewed.

A flaky suite hides real regressions and costs reruns on every workspace run.

## What Changes

- **Hold connections with a gate, not with time.** A test helper holds a Postgres advisory lock in a separate
  transaction; the queries under test block on that lock at the server, so they stay in flight until the test
  releases them. An observer connection waits until the expected number of queries is blocked (with a long deadline
  that only turns a hang into a clear failure), keeps sampling for a short settle window to catch any excess, then
  releases. The limit is observed for as long as needed, whatever the load.
- **Waits measured against events.** Holder transactions are released by the test (a oneshot), not by a sleep. Timeout
  cases release only after the query has failed, so the query can only end by timing out. Success cases check that
  the query is still pending at the recorded hold time, then release and require success.
- **Asymmetric bounds.** Lower bounds stay tight (timers never fire early, and load only adds time). Upper bounds sit
  below the nearest value a wrong implementation would produce (the other value of a repeated key, the 10 s default
  `pool_timeout`, the 5 s default `connect_timeout`, or no limit), instead of a symmetric tolerance around the
  original's timing.
- **Handshakes instead of sleeps** in `pg_connect_timeout.rs` (`waiting_for_a_slot_is_not_cut_by_the_connect_timeout`,
  `pool_timeout_still_bounds_the_wait_for_a_slot`, `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back`)
  and a deterministic slot check in `cancelled_queries_and_scripts_do_not_leak_slots`.
- **Robust counting:** backends are counted with `backend_type = 'client backend'` and without the observer and holder
  connections, so an autovacuum worker or the helper itself cannot change the count.
- What the tests prove stays the same: the pool size per URL (and builder) case, the wait bound, the error codes and
  messages recorded from the original.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `orm/connection-pool`: adds a requirement on how the pool size and wait rules are verified (deterministic under
  machine load). No runtime behavior changes.

## Impact

- Tests only: `crates/corvis-orm/tests/pg_pool_url.rs`, `crates/corvis-orm/tests/pg_connect_timeout.rs`, and a new
  helper `crates/corvis-orm/tests/common/gate.rs` (included with `#[path]` like `common/pg.rs`).
- No change to `crates/corvis-orm/src/`, to the fixtures, or to any public API; no docs page describes these tests.
- No framework runtime change: the corvis-parity check does not apply.
- Test duration: about the same (the sleeps of 0.3 s per wave are replaced by gate waits that end as soon as the
  observer confirms the state).

## Out of scope

- The server-side connection budget: `recorded_connection_counts_match` opens up to `2 * physical cores + 1`
  connections for the `default` case (49 on the recording machine), and parallel workspace runs from other sessions
  can reach Postgres' `max_connections`. That is a resource limit, not a timing race (possible follow-up issue).
- Timing assertions in the crate's unit tests (`crates/corvis-orm/src/client_pool_tests.rs:163,177,204,217,228`),
  which use a local silent listener and were not reported as flaky.
- Other timing-based suites in the workspace (scheduler, WebSocket); #138 covered the scheduler races.
- Running these tests serially or marking them `#[ignore]`: not needed once they are deterministic (design.md,
  decision 6).

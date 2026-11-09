# Design

## Context

### `crates/corvis-orm/tests/pg_pool_url.rs`

- `peak(base, db, n)` (lines 23-60): opens a monitor client with `connection_limit=1`, spawns a poller that every
  30 ms runs `SELECT count(*) FILTER (WHERE query LIKE '%pg_sleep%' AND state = 'active') AS a, count(*) AS b FROM
  pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid()` and keeps the maxima; spawns `n`
  tasks of `SELECT 1 FROM pg_sleep(0.3)` on `db`; waits for them; stops the poller. `b` counts every backend of the
  database, including autovacuum workers (no `backend_type` filter).
- Users of `peak`: `recorded_connection_counts_match` (lines 62-85, every `concurrency` case of the fixture except
  `connection_limit=0` and `connection_limit=4294967296`; `default` expects `min(2 * physical cores + 1, n)`),
  `limit_of_two_serves_six_queries` (87-94), `builder_wins_over_the_url` (96-103, `connection_limit=2` in the URL,
  `max_connections(4)`, 12 queries, expects `(4, 4)`: the reported failure at line 101),
  `tls_parameter_is_kept_and_limit_applies` (105-114).
- `wait_behind_transaction(url, hold)` (lines 116-140): a holder task opens `db._transaction`, runs `SELECT 1`, sends
  on a oneshot, then `sleep(hold)`; the test awaits the oneshot, times `db.query_raw("SELECT 1")`, and returns the
  result and the wait. `recorded_waits_match` (142-160) asserts `|waited - waited_ms| < 500 ms` for every `wait` case,
  then the success, or the error code and message prefix.
- The fixture `tests/fixtures/pool_url.json` (`wait` section) has: `connection_limit=1&pool_timeout=1 (hold 3s)` ->
  `P2024` after 1015 ms; `pool_timeout=0 (hold 2s)` -> ok after 2011 ms; `pool_timeout=3 (hold 1s)` -> ok after 1007
  ms; `pool_timeout=1&pool_timeout=5 (hold 3s)` -> ok after 3015 ms; `pool_timeout=5&pool_timeout=1 (hold 3s)` ->
  `P2024` after 1011 ms. The error messages end with `(Current connection pool timeout: 1, connection limit: 1)`.

### `crates/corvis-orm/tests/pg_connect_timeout.rs`

| Test | Lines | Timing logic | Race under load |
|---|---|---|---|
| `waiting_for_a_slot_is_not_cut_by_the_connect_timeout` | 28-50 | holder sleeps 3 s; test sleeps 300 ms, then asserts the query waited >= 2000 ms | no handshake: if the holder has not begun its transaction within 300 ms, the query takes the connection first and returns at once |
| `pool_timeout_still_bounds_the_wait_for_a_slot` | 52-76 | same holder; asserts `P2024` with 900 ms <= wait < 2500 ms | same missing handshake (then `unwrap_err` panics); upper bound 1.5 s above the timeout |
| `role_at_its_connection_limit_fails_after_the_connect_timeout` | 78-94 | 900 ms <= elapsed < 3 s, `connect_timeout=1` | upper bound only 2 s above the timeout |
| `server_lost_after_start_fails_the_next_query_after_the_connect_timeout` | 96-118 | elapsed < 4 s, `connect_timeout=1`, `pool_timeout=0` | upper bound 3 s above |
| `a_schema_the_database_cannot_encode_is_reported_at_once` | 120-145 | two calls, each < 1 s, default `connect_timeout` (5 s) | tightest bound in the file: connecting under load can take more than 1 s |
| `cancelled_queries_and_scripts_do_not_leak_slots` | 147-166 | final pair of queries < 2 s | timing does not even detect one leaked slot: with 1 of 2 slots left the two queries run one after the other, both fast |
| `fixture_silent_server_timings_match` | 209-237 | `|got - s| < 1 s` for `s` in {5, 1, 2, 3, 1} | upper side under load; the lower side is loose (`0.05 s` passes for `s = 1`) |
| `fixture_refused_port` | 239-261 | `|took - limit| < 1 s` for limits 1, 5, 2 | upper side under load |
| `fixture_database_side_failures` | 311-334 | 900 ms <= took < 3 s | upper bound 2 s above |
| `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back` | 336-365 | cancels after a fixed 200 ms, then asserts the next query waited >= 1500 ms | if the transaction has not sent `pg_sleep(3)` within 200 ms, nothing holds the connection and the next query returns at once |

### Library facts the design relies on

- `OrmClient::_transaction` waits at most `maxWait` (2 s) to start and is rolled back after `timeout` (5 s)
  (`crates/corvis-orm/src/client.rs:507-528`); `TransactionOptions::max_wait` / `timeout` change them
  (`src/transaction.rs:89-97`). A gate held longer than 5 s needs an explicit `timeout`.
- Defaults: `pool_timeout` 10 s (`src/client.rs:157`), `connect_timeout` 5 s (`src/client.rs:159`).
- Each test uses its own database from `pg::fresh_db` (`tests/common/pg.rs:22-56`); advisory locks are scoped to
  the current database, so keys cannot collide across tests.
- `peak`'s existing query shape `SELECT 1 FROM pg_sleep(0.3)` shows that a `void` function in `FROM` works through
  `query_raw`; the gate uses the same shape.

## Goals / Non-Goals

**Goals:**

- `pg_pool_url` and `pg_connect_timeout` pass on a loaded machine, every time, without retries.
- Each test still proves what it proves today (pool size per case, wait bound, error code and message) and still fails
  for the implementation bugs it targets.

**Non-Goals:**

- Changing the pool, the fixtures, or the runtime crates.
- Making other suites deterministic.

## Decisions

1. **A server-side gate holds the queries under test.** New helper `tests/common/gate.rs`:
   - `Gate::close(base, key) -> Gate`: a holder client (`connection_limit=1`) starts
     `_transaction_with(TransactionOptions::new().max_wait(30_000).timeout(120_000), ..)`, runs
     `SELECT 1 FROM pg_advisory_xact_lock(<key>)` and `SELECT pg_backend_pid()::int8 AS pid`, signals on a oneshot,
     and waits on a second oneshot before returning `Ok`. The holder's PID is kept. An observer client
     (`connection_limit=1`) is opened and connected.
   - `Gate::query_sql(key)`: `SELECT 1 FROM pg_advisory_xact_lock_shared(<key>)`, the statement the tests run instead
     of `pg_sleep`. It blocks at the server while the holder's exclusive lock is held and returns at once afterwards;
     the shared transaction-level lock ends with the statement, so later waves never block.
   - `Gate::sample()`: one observer query returning `(blocked, backends)`: `blocked` from `pg_locks` (`locktype =
     'advisory'`, `NOT granted`, `objid = <key>`, the current database's OID), `backends` from `pg_stat_activity` with
     `datname = current_database()`, `backend_type = 'client backend'`, and `pid` not the observer's nor the holder's.
   - `Gate::wait_for(expected, tasks)`: samples every 20 ms, keeping maxima, until `blocked == expected`; then keeps
     sampling for a settle window of 250 ms; returns the maxima. If one of `tasks` (the spawned query handles) finishes
     before the count is reached, it fails at once with that task's result: a gated query can only finish early by
     an error. Reaching the deadline (30 s) fails with
     `only <blocked> of the expected <expected> queries reached the server within 30 s`. The deadline is a hang guard,
     not a tolerance: on a correct pool it is never reached.
   - `Gate::open()`: sends on the release oneshot and awaits the holder.
   - Why it is deterministic: the blocked count rises to the pool limit and stays there until the test releases it, so
     the observer sees it however slow connections, scheduling or the observer itself are. Exceeding the limit can only
     add samples above it, so the upper check can miss a violation that appears after the settle window but can never
     fail a correct pool.
   - Alternative: hold each pooled connection in an interactive transaction that waits on a `tokio::sync::Barrier`.
     Rejected: it tests transactions instead of the raw queries the original was recorded with, and the queued tasks
     would hit `maxWait`.
   - Alternative: longer `pg_sleep` and faster sampling. Rejected: still a race, only a less likely one.

2. **`peak` becomes `gated_peak(base, db, n, expected)`.** It closes a gate, spawns `n` tasks of
   `db.query_raw(Gate::query_sql(key))`, calls `wait_for(expected)`, opens the gate, awaits every task (each must
   succeed), and returns `(max_blocked, max_backends)`. Callers keep their assertions `== (expected, expected)`; the
   `default` case keeps `min(2 * physical cores + 1, n)`. `expected` is passed in so the wait knows when the state is
   reached; the assertion still compares the observed maxima with it.

3. **Waits are released by the test.** `wait_behind_transaction` keeps the handshake it already has and replaces
   `sleep(hold)` with a release oneshot. The holder transaction gets `timeout(120_000)`. Per case:
   - error expected (`ok: false`): the query runs to completion while the holder still holds; then the holder is
     released. Assertions: code and message as today; `waited >= T - 50 ms` and `waited < T + 2 s`, where `T` is the
     timeout parsed from the recorded message (`Current connection pool timeout: <T>`). Nearest wrong values: 5 s (the
     other value of a repeated key) and 10 s (default), both above `T + 2 s = 3 s`.
   - success expected: the query is spawned, the test sleeps the recorded hold (from the key, `(hold <h>s)`), asserts
     the query task is not finished, releases, awaits, and requires `Ok` and `waited >= h`. The margin to a false
     timeout is `pool_timeout - h`: 2 s for `pool_timeout=3 (hold 1s)` and `pool_timeout=1&pool_timeout=5 (hold 3s)`,
     unlimited for `pool_timeout=0`. No upper bound: once released the query only needs a free connection.
   - The recorded `waited_ms` stays in the failure messages for comparison.

4. **`pg_connect_timeout.rs`, per test.**
   - `waiting_for_a_slot_is_not_cut_by_the_connect_timeout`: oneshot handshake before the query; holder released by
     the test. The query is spawned, the test sleeps 2.5 s (more than twice `connect_timeout=1`), asserts the task is
     not finished, releases, and requires `Ok`. The `>= 2000 ms` assertion becomes the pending check.
   - `pool_timeout_still_bounds_the_wait_for_a_slot`: handshake; holder released after the query fails; `P2024`,
     `waited >= 950 ms` and `< 3 s` (wrong values: `connect_timeout=30`, the 10 s default); the recovery query stays.
   - `role_at_its_connection_limit_fails_after_the_connect_timeout` and `fixture_database_side_failures`: lower bound
     `>= 950 ms` kept; upper bound `< 4 s` (below the 5 s default it would show if `connect_timeout` were ignored).
   - `server_lost_after_start_fails_the_next_query_after_the_connect_timeout`: the wrong behavior is waiting forever
     (`pool_timeout=0`); keep `< 4 s`, which is 3 s above the timeout, and wrap the call in a 10 s
     `tokio::time::timeout` so a hang fails with a message instead of stalling the run.
   - `a_schema_the_database_cannot_encode_is_reported_at_once`: add `connect_timeout=30` to the URL and bound each
     call by `< 10 s`. What the test proves is that the encoding error is reported without retrying until the connect
     timeout; with 30 s the two behaviors are 20 s apart instead of 4 s. The error assertions do not change.
   - `cancelled_queries_and_scripts_do_not_leak_slots`: replace the final timed pair with a gate check: two queries of
     `Gate::query_sql` must both be blocked at the server at once (`wait_for(2)`), then the gate opens and both succeed.
     A leaked slot leaves only one blocked query while the second waits for a slot and fails with `P2024` after
     `pool_timeout=3`; `wait_for` takes the spawned task handles and stops with a failure as soon as one of them has
     finished before the expected count was reached, so a leak fails the test in about 3 s instead of passing
     silently.
   - `fixture_silent_server_timings_match`: `got >= s - 0.05` (the configured value equals the rounded original for
     every case) and `got < s + 1.5`; for `s = 3` (`connect_timeout=1&connect_timeout=3`) that is below the 5 s
     default.
   - `fixture_refused_port`: upper side `took < limit + 1.5`; lower side kept at `limit - 1.0` (the retry loop may give
     up one backoff before the deadline).
   - `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back`: instead of a fixed 200 ms, an observer
     client polls `pg_stat_activity` until the `pg_sleep(3)` of the transaction is `active`, then drops the transaction
     future. The `>= 1500 ms` lower bound keeps a margin of 1.5 s minus the observer's detection delay; the observer
     query also uses a 10 s deadline.

5. **Bound policy written once.** A short comment block at the top of each file states the rule (tight lower bound;
   upper bound below the nearest wrong value, named in the assertion message) so later tests follow it.

6. **No serial runs, no `#[ignore]`.** Once nothing depends on overlapping sleeps or fixed delays, the remaining
   duration checks have margins of at least 1.5 s on the side load affects. A process-wide mutex inside a test binary
   would not help against other binaries and other sessions, which is where the load comes from.

## Risks / Trade-offs

- [Advisory lock semantics] -> `pg_advisory_xact_lock_shared` waits behind an exclusive holder and shared holders do
  not block each other (Postgres documentation, "Advisory Locks"); task 1.2 checks the gate against a pool of 2
  before the tests move to it.
- [The settle window can miss an over-limit connection that appears late] -> accepted: it cannot cause a false
  failure, and a pool without a cap would put all `n` queries on the server within milliseconds of the first ones.
- [Success-case waits still have a 2 s margin to a false `P2024`] -> a 2 s scheduling delay of a sleeping Tokio task is
  far beyond what was observed; the hold values come from the fixture and are kept.
- [Many connections in the `default` case] -> unchanged from today; see Out of scope.

## Open questions

1. Settle window length: 250 ms (recommended) keeps the suite fast; a longer window catches late extra connections
   with more certainty but adds time to each of the 12 gated cases.
2. `a_schema_the_database_cannot_encode_is_reported_at_once`: is adding `connect_timeout=30` acceptable (recommended),
   or must it keep the default URL and use `< 4 s`?
3. If the maintainer wants a permanent load check, should CI run `pg_pool_url` and `pg_connect_timeout` a second time
   alongside a CPU burner? Recommendation: no; the stress run of task 4.1 is a one-off verification.

## Review notes (2026-11-08)

1. A 250 ms settling window is accepted.
2. Adding `connect_timeout=30` to the URL of the encoding test is accepted.
3. No permanent under-load run in CI; the stress run stays a manual task.

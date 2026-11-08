# Design

## Context

See proposal.md. Two intermittent tests; the investigation showed different causes.

### Postgres test: evidence

1. `security-<hash>.exe delete_and_upsert` looped 60 times: 2 failures, both at `security.rs:401` with `os error
   10022`. The test fails alone, so parallel test files are not needed.
2. A raw Tokio probe (no sqlx, 8 worker threads, 3200 sequential `TcpStream::connect` + `set_nodelay(true)` against
   the same Postgres port) failed once per run in `set_nodelay` with `os error 10022`, never in `connect`. On Windows
   `setsockopt` returns `WSAEINVAL` for a socket whose connection was already reset by the peer (here the Docker port
   proxy).
3. sqlx 0.8.6 `connect_tcp` does exactly `TcpStream::connect(..).await?; stream.set_nodelay(true)?`
   (`sqlx-core/src/net/socket/mod.rs:200`), and the pool's connect loop retries only `Error::Io` with kind
   `ConnectionRefused` and transient database errors (`pool/inner.rs:374-377`). Any other I/O error is returned to
   the caller, so `OrmClient` surfaced it as `P1001 Can't reach database server`.

So the failure comes from the connection being opened, not from the race. The test's 20 tasks share one pool of 10,
so the pool opens up to 10 connections at once and the chance of hitting the window grows with concurrency. The
test's own use of the pool is correct.

### Scheduler test: analysis

The cron expression `* * * * * *` ticks on whole seconds, so the first tick comes `t0` in [0, 1] s after `start(true)`.
The test ran a 2.5 s job, observed for 3.1 s, and asserted exactly one start with `wait_for_completion`. With `t0` <=
0.1 s a fourth tick at `t0 + 3` falls inside the window after the first run ended (`t0 + 2.5`), so a second run
starts legitimately and the assertion fails (about 10% of runs; 3 failures in 50 were observed). The overlap case
(`>= 3`) has only 0.1 s of margin as well.

## Decisions

### Decision 1: retry only the acquisition of a PostgreSQL connection

`db::retry_connect` repeats an async connect operation while it fails with `sqlx::Error::Io` whose kind is not
`ConnectionRefused`, up to 4 times (5 attempts), sleeping 20, 40, 80 and 160 ms. `acquire_pg` applies it to
`PgPool::acquire`. It is used for: the eager first connection of `PgPoolOptions::connect`, the pool executor path of
`fetch_driver`, `execute_driver` and `execute_script_raw`, and `DbPool::begin` (`acquire` followed by
`Transaction::begin`).

- Safe: nothing has been sent when a connection is being opened, and `acquire` yields a connection only after the
  handshake. A failure of the query itself is a different error (`Database`, or `Io` from an open connection) that
  is never retried because the statement runs after `acquire` returned.
- Not retried: `ConnectionRefused` (sqlx handles it until `acquire_timeout`), `PoolTimedOut`, `Tls`, `Database`.
- When the attempts run out, the last error is returned unchanged and maps to the same `P1001` as before, so a
  server that is really down fails with the same message about 300 ms later in the worst case.
- Alternative rejected: `connect_lazy` or a custom `Connection` wrapper. sqlx 0.8 offers no hook on the connect
  error, and lazy connection changes when initialization errors are reported.
- Alternative rejected: patching sqlx. Out of scope, and the retry also covers other transient socket errors.

The pool still opens its extra connections inside `acquire`, so each of those goes through `acquire_pg` too.

### Decision 2: the Postgres test helper retries before skipping

`tests/common/pg.rs::fresh_db` skips a test when Postgres is not reachable. It now retries non-refused I/O errors
the same way, so a transient error cannot make a test pass silently by skipping. A refused connection still skips
(Postgres is down).

### Decision 3: scheduler test timings that hold for any phase of the first tick

Job duration 4.5 s, observation window 3.5 s. For any `t0` in [0, 1]:

- ticks at `t0`, `t0 + 1`, `t0 + 2` (<= 3.0 s) are inside the window: at least three, with 0.5 s of margin;
- the first run ends at `t0 + 4.5` >= 4.5 s, after the window: it is still running when the window closes, with 1 s
  of margin.

With `wait_for_completion` the assertion stays "exactly one start" (the two or more skipped ticks prove the
skipping); without it, "at least three starts" (proves the runs overlap). The assertions are not weakened; only the
durations change, and the test takes about 7 s instead of 6.2 s.

## Risks

- A retry hides a flaky network for up to about 300 ms per acquisition. Accepted: the error is returned after the
  last attempt, and applications gain robustness.
- The scheduler test depends on real time; the margins above are 0.5 s and 1 s on a loaded machine.

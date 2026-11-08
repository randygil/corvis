# Design

## Context

See proposal.md for motivation; the behavior is in the `orm/connection-pool` spec delta. This change builds on #133
and #137, which are implemented but not merged: the line references below are to their branch
(`worktree-agent-a4b0fde783a7ea7a2`, working tree of 2026-11-08). sqlx is 0.8.6 (`Cargo.lock`). Every statement below
was checked in that code.

**corvis-orm (with #133 and #137)**

- `crates/corvis-orm/src/url_params.rs` is the single URL parser. `PoolTimeout { Seconds(u64), Unlimited }` (`:8-13`),
  `UrlParams { connection_limit, pool_timeout, schema }` (`:15-22`), `invalid(what, value)` builds the `P1013` message
  and echoes only short plain values (`:47-58`), `parse_timeout` maps digits to seconds, `0` and `u64` overflow to
  `Unlimited`, anything else to `None` (`:103-111`). `split_url_params(url, dialect)` (`:124-153`) decodes each key and
  matches `connection_limit`, `schema` and `pool_timeout` (`:140-149`); an invalid `pool_timeout` is ignored on SQLite
  and fails on Postgres (`:143-147`); other pairs are kept byte for byte (`:148`, `:151`).
- `crates/corvis-orm/src/client.rs`: `OrmClientOptions::acquire_timeout` "Timeout to acquire a connection. Overrides the
  `pool_timeout` URL parameter" (`:36-38`). `ClientInner` holds `driver_url` and `params` (`:139-146`).
  `UNLIMITED_POOL_TIMEOUT` is `u32::MAX` seconds because sqlx panics on `Instant + Duration::MAX` (`:148-150`);
  `DEFAULT_POOL_TIMEOUT` is 10 s (`:151-152`). `with_options` parses the URL once (`:202-213`).
- `OrmClient::pool` (`client.rs:263-323`) builds the pool lazily under a write lock. One `acquire_timeout`
  (code > URL > 10 s, clamped, `:276-283`) is given to sqlx for both dialects. Postgres (`:308-319`):
  `PgPoolOptions::max_connections(..).acquire_timeout(..)`; with `schema`, an `after_connect` hook runs
  `SET search_path TO "<schema>"` (`:310-317`); then `db::retry_connect(|| pool_opts.clone().connect(url))` and any error
  becomes `init_error` (`:318`), which is `P1001` "Can't reach database server at `host:port`: {e}" (`:717-722`, host
  from `server_of`, `:726-739`).
- Every later Postgres connection comes from `db::acquire_pg` (`crates/corvis-orm/src/db.rs:252-254`), which is
  `retry_connect(|| pool.acquire())`: `fetch_driver` (`client.rs:353-373`), `execute_driver` (`:375-395`),
  `execute_script_raw` (`:413-443`, which `detach()`es the connection and closes it after the script, `:434-436`) and
  `DbPool::begin` (`db.rs:209-216`). Their sqlx error is put in the inner result, so a normal query maps it with
  `map_sqlx` and a raw query with `from_sqlx_raw`, which turns any `sqlx::Error::Database` into `P2010` "Raw query
  failed" (`crates/corvis-orm/src/error.rs:223-248`).
- `retry_connect` (`db.rs:234-249`) repeats up to `CONNECT_RETRIES = 4` times (`:220`) only for socket I/O errors other
  than "connection refused" (`is_transient_connect_error`, `:227-229`); `PoolTimedOut` and refused connections are not
  repeated (`db.rs:859-875`, spec `openspec/specs/orm/connection-retry/spec.md`).
- `from_sqlx_other` maps `PoolTimedOut` to `P2024` and `Io` / `Tls` / `Configuration` / `PoolClosed` to `P1001`
  (`error.rs:268-283`).
- Interactive transactions wrap `pool.begin()` in `tokio::time::timeout(max_wait, ..)` (2 s default, `P2028`,
  `client.rs:519-526`); sequential batches use a one-year `max_wait` (`:573-575`). `DbTx::Postgres` holds a
  `Transaction<'static, Postgres>` built from the pool connection (`db.rs:256-259`, `:212-214`).
- `OrmClient::ensure_database` (`crates/corvis-orm/src/migrate.rs:159-184`), called by `migrate deploy` / `status`,
  opens a direct `PgConnectOptions::connect()` without any timeout (`:167`) and, for SQLSTATE 3D000, a second one to the
  `postgres` database (`:175`); any other error is swallowed (`Err(_) => Ok(())`, `:182`) so that `pool()` reports it.
- `tests/security.rs:490-511` connects to the refused port `127.0.0.1:5299` with `acquire_timeout = 2 s` and expects
  `P1001` "Can't reach database server at `127.0.0.1:5299`".
- Docs: "Connection pool" section `docs/data/corvis-orm.md:158-184`, parameter table `:169-170`, and the sentence
  "`pool_timeout=0` with an unreachable server is bounded only by the operating system's connection timeout" (`:178`);
  same lines in `docs/es/data/corvis-orm.md`.

**sqlx 0.8.6**

- `PoolInner::acquire` (`sqlx-core-0.8.6/src/pool/inner.rs:242-319`) computes `deadline = start + acquire_timeout`
  (`:248`) and wraps the whole loop in `rt::timeout(acquire_timeout, ..)` (`:250-293`): waiting for a semaphore permit,
  popping and pinging an idle connection (`check_idle_conn`, `:465-500`, `test_before_acquire` is on by default), and
  `self.connect(deadline, guard)` (`:288`). Expiry is `Error::PoolTimedOut` (`:293`).
- `PoolInner::connect` (`inner.rs:321-392`) loops until the deadline: each attempt is `rt::timeout(remaining,
  connect_options.connect())` (`:346`); on success it runs `after_connect` (`:355-359`); a hook error is only logged
  (`tracing::error!(%error, "error returned from after_connect")`, `:364`), the connection is closed hard and the loop
  retries. `Io(ConnectionRefused)` (`:374`) and database errors with `is_transient_in_connect_phase()` (`:377`; 53300
  and 57P03, `sqlx-postgres-0.8.6/src/error.rs:189-201`) are retried too; any other error returns at once (`:381`);
  expiry is `PoolTimedOut` (`:384`). Backoff doubles from 10 ms (`:390`).
- `PgConnectOptions` has no connect timeout: `PgStream::connect` calls `net::connect_tcp`
  (`sqlx-postgres-0.8.6/src/connection/stream.rs:44-47`), which is a plain `TcpStream::connect` (`sqlx-core-0.8.6/src/
  net/socket/mod.rs:199`). The only option that touches startup is `options()` (`options/mod.rs:426`), which #137
  rejected for `search_path` (no escaping; refused by PgBouncer).
- `PoolOptions::connect_with` (`sqlx-core-0.8.6/src/pool/options.rs:537-558`) creates the pool and runs one
  `inner.acquire()`; `connect_lazy_with` (`:579-582`) creates it without connecting. `PoolOptions::acquire_timeout`
  (`:261`) is fixed for the pool's life.
- The pool has no public hook before a connect attempt, no way to add an externally opened connection, and its
  connect options type is fixed to `PgConnectOptions`, so sqlx cannot be told to time the queue and the connect
  differently. Public pieces that help: `Pool::acquire` (cancel-safe: a dropped acquire drops the half-open connection,
  `pool/mod.rs:340-360`), `PoolConnection::return_to_pool` (`connection.rs:134`), `PoolConnection::detach`
  (`connection.rs:113`, releases the pool permit, so the pool may open a replacement while the detached connection is
  still open).
- `after_connect` runs inline in the future of the `acquire()` call that opens the connection (`inner.rs:288` →
  `:355-359`), so it runs in the caller's task.
- A dropped `PoolConnection` returns to the pool in a spawned task that pings it first (`connection.rs:200-210`,
  `:275-332`); the sqlx permit is held until that ping finishes.

**Original client (6.x query engine)**

- The engine binary of `@prisma/client` 6.19.3 (`corvis-tests/nest/facten-api/node_modules/@prisma/engines/
  query_engine-windows.dll.node`) lists the Postgres URL keys of `quaint\src\connector\postgres\url.rs` as
  `pgbouncer sslcert sslidentity socket_timeout connect_timeout pool_timeout max_connection_lifetime
  max_idle_connection_lifetime channel_binding`, followed by "Discarding connection string param" for any other key. The
  error kind `ConnectTimeout` sits next to the user-facing `DatabaseNotReachable`, whose text is "Can't reach database
  server at `host`:`port`  Please make sure your database server is running at `host`:`port`." (`P1001`). The 5 s
  default and `0` = no limit are the original's documented behavior (connection URL reference of the 6.x docs); the
  binary does not show the default, so the fixture records it (task 5.1).
- The SQLite parameter parser of the engine (`quaint\src\connector\sqlite\params.rs`) does not list `connect_timeout`;
  #133's fixture showed that the same parser accepts and ignores any `pool_timeout` on SQLite
  (`openspec/changes/e13-connection-limit-and-pool-timeout-in-the-database-url/design.md`, "Fixture results"). The
  same is expected for `connect_timeout`; the fixture confirms it.
- In the 7.x line (consi, houseplan) the client connects through `pg`, where `connect_timeout` in the URL only reaches
  native libpq and the JS client uses `connectionTimeoutMillis`, default `0`
  (`corvis-tests/nest/consi-backend/node_modules/pg/lib/client.js:114`, `connection-parameters.js:127-129`). This change
  follows the 6.x engine, as #133 did.

## Goals / Non-Goals

**Goals:**
- `pool_timeout=0` never means "hang forever" when the server is unreachable, silent, full or rejecting the setup
  statement, unless `connect_timeout=0` asks for it too.
- The wait for a free slot and the time to open a connection are bounded by different settings, with the original's
  defaults (10 s and 5 s).
- A setup statement that the database rejects is reported with the database's message, at once.
- `connect_timeout` in the URL follows the parsing, validation and SQLite rules of the other URL parameters.

**Non-Goals:**
- A per-query timeout (`socket_timeout`) or connection lifetimes.
- Reproducing the original's per-cause error codes (`P1000`, `P1003`, `P2037`).
- Changing how SQLite connections are opened or timed.

## Decisions

1. **Parse `connect_timeout` in `split_url_params`.** Add `"connect_timeout"` to the key match of `url_params.rs:140-149`
   and `connect_timeout: Option<Timeout>` to `UrlParams`. `PoolTimeout` is renamed `Timeout` (it now serves two keys;
   mechanical rename) and `parse_timeout` is reused: digits only after form decoding, `0` and `u64` overflow are
   `Unlimited`. On Postgres an unparsable value fails with
   `invalid("`connect_timeout` must be a whole number of seconds", ..)` (`P1013`, same echo rule); on SQLite any value is
   accepted and stored as `None` (decision 8). The last occurrence wins, keys are decoded before matching, the pair is
   removed from the driver URL. Doc comments of the module and of `split_url_params` list the new key.
   - Alternative: parse it separately in `pool()`. Rejected: one parser is the rule of #133 and #137, and an invalid
     value must fail at creation, not on the first query.

2. **Separate the two waits with a slot semaphore in front of the sqlx pool.** This is the core of the change.
   - A Postgres pool becomes a small handle: `DbPool::Postgres(PgHandle)` with
     `PgHandle { pool: PgPool, slots: Arc<tokio::sync::Semaphore>, pool_timeout: Option<Duration>,
     connect_timeout: Option<Duration>, server: Arc<str> }` (`None` = no limit; `server` is `server_of(driver_url)` for
     messages). `slots` has `max_connections` permits, the same number as the sqlx pool.
   - `acquire_pg(&PgHandle)` does two steps. Step 1, the queue: `slots.clone().acquire_owned()`, under
     `tokio::time::timeout(pool_timeout)` when finite; expiry gives `P2024` with the existing text. Step 2, the
     connection: `retry_connect(|| pool.acquire())`, under `tokio::time::timeout(connect_timeout)` when finite, with
     sqlx's own `acquire_timeout` also set to `connect_timeout` (or `UNLIMITED_POOL_TIMEOUT`) as a second guard. Because
     the request already holds one of `max_connections` slots, sqlx finds a permit at once (see Risks for the short
     return window), so its deadline in practice covers only the idle check and the connect loop of
     `inner.rs:321-392`.
   - The slot is held as long as the connection is in use: `acquire_pg` returns `PgConn { conn: PoolConnection<Postgres>,
     slot: OwnedSemaphorePermit }` (deref to `PgConnection`; `detach()` returns the raw connection together with the
     slot, so `execute_script_raw` keeps it until the connection is closed), and `DbTx::Postgres` keeps the slot next to
     its `Transaction` until commit, rollback or the transaction timer drops it.
   - The semaphore is FIFO (tokio's is fair), as the sqlx queue is.
   - Alternative A, wrap the first acquire in `tokio::time::timeout(connect_timeout)`: fixes `connect()` on an
     unreachable server, which is the issue's first example, but every later acquire still waits up to `pool_timeout`
     (forever with `0`) when the server goes away after start. Rejected as incomplete; it is kept as part of decision 4.
   - Alternative B, a connect check in `acquire_pg` / `retry_connect` alone (for example, timing out every acquire at
     `min(pool_timeout, connect_timeout)`): an acquire that is only waiting behind busy connections cannot be told apart
     from one that is stuck connecting, so a busy pool would fail with `P1001` after 5 s although `pool_timeout=0` asks
     to wait. Rejected.
   - Alternative C, a `PgConnectOptions` setting: sqlx 0.8.6 has none for connect or socket timeouts; the only startup
     knob is `options()`, which does not bound anything. Not available.
   - Alternative D, `connect_lazy_with` plus a bounded first acquire: same coverage as A (only the first connection).
     Used inside decision 4 to give the first connection the same path as the others, not as the mechanism.
   - Alternative E, keep `acquire_timeout = pool_timeout` and, when an acquire has waited longer than
     `connect_timeout`, probe the server with one direct connection (single flight per client) and fail the waiters if
     the probe fails: no new types and it yields the real error, but it opens extra connections while the pool is
     saturated, can fail healthy waiters when the server is merely full, and depends on a probe matching what the pool
     sees. Rejected; kept in mind as a fallback if decision 2 proves too invasive.
   - Alternative F, a custom pool in corvis-orm or a patched sqlx: far larger than the issue; rejected.

3. **Setup failures fail fast with the real error.** The `after_connect` hook keeps running `SET search_path` (#137
   decision 2) but no longer lets sqlx swallow the error.
   - `acquire_pg` creates a per-request `Arc<SetupFailure>` (a `Mutex<Option<sqlx::Error>>` plus a `tokio::sync::Notify`)
     and runs step 2 inside a `tokio::task_local!` scope that holds it. The hook looks up the task-local: on error it
     stores the error and, when it is `sqlx::Error::Database` (the server answered: the same statement will fail on every
     attempt), calls `notify_one`; it then returns an error to sqlx as before. Step 2 is a `tokio::select!` between the
     sqlx acquire and the notification: a notification returns the stored error at once and drops the sqlx acquire,
     which closes the half-open connection (sqlx's documented cancellation behavior).
   - Socket errors in the hook are not final: sqlx retries them within `connect_timeout`; if the bound then expires the
     stored (last) error is added to the timeout message.
   - The error becomes `OrmError::Initialization` `P1001` built by `init_error` (`client.rs:717-722`) with the handle's
     `server`: "Can't reach database server at `host:port`: error returned from database: ...". It is returned through
     the outer `Result` of `fetch_driver` / `execute_driver` / `execute_script_raw` / `begin`, never through the inner
     driver result, so raw queries do not turn it into `P2010` and normal queries do not map its SQLSTATE.
   - The task-local works because sqlx runs `after_connect` inline in the acquiring task (Context). If a future sqlx
     moves connects to a background task, the hook finds no task-local and behaves as today (logged, retried, bounded by
     `connect_timeout`); the regression test of task 3.3 catches it.
   - Alternative: one client-wide "last setup error" slot instead of a task-local. Simpler, but an unrelated request
     could read or be woken by another request's failure. Rejected.
   - Alternative: send `search_path` as a startup parameter so a bad value fails the connect itself (sqlx returns
     non-transient connect errors at once, `inner.rs:381`). Rejected for the reasons of #137 decision 2 (no escaping in
     `PgConnectOptions::options`, PgBouncer refuses the `options` startup parameter).

4. **The first connection takes the same path.** `pool()` builds the sqlx pool with `connect_lazy_with`
   (`PgConnectOptions::from_str(driver_url)`; a parse error still becomes `init_error`), wraps it in `PgHandle`, and
   calls `acquire_pg` once, dropping the connection back to the pool. A refused, silent or rejecting server therefore
   fails `connect()` after `connect_timeout` with the same `P1001` as later acquires; the pool is stored only after that
   first acquire succeeds, as today. Concurrent first callers wait on the `pool()` write lock, at most `connect_timeout`.
   - Alternative: keep `retry_connect(|| pool_opts.connect(url))` and wrap it in `tokio::time::timeout`. Equivalent in
     time, but it would need its own error capture for the hook; one path is simpler to test.

5. **Error messages and codes.**
   - Slot wait expired: `P2024`, existing text (`error.rs:270-274`).
   - Connect bound expired: `P1001`, "Can't reach database server at `host:port`: no connection within N s
     (`connect_timeout`)", plus "; last error: {e}" when the hook stored one. Never the URL, user or password.
   - Setup statement rejected: `P1001` via `init_error` with the database message (decision 3).
   - Other driver errors while connecting (TLS failure, `28P01`, `3D000`, non-transient socket errors after
     `retry_connect`): unchanged, they keep flowing through the inner result and today's mappings.
   - `P1001` matches the original for a connect timeout. For a rejected setup statement the original also reports
     "Can't reach database server" in the one case #137's fixture saw (a NUL in `schema`); the fixture of task 5.1
     records the 22P05 case.

6. **Refused connections and 53300 / 57P03 stay retried, inside `connect_timeout`.** sqlx retries them in its connect
   loop and corvis-orm cannot turn that off without replacing the loop. The original fails a refused connection at once
   (no retry); corvis-orm fails after `connect_timeout` (5 s by default) with the same `P1001`. This keeps apps that
   start next to a database that is still booting working, as they do today, and it is bounded. Documented as a
   difference; the fixture records the original's timing.
   - Alternative: a direct probe connection before the pool to fail fast on refusal. Rejected: an extra connection per
   client start and a second connect path with its own retry rules.

7. **`ensure_database` uses the same bound.** Both direct connects of `migrate.rs:167` and `:175` run under
   `tokio::time::timeout(connect_timeout)` (read from `self.inner.params`, default 5 s). On expiry it returns `P1001`
   at once ("Can't reach database server at `host:port`: no connection within N s (`connect_timeout`)") instead of
   swallowing it, so `migrate deploy` does not wait twice (once here, once in `pool()`). Other errors keep today's
   behavior.

8. **SQLite: accepted and ignored.** The key is removed from SQLite URLs (sqlx's SQLite parser rejects unknown keys) and
   any value is accepted, matching how the engine's SQLite parser treats keys it does not know (`pool_timeout` in #133's
   fixture). `pool()` never reads it for SQLite: opening a SQLite file does not wait on a network, and lock waits are
   `busy_timeout`'s job. The fixture confirms the original accepts `connect_timeout` on SQLite.

9. **`acquire_timeout` keeps its meaning of "`pool_timeout` in code".** It now bounds only the slot wait. Its doc comment
   (`client.rs:36-38`) says so and points to `connect_timeout` for connection setup. No code option for
   `connect_timeout` (Out of scope, open question).

## Risks / Trade-offs

- [Slot released before the sqlx permit] → a dropped `PoolConnection` releases our slot at once but the sqlx permit only
  after the spawned return ping (`connection.rs:200-210`, `:316-332`); the next request may wait that ping inside step 2,
  counted against `connect_timeout`. It is one round trip on a healthy server. If the server is so slow that a ping
  takes 5 s, failing with `P1001` is reasonable. Optional refinement for `PgConn`: on drop, spawn
  `conn.return_to_pool()` and release the slot after it completes; transactions keep the window (`Transaction` does not
  give its connection back).
- [More code on the hot path] → one uncontended semaphore acquire and one `select!` per Postgres query; negligible next
  to a network round trip.
- [Behavior change for `acquire_timeout` users] → it no longer bounds connecting. `tests/security.rs:490-511` still
  passes (refused port, `P1001`) but takes up to 5 s instead of 2 s; the task adds `connect_timeout` handling there.
- [Refused port takes 5 s instead of failing at once like the original] → decision 6; bounded and documented.
- [53300 from a full server is reported as a timeout after 5 s, with no database message] → sqlx hides the cause;
  the original reports a specific error at once. Documented; mapping codes is out of scope.
- [Dependence on sqlx internals] → decision 3 relies on `after_connect` running inline; decision 2 relies on a permit
  being free when a slot is held. Both are covered by tests that fail if sqlx changes (tasks 3.2-3.4).
- [Detached script connections] → holding the slot until the detached connection closes makes the total number of
  server connections stay within `max_connections`, which sqlx's `detach` alone does not guarantee
  (`connection.rs:100-114`). A small improvement, not a risk.
- [Disconnect while requests wait for a slot] → `disconnect()` closes the sqlx pool; waiters then get `PoolClosed`
  (`P1001`) from step 2. Closing the semaphore in `disconnect()` wakes them sooner; the task does it.

## Open questions

- Default: the 6.x docs say 5 s and `0` = no limit; the fixture (task 5.1) confirms both from the binary's behavior. If
  it shows a different default, the constant follows it.
- Does the original accept `connect_timeout` on SQLite with any value (decision 8)? Expected yes; the fixture settles
  it. If it rejects invalid values, SQLite validates like Postgres.
- What error does the original give for a rejected setup statement (22P05) and for a role at its connection limit
  (53300)? Recorded by the fixture; corvis-orm keeps `P1001` either way (code mapping is out of scope), and design.md is
  updated with the result.
- Should `OrmClientOptions` get a `connect_timeout` field, like `acquire_timeout` for `pool_timeout`? Not in this change
  (public struct with public fields; the original has no code option). A maintainer decides whether to file it.
- Should refused connections fail at once like the original (decision 6)? This change keeps sqlx's retry within the
  bound; a maintainer may prefer strict parity.

## Review notes (2026-11-08)

Maintainer decisions; they override the text above where they differ.

1. Decision 2 (slot semaphore) is accepted. A simpler variant was considered and rejected: wait without limit only while
   the pool still has live connections. It does not work because sqlx counts connections that are still being opened
   in `size()`, so a pool stuck connecting cannot be told apart from a busy one without our own slot count.
2. Implement the optional refinement from Risks: on drop of `PgConn`, release the slot only after the connection has
   gone back to the sqlx pool, so the next request does not spend its `connect_timeout` on the return ping. If this is
   not possible for `Transaction`, keep the window there and say so in the implementation notes.
3. Refused connections keep sqlx's retry within `connect_timeout` (decision 6). Strict parity (fail at once) is not
   required; document the difference.
4. No `connect_timeout` field on `OrmClientOptions` in this change.
5. The hot path changes for every Postgres query, so measure it. Run a short local loop (for example 10 000
   `SELECT 1` through the client on the local Postgres, before and after) and record both timings in the
   implementation notes. Do not use the shared benchmark setup.
6. This change starts only after #133 and #137 are merged into main (the delta extends `orm/connection-pool`, which
   #133 creates). Run the corvis-parity check before archiving.

## Implementation notes

- Tasks 1.1, 1.2 and 4.2 (the fixture recorded with the original 6.x client, its "Fixture results" section and its replay)
  were not done: recording needs Node, the scratch database objects and the maintainer's go-ahead. The behaviors it was
  meant to confirm (default 5 s, `0` = no limit, SQLite acceptance, the original's error for 22P05 and 53300) are still
  open questions above; the implementation follows the 6.x documentation for them.
- Hot path measurement (task review note 5), debug profile, local Postgres, 5 pooled connections, `query_raw("SELECT 1")`
  through the client, 500 warm-up queries, three runs of 10 000 sequential queries, and 8 tasks x 1 250 queries:
  - before (main at a380499): sequential 3728 / 3698 / 3713 ms (about 372 us per query); concurrent 1370 ms.
  - after: sequential 3990 / 3921 / 3831 ms and 3878 / 3955 / 3993 ms in a second run (about 390 us per query, +5%);
    concurrent 1351 ms and 1364 ms (no difference). The measuring test was temporary and is not part of the change.
- `PgConn` releases its slot only after `PoolConnection::return_to_pool()` has finished (spawned on drop), as review note 2
  asks. (First version: `PgTx` wrapped sqlx's `Transaction` and released the slot on drop, before the connection was
  back; see the security review fix below.)
- `report_setup_error` stores the error text (`String`) and a flag instead of a `sqlx::Error` (it is not `Clone` and only
  the text is shown); `open_connection` is generic over the acquire future and holds the timeout, the `select!` and the
  mapping of sqlx's own `PoolTimedOut` (set equal to `connect_timeout`) to the same `P1001`.
- `ensure_database` also fails with `P1001` when the second (administrative) direct connection times out.
- "Server lost after start" is tested with `ALTER ROLE .. CONNECTION LIMIT 0` plus `pg_terminate_backend`, the variant
  allowed by task 4.1; a role at its limit is reported by sqlx as a timeout, without the database's message (53300 is
  retried inside its connect loop), as design.md predicted.
- `DbPool::close` closes the slot semaphore before the sqlx pool so waiting requests fail at once.

## Fixture results

Recorded with `@prisma/client` 6.19.3 by `corvis-tests/tmp/orm/connect/cases.js` into
`crates/corvis-orm/tests/fixtures/connect_timeout.json`; replayed by `tests/pg_connect_timeout.rs` (`fixture_*`).

- Default: 5 s. A silent server fails after 5017 ms with no parameter, 1014 ms with `connect_timeout=1`, 2015 ms with
  `=2`, 3007 ms with `=1&=3` (last wins), 1003 ms with `connect_timeout=1&pool_timeout=0`. The error is
  `PrismaClientInitializationError` "Can't reach database server at `127.0.0.1:<port>`" (`P1001` on `$connect`).
- `connect_timeout=0&pool_timeout=0` against the silent server was still pending after 8 s: `0` is no limit.
- Refused port (`127.0.0.1:1`): fails after about 2 s whatever the parameters are (none, `connect_timeout=1`,
  `connect_timeout=0`, `pool_timeout=0`), with the same "Can't reach database server" text. corvis-orm retries the refused
  connection inside `connect_timeout` (decision 6) and fails with the same `P1001` after that time; with
  `connect_timeout=0` it keeps retrying where the original fails after 2 s.
- Busy pool with `connection_limit=1&pool_timeout=0&connect_timeout=1`: the second query waits the 3 s and succeeds
  (3004 ms), so waiting for a slot is not cut by `connect_timeout`.
- Setup statement rejected: the original connects to a LATIN1 database with `?schema=%E2%82%AC&pool_timeout=0` (both
  `$connect` and `$queryRaw` succeed in about 20 ms): it does not run a `SET search_path` per connection. corvis-orm,
  which does (#137 decision 2), reports `P1001` with the database's message at once. Kept and documented as a difference.
- Role with `CONNECTION LIMIT 0` (53300): the original fails after 5 ms with "Too many database connections opened:
  FATAL: too many connections for role ..." (a specific code, `P2037`, is out of scope). corvis-orm reports `P1001`
  after `connect_timeout` (sqlx retries 53300). Documented difference.
- Invalid values on Postgres: `-1`, `1.5`, `abc` and the empty value fail with "The provided database string is invalid.
  The provided arguments are not supported in database URL." `4294967296` and `18446744073709551615` are accepted;
  `18446744073709551616` and `99999999999999999999` are rejected: the original parses `u64`. The spec delta was changed to
  match for `connect_timeout` (values beyond 64 bits are `P1013`; code: `fits_u64` in `url_params.rs`). `pool_timeout`
  has the same rejection in the original (`pool_timeout=99999999999999999999` fails), but #133 made that case "no limit";
  this change leaves it alone.
- SQLite: `connect_timeout=1`, `=0`, `=abc`, `=-1` and the empty value are all accepted, as for `pool_timeout`.

Open questions answered: default 5 s and `0` = no limit (confirmed); SQLite accepts any value (confirmed); the original's
errors for 22P05 (none: it connects) and 53300 (immediate "Too many database connections opened"); refused-port timing
(about 2 s, independent of the parameters).

### Security review fixes (c5edf23)

- LOW, cancelled transaction released its slot early: `PgTx` now owns the `PgConn` and drives BEGIN, COMMIT and ROLLBACK
  through `PgTransactionManager`; when it is dropped while open it calls `start_rollback` and the `PgConn` drop waits for
  sqlx's return (rollback and ping) before releasing the slot. So the "window" for transactions is gone, and a request
  after a cancelled transaction waits for the slot (no `P1001` after `connect_timeout`, nor an unbounded wait with
  `connect_timeout=0`). Regression test: `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back`.
- INFORMATIONAL, `P1013` could echo a password fragment: when the query string contains `@` (the URL was split inside the
  user info) values are never echoed, for every parameter. Test: `values_are_not_echoed_when_the_query_holds_an_at_sign`.
- `fits_u64` (values beyond 64 bits are invalid for `connect_timeout`) stays, tested in `url_params.rs`.

# Tasks

Prerequisite: #133 (`e13-connection-limit-and-pool-timeout-in-the-database-url`) and #137
(`e13-schema-parameter-in-the-database-url`) are merged into `main`; start from that state.

## 1. Record the original's behavior

- [x] 1.1 Write `corvis-tests/tmp/orm/connect/cases.js` next to `tmp/orm/pool/cases.js`, reusing its scratch clients
  (`gen-pg`, `gen-sqlite`, generated from `../pool/schema-pg.prisma` / `schema-sqlite.prisma`) and its `describe()`
  helper, with `@prisma/client` 6.19.3. Per case record `{ ok, error: { name, code, message }, elapsed_ms }`. Cases
  (design decisions 1, 3, 5, 6, 8):
  - refused port `postgresql://u:p@127.0.0.1:1/x` with no parameter, `connect_timeout=1`, `connect_timeout=0`,
    `pool_timeout=0`;
  - a silent listener started by the script (`net.createServer` on `127.0.0.1`, port 0, accepts and never writes) with
    no parameter, `connect_timeout=1`, `connect_timeout=2`, `connect_timeout=1&connect_timeout=3`,
    `connect_timeout=1&pool_timeout=0`; and `connect_timeout=0&pool_timeout=0` recorded as "still pending after 8 s"
    (then the client is abandoned and the listener closed);
  - busy pool on the scratch database: `connection_limit=1&pool_timeout=0&connect_timeout=1`, a transaction holds the
    connection 3 s, a second query outside it (expect success after about 3 s);
  - setup statement rejected: a scratch database created `ENCODING 'LATIN1' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE
    template0`, URL `?schema=%E2%82%AC&pool_timeout=0`, with `$connect()` and with `$queryRaw`;
  - full server: a scratch role with `CONNECTION LIMIT 0`, with no parameter;
  - invalid values on Postgres: `-1`, `1.5`, `abc`, empty, `99999999999999999999`;
  - SQLite: `file:<db>?connect_timeout=1`, `=0`, `=abc`, `=-1`, `=`.
  Output `crates/corvis-orm/tests/fixtures/connect_timeout.json` with `recorded_with` and `script` in its header, as
  `pool_url.json` has. Do not run it from an agent session without the maintainer's go-ahead; it needs the local
  Postgres (`127.0.0.1:55432`) and creates and drops `orm_connect_scratch`, the LATIN1 database and the scratch role.
  Verify: the file exists, every case has a result, and the scratch objects are gone afterwards
  (`SELECT datname FROM pg_database`, `SELECT rolname FROM pg_roles`).
- [x] 1.2 Update design.md with a "Fixture results" section: the default (5 s?), `0`, SQLite handling, the original's
  error for 22P05 and 53300 and its timing for the refused port; adjust the spec delta where the fixture disagrees
  (except the documented differences of decisions 5 and 6). Verify the open questions it answers are marked answered.

## 2. Parse `connect_timeout`

- [x] 2.1 In `crates/corvis-orm/src/url_params.rs`: rename `PoolTimeout` to `Timeout` (update `client.rs` and
  `client_pool_tests.rs`), add `connect_timeout: Option<Timeout>` to `UrlParams`, match the `connect_timeout` key in
  `split_url_params` (Postgres: invalid value → `P1013` "`connect_timeout` must be a whole number of seconds"; SQLite:
  any value accepted, stored as `None`), update the module and function doc comments. Unit tests: removal with other
  parameters (order and encoding kept), `0` → `Unlimited`, overflow → `Unlimited`, repeated key last wins, decoded key
  `connect%5Ftimeout`, the invalid values of task 1.1 on Postgres (code, message, no URL parts, password-echo case), and
  SQLite acceptance of every value. Verify `cargo test -p corvis-orm --lib url_params` passes.

## 3. Separate the slot wait from the connection setup (Postgres)

- [x] 3.1 In `crates/corvis-orm/src/db.rs`: add `PgHandle` (sqlx pool, slot semaphore, `pool_timeout`,
  `connect_timeout`, `server`) and `PgConn` (pool connection + owned slot permit, `Deref`/`DerefMut` to
  `PgConnection`, `detach()` returning the raw connection with the slot); change `DbPool::Postgres` to hold `PgHandle`
  and `DbTx::Postgres` to keep the slot with its `Transaction`; rewrite `acquire_pg` as the two steps of design
  decision 2 (queue under `pool_timeout` → `P2024`; connection under `connect_timeout`, wrapping `retry_connect`).
  Close the semaphore in `DbPool::close`. Update the call sites in `client.rs` (`fetch_driver`, `execute_driver`,
  `execute_script_raw`, `begin_tx`) and `DbPool::begin`. Verify `cargo test -p corvis-orm --lib` passes and the existing
  `connect_retry_tests` still pass.
- [x] 3.2 Unit tests without a database (in `db.rs` or `client_pool_tests.rs`), using a `tokio::net::TcpListener` on
  `127.0.0.1:0` that accepts and holds sockets without writing: `connect()` with `connect_timeout=1&pool_timeout=0`
  fails in under 3 s with `P1001` and a message starting "Can't reach database server at `127.0.0.1:<port>`" that does
  not contain the password; the default fails after about 5 s; `connect_timeout=0&pool_timeout=0` is still pending after
  3 s (`tokio::time::timeout` around it); refused port `127.0.0.1:1` with `pool_timeout=0` fails within about 5 s.
  Verify these tests pass with `cargo test -p corvis-orm --lib connect_timeout`.
- [x] 3.3 Setup failures (design decision 3): a `tokio::task_local!` slot for the current acquire, filled by the
  `after_connect` hook in `OrmClient::pool`; `acquire_pg` returns the stored database error at once through
  `select!` on its `Notify`, as `P1001` via `init_error`, through the outer `Result` of the callers (not the inner
  driver result). The timeout message of step 2 appends the last stored error. Keep step 2 generic over the acquire
  future (as `retry_connect` is) so it can be tested without a server: a fake acquire that reports a database error
  through the task-local slot and then never completes must return that error at once; one that reports an I/O error
  and never completes must end at `connect_timeout` with the error in the message; a fake that never reports anything
  must end with the plain timeout message. Verify these tests pass with `cargo test -p corvis-orm --lib setup_failure`
  (the real hook is covered by task 4.1).
- [x] 3.4 `OrmClient::pool` (design decision 4): Postgres builds the pool with `connect_lazy_with`, sets sqlx's
  `acquire_timeout` to `connect_timeout` (or `UNLIMITED_POOL_TIMEOUT`), wraps it in `PgHandle` and runs `acquire_pg`
  once before storing it; `DEFAULT_CONNECT_TIMEOUT` = 5 s next to `DEFAULT_POOL_TIMEOUT`. Update the doc comments of
  `OrmClientOptions::acquire_timeout` (slot wait only) and `OrmClientOptions::url` (lists `connect_timeout`). Verify the
  tests of 3.2 and `cargo test -p corvis-orm --test client` pass.
- [x] 3.5 `OrmClient::ensure_database` (`migrate.rs`): both direct connects under `tokio::time::timeout(connect_timeout)`;
  on expiry return `P1001` at once (design decision 7). Verify with a unit test against the silent listener that
  `migrate_deploy` with `connect_timeout=1` fails in under 3 s with `P1001`.
- [x] 3.6 `tests/security.rs` `connection_errors_do_not_show_credentials`: keep its assertions; it now waits
  `connect_timeout` (5 s) per URL, so add `connect_timeout=1` where the URL form allows it, or accept the longer run and
  note it. Verify `cargo test -p corvis-orm --test security connection_errors` passes.

## 4. Postgres integration tests and fixture replay

- [x] 4.1 New `crates/corvis-orm/tests/pg_connect_timeout.rs` (skipped without a server, `tests/common/pg.rs`): the spec
  scenarios "Waiting for a slot is not cut by the connect timeout", "Role at its connection limit" (scratch role with
  `CONNECTION LIMIT 0`, `connect_timeout=1&pool_timeout=0`), "Server lost after start" (connect, then
  `pg_terminate_backend` the client's backends and point a second client at a stopped port, or use a role with
  `ALTER ROLE .. CONNECTION LIMIT 0` after the first connection so new ones get 53300), "Schema name the database
  cannot encode" (LATIN1 scratch database, `connect()` and `query_raw` both `P1001` in under 1 s, message contains "has
  no equivalent in encoding"), and "Parameter removed from the driver URL" with `sslmode=disable&connect_timeout=3&schema=s`.
  Verify it passes against the local Postgres and leaves no scratch database or role.
- [x] 4.2 Replay `tests/fixtures/connect_timeout.json` in the same file: codes, the elapsed-time class of each case
  (immediate / about N s / pending) with a tolerance, invalid-value messages compared on their first sentence, SQLite
  acceptance. Cases where corvis-orm deliberately differs (refused port timing, 53300) assert corvis-orm's documented
  behavior and name the difference in a comment. Verify the test passes.

## 5. Docs

- [x] 5.1 `docs/data/corvis-orm.md`, "Connection pool": add `connect_timeout` to the parameter table (seconds to open a
  connection, `0` no limit, default `5`, Postgres only, ignored on SQLite); state that `pool_timeout` /
  `acquire_timeout` bound only the wait for a free connection; replace the sentence "`pool_timeout=0` with an unreachable
  server is bounded only by the operating system's connection timeout" with the new behavior (`P1001` after
  `connect_timeout`, setup failures reported at once with the database's message); note the differences (refused
  connections retried within the bound, `P1001` for every connection failure); fix the closing line that maps the newer
  clients' "connection timeout" to `acquire_timeout` (it maps to `connect_timeout`); add a "changed" note for programs
  that relied on `acquire_timeout` to fail fast. Same in `docs/es/data/corvis-orm.md`. Verify `npm run docs:check` in
  `docs/` passes.

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of groups 2-4
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 6.5 Framework runtime code changed (corvis-orm connection path): run the `corvis-parity` check and verify the three
  port apps keep 0 apidiff differences (and 0 dbdiff differences) (not run by the implementer: the maintainer runs it)
- [x] 6.6 Security review of the change (`security-reviewer`): slot accounting under cancellation, transaction timeout
  and `detach`, no URL parts in any new message (security-reviewer on c5edf23: nothing critical, high or medium; the low finding (cancelled transaction released its slot early) and the informational one (value echo with `@` in the query) are fixed in 6b583c6)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after #133's change, which creates `orm/connection-pool`).
- Close issue #142, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

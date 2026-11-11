# Proposal

Implements the second issue filed from the engineering-quality review of `crates/corvis-orm` and
`crates/corvis-orm-macros` (epic [#187](https://github.com/randygil/corvis/issues/187), code quality audit). It lands after
`q-orm-correctness` (which moves the engine's key lists onto one helper and changes `schema.rs` and the typed params) and
before `q-orm-api-hygiene`. Source: a read-only review of `dev` at `d46758a`; every finding was re-read against the code
(design.md "The findings, confirmed against the code"); task 1.1 re-checks the lines after `q-orm-correctness` merges.

## Why

The review found no wrong result on these paths. It found work that grows with the data or with failures where it does
not need to:

- `createMany` (and the nested `createMany`) sends one `INSERT` per row: 10,000 rows are 10,000 round trips.
- Every Postgres statement clones its whole `Sql` (text and every parameter, `Bytes` and `Json` included) only to append a
  type comment; every bound text, byte string and JSON value is cloned again to bind it; every connection returned to the
  pool spawns a task; every SQLite cell upper-cases its type name; every row is turned into a map twice; `pk()` allocates
  on every call and relations are resolved again for every include.
- A paginated include (`take`/`skip` inside `include`) runs one query per parent, and the many-to-many include matches
  children to parents in O(parents × children).
- `distinct` reads every matching row before it applies `take`.
- While the first connection of a lazy client is being opened, every other request waits on a write lock and then makes
  its own attempt: with the database down, the k-th concurrent request waits about k × `connect_timeout`.
- A transaction's `timeout` does not stop the statement that is running, and a request that is cancelled mid-statement
  keeps its pool slot until the statement ends: long statements behind timed-out or abandoned requests can exhaust the pool.

There is no ORM benchmark, so the first task builds one, and every decision names the number it must move.

## What Changes

- **Benchmark.** `bench/orm/`: a release-mode benchmark against SQLite (a temporary file) and Postgres (`127.0.0.1`):
  `find_unique`, `find_many`, includes (plain, paginated, many-to-many), `create_many`, `create_many_and_return`,
  `update_many`, an interactive transaction, `distinct`; operations per second, latency percentiles, allocations and bytes
  per operation, statements per operation. Run before and after; the numbers go to design.md.
- **Multi-row inserts.** `createMany`, `createManyAndReturn` and the nested `createMany` group consecutive rows with the same
  columns into `INSERT .. VALUES (..), (..)` statements sized from the dialect's parameter limit, with `RETURNING` for the
  keys; same results, counts and errors.
- **Fewer allocations per statement and per row.** The type comment is written into a new text without cloning the
  parameters; values are bound by reference; identifiers and placeholders are written into the buffer; the slot of a
  returned connection is released inline when the return is already complete; type names are compared without
  allocation; one map per row; the primary key is precomputed by the derive and resolved relations are cached.
- **Includes in one statement per relation.** A paginated include uses `ROW_NUMBER() OVER (PARTITION BY ..)`; children
  are indexed by key once. Cursors and `distinct` inside an include keep the per-parent query.
- **`distinct` reads only what it needs.** Rows are streamed and reading stops once `skip + take` distinct rows are found.
- **One connection attempt for concurrent first requests.** Requests that find no pool share one attempt; a failure is
  delivered to all of them and the next request starts a new attempt.
- **Timeouts and cancellations end the statement.** On Postgres, when an interactive transaction expires, or is dropped
  because its request was cancelled, while a statement runs, the statement is cancelled on the server, so the connection
  comes back without waiting for it (one extra round trip per interactive transaction reads the backend's process id).
  Statements outside interactive transactions keep today's behavior, documented.

## Capabilities

### New Capabilities

- `bench/orm`: the ORM benchmark (scenarios, what it reports, where it builds, how results are recorded).
- `orm/transactions`: what the timeout of an interactive transaction and the cancellation of a request do to the running
  statement and to the connection.

### Modified Capabilities

- `orm/connection-pool`: added requirement that concurrent first requests share one connection attempt.
- `orm/client`: added requirements for multi-row `createMany` statements and for paginated includes in one statement per
  relation, with unchanged results.

## Impact

- New: `bench/orm/{Cargo.toml,src/main.rs,run.sh,README.md}` (workspace member, `publish = false`),
  `crates/corvis-orm/src/testing.rs` (`#[doc(hidden)]`: the statement and decoded-row counters the benchmark and the tests
  read), `crates/corvis-orm/tests/{create_many,includes,pg_cancel}.rs`, a transaction fixture recorded from the original
  (`tests/fixtures/tx_timeout.json`).
- Edited: `crates/corvis-orm/src/{db,client,transaction,meta}.rs`, `crates/corvis-orm/src/engine/{mod,read,write}.rs`,
  `crates/corvis-orm-macros/src/lib.rs` (precomputed primary key), `Cargo.toml` (workspace member),
  `crates/corvis-orm/src/client_pool_tests.rs`, `crates/corvis-orm/tests/pg_connect_timeout.rs` (the cancelled-transaction
  test now asserts the earlier release), `bench/README.md`, `docs/data/corvis-orm.md`, `docs/es/data/corvis-orm.md`.
- Public API: `ModelMeta` gains a public field `primary_key_index` filled by the derive (code that builds a `ModelMeta` by
  hand must add it; nothing else changes signature). The `testing` module is hidden and not part of the API.
- Behavior visible to applications: a cancelled or timed-out statement ends on the server instead of running to completion;
  statement counts drop; the order and content of results do not change. Listed in design.md "Behavior visible to
  applications".
- No new dependency. The parity check is run.

## Out of scope

- A public streaming API (`find_many_stream`): a feature, not a fix; possible follow-up.
- `DISTINCT ON` on Postgres (the original resolves `distinct` in memory; parity first).
- Row-value key lists for compound keys (`q-orm-correctness` chunks them; revisit only if the numbers here call for it).
- Cancelling statements on SQLite (sqlx exposes no safe interrupt), and statements outside interactive transactions on
  Postgres (design.md Decision 7); both documented.
- Error types, migration locking, public-surface cleanup (`q-orm-api-hygiene`).

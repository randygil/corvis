# Design

## Context

Source: the engineering-quality review of `crates/corvis-orm` and `crates/corvis-orm-macros` filed under epic #187, read
on `dev` at `d46758a` (2026-11-11), `sqlx` 0.8.6, `tokio` 1.54. Line numbers are those of `d46758a`. This change is
implemented after `q-orm-correctness`, which edits `engine/read.rs`, `engine/write.rs` (key lists, `Decimal` casts) and
`schema.rs`: **task 1.1 re-checks every line below against `dev` at implementation time**; the decisions depend on what
is allocated, sent and waited for, not on line numbers.

The parity rule of the module holds here unchanged: results, their order, counts and errors must equal the recorded
original (the Postgres oracle `tests/pg_oracle.rs`, the SQLite oracle added by `q-orm-correctness`, the topic fixtures),
and the three port apps (Postgres) must keep 0 apidiff differences.

### The findings, confirmed against the code

| Finding | Where now | Verdict |
|---|---|---|
| M2 `createMany` sends one `INSERT` per row | `engine/write.rs:1164-1172` (loop over `insert_scalars`), nested `createMany` at `write.rs:699-705` | Confirmed. Each row also re-sorts its columns (`complete_insert`, `write.rs:179`) and, on Postgres, goes through the tagged clone below |
| S5 tagged clone per Postgres statement | `db.rs:657-665` (`sql.clone()` of text and all parameters, plus a `Vec<String>` of tags, to append `/* corvis-types .. */`) | Confirmed |
| S5 values cloned to bind | `db.rs:521` (`s.clone()`), `528` (`b.clone()`), `593`, `598` (`Json(j.clone())`), `600`, and the array arms `606-626` | Confirmed. sqlx binds `&'q str`, `&'q [u8]` and `Json<&T>` for the lifetime of the `Sql` |
| S5 a task per returned connection | `db.rs:276-293` (`PgConn::drop` spawns a task that awaits `return_to_pool` and then releases the slot) | Confirmed: one task per Postgres statement outside a transaction |
| S5 per-cell type names | `db.rs:743` (`to_uppercase()` per SQLite cell), `db.rs:944` (`type_info().into_owned()` per Postgres cell) | Confirmed |
| S5 two maps per row | `engine/read.rs:248-253` (`decode_scalars`: a map with `f.name.to_string()` keys) and `read.rs:269-285` (`shape`: a second map) | Confirmed. Sibling: `db.rs:503-508` (`Rows::into_objects` clones every column name for every row of a raw query) |
| S5 `pk()` allocates on every call | `meta.rs:326-341`, called several times per statement (e.g. `write.rs:143,302,320`, `read.rs:358,830`) | Confirmed |
| S5 relations resolved per use | `meta.rs:461-564` (`resolve_relation`, with `format!` of the join table name), called from `engine/mod.rs:219-221` for every include, relation filter and nested write | Confirmed |
| S5 boxed iterator per node | `engine/mod.rs:68-82` (`exceeds_depth`) | Confirmed (one box per object or array of the arguments) |
| S5 siblings | `db.rs:99-101` (`quote_ident`: two allocations per identifier, for every column of every statement), `db.rs:173-191` (`format!("${n}")` per bound value), `engine/mod.rs:680-683` (`key_of`: a `String` per row for grouping) | Found while confirming |
| S6 N+1 for paginated includes | `engine/read.rs:620-641` (one-to-many with `take`/`skip`/`cursor`/`distinct`: one query per parent), `read.rs:541-548` (many-to-many, same) | Confirmed |
| S6 quadratic many-to-many matching | `engine/read.rs:555-563` (for each parent, filter every child, with `to_string()` and `.cloned()`) | Confirmed |
| S7 everything in memory; `distinct` without `LIMIT` | `db.rs:639`, `db.rs:697` (`fetch_all` only), `engine/read.rs:386-399` (`distinct` filtered in memory after reading every row) | Confirmed for `distinct`. The in-memory `distinct` itself is the original's behavior (the oracle has a `distinct` case) and stays; reading every row before applying `take` is the cost. A public streaming API is a feature, out of scope |
| S2 pool initialization serializes retries | `client.rs:279-365` (`pool.write()` held across `connect_with` at 322 and `acquire_pg` at 355) | Confirmed. Tokio's `RwLock` is fair, so concurrent first requests queue and each makes its own attempt after the previous one failed: the k-th waits about k × `connect_timeout` |
| S3 the timeout does not stop the statement | `transaction.rs:154-177` (the timer takes `s.conn.lock()` at 163; `client.rs:400` holds that lock for the whole statement) | Confirmed, and pinned by `tests/pg_connect_timeout.rs:370-420` (`a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back` asserts the slot comes back only after the long statement ends). Same for a pooled statement dropped mid-flight: `PgConn::drop` returns the connection, which the pool can reuse only after the statement ends |

No finding of this change is dropped. One is narrowed: S7 keeps the in-memory `distinct` (parity) and only stops reading
early; the streaming API is out of scope.

## Goals / Non-Goals

**Goals:**

- Measure first, then remove the round trips, allocations and waits the review found, each against a number.
- Same results, order, counts and errors as before, on both dialects.

**Non-Goals:**

- New public APIs (streaming), `DISTINCT ON`, SQLite statement cancellation.

## Decisions

### 1. An ORM benchmark, run before and after

`openspec/specs/bench` has `bench/synthetic` (HTTP throughput with `oha` against `bench/corvis-app`) and
`bench/port-vs-original` (whole apps). Neither isolates the ORM. New capability `bench/orm`, following
`bench/synthetic`'s rules (release build in the shared target directory, servers on `127.0.0.1`, results file overwritten
per run, raw output ignored by git):

- `bench/orm/` is a workspace member (`publish = false`) with one binary, `corvis-orm-bench`, which installs a global
  counting allocator and runs one scenario per invocation against SQLite (a database file in a temporary directory) or
  Postgres (`CORVIS_ORM_BENCH_PG`, default the local `corvis-pg` server; a scratch database created and dropped).
- Models: the oracle's `User`, `Post`, `Comment`, `Category` (one-to-many and many-to-many) with a seeded data set (1,000
  users, 10 posts each, 3 comments per post, 10 categories per post).
- Scenarios: `find_unique` (by id, the hot path), `find_many` (100 rows), `include` (100 parents with their posts),
  `include_paginated` (100 parents, `take: 3, skip: 1`), `include_m2m` (1,000 posts with categories), `create_many`
  (10,000 rows), `create_many_and_return` (1,000 rows), `update_many` (1,000 rows), `transaction` (an interactive
  transaction with three statements), `distinct` (100,000 rows, 10 distinct values, `take: 10`).
- Reported per scenario and dialect: operations per second, p50 / p99 / max latency, allocations and bytes allocated per
  operation (counting allocator, over the measured window only), statements per operation and rows decoded per operation
  (from the hidden counters of `corvis_orm::testing`, which the tests use too), peak resident memory. Each measurement runs
  three times; the median is reported with min and max.
- `bench/orm/run.sh` builds and runs the matrix; `bench/orm/README.md` documents it; results go to
  `bench/orm/results.md`.

The before run is made at the start (task 2.3) on `dev` with `q-orm-correctness` merged, the after run at the end
(task 9.1), same machine, same matrix; both tables go to "Implementation notes". Benchmarks are not tests: no test compares
timings; the tests assert statement and row counts, which are deterministic.

Alternative considered: Criterion microbenchmarks of the binder and the decoder. Rejected as the first measurement: round
trips and per-row costs only show against a real database; microbenchmarks can be added for regressions.

### 2. Multi-row inserts

- `insert_many(model, rows, skip_duplicates, returning)`: each row's columns are computed as today (`scalar_data`,
  `data_value`, `complete_insert`, so defaults generated in Rust stay the same); consecutive rows with the same column list
  form a group; a group is split into statements of at most `min(1000, floor(limit / columns))` rows, where `limit` is the
  dialect's parameter limit (32766, 65535). Each statement is `INSERT [OR IGNORE] INTO t (cols) VALUES (..), (..)
  [ON CONFLICT DO NOTHING] [RETURNING pk]`. All statements run in the transaction `createMany` already opens.
- `createManyAndReturn` collects the keys from `RETURNING` in statement order and reads the rows back in that order through
  `read_back_many` (chunked by `q-orm-correctness`); this is the order the original returns (its engine also issues a
  multi-row `INSERT .. RETURNING`), checked by new oracle cases.
- Counts: the sum of `rows_affected` (`OR IGNORE` / `ON CONFLICT DO NOTHING` count only inserted rows, as today).
- Errors: a constraint violation fails the statement and the transaction rolls back, as today; the error mapping is
  unchanged, so code, message and meta are the same (the oracle's `createMany.skipDuplicates` and `err.P2002_*` cases, and
  new cases where the duplicate is in the middle of a batch).
- The nested `createMany` (`write.rs:699-705`) uses `insert_many` with the foreign key added to each row.
- Postgres parameter types: each value keeps its own bound type and cast, so `VALUES` rows type the same way single-row
  inserts do; the type comment of Decision 3 keeps the statement cache apart per signature.

Numbers it must move (`create_many`, 10,000 rows of `Post`): `INSERT` statements per operation from 10,000 to
`ceil(10,000 / rows per statement)`, i.e. 10 with 1,000 rows per statement (plus `BEGIN` and `COMMIT`, unchanged);
operations per second on Postgres over loopback up by at least 5× (recorded). Tests (`tests/create_many.rs`, statement counter):
the statement bound above on both dialects; a 10,000-row `createManyAndReturn` returns rows equal, in order, to the
per-row version's output captured before the change (task 3.1 records it); `skipDuplicates` with duplicates spread across
two statements counts only inserted rows; a duplicate in the 1,500th row fails with the same `P2002` code, message and
meta as before and inserts nothing.

### 3. Fewer allocations per statement and per row

- **Type comment without the clone.** `pg_type_signature` returns only the tagged text (`String`), written in one pass
  with `write!` into a buffer reserved for the text plus the tags; `pg_query` takes the text and `&sql.params`. No
  parameter is cloned, no `Vec<String>` of tags.
- **Bind by reference.** `sqlite_query` and `pg_query` bind `&str`, `&[u8]`, `Json(&Value)` and `&Decimal` for the
  lifetime of the `Sql`; the array arms build their typed vectors once (unavoidable for Postgres arrays) without cloning
  the elements' strings where sqlx accepts slices of references.
- **Identifiers and placeholders in place.** `Sql::ident`/`col`/`table` write the quoted name directly into `text`
  (doubling `"` as they go); `bind` writes `$n` with `write!`. No intermediate `String`.
- **Slot release without a task.** `PgConn::drop` polls `return_to_pool()` once (`now_or_never`); when it is already
  complete (the common case: no `after_release` hook, no liveness check), the slot is released inline; otherwise the task is
  spawned as today. The rule that the slot is released only after the connection is back in the pool is unchanged.
- **Type names without allocation.** SQLite compares `raw.type_info().name()` with `eq_ignore_ascii_case` against the known
  names; Postgres reads the OID and kind from the borrowed type info and clones nothing per cell.
- **One map per row.** The engine decodes a row into the selection's shape directly: `decode_scalars` keeps positional
  values, and `shape` builds the single output map from them (relations and `_count` are inserted into it). Raw queries
  (`Rows::into_objects`) build each row's keys from one shared column list without cloning the list per row (keys are still
  `String`, which `serde_json::Map` requires).
- **Precomputed primary key.** The derive emits `primary_key_index: &[usize]` (positions in `fields`, computed with the
  current `pk()` rules at compile time); `ModelMeta::pk_fields()` returns an iterator over them without allocating, and the
  engine uses it; `pk()` stays (returns a `Vec`) for compatibility. A test checks `pk_fields()` equals `pk()` for every model
  of the oracle, the generated apps' schemas and the tests.
- **Cached relations.** `resolve_relation` results are cached per (model, field) in a process-wide map keyed by the
  `&'static FieldMeta` address, filled on first use and returning `Arc<ResolvedRelation>`; the result is a pure function of
  static metadata, so the cache cannot go stale.
- **No boxed iterator per node.** `exceeds_depth` pushes children directly in a `match` on the value.
- **Grouping keys.** `key_of` writes into one reused buffer per call site, or groups by a hash of the values where the
  callers already keep the values (decided in task 4.3 with the benchmark).

Numbers it must move (`find_unique` by id, `User` with 16 scalar columns): allocations per operation down by at least 30
on Postgres (tag vector and strings, the `Sql` clone, the spawned task, two allocations per identifier and one per
placeholder, the second map and its keys) and by at least 20 on SQLite (16 upper-cased type names, the second map),
bytes per operation down accordingly; `find_many` of 100 rows: allocations per row down by at least the 16 keys of the
second map. Operations per second must not fall. Tests: the existing suites and both oracles unchanged; a unit test that
the tagged text equals the old one byte for byte for a statement with every value kind; `pk_fields()` equal to `pk()`.

### 4. Includes in one statement per relation

- **Paginated one-to-many** (`take` and/or `skip`, no `cursor`, no `distinct`): one statement:
  `SELECT * FROM (SELECT t0.*, ROW_NUMBER() OVER (PARTITION BY <fk cols> ORDER BY <order>, <pk>) AS __rn FROM <child> t0
  WHERE <fk> IN <parent keys> AND <user where>) s WHERE __rn > <skip> [AND __rn <= <skip + take>] ORDER BY <fk>, __rn`.
  A negative `take` reverses `<order>` inside the window and the rows of each parent are reversed after grouping, which is
  what `query_rows` does for one parent today. The parent keys go through the key helper of `q-orm-correctness`
  (chunked when compound).
- **Paginated many-to-many**: the same window over the join table joined to the target, partitioned by
  `join.parent_column`.
- **Cursor or `distinct` inside an include**: keep the per-parent query (rare; documented in the rustdoc of `include`).
- **Many-to-many matching**: children indexed once in a `HashMap` by their key; each parent takes its linked ids in link
  order and looks them up, keeping today's output order (the children's own `orderBy` order, filtered to the parent's
  links), checked by the oracle's `include.m2m` and new cases.
- SQLite needs window functions (3.25, 2018); the bundled library is newer.

Numbers it must move (`include_paginated`, 100 parents): statements per operation from 101 to 2; operations per second up
(recorded). `include_m2m` (1,000 posts × 10 categories): time per operation down (recorded), no longer quadratic. Tests
(`tests/includes.rs`, statement counter): the statement counts above; results equal to the per-parent version for
`take`, `skip`, both, negative `take`, ties in the order, `skip` past the end, a parent with no children, compound keys.
New oracle cases (appended to the Postgres recorder and added to the SQLite recorder): nested negative `take`, nested
`orderBy: desc` with ties, `skip` past the end, many-to-many with `take`.

Alternative considered: `LATERAL` joins on Postgres. Rejected: Postgres only, and the window form is the same on both
dialects.

### 5. `distinct` reads only what it needs

When `distinct` is given, `query_rows` reads the ordered rows as a stream (`fetch` instead of `fetch_all`, through a new
`fetch_until` in `db.rs` that decodes row by row and stops when a predicate says so), keeps the first row of each distinct
key in order, and stops once it has `skip + |take|` of them (all of them when there is no `take`). The statement is the same
as today (no `LIMIT`, since the distinct count is not known in SQL), so results are identical; the stream is dropped early,
which ends the statement (sqlx closes the portal). Inside an interactive transaction the stream holds the transaction's
connection lock as `fetch_all` does today.

Number it must move (`distinct`, 100,000 rows, 10 distinct values in the first 100 rows, `take: 10`): rows decoded per
operation from 100,000 to at most 100 (decoded-row counter); peak memory down (recorded). Tests: rows decoded bounded as
above; results equal to the oracle's `distinct` case and to the full read for `skip`, negative `take` and no `take`.

### 6. One connection attempt for concurrent first requests

`ClientInner.pool` becomes a small state machine under a `std::sync::Mutex` held only to read or swap the state:
`Ready(DbPool)`, `Connecting(Shared<BoxFuture<'static, Result<DbPool>>>)`, `Empty`. `pool()` returns the pool when ready;
otherwise it joins the current attempt or starts one (the current body of `pool()` becomes that future), and awaits it
without holding any lock. When the attempt finishes, the first waiter to see the result installs `Ready` on success or
`Empty` on failure (only if the state still holds that same attempt), and every waiter returns the same result (the error
is cloned). `connect()` and `is_connected()` read the same state; `disconnect()` closes a ready pool and sets `Empty` (an
attempt in progress finishes and is discarded if the state changed).

Number it must move: with a silent server, 50 concurrent first queries with `connect_timeout=1`: TCP connections accepted
by the server from 50 or more to at most 5 (one attempt and its retries), and every query fails with the same `P1001`
message; with a reachable server, 50 concurrent first queries open the pool once. Tests (`client_pool_tests.rs`, counting
accepted sockets, no durations): those two, plus a failed attempt followed by a successful one (the second request starts
a new attempt), and `disconnect` during an attempt.

Alternative considered: `tokio::sync::OnceCell::get_or_try_init`. Rejected: after a failed initializer the next waiter runs
its own, which is the serial behavior this fixes.

### 7. Timeouts and cancellations end the statement

- When an interactive Postgres transaction (`_transaction`, `_transaction_with`) begins, the backend's process id is read
  once (`SELECT pg_backend_pid()`, in the same statement as `SET TRANSACTION ISOLATION LEVEL` when there is one) and kept in
  `TxState`. sqlx does not expose the process id it receives at startup, so this costs one round trip per interactive
  transaction; the benchmark's `transaction` scenario records it. Engine-internal transactions (nested writes,
  `createMany`) and batches have no timeout and do not read it.
- A statement in flight marks its state (`in_flight: AtomicBool` beside the connection lock, set before the statement is
  sent and cleared after its result is read).
- **Timeout**: when the timer fires and the statement is in flight, it sets `expired` and sends
  `SELECT pg_cancel_backend(<pid>)` on a direct connection (not from the pool, so a full pool cannot block it), bounded by
  `connect_timeout`. The running statement fails with SQLSTATE 57014; the query maps it to the expired-transaction error
  (`P2028`, the message of `closed_error("query")`), then the timer takes the lock and rolls back as today.
- **Drop**: when the `TxState` of an interactive transaction is dropped with a statement in flight (the request was
  cancelled), the same cancel is spawned before the connection goes back; the slot is released when the connection is
  back, as today.
- Statements outside an interactive transaction: unchanged. Their process id is unknown without a round trip per
  statement, and closing the socket instead would free the slot but leave the statement running on the server. Documented,
  and listed as a possible follow-up.
- SQLite: unchanged (no safe interrupt in sqlx); documented.
- What the original does, settled by recording (task 7.2, `corvis-tests/tmp/orm/tx/cases.js` →
  `tests/fixtures/tx_timeout.json`): an interactive transaction with `timeout: 1000` running `pg_sleep(5)` (the error class,
  code and message it returns, when it returns, and whether the statement is still active on the server afterwards). The
  error corvis-orm returns must equal the recorded one; if the original lets the statement run, the cancellation is a
  documented improvement (the observable result, the error, is the same; only the server work stops earlier).

The pid is never taken from client input; `pg_cancel_backend` only cancels the current statement of a backend the same
role owns, and only while this process holds that connection (the connection cannot be reused by another request while
it is held, so the pid cannot have been recycled).

Numbers it must move: after a timed-out interactive transaction, or a cancelled request inside one, whose statement is
`pg_sleep(30)`, the slot comes back as soon as the cancel is acknowledged instead of after 30 s; the `transaction` scenario's
statements per operation rise by one (the process id) and its rate is recorded. Tests (`tests/pg_cancel.rs`, condition waits on
`pg_stat_activity` with a generous deadline that only fails the test): the timed-out transaction returns the recorded
error and the backend is idle (not `active`, not `idle in transaction`) before the 30-second statement could have ended;
a cancelled request's next query gets the only slot and the statement is gone; a statement that finishes normally is never
cancelled (a counter of cancels stays 0 over 1,000 statements); `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back`
is rewritten to assert the new release (the connection is clean, and the long statement is gone).

## Security invariants that must hold

From the hardening review of the ORM and the archived changes on the pool parameters and `connect_timeout`:

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| Every value a parameter, every identifier quoted | hardening review | 2, 3 (identifiers written in place) | `tests/fuzz_filters.rs`, `tests/security.rs`; new unit test: `ident` doubles `"` exactly as `quote_ident` did |
| Statement cache separated by parameter types | the type-comment fix (`db.rs:650-656`) | 3 | new byte-for-byte test of the tagged text; `pg_oracle` (`rawt.param_types`) |
| A write repeats its filter in the statement | hardening review | 2 | `update_applies_its_filter_atomically_*` unchanged |
| The pool never exceeds its size; `pool_timeout` and `connect_timeout` bound every wait | `orm/connection-pool` | 3 (slot release), 6, 7 | `pg_pool_url`, `pg_connect_timeout` suites (the cancelled-transaction test rewritten, the rest unchanged); new tests of Decision 6 |
| The slot is released only after the connection is back in the pool | connect-timeout change | 3, 7 | `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back` (rewritten: still released only once the connection is back) |
| No credentials in errors | connect-timeout change | 6 (shared error), 7 (cancel connection) | existing `topsecret` assertions; new: the cancel connection's failure is logged without the URL |
| A transaction never reports success for uncommitted work | transactions | 7 | `interactive_transaction_timeout_rolls_back` unchanged |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| A statement inside an expired transaction | Runs to completion, then `P2028` | Cancelled on the server, `P2028` (as recorded) | Server work stops at the timeout |
| A statement of a request cancelled inside an interactive transaction | Runs to completion; the slot waits for it | Cancelled; the slot comes back once the connection is back | Abandoned transactions cannot exhaust the pool |
| An interactive transaction on Postgres | `BEGIN` | `BEGIN` and one `pg_backend_pid()` read | One more round trip per interactive transaction |
| Concurrent first requests with the database down | The k-th waits about k × `connect_timeout` | All fail together after one attempt | Faster failure, one attempt |
| `createMany` round trips | One per row | One per batch | Results, counts and errors unchanged |
| Paginated include, `distinct` | Per-parent queries; full read | One statement; early stop | Results unchanged |
| `ModelMeta` | No `primary_key_index` | New public field set by the derive | Hand-built `ModelMeta` values must add it |

## Risks / Trade-offs

- Multi-row `RETURNING` order on SQLite is documented as arbitrary → the oracle cases and the 10,000-row comparison check
  it; if it ever differs, `createManyAndReturn` sorts by the position recorded per row (generated keys are known in Rust
  except autoincrement, for which SQLite assigns increasing ids within one statement).
- The window-function include changes the SQL shape → results compared with the per-parent version on many shapes and with
  new oracle cases.
- Cancelling by pid needs a second connection → only on expiry or drop with a statement in flight; bounded by
  `connect_timeout`; its failure is logged and the old behavior (wait for the statement) remains.
- The shared connection attempt changes who sees which error → all concurrent waiters see the same error, which is what a
  single attempt means; tested.
- The benchmark measures one machine → before and after on the same machine; the numbers named per decision are statement
  and allocation counts or relative rates.

## Open questions

1. Rows per multi-row statement: cap at 1,000 or use the whole parameter budget? Recommendation: 1,000 (smaller statements,
   no plan-size surprises), revisited with the benchmark.
2. Cancel the statement of a dropped request outside transactions too, or only inside interactive transactions?
   Recommendation: only inside; outside, the process id would cost a round trip per statement (Decision 7).
3. Add `find_many_stream` now that `fetch_until` exists? Recommendation: no, a follow-up feature.
4. Cache relations in a process-wide map, or emit them from the derive? Recommendation: the map; the derive cannot resolve
   relations across models at compile time.

## Review notes

These settle the open questions and override the design where they differ.

1. 1,000 rows per statement at most; task 9.1 records whether a larger batch would have moved the number further.
2. Only statements inside interactive transactions are cancelled; the rest is documented, and a follow-up is opened only
   if the maintainer wants it.
3. No streaming API in this change.
4. Relations are cached in a process-wide map keyed by field address.
5. Implemented after `q-orm-correctness` is merged; task 1.1 is mandatory and written in "Implementation notes" before any
   other task.
6. The security review is mandatory before the merge: the cancel path (pid handling, second connection, races with the
   statement's end), the shared connection attempt, the bind-by-reference and in-place identifier writing.
7. Input from `q-orm-correctness` (its design.md, "Group 8" notes): engine-built lists of compound keys are now split
   into statements of at most 500 `OR`ed groups. Each statement is bounded, but the total work of a long list over
   columns without an index is still rows x keys: Postgres evaluates the whole `OR` on every row of a sequential scan
   (measured: 396 ms for 500 groups over 70,000 rows; a 16,383-group statement ran for 13 minutes and could not be
   cancelled). Evaluate a form whose cost is linear, such as `(a, b) IN (SELECT * FROM unnest($1::text[], $2::int[]))`
   on Postgres (typed arrays per column, with the enum, `uuid` and `date` casts) and a `json_each` join on SQLite, with
   the benchmark of this change; keep the `OR` form for lists that fit one statement, whose Postgres SQL the oracle
   snapshot pins.

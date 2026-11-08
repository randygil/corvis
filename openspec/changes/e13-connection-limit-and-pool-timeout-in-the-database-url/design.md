# Design

## Context

See proposal.md for motivation; the behavior is in the `orm/connection-pool` spec delta. Current state, verified in the
code (corvis-orm at this branch, sqlx 0.8.6 from `Cargo.lock`):

**corvis-orm**

- `OrmClientOptions` (`crates/corvis-orm/src/client.rs:21-43`) has `max_connections: Option<u32>` documented as
  "(`connection_limit`)" (`:28-29`) and `acquire_timeout: Option<Duration>` documented as "(`pool_timeout`)"
  (`:30-31`). Both default to `None` (`:46-56`); only `max_connections` has a builder (`:74-77`). The doc comments
  name the URL parameters, but nothing reads them from the URL.
- `OrmClient::with_options` (`client.rs:144-147`) only validates the protocol through `dialect_of` (`:120-132`); the
  pool is created lazily in `OrmClient::pool` (`:197-244`).
- SQLite branch (`client.rs:208-233`): for `file:` URLs the query string is split off and dropped, except that
  `mode=memory` or a `:memory:` path selects an in-memory database (`:209-215`); other URLs go to
  `SqliteConnectOptions::from_str` (`:217`). In-memory databases get exactly one connection (`:223-225`); file
  databases get `max_connections.unwrap_or(5)` (`:227`). `acquire_timeout` is applied only when set (`:229-231`).
- Postgres branch (`client.rs:234-240`): `PgPoolOptions::new().max_connections(max_connections.unwrap_or(10))`, the
  optional `acquire_timeout`, then `connect(url)` with the URL unchanged.
- `init_error` (`client.rs:593-598`) and `server_of` (`:602-616`) build `P1001` messages with `host:port` only, never
  the credentials or the query string; `crates/corvis-orm/tests/security.rs:471-492` pins that.
- `sqlx::Error::PoolTimedOut` is mapped to `P2024` "Timed out fetching a new connection from the connection pool..."
  (`crates/corvis-orm/src/error.rs:282-286`).
- `OrmClient::ensure_database` (`crates/corvis-orm/src/migrate.rs:151-177`) parses `options().url` again with
  `PgConnectOptions::from_str` (`:159`) to create a missing database.
- `OrmModuleOptions::max_connections` (`crates/corvis-orm/src/module.rs:49-50`, builder `:95-98`) is copied into the
  client options by `OrmService::new` (`:123-129`).
- Tests that set the pool from code: `tests/pg_features.rs:21`, `tests/pg_json_paths.rs:71`, `tests/security.rs:52`
  and `:480-488` (`acquire_timeout = 2 s` so an unreachable server fails fast). None uses a URL pool parameter.
- Docs: `docs/data/corvis-orm.md:151` lists `max_connections(n)` as "Pool size" (same line in
  `docs/es/data/corvis-orm.md`); the URL parameters are not mentioned.

**sqlx 0.8.6**

- Postgres URL parsing (`sqlx-postgres-0.8.6/src/options/parse.rs:50-108`): an unknown key logs
  `ignoring unrecognized connect parameter` through `tracing` and is skipped.
- SQLite URL parsing (`sqlx-sqlite-0.8.6/src/options/parse.rs:37-110`): an unknown key is a configuration error
  `unknown query parameter ... while parsing connection URL`.
- `PoolOptions::new()` (`sqlx-core-0.8.6/src/pool/options.rs:143-165`): `max_connections: 10`,
  `acquire_timeout: 30 s`. The acquire deadline is `acquire_started_at + acquire_timeout`
  (`sqlx-core-0.8.6/src/pool/inner.rs:248`), a plain `Instant + Duration` that panics on overflow, so "no limit" cannot
  be `Duration::MAX`. The same deadline bounds opening a new connection (`inner.rs:288`, `:323-334`).

**Original client**

- The connection-pool page of the original's docs documents, for the 6.x engine, the pool size default
  `num_cpus::get_physical() * 2 + 1` and the `pool_timeout` default of 10 s. The page does not state the error on
  timeout; corvis-orm already reproduces the original's `P2024` for it (`error.rs:282-286`). The 7.x line
  moves pool settings to driver adapters (for `pg`: `max` 10), configured in code, not in the URL.
- The port originals use both lines (`@prisma/client` 6.19.3 and 7.x in `corvis-tests`); their URLs carry only
  `?schema=public`. consi and facten build the pool from the URL through `OrmModuleOptions::new(url)` without
  `max_connections` (consi `src/prisma/prisma.module.rs:8`, facten `src/prisma/prisma.module.rs:17`).

## Goals / Non-Goals

**Goals:**
- `connection_limit` and `pool_timeout` in the URL size and time the pool, with the 6.x defaults, for Postgres and
  SQLite files.
- No URL pool parameter reaches sqlx (no warning on Postgres, no error on `sqlite://`).
- Invalid values fail early with a clear, credential-free message.

**Non-Goals:**
- Any other URL parameter (see proposal, Out of scope).
- Reproducing the 7.x driver-adapter defaults.

## Decisions

1. **Parse once, in `with_options`, into a private `PoolUrl` value.** A pure helper
   `fn split_pool_params(url: &str) -> Result<(String, PoolUrlParams)>` in `client.rs` (or a new private module
   `pool_url.rs`) returns the URL without `connection_limit` / `pool_timeout` and the parsed values
   (`connection_limit: Option<u32>`, `pool_timeout: Option<PoolTimeout>` where `PoolTimeout` is `Seconds(u64)` or
   `Unlimited`). `with_options` calls it after `dialect_of` and stores both in `ClientInner` (new fields
   `driver_url: String`, `pool_params: PoolUrlParams`); `OrmClientOptions::url` keeps the user's URL unchanged so
   `options()` still returns what the user passed. `pool()` and `ensure_database` use `driver_url`.
   - The query string is split on the first `?` and on `&`; a pair is removed when its key (before `=`) is exactly
     `connection_limit` or `pool_timeout`. Other pairs are copied byte for byte in their original order, and the `?`
     is dropped when nothing remains. Values are percent-decoded before parsing (`%31` is `1`).
   - A repeated key: the last occurrence wins (what `URLSearchParams.get` would not do, but what a sequential parser
     like the engine's does); listed as an open question for the differential fixture to settle.
   - Alternative: parse with the `url` crate. Rejected: `file:./dev.db` and `sqlite::memory:` are not hierarchical
     URLs, and re-serializing would change the encoding of other parameters.
   - Alternative: parse lazily in `pool()`. Rejected: an invalid value would surface on the first query instead of at
     creation, and `connect()` errors are reported as `P1001` "Can't reach database server", which would be wrong.

2. **Defaults of the 6.x engine.** Pool size `num_cpus::get_physical() * 2 + 1`, wait 10 s, applied to Postgres and
   SQLite files. In-memory SQLite keeps its single permanent connection and ignores `connection_limit` (more than one
   connection would open different empty databases); `pool_timeout` still applies to it.
   - `num_cpus` is added to `crates/corvis-orm/Cargo.toml`: the standard library only offers
     `available_parallelism()`, which counts logical CPUs (twice the pool on hyper-threaded machines) and honors
     cgroup quotas, unlike the original.
   - Alternative: keep sqlx's 10 connections (matches the 7.x `pg` adapter default). Rejected for now because the
     issue asks for the URL semantics, whose defaults belong to the 6.x engine; listed as an open question.
   - Alternative: keep 5 for SQLite files. Rejected: the original documents one default without distinguishing
     providers; whether its SQLite pool really uses the same formula is confirmed by the fixture (open question).

3. **Precedence: code > URL > default.** `opts.max_connections.or(url.connection_limit).unwrap_or(default)`, and the
   same for `acquire_timeout` / `pool_timeout`. Code options are set deliberately by the program (tests set them to
   bound resources); the URL is the deployment knob for apps that leave them unset, which is every port app.
   - Alternative: URL over code, so operators can always tune. Rejected: it would silently override the explicit
     values that existing tests and `OrmModuleOptions::max_connections` users rely on.

4. **"No limit" is a very long finite timeout.** `pool_timeout=0` maps to
   `acquire_timeout(Duration::from_secs(u32::MAX as u64))` (about 136 years), which `Instant + Duration` handles on
   every supported platform, instead of `Duration::MAX`, which panics in sqlx (`inner.rs:248`). A named constant
   documents it.

5. **Validation and error.** `connection_limit` parses as `u32` and must be at least 1 (sqlx cannot run a pool of 0);
   `pool_timeout` parses as `u64` (whole seconds, no fraction, no sign). On failure `with_options` returns
   `OrmError::Initialization` with `error_code: Some("P1013")` and the message
   ``The provided database string is invalid. `connection_limit` must be a positive integer, got `abc`.``
   (`pool_timeout`: "must be a whole number of seconds"). The rejected value is echoed because it is a query value of
   a pool parameter, never the user or password; the rest of the URL is not.
   - `P1013` is the original's code for an invalid database string; the exact wording of the original for these two
     parameters is captured by the differential fixture (task 4.1) and the message adjusted to it if it differs.

6. **Differential fixture against the original 6.x client.** A Node script in `corvis-tests/tmp/orm/pool/` with
   `@prisma/client` 6.19.3 (the version of the port originals) records, for a fixed list of URL cases, the maximum
   number of server connections observed in `pg_stat_activity` while 6 `SELECT pg_sleep(0.3)` run concurrently, the
   error code of a query that waits behind a held connection with `pool_timeout=1`, and the error code and message for
   invalid values. The results go to `crates/corvis-orm/tests/fixtures/pool_url.json`; `tests/pg_pool_url.rs` replays
   the cases through corvis-orm (skipped without a server, like `pg_oracle.rs`). The default-size case is recorded as
   the formula, since the core count differs between machines.

## Risks / Trade-offs

- [Default pool size changes for every app that does not set it (10 → `2n+1` on Postgres, 5 → `2n+1` on SQLite
  files)] → on a 4-core server Postgres goes from 10 to 9 connections and SQLite from 5 to 9; documented with a
  "changed" note; `connection_limit` or `max_connections` restore any value.
- [Default wait goes from 30 s to 10 s] → under load, queries fail with `P2024` sooner, like the original. Because sqlx
  uses the same deadline to open a connection, `connect()` to an unreachable server also gives up after 10 s instead of
  30 s, which is closer to the original's 5 s `connect_timeout`.
- [`pool_timeout=0` with an unreachable server] → connecting is bounded only by the OS TCP timeout; documented.
- [More SQLite connections on file databases] → more writers can contend for the lock (`SQLITE_BUSY`), as in the
  original; apps that need a single writer set `connection_limit=1`.
- [New dependency `num_cpus`] → small and widely used; its only dependencies are platform crates (`libc` on Unix).

## Open questions

- Default pool size: the 6.x formula (`2 * physical cores + 1`, this change) or the 7.x `pg` adapter's fixed 10? The
  port originals use both lines. A maintainer should confirm before implementation.
- Precedence: is "code wins over URL" right, or should the URL win so operators can always tune the pool through
  `DATABASE_URL`?
- Repeated keys (`connection_limit=2&connection_limit=5`): last wins here; confirm with the differential fixture.
- Does the original apply `pool_timeout` to SQLite? This change accepts it there (harmless superset); confirm with the
  fixture and drop it for SQLite if the original rejects the URL.
- Exact error text of the original for invalid values; the message of decision 5 is aligned to the fixture.

## Review notes (2026-11-08)

Decisions taken for the open questions so the change can start; the maintainer can still overrule them in review:

- Default pool size follows the 6.x engine formula (`2 × physical cores + 1`) and 10 s timeout: the port originals
  and the differential fixture use `@prisma/client` 6.19.3. Revisit if the ports move to the 7.x `pg` adapter.
- Precedence stays code > URL > defaults: a value set in code is a deliberate choice of the application; operators can
  still tune everything the code leaves unset through `DATABASE_URL`.
- Repeated keys, `pool_timeout` on SQLite and the exact error text are settled by the recorded fixture (task 1.x)
  before implementing; the spec delta is adjusted to match.
- `?schema=` being ignored is tracked as its own issue (filed from this review), not in this change.
- The folder was renamed to `e13-connection-limit-and-pool-timeout-in-the-database-url` (issue #133).

## Fixture results (recorded with @prisma/client 6.19.3, implementation)

Recorded by `corvis-tests/tmp/orm/pool/cases.js` into `crates/corvis-orm/tests/fixtures/pool_url.json` (Postgres 127.0.0.1:55432,
temp SQLite file). Where it settles the open questions, and where corvis-orm deliberately differs:

- **Default pool size** is `2 * physical cores + 1` on Postgres (49 active with 24 physical cores, 64 queries). On SQLite
  the default pool runs 16 CPU-bound reads in parallel (364 ms against 4218 ms with `connection_limit=1`), consistent with
  the same formula; the original serializes interactive transactions on SQLite whatever the limit, which is engine
  behavior, not pool size, and is not reproduced.
- **Repeated keys**: the last occurrence wins (`2&5` gives 5, `5&2` gives 2; same for `pool_timeout`), as designed.
  Percent-decoding applies (`%32` is 2).
- **`pool_timeout` on SQLite**: the original accepts any value, including invalid ones, and ignores it (a query waited 3 s
  and 13 s behind a held connection with and without `pool_timeout=1`). corvis-orm keeps applying a valid value there (the
  harmless superset of decision 2) and, like the original, ignores an invalid one without failing.
- **`pool_timeout=0`** waits without limit on Postgres (as designed). **P2024** after `pool_timeout=1` takes 1.0 s; the
  original's message is "Timed out fetching a new connection from the connection pool. More info:
  http://pris.ly/d/connection-pool (Current connection pool timeout: 1, connection limit: 1)"; corvis-orm keeps its existing
  `P2024` text (out of scope).
- **Invalid values** (Postgres): `-1`, `abc`, `1.5` and empty for `connection_limit`, and `-1`, `1.5`, `abc` and empty for
  `pool_timeout`, fail with an `InitializationError` without an error code whose message starts "The provided database
  string is invalid." followed by a sentence and a link to the original's documentation. corvis-orm uses the same first
  sentence, then names the parameter and the value, and sets `P1013` (the original's runtime error has no code; `P1013`
  is its schema-engine code for the same condition and keeps the style shared with the `schema` change). The link is not
  reproduced. On SQLite only `connection_limit` is validated (same values rejected, with a different message).
- **Deviation**: the original accepts `connection_limit=0` and `connection_limit=4294967296` and runs without a pool limit
  (64 concurrent queries on 64 connections), on both providers. corvis-orm rejects both: sqlx preallocates its idle queue
  to the pool size, so an unbounded size is not representable, and a pool needs at least one connection. The decision 5
  rule (at least 1, at most `u32`) stays.

## Implementation notes

- **Deviation: the pool size has an upper bound.** The rule above ("at most `u32`") was too wide. sqlx 0.8.6
  preallocates its idle queue with `ArrayQueue::new(max_connections as usize)`, so `connection_limit=4294967295` (or
  `OrmClientOptions::max_connections(u32::MAX)`) made pool creation try to allocate hundreds of gigabytes and abort the
  process. The size is now limited to 1..=262143, the largest value Postgres accepts for its own `max_connections`
  (`MAX_POOL_SIZE` in `url_params.rs`). A larger URL value fails with the same `P1013` shape ("must be a positive
  integer no greater than 262143"); a code-provided `max_connections` of 0 or above the cap fails in
  `OrmClient::with_options` with an initialization error that names the field. The original accepts these values; corvis-orm
  bounds them because of the preallocation.
- **Review fixes (2026-11-08).** The message of an invalid value now shows the value only when it is at most 40
  characters of `[0-9A-Za-z+._-]` after decoding, otherwise "got an invalid value" (an unencoded password can contain a
  parameter key and was echoed in part). The parser also follows the original and the `url` crate: the query ends at
  the first `#`, keys are percent-decoded before matching, `+` in a value is a space, and `%` needs two ASCII hex
  digits. A code `acquire_timeout` is clamped like `pool_timeout` (`Duration::MAX` panicked). `OrmClientOptions` prints
  its URL without the password in `Debug`. See #137's design for the details.

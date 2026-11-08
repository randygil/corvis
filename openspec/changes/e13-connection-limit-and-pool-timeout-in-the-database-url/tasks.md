# Tasks

## 1. Parse the pool parameters

- [x] 1.1 Add the private helper `split_pool_params(url) -> Result<(String, PoolUrlParams)>` (design decision 1) with
  unit tests in `crates/corvis-orm/src/client.rs` (or `pool_url.rs`): no query string; only pool parameters (the `?`
  disappears); pool parameters mixed with `sslmode` / `application_name` / `options=-c%20search_path%3Dx` (order and
  encoding of the others unchanged); `file:./dev.db?connection_limit=1`; `sqlite://dev.db?connection_limit=1`;
  `file::memory:?connection_limit=4`; percent-encoded value `%32`; repeated key (last wins); `pool_timeout=0` →
  `Unlimited`; verify with `cargo test -p corvis-orm --lib pool` that they pass
- [x] 1.2 Validation per design decision 5: `connection_limit` `0`, `-1`, `abc`, `1.5`, empty and `4294967296`;
  `pool_timeout` `-1`, `1.5`, `abc`; each returns an initialization error with code `P1013` naming the parameter and the
  value; a test with `postgresql://app:secret@host/db?connection_limit=abc` asserts the message contains neither `app`
  nor `secret`; verify the unit tests pass

## 2. Apply them to the pool

- [x] 2.1 Call the helper in `OrmClient::with_options` (`client.rs:144-147`), store `driver_url` and the parsed values
  in `ClientInner`, keep `OrmClientOptions::url` as given; verify `OrmClient::with_options` with an invalid value fails
  without connecting (unit test with an unreachable host returns at once)
- [x] 2.2 In `OrmClient::pool` (`client.rs:197-244`) use `driver_url` for both dialects and compute
  `max_connections` / `acquire_timeout` with the precedence code > URL > default (decisions 2-4); add `num_cpus` to
  `crates/corvis-orm/Cargo.toml` and a `DEFAULT_POOL_TIMEOUT` (10 s) and `UNLIMITED_POOL_TIMEOUT` constant; keep the
  single in-memory SQLite connection; verify with a `#[cfg(test)]` test that reads
  `DbPool::Sqlite(p).options().get_max_connections()` / `get_acquire_timeout()` for a temp-dir SQLite file: default
  (`2 * physical + 1`, 10 s), URL (`connection_limit=3&pool_timeout=7` → 3, 7 s), builder over URL
  (`max_connections(4)` → 4), in-memory (1)
- [x] 2.3 Use `driver_url` in `OrmClient::ensure_database` (`crates/corvis-orm/src/migrate.rs:159`); verify a unit or
  integration test that `migrate deploy` against a Postgres URL with `?connection_limit=2` creates the database
  (skipped without a server)
- [x] 2.4 Update the doc comments of `OrmClientOptions::max_connections` / `acquire_timeout` (`client.rs:28-31`) and
  `OrmModuleOptions::max_connections` (`module.rs:49-50`) to state the URL parameter, the default and the precedence;
  verify `cargo doc -p corvis-orm --no-deps` has no warnings

## 3. Integration tests

- [x] 3.1 SQLite: in `crates/corvis-orm/tests/client.rs`, a client on `sqlite://<tmp>/pool.db?connection_limit=1`
  connects and runs `db_push` + a query (today it fails with `unknown query parameter`), and `file:<tmp>/pool.db?pool_timeout=1&connection_limit=1`
  returns `P2024` for a query issued while an interactive transaction holds the only connection; verify
  `cargo test -p corvis-orm --test client pool` passes
- [x] 3.2 Postgres (skipped without a server, `tests/common/pg.rs`): new `crates/corvis-orm/tests/pg_pool_url.rs` with
  the spec scenarios "Postgres URL with a limit" (max backends in `pg_stat_activity` for `current_database()` while 6
  `pg_sleep(0.3)` run), "Short pool timeout", "No limit", "Builder wins over the URL" and "TLS parameter kept"
  (with `sslmode=disable` so it runs on the local server); verify it passes against the local Postgres

## 4. Differential check against the original 6.x client

- [x] 4.1 Write `corvis-tests/tmp/orm/pool/cases.js` with `@prisma/client` 6.19.3 (design decision 6) and record
  `crates/corvis-orm/tests/fixtures/pool_url.json`: observed max connections for `connection_limit=1|2|5`, `P2024` with
  `pool_timeout=1`, success with `pool_timeout=0`, the code and message for each invalid value of task 1.2, repeated
  keys, and whether a SQLite URL with `pool_timeout` is accepted; verify the file is produced and committed with the
  script's path in its header
- [x] 4.2 Replay the fixture in `tests/pg_pool_url.rs` (connection counts, codes; messages compared after removing the
  original's version-specific suffix, if any); adjust decision 5's wording or the repeated-key rule if the fixture
  disagrees and record it in design.md; verify the test passes

## 5. Docs

- [x] 5.1 `docs/data/corvis-orm.md`: new "Connection pool" section after the `OrmModuleOptions` table (`connection_limit`
  and `pool_timeout` in the URL with defaults, `0` = no limit, precedence of `max_connections(n)`, in-memory SQLite
  always 1, a "changed" note for the old defaults 10 / 5 / 30 s, and that the 7.x driver-adapter settings map to
  `max_connections` / `acquire_timeout`); update the `max_connections(n)` row (line 151); same in
  `docs/es/data/corvis-orm.md`
- [x] 5.2 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Definition of done

- [x] 6.1 `cargo build --workspace --all-targets` with no warnings
- [x] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 6.3 `cargo test --workspace` passes, including the regression tests of groups 1-4
- [x] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 6.5 Framework runtime code changed (corvis-orm pool defaults): run the `corvis-parity` check and verify the three
  port apps keep 0 apidiff differences (and 0 dbdiff differences)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #133, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

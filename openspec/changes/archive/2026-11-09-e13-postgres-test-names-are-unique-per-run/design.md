# Design

## Context

- `crates/corvis-orm/tests/common/pg.rs:30` builds `orm_test_{name}` and recreates it with `DROP DATABASE IF EXISTS ...
  WITH (FORCE)`; `fresh_db(name)` is used by `pg_features`, `pg_oracle`, `pg_json_paths`, `pg_houseplan`,
  `pg_schema_url`, `pg_pool_url`, `pg_connect_timeout`, `nul_characters` and `security`.
- Names built by hand: `pg_connect_timeout.rs` (roles `orm_ct_role_a/b/c`, database `orm_test_ct_latin1`, and
  `orm_test_ct_role`, `orm_test_ct_lost`, `orm_test_ct_fixture_role` used in role URLs), `pg_pool_url.rs`
  (`orm_test_pool_ensure`), `pg_schema_url.rs` (`orm_test_schema_role`), `security.rs` (`orm_test_sec_tls` in the TLS
  URLs). `common/gate.rs` uses only the URLs returned by `fresh_db`.
- Doc comments naming the databases: `nul_characters.rs:5`, `pg_json_paths.rs:3`, `security.rs:5`,
  `pg_connect_timeout.rs:3`, `CONTRIBUTING.md` ("Postgres tests") and `crates/corvis-orm/README.md`.
- Nothing drops a database at the end of a run today: databases accumulate until the next run recreates them by name.

## Goals / Non-Goals

**Goals:** two runs against one server never share a database or role name; failed runs do not leave objects
forever.

**Non-Goals:** changing what any test checks; limiting connections; touching runtime code.

## Decisions

1. **Run id `<unix seconds>_<pid>_<4 hex>`.** The process id separates concurrent processes on one machine; the random
   part (from `RandomState`, no new dependency) separates processes on different machines sharing a server and pid
   reuse; the timestamp lets the sweep date a name without asking the server. It is about 23 bytes. Names are
   `orm_test_<name>_<run>` and `orm_<name>_<run>`; `<name>` is filtered to `[A-Za-z0-9_]` and cut so the whole stays
   within 63 bytes.
2. **API in `common/pg.rs`:** `run_id()`, `scratch_name(prefix, name)`, `scratch_db(name)` and `scratch_role(name)`
   (build the name and register it for clean-up), `db_of(url)` (database name of a URL), `sweep_stale(conn)`.
   `fresh_db_at` registers the database after creating it, so a failed attempt registers nothing. Tests that need a
   name derive it from the URL (`db_of`) or ask for a role/database name; none hard-code one.
3. **Clean-up at exit.** The first registration installs a C `atexit` handler (declared with `unsafe extern "C"`, no
   new dependency). It drops the registered databases with `DROP DATABASE ... WITH (FORCE)` and then the roles, on a
   fresh single-thread runtime in its own thread, because the test runtimes are gone. Errors are ignored. Tests that
   already drop their own objects keep doing so.
   - Alternative: a drop guard per test. Rejected: every test returns a URL string, not a guard, and the helper would
     have to change shape in every file.
4. **Sweep of leftovers.** The first successful `fresh_db_at` of a process runs `sweep_stale` once: it lists
   `orm\_test\_%` databases and `orm\_%` roles and drops those whose name ends in `_<secs>_<pid>_<4 hex>` with `secs`
   more than 3 hours old (`STALE_AFTER_SECS`). The age comes from the name, not from the server, so it works the same
   on the Docker `corvis-pg` server and on CI's service container, and needs no file access (`pg_stat_file`) or
   comments. Names without a run suffix (from before this change) are never touched by the sweep.
5. **Convention in docs.** `CONTRIBUTING.md` (English) and the test file headers name `orm_test_<name>_<run>` and
   `orm_<name>_<run>`.

## Risks / Trade-offs

- [A test binary that fails exits without running the `atexit` handler] -> observed with a temporary failing test: the
  databases of a failed run stay. This is acceptable and slightly useful (state to inspect); the sweep removes them
  after 3 hours.
- [A killed run leaves objects] -> same sweep.
- [A run longer than 3 hours loses its databases to another run's sweep] -> no ORM suite runs that long; the threshold
  is one constant.
- [`unsafe` in test support code] -> one call to the C runtime's `atexit` with a plain function.

## Open questions

None.

## Test plan

- `pg_scratch.rs`: names within 63 bytes and ending in the run id; the sweep drops a stale database and role and
  keeps a recent one and one without a run suffix.
- Two concurrent runs of the whole corvis-orm test executables against one server both pass (task 3.1).

## Implementation notes

- Implemented as designed. Extra: `pub sweep_stale` is the unguarded sweep (the once-per-process guard wraps it), so `pg_scratch.rs` can call it.
- `migrate_deploy_creates_the_database_with_a_pool_parameter` takes the database name from the URL of `fresh_db` (`db_of`): `migrate_deploy` ensures the database named in that URL, which before was the fixed `orm_test_pool_ensure`.
- Clean-up at exit runs only when the test binary exits normally. With a failing test libtest exits without running `atexit` handlers (checked with a temporary failing test): the failed run keeps its databases, and the sweep drops them after 3 hours. CONTRIBUTING.md says so.
- Concurrent run (3.1): all 18 corvis-orm test executables, built once, ran twice in parallel against the Docker `corvis-pg` server with `CORVIS_ORM_PG_REQUIRED=1`; both passed every test binary (same counts in A and B, for example `pg_pool_url` 9/9, `pg_connect_timeout` 13/13), and afterwards the server held no `orm_*` database or role.
- Unsuffixed databases left by earlier runs (`orm_test_<name>`) are not matched by the sweep; they were dropped by hand once on the local server.
- `openspec validate` warns that the requirement text is long (over 500 characters); kept as one requirement.


## Verification (2026-11-09)

Verified together in batch-m (#120, #135, #128, #151): `cargo fmt --check`, `cargo clippy --workspace --all-targets
-- -D warnings` and per-feature clippy of corvis are clean; `cargo test --workspace --no-fail-fast` with
`CORVIS_ORM_PG_REQUIRED=1` passed 1064 tests with 0 failures (and `cargo test -p corvis` passed 689 again after the
last #135 fixes); naming, crate-files, the examples (32/32 smoke checks) and `npm run docs:check` pass. CI run
42599828082 was green except `audit`/`deny`, which failed on RUSTSEC-2026-0459 (unrelated; ignored on main with #155).
corvis-parity: consi 832/832, facten 727/727, houseplan 636/636 equal, databases equivalent.

# Proposal

Implements [randygil/corvis#151](https://github.com/randygil/corvis/issues/151) (epic
[#13](https://github.com/randygil/corvis/issues/13)).

## Why

The corvis-orm Postgres tests use fixed names for their scratch databases and roles on the shared server
(`orm_test_<name>`, `orm_ct_role_a/b/c`, `orm_test_ct_latin1`, `orm_test_pool_ensure`, `orm_test_schema_role`), and
recreate them with `DROP DATABASE ... WITH (FORCE)`. Two runs against the same server at the same time (two
worktrees, or a developer and an agent) drop each other's databases and terminate each other's connections. On
2026-11-08 this showed up as hangs and failures blamed on load (#147, a hang in `nul_is_rejected_when_compiling_sql`
and its neighbours). CI is not affected (one run per job), but local work and parallel agent sessions are.

## What Changes

- Every scratch database and role gets a per-run suffix: `<unix seconds>_<process id>_<4 hex digits>`, so the names are
  `orm_test_<name>_<run>` and `orm_<name>_<run>`, cut to fit Postgres' 63-byte identifier limit.
- `common/pg.rs` owns the naming (`scratch_name`, `scratch_db`, `scratch_role`, `db_of`). Tests that built names by
  hand (`pg_connect_timeout.rs`, `pg_pool_url.rs`, `pg_schema_url.rs`, `security.rs`) use it or derive the name from the
  URL returned by `fresh_db`.
- What a process created is dropped when it exits (databases first, then roles). A sweep, run by the first
  `fresh_db` of each process, drops `orm_test_*` databases and `orm_*` roles whose embedded timestamp is older than
  3 hours, so runs that died do not pile up databases.
- `CONTRIBUTING.md` and the test headers name the convention.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `infra/ci`: adds a requirement that the Postgres test databases and roles are unique per run and cleaned up. No
  runtime behavior changes.

## Impact

- Tests only: `crates/corvis-orm/tests/common/pg.rs`, the tests that name scratch objects, a new
  `crates/corvis-orm/tests/pg_scratch.rs`, `CONTRIBUTING.md` and the Postgres line of `crates/corvis-orm/README.md`.
- No change to `crates/corvis-orm/src/`, the fixtures or any public API. The corvis-parity check does not apply.

## Out of scope

- The server-side connection budget (`max_connections`) shared by parallel runs (#147 notes it).
- Cleaning up after a run whose test binary exits through a failing test (see design.md, Risks).

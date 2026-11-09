# Tasks

## 1. Helper

- [x] 1.1 In `crates/corvis-orm/tests/common/pg.rs`: `run_id`, `scratch_name`, `scratch_db`, `scratch_role`, `db_of`,
  the exit clean-up and `sweep_stale` (design.md, decisions 1 to 4); `fresh_db_at` uses the per-run name. Verify:
  `cargo test -p corvis-orm --no-run` with no warnings
- [x] 1.2 `crates/corvis-orm/tests/pg_scratch.rs`: name length and run id, and the sweep. Verify:
  `cargo test -p corvis-orm --test pg_scratch` with Postgres running (`CORVIS_ORM_PG_REQUIRED=1`)

## 2. Tests that build names by hand

- [x] 2.1 `pg_connect_timeout.rs` (roles, `latin1` database, URLs of role cases), `pg_pool_url.rs`, `pg_schema_url.rs`,
  `security.rs` use the helper or the URL of `fresh_db`; update the header comments of the files that name the
  databases (`nul_characters.rs`, `pg_json_paths.rs`, `security.rs`, `pg_connect_timeout.rs`). Verify: every Postgres
  test binary passes

## 3. Concurrent runs

- [x] 3.1 Build once, then run the same corvis-orm test executables twice in parallel against one server (separate
  processes, different run ids) with `CORVIS_ORM_PG_REQUIRED=1`; both pass and no `orm_*` object of those runs remains.
  Record the result in design.md under "Implementation notes"

## 4. Docs

- [x] 4.1 `CONTRIBUTING.md` (English) names the convention, the exit clean-up and the sweep; the Postgres line of
  `crates/corvis-orm/README.md` follows

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes (with Postgres, `CORVIS_ORM_PG_REQUIRED=1`)
- [x] 5.4 Regression tests: `pg_scratch.rs` (1.2) and the concurrent run (3.1)
- [x] 5.5 Docs: no page under `docs/` or `docs/es/` describes these tests; confirm with
  `grep -rn "orm_test_" docs` and state it in the commit message
- [x] 5.6 corvis-parity: not required, only test files change (no framework runtime code); state this in the commit
  message

## Workflow follow-up

- [ ] Archive the change with `/opsx:archive`
- [ ] Close issue [#151](https://github.com/randygil/corvis/issues/151)
- [ ] Tick #151 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [ ] Move the card to Done in the Corvis Roadmap project

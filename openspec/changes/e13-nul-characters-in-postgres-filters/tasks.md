# Tasks

## 1. Reproduce and compare

- [x] 1.1 Reproduce every NUL case (path segment, `equals`, `contains`, `startsWith`, `endsWith`, `in`, JSON values) on SQLite and PostgreSQL and record the results (table in the proposal)
- [x] 1.2 Run the same cases with the original client against a scratch PostgreSQL database and record its error (raw `22021`/`22P05`, no request error code); the divergence is decision 1 of the design

## 2. Validate

- [x] 2.1 In `crates/corvis-orm/src/engine/filter.rs`, add `find_nul` and make `where_sql` reject a NUL anywhere in the `where` once, with the recursion on `where_node`; verify with `cargo build -p corvis-orm` without warnings
- [x] 2.2 Keep the #109 guarantees (parameterized path, 128-segment cap); verify with `cargo test -p corvis-orm --test pg_json_paths`

## 3. Tests

- [x] 3.1 Add `crates/corvis-orm/tests/nul_characters.rs`: every filter shape on SQLite and PostgreSQL for `findMany`, `count`, `findFirst`, `deleteMany`, `updateMany`, plus SQL compilation without a server; verify it passes with `CORVIS_ORM_PG_REQUIRED=1`
- [x] 3.2 Add the relation and `include` filter test to `crates/corvis-orm/tests/client.rs`
- [x] 3.3 Add the failing seed to `crates/corvis-orm/proptest-regressions/fuzz_filters.txt` and make the fuzz properties assert a validation error for NUL; verify `PROPTEST_CASES=20000 cargo test -p corvis-orm --test fuzz_filters` three times

## 4. Docs

- [x] 4.1 Add the NUL rule to the Security section of `docs/data/corvis-orm.md` and `docs/es/data/corvis-orm.md`; verify with `npm run docs:check`

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo fmt --all` then `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`
- [x] 5.3 `CORVIS_ORM_PG_REQUIRED=1 cargo test --workspace`
- [x] 5.4 `node scripts/check-naming.mjs`
- [ ] 5.5 Runtime code changed (`engine/filter.rs`): run the `corvis-parity` check and verify the three ports keep 0 differences (left to the maintainer)

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #141, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

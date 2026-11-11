# Tasks

Shared Cargo target directory only (never a target directory per task). Postgres tests need the local server
(`corvis-pg`, `127.0.0.1:55432`, `CORVIS_ORM_PG`); run them with `CORVIS_ORM_PG_REQUIRED=1` so they cannot skip. Recorder
scripts live in `corvis-tests/tmp/orm/oracle/` (outside the repository) and need the original client installed there, as
the existing ones do. Timing rule for every test: assert results, errors and statement counts, never durations; no
`sleep` to synchronize. Behavior must not change except where design.md "Behavior visible to applications" says so, and
Postgres SQL must not change except where Decision 6 replaces a statement that failed.

## 1. Reconcile and snapshot

- [x] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev`; write moved
  lines and anything that no longer holds in a new "Implementation notes" section of design.md before any other task;
  verify `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm` passes on the untouched tree
- [x] 1.2 Postgres SQL snapshot (design.md "Postgres stays unchanged"): a unit test in `crates/corvis-orm/src/engine/`
  that builds, with a lazily connected Postgres client and without sending anything, the SQL text and parameter types of
  `where_to_sql` and of the read, aggregate and update builders for every case of the Postgres oracle, and compares them
  with `tests/fixtures/pg_sql_snapshot.json`; generate that file now, on the untouched tree; verify the test passes and the
  file is committed before any engine change

## 2. Failing tests first (one per must-fix finding)

- [x] 2.1 `crates/corvis-orm/tests/sqlite_decimal.rs` (M1): a model with a `Decimal` field pushed on SQLite with the
  amounts `99.5`, `100`, `1000`, `-5`; `gte 100`, `lt 0`, `in`, `orderBy` asc and desc, `_min`, `_max`, `having` on
  `_sum`; the same on a table created with a `DECIMAL` column by raw SQL; verify the pushed-table tests fail on the
  untouched tree and copy the failure output into Implementation notes
- [x] 2.2 `crates/corvis-orm/tests/db_push.rs` (M3): `schema_sql` of the oracle `Pair` contains `PRIMARY KEY ("a", "b")`;
  pushed `Category`/`Post` with a nested `categories: { create }`; on Postgres, pushed `User` with `role: "ADMIN"`, a
  `@db.Uuid` filter on pushed `Wallet`, `scores: [1, 2]`; verify each fails today with the error design.md names (`P2021`,
  `type "role_t" does not exist`, `operator does not exist: text = uuid`) and record the output
- [x] 2.3 `crates/corvis-orm/tests/typed_params.rs` (M4): `score::equals(f64::NAN).value()` and `score::not(..)` are not
  `null`; on SQLite with rows `1.0`, `2.0`, `null`, `delete_many(vec![score::not(f64::NAN)])` leaves at least the rows a
  value comparison leaves; a helper value whose `Serialize` fails makes `find_many` fail; verify they fail today and record
  the output
- [x] 2.4 `crates/corvis-orm/tests/parameter_limits.rs` (S4): on SQLite, `createManyAndReturn` of 40,000 rows,
  `updateManyAndReturn` of 40,000 rows, an include over a two-field relation with 20,000 parents, a nested `set` of 40,000
  children, `in` with `mode: insensitive` and 40,000 values; the Postgres versions with 70,000 rows when Postgres is
  available; verify each fails today with the database's parameter-limit error and record it

## 3. Reference recordings (Decision 1)

- [ ] 3.1 SQLite recorder `corvis-tests/tmp/orm/oracle/sqlite/`: `schema.prisma` (the oracle schema minus what the
  original's SQLite connector rejects, listed in its header), `migration.sql` produced by the original's
  `migrate diff --from-empty --to-schema-datamodel`, `cases.js` (the applicable Postgres cases plus the SQLite cases of
  Decisions 2, 4 and 7: the four amounts, `increment` by `0.01` of `12345678901234.56`, `0.1 + 0.2`, `_sum`, `_avg`,
  `having`; `ratio` rows `NaN` and `null` with `equals`/`not`/`delete` by `NaN`; `divide: 0` on `Int`/`Float`/`Decimal`;
  `increment` past `2147483647`; `contains` insensitive with `José`/`JOSÉ`; a JSON path `["0"]` on an object; a two-term
  `search`), `$float` in `revive`, `regen.sh` writing `crates/corvis-orm/tests/fixtures/oracle_sqlite/`; generate
  `tests/generated/oracle_sqlite.rs` with `corvis-orm generate`; verify the results file names `recorded_with` and `script`
- [ ] 3.2 Postgres oracle: append the Decision 4 and 7 cases at the end of `corvis-tests/tmp/orm/oracle/full/cases.js`, add
  `$float` to `revive`, run `regen.sh`; add the `$float` marker to `tests/pg_oracle.rs`; verify `git diff` of
  `tests/fixtures/oracle/results.json` shows only appended cases, and record in Implementation notes what the original
  does for each of them (non-finite floats as values or as `null`; each arithmetic case)
- [ ] 3.3 `crates/corvis-orm/tests/sqlite_oracle.rs`: the runner of design.md Decision 1 (migrated database and pushed
  database, `KNOWN_DIFFERENCES` with label, database and reason, failing when an entry no longer differs); add its
  allowlist entry to `scripts/check-naming.mjs` (it maps errors to the original's class names, like `pg_oracle.rs`);
  verify it runs, and list in Implementation notes the cases that fail before groups 4-9

## 4. `Decimal` on SQLite (Decision 2)

- [ ] 4.1 `schema.rs`: `DECIMAL` for `Decimal` on SQLite; `engine/filter.rs`, `engine/read.rs` (`order_sql`,
  `cursor_sql`), `engine/aggregate.rs` (`agg_expr_sql`, `having`), `engine/write.rs` (`set_list_sql` arithmetic): a
  `Decimal` column on SQLite written `CAST(<col> AS NUMERIC)`, through one helper; the `data_value` comment of
  `engine/mod.rs` rewritten; if the recording of 3.1 contradicts design.md's expectation, follow the recording and write
  why in Implementation notes; verify `--test sqlite_decimal` passes and the `Decimal` cases of `--test sqlite_oracle`
  pass on both databases (code and `sqlite_decimal` done; the `sqlite_oracle` verification waits for the recording of
  3.1)
- [x] 4.2 Verify the snapshot test of 1.2 passes unchanged, and `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm --test
  pg_oracle --test pg_houseplan` passes

## 5. `db push` (Decision 3)

- [x] 5.1 `schema.rs`, Postgres: `CREATE SCHEMA IF NOT EXISTS` for `@@schema`, enum types (mapped values, schema order,
  qualified), primary key over `pk()` named by `primary_key_map` or `<table>_pkey`, native types of design.md's list,
  scalar lists as arrays, `TIMESTAMP(3)`, defaults (`CURRENT_TIMESTAMP`, `dbgenerated`, enum `@map` values),
  `Unsupported` columns, every identifier through `quote_ident`; verify the Postgres tests of 2.2 pass
- [x] 5.2 `schema.rs`: `@unique`/`@@unique` and `@@index` (names, `map`, `USING <type>`), implicit many-to-many join tables
  (columns, key and `B` index named as the reference, cascading foreign keys), foreign keys as `ALTER TABLE .. ADD
  CONSTRAINT` named by `RelationMeta.map` or `<table>_<cols>_fkey`, statement order of the reference; verify the
  many-to-many test of 2.2 passes on both dialects
- [ ] 5.3 `schema.rs`, SQLite: the types, keys, indexes, join tables and inline foreign keys of the recorded
  `oracle_sqlite/migration.sql`; verify the SQLite tests of 2.2 pass (the SQLite tests of 2.2 pass; matching the recorded
  SQLite migration waits for the recording of 3.1)
- [x] 5.4 Refusals before any statement (unknown native type, index type or option; views skipped) with a validation
  error naming model, field and construct; `client.rs` `db_push_models` and `db_push_force_reset` in one transaction with
  `ensure_schema` (N6); tests: a push whose third model fails leaves no table; a model and an enum whose names contain `"`
  and `;` are pushed with quoted identifiers; verify `cargo test -p corvis-orm --test db_push`
- [ ] 5.5 Catalog comparison tests in `tests/db_push.rs`: Postgres reference migration (without its extras block) vs
  `db_push` of the oracle models; SQLite recorded migration vs `db_push` of the SQLite oracle models; the houseplan models
  vs the houseplan migrations when `corvis-tests` and the `vector` extension are available (skipped with a message
  otherwise); every remaining difference fixed or listed in the test with its reason; then both oracles on pushed
  databases (the Postgres runner gains a pushed run with the extras block applied after the push); verify all pass
  (Postgres catalog, houseplan catalog and the pushed Postgres oracle run pass; the SQLite catalog and the pushed SQLite
  oracle run wait for the recording of 3.1)

## 6. Typed helpers (Decision 4)

- [x] 6.1 `query.rs`: `to_json` returning `Result`, non-finite floats handled before `serde_json` per the recording of 3.2
  (engine strings or a validation error); a private `error` on the five param types with `try_new`; `where_object`,
  `data_object`, `and`, `or`, `not`, `Args`, `WhereArg`, `DataArg` and `Query::with_error` carrying the first error;
  verify `cargo test -p corvis-orm --lib` (non-finite floats are refused for now: provisional until the recording of 3.2
  decides between refusal and the engine strings)
- [ ] 6.2 `corvis-orm-macros/src/lib.rs`: every helper through `try_new`; nested relation helpers carry errors; tests in
  `tests/typed_params.rs`, one per helper family of design.md Decision 4, plus the `NaN` cases of both oracles; verify 2.3
  passes and `--test client --test codegen_schemas` pass (all done except the `NaN` cases of both oracles, which wait for
  the recordings of 3.1 and 3.2)

## 7. No panic on caller input (Decision 5)

- [x] 7.1 `decimal.rs`: remove `From<&str>`, `From<String>`, `From<&String>`; add `TryFrom<&str>` and `TryFrom<String>`;
  `decimal!` proc macro in `corvis-orm-macros` (compile-time parse with the same grammar), re-exported from `corvis-orm`
  and its prelude, with a `compile_fail` doctest for an invalid literal; move the in-repository callers (`decimal.rs`
  tests, `tests/decimal_js.rs`, `tests/pg_features.rs`, `tests/pg_oracle.rs`, README) to `decimal!` or `parse`; verify
  `cargo test -p corvis-orm --doc` and `--test decimal_js`
- [x] 7.2 `raw.rs`: `RawSql` carries an optional error; `join` of an empty list sets it; `append` and the `sql!`
  interpolation propagate it; `_query_raw`, `_query_raw_as`, `_execute_raw` fail with a validation error before any SQL;
  tests for direct and nested use; verify `cargo test -p corvis-orm --lib raw`
- [ ] 7.3 Port apps (outside the repository, not git repositories: back up each file first): replace the `Decimal::from`
  calls with text in `consi-backend` (`common/money/money.spec.rs`, `common/money/commission.spec.rs`,
  `exchange-rate/rate-oracle.service.rs`, `fx/fx.service.spec.rs`, `settlement/settlement.service.spec.rs`) and
  `facten-api` (`emissions/emission-document.service.rs`, `fiscal-audit/fiscal-audit.service.rs`,
  `fiscal-data/fiscal-data.service.rs`, `fiscal-notes/fiscal-notes.service.rs`, the run-time conversion, with an error
  path instead of a panic) and any further site the compiler reports; list the files in Implementation notes; verify the
  three apps build without warnings against the change and their unit tests pass

## 8. Key lists within the parameter limit (Decision 6)

- [x] 8.1 `engine/`: `key_condition` with chunking for compound keys (budget: half the dialect's limit); move
  `read_back_many`, `ids_filter` and its four callers, `m2m_unlink`, the compound include, `count_related`, the nested
  `delete`/`disconnect` lists and the insensitive `in`/`notIn` to it; a unit test that a chunked update repeats the
  caller's filter in each statement; verify `--test parameter_limits` passes (2.4) and `--test security` passes unchanged

## 9. Dialect differences (Decision 7)

- [ ] 9.1 `engine/write.rs` (and the `Int` handling of `engine/mod.rs` if the recording needs it): `divide: 0` and the
  `Int` overflow of `increment` made equal to the recording on each dialect where they differ today, on that dialect
  only; verify the arithmetic cases of both oracles pass
- [ ] 9.2 `docs/data/corvis-orm.md` and `docs/es/data/corvis-orm.md`: the table "PostgreSQL and SQLite differences"
  (case folding, JSON path segments, `search`, `Decimal` precision, scalar lists as a Corvis extension on SQLite, the
  arithmetic cases as recorded); verify `npm run docs:check` in `docs/` (table written, docs check passes; its two
  arithmetic rows state Corvis' current behavior and wait for the recordings of 9.1)

## 10. Docs and review

- [x] 10.1 Docs (English and Spanish pages, `crates/corvis-orm/README.md`, rustdoc): what `db push` creates, refuses and
  does not alter, and how to re-create a development database; `Decimal` on SQLite; `decimal!` and the removal of
  `From<&str>` (with the migration of a call site); typed helpers and non-finite floats; `RawSql::join` of an empty list;
  verify `cargo doc -p corvis-orm --no-deps` without warnings and `npm run docs:check`
- [ ] 10.2 Security review with the `security-reviewer` agent (DDL generation and identifier quoting, `dbgenerated` and
  `Unsupported` text, the `CAST` insertion, chunked statements and their filters, deferred errors in typed params and
  `RawSql`); repeat rounds until no open finding; record findings and fixes in design.md under "Security review"; verify
  every finding has a regression test or a recorded reason

## 11. Definition of done

- [x] 11.1 `cargo build --workspace --all-targets` with no warnings
- [x] 11.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis-orm --no-default-features -- -D warnings`
- [x] 11.3 `CORVIS_ORM_PG_REQUIRED=1 cargo test --workspace` passes, including `pg_oracle` (and its pushed run),
  `sqlite_oracle`, `db_push`, `sqlite_decimal`, `typed_params`, `parameter_limits` and the snapshot test of 1.2
  (2114 passed, 0 failed, 97 ignored; `sqlite_oracle` does not exist yet, see 3.3)
- [x] 11.4 Per-feature checks: `cargo check -p corvis-orm --tests --no-default-features` and
  `cargo check -p corvis-orm --tests --no-default-features --features corvis`, with no warnings
- [ ] 11.5 Regression tests: one per spec scenario, as listed in groups 2-9
- [x] 11.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 11.7 Framework code changed (`corvis-orm`, `corvis-orm-macros`): run the `corvis-parity` check and verify the three
  port apps keep 0 apidiff differences and equivalent databases
- [x] 11.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 11.9 The security review of 10.2 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (new capabilities `orm/reference-results` and `orm/db-push`; deltas of
  `orm/query-filters` and `orm/client` applied).
- Close the issue, tick it in epic #187 and move its card to Done (see the `corvis-task` skill).
- Then start `q-orm-hot-path`, whose task 1.1 re-checks its lines against the code this change leaves.
- Open the follow-ups the maintainer accepts: exact decimals on SQLite as a Corvis extension; row-value key lists if
  `q-orm-hot-path`'s numbers call for them.

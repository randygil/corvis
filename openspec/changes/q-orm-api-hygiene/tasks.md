# Tasks

Start after `q-orm-hot-path` is merged into `dev`. Shared Cargo target directory only (never a target directory per task).
Postgres tests need the local server (`corvis-pg`, `127.0.0.1:55432`) and run with `CORVIS_ORM_PG_REQUIRED=1`. Recorders
live in `corvis-tests/tmp/orm/` (outside the repository) and use scratch clients of the original, as `connect/cases.js`
does. Timing rule for every test: wait for conditions with a deadline that only fails the test, use the paused clock where
the code under test only uses tokio's timers, never `sleep` to synchronize, never compare durations between runs. Behavior
must not change except where design.md "Behavior visible to applications" says so.

## 1. Reconcile

- [ ] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev` with the two
  earlier ORM changes merged; write moved lines and any conflict in a new "Implementation notes" section of design.md
  before any other task; verify `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm` passes on the untouched tree

## 2. Migration lock (Decision 1)

- [ ] 2.1 Recording: `corvis-tests/tmp/orm/migrate/cases.js` records, on Postgres, whether the original's `migrate deploy`
  waits on an advisory lock held by another session and on which `classid`/`objid` (from `pg_locks`), and that it finishes
  once the lock is released; the same question on SQLite with an exclusive transaction held; output
  `crates/corvis-orm/tests/fixtures/migrate_lock.json` with `recorded_with` and `script`; verify the file and write the
  lock id and the SQLite behavior in Implementation notes
- [ ] 2.2 `migrate.rs`: the lock connection (direct, bounded by `connect_timeout`), `pg_advisory_lock(<recorded id>)`
  before reading the history, unlock and close on every exit path; SQLite per the recording; each migration read once (the
  hashed bytes are the script, non-UTF-8 is an initialization error naming the file); `Row::id` and `let _ = &r.id;`
  removed; verify `--test pg_schema_url --test pg_connect_timeout` pass unchanged
- [ ] 2.3 `crates/corvis-orm/tests/pg_migrate_lock.rs`: the three scenarios of the spec requirement "Concurrent deploys apply
  each migration once" (condition waits on `pg_locks`), the checksum scenario, and the lock connection to a silent server
  failing with `P1001`; verify the suite passes 5 runs in a row

## 3. Errors (Decision 2)

- [ ] 3.1 Recording: `corvis-tests/tmp/orm/errors/cases.js` records the original's error (class, code, meta, message) for a
  deadlock between two interactive transactions (40P01), a serialization failure (40001) and a statement timeout (57014);
  output `crates/corvis-orm/tests/fixtures/errors.json` with `recorded_with` and `script`; verify the file
- [ ] 3.2 `error.rs`: boxed variants; `#[non_exhaustive]` structs with constructors; private `cause` filled by the three
  `from_sqlx*` functions; `source()`; the mappings of 3.1; remove `#[allow(clippy::result_large_err)]` from the four test
  files; update the crate's own constructions; verify `cargo clippy -p corvis-orm --all-targets -- -D warnings` and both
  oracles pass
- [ ] 3.3 `crates/corvis-orm/tests/errors.rs`: the scenarios of the spec requirements "Errors keep their cause and stay
  small" and "Database errors are mapped as the original maps them" (the deadlock driven with a handshake between the two
  transactions, no sleep), and an error with a cause whose `Display` holds no URL part; verify the suite

## 4. Public surface and ids (Decisions 3 and 4)

- [ ] 4.1 `#[non_exhaustive]` on the eleven enums of design.md Decision 3 with a `compile_fail` doctest each; check that the
  derive's output only constructs them; `#[doc(hidden)]` on `SqlValue::Cast` and `SqlValue::OutOfRange`; `util` made
  `pub(crate)`; `_query_raw_unsafe` and `_execute_raw_unsafe` removed (and their mentions in docs and README); verify
  `cargo test -p corvis-orm --doc`, `cargo doc -p corvis-orm --no-deps` without warnings, and that the three port apps build
- [ ] 4.2 `util.rs`: `random_bytes` from `getrandom` 0.4 (direct dependency in `crates/corvis-orm/Cargo.toml`, same version
  as in `Cargo.lock`); `cuid`, `cuid2`, `nanoid`, `ulid` drawing from it; a generator failure surfaced through
  `default_value` as an `OrmError`; the randomness tests of design.md Decision 4; verify `cargo test -p corvis-orm --lib util`
  and `cargo deny check`

## 5. Small items (Decision 5)

- [ ] 5.1 `db.rs`: `connect_timed_out` in seconds or milliseconds, its two unit tests updated; the duplicated summary line of
  `coerce_field_d` removed; verify `cargo test -p corvis-orm --lib`

## 6. Refactors as pure moves (Decision 5)

- [ ] 6.1 Before touching the macro, a unit test in `corvis-orm-macros` that runs `expand` on the models of
  `crates/corvis-orm/tests/generated/{oracle,consi,facten,houseplan}.rs` (and the SQLite oracle models) and compares the
  token text with a snapshot file it creates on the first run; commit the snapshot; then split `expand` into functions per
  concern; verify the snapshot test passes unchanged and `cargo test -p corvis-orm --test codegen_schemas`
- [ ] 6.2 `engine/write.rs`: `nested_relation_writes` split into one function per nested operation; verify both oracles and
  `--test client --test security` pass unchanged

## 7. Tests (Decision 6)

- [ ] 7.1 `db.rs` unit tests with fake futures on `#[tokio::test(start_paused = true)]`; `pg_features.rs`: the timed-out
  transactions wait on the server state, the `max_wait` holder waits for a release message; a comment on
  `client_pool_tests.rs:170-178` citing the requirement "Pool rules are verified without timing races"; grep the ORM suites
  for `sleep(` and `elapsed()` and justify each one left in a comment beside it; verify each changed suite passes 5 runs in a
  row and record the grep result in Implementation notes
- [ ] 7.2 `tests/fuzz_filters.rs`: the differential property of design.md Decision 6 (SQLite and Postgres, same seeded rows,
  same ids), excluding only the documented differences; every new difference it finds is fixed or documented and excluded
  with a comment; verify `CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm --test fuzz_filters`

## 8. Docs and review

- [ ] 8.1 Docs (English and Spanish pages of `docs/data/corvis-orm.md`, README, rustdoc): concurrent deploys and the lock
  (Postgres and SQLite), errors' `source()`, the removed aliases and `util`, `#[non_exhaustive]` enums; verify
  `npm run docs:check` in `docs/`
- [ ] 8.2 Security review with the `security-reviewer` agent (the lock connection and its release on every path, secrets in
  errors with a cause, the id generator); repeat rounds until no open finding; record findings and fixes in design.md under
  "Security review"; verify every finding has a regression test or a recorded reason

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes (with no `result_large_err` allowance left in the ORM
  tests), and `cargo clippy -p corvis-orm --no-default-features -- -D warnings`
- [ ] 9.3 `CORVIS_ORM_PG_REQUIRED=1 cargo test --workspace` passes, including `pg_migrate_lock`, `errors`, both oracles and
  the differential fuzz
- [ ] 9.4 Per-feature checks: `cargo check -p corvis-orm --tests --no-default-features` and
  `cargo check -p corvis-orm --tests --no-default-features --features corvis`, with no warnings
- [ ] 9.5 Regression tests: one per spec scenario, as listed in groups 2-4
- [ ] 9.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 9.7 Framework code changed (`corvis-orm`, `corvis-orm-macros`): run the `corvis-parity` check and verify the three port
  apps keep 0 apidiff differences and equivalent databases
- [ ] 9.8 `cargo deny check`, `cargo audit` and `node scripts/check-naming.mjs` pass
- [ ] 9.9 The security review of 8.2 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (new capability `orm/migrations`; delta of `orm/client` applied).
- Close the issue, tick it in epic #187 and move its card to Done (see the `corvis-task` skill).
- The ORM part of epic #187 is then complete; open any follow-up the maintainer accepted in the three changes' review notes.

# Tasks

## 1. Reproduce

- [x] 1.1 Loop the `security` test binary filtered to `delete_and_upsert` (60 runs) and the `schedule` binary filtered
  to `wait_for_completion_skips` (50 runs); record the failure rates (design.md, evidence)
- [x] 1.2 Find the origin of `os error 10022` with a raw Tokio connect probe (temporary, not committed)

## 2. Connection retry in corvis-orm

- [x] 2.1 Add `retry_connect` and `acquire_pg` to `crates/corvis-orm/src/db.rs` and use them for the first pool
  connection, `fetch_driver`, `execute_driver`, `execute_script_raw` and `DbPool::begin` (Decision 1); verify
  `cargo build -p corvis-orm` has no warnings
- [x] 2.2 Unit tests of the helper: a transient error is retried until success, a persistent one is returned
  unchanged after five attempts, refused/timeout/closed are not retried; verify with `cargo test -p corvis-orm --lib`
- [x] 2.3 Make `tests/common/pg.rs::fresh_db` retry before skipping (Decision 2)

## 3. Scheduler test

- [x] 3.1 Change the durations of `wait_for_completion_skips_overlapping_ticks_and_default_overlaps` and document the
  margins (Decision 3); verify 50 consecutive green runs of the test binary

## 4. Verification

- [x] 4.1 Postgres test: 300 runs of the filtered binary and repeated full `cargo test -p corvis-orm` runs, all green
- [x] 4.2 `cargo build --workspace --all-targets` without warnings, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, `cargo fmt --all --check` if `rustfmt.toml` exists
- [x] 4.3 Run the corvis-parity check before archiving (runtime code of corvis-orm changed) — maintainer run on 5500e5b: consi 832/832, facten 727/727, houseplan 598/598 (quiet-hours count), routes identical, databases equivalent

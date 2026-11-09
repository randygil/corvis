# Tasks

## 1. Gate helper

- [x] 1.1 Add `crates/corvis-orm/tests/common/gate.rs` with `Gate::close`, `Gate::query_sql`, `Gate::sample`,
  `Gate::wait_for` and `Gate::open` (design.md, decision 1), `#![allow(dead_code)]` like `common/pg.rs`, and a
  module comment that states the bound policy of decision 5. Include it from `pg_pool_url.rs` with
  `#[path = "common/gate.rs"] mod gate;`. Verify: `cargo test -p corvis-orm --test pg_pool_url --no-run` builds
  with no warnings
- [x] 1.2 Self-test of the gate in `pg_pool_url.rs` (`gate_holds_queries_until_opened`): a client with
  `connection_limit=2`, 5 gated queries; `wait_for(2)` returns `(2, 2)`; no query finished before `open`; all 5 succeed
  after it. Verify: `cargo test -p corvis-orm --test pg_pool_url gate_` with Postgres running
  (`CORVIS_ORM_PG_REQUIRED=1`)

## 2. `pg_pool_url.rs`

- [x] 2.1 Replace `peak` with `gated_peak(base, db, n, expected)` (decision 2) and update
  `recorded_connection_counts_match`, `limit_of_two_serves_six_queries`, `builder_wins_over_the_url` and
  `tls_parameter_is_kept_and_limit_applies`; assertion messages name the URL case and the observed maxima. Verify:
  `cargo test -p corvis-orm --test pg_pool_url`
- [x] 2.2 Rework `wait_behind_transaction` and `recorded_waits_match` (decision 3): release oneshot, holder
  `timeout(120_000)`, `T` parsed from the recorded message, success cases checked for pending at the hold time.
  Verify: same command; then, as a temporary mutation, make the client ignore `pool_timeout` (in `src/client.rs:292-299`
  use `DEFAULT_POOL_TIMEOUT` for `Timeout::Seconds`) and confirm the timeout cases fail on the upper bound (a wait near
  10 s is above `T + 2 s`); revert
- [x] 2.3 Mutation check of the size tests: temporarily make the Postgres pool ignore `connection_limit` (in
  `src/client.rs:290` use `default_pool_size()` instead of `params.connection_limit`) and confirm
  `recorded_connection_counts_match` fails with an over-limit maximum for a limited case; revert. Record the results of
  2.2 and 2.3 in design.md under "Implementation notes"

## 3. `pg_connect_timeout.rs`

- [x] 3.1 Include the gate helper; rewrite `waiting_for_a_slot_is_not_cut_by_the_connect_timeout` and
  `pool_timeout_still_bounds_the_wait_for_a_slot` with the handshake and test-driven release (decision 4). Verify:
  `cargo test -p corvis-orm --test pg_connect_timeout slot`
- [x] 3.2 Adjust the bounds of `role_at_its_connection_limit_fails_after_the_connect_timeout`,
  `server_lost_after_start_fails_the_next_query_after_the_connect_timeout` (plus the 10 s guard),
  `a_schema_the_database_cannot_encode_is_reported_at_once` (URL with `connect_timeout=30`),
  `fixture_silent_server_timings_match`, `fixture_refused_port` and `fixture_database_side_failures`, each assertion
  message naming the wrong value its upper bound excludes. Verify: `cargo test -p corvis-orm --test pg_connect_timeout`
- [x] 3.3 Replace the final timed pair of `cancelled_queries_and_scripts_do_not_leak_slots` with the gate check, and
  the fixed 200 ms cancel of `a_cancelled_transaction_keeps_its_slot_until_the_connection_is_back` with the observer
  wait. Verify: same command; and, in a scratch copy, skip releasing one slot on cancellation (or hold one extra slot
  from the test) to confirm the leak test now fails; revert

## 4. Load verification

- [x] 4.1 With Postgres running and `CORVIS_ORM_PG_REQUIRED=1`, run both test binaries 10 times in a loop while every
  core is busy (a CPU burner with one busy thread per logical core, for example a small Node or PowerShell script in
  the scratchpad, plus `cargo test -p corvis --no-run` against the shared target dir). Do not run two instances of the
  same test binary at once: they share database names. Verify: 10/10 passes for both binaries; record the runs and the
  load in design.md under "Implementation notes"

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 5.3 `cargo test --workspace` passes (with Postgres, `CORVIS_ORM_PG_REQUIRED=1`)
- [x] 5.4 Regression tests: the gate self-test (1.2), the reworked pool and connect-timeout tests, and the mutation
  checks of 2.2, 2.3 and 3.3 recorded in design.md
- [x] 5.5 Docs: no page under `docs/` or `docs/es/` describes these tests, so no docs change; confirm with
  `grep -rn "pg_pool_url\|pg_connect_timeout" docs` and state it in the commit message
- [x] 5.6 corvis-parity: not required, only test files of `crates/corvis-orm/tests/` change (no framework runtime
  code); state this in the commit message

## Workflow follow-up

- [x] Archive the change with `/opsx:archive`
- [x] Close issue [#147](https://github.com/randygil/corvis/issues/147)
- [x] Tick #147 in its epic [#13](https://github.com/randygil/corvis/issues/13)
- [x] Move the card to Done in the Corvis Roadmap project

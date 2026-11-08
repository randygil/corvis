# Tasks

## 1. Bind the Postgres path

- [x] 1.1 In `crates/corvis-orm/src/engine/filter.rs` (`json_at`), bind the Postgres JSON path as a `text[]` parameter (`#> $n::text[]`, `#>>` for text) instead of an escaped literal; verify `cargo build -p corvis-orm` has no warnings
- [x] 1.2 Check the other dialect branches: SQLite already binds the path (unchanged) and corvis-orm has no MySQL dialect; verify with a search for inlined path text in `src/`

## 2. Tests

- [x] 2.1 Add `crates/corvis-orm/tests/pg_json_paths.rs` with an SQL-text test (no server needed): no path segment in the generated SQL, `#> $1::text[]` present; verify with `cargo test -p corvis-orm --test pg_json_paths`
- [x] 2.2 In the same file, add Postgres tests with hostile keys (quotes, backslashes, braces, commas, double quotes, unicode, statement-shaped text, empty key, `$.a`) that check exact rows; start the server with `docker start corvis-pg` and verify they pass
- [x] 2.3 Add the Postgres JSON-path differential property against a Rust evaluation, skipped without a server; verify it passes at the default count and with `PROPTEST_CASES=5000`

## 3. Review follow-up: memory amplification

- [x] 3.1 Add `Sql::bind_shared` / `reset_shared` (`db.rs`) and use them in `json_at` so one filter binds its Postgres path once and references `$n::text[]` after that; verify with the SQL-text test `a_filter_binds_its_path_once_whatever_the_number_of_operators`
- [x] 3.2 Cap JSON filter paths at 128 segments (list and string form, both dialects) with a validation error before any SQL is built; verify with `paths_over_the_segment_cap_are_a_validation_error_on_both_dialects`
- [x] 3.3 Extend the SQL-text test to `string_starts_with`/`ends_with`, `array_starts_with`/`ends_with`, `mode` and string-form paths, and add a Postgres test that a shared path parameter works with several operators and that the cap is enforced end to end
- [x] 3.4 Verify with `cargo clean -p` of the workspace crates before each build, since worktrees share one target dir and a stale `libcorvis_orm` rlib can be reused

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings`: the code added by this change adds no lints (`cargo clippy -p corvis-orm --all-targets`); the existing workspace lints are tracked in #108
- [x] 4.3 `cargo test --workspace` passes (the 3 known CRLF failures of `corvis-swagger` are tracked in #110)
- [x] 4.4 Runtime code changed (`engine/filter.rs`): run the `corvis-parity` check and verify the three ports keep 0 differences (left to the maintainer's parity run)
  - Result (with the review fix `aef5678`): consi 832/832, facten 727/727, houseplan 598/598 (quiet-hours count), routes identical, databases equivalent. Security review of the fix: verified (no findings; memory back to baseline).
- [x] 4.5 Docs: no user-facing change, so no `docs/` page (EN/ES) is needed; confirm

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #109, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

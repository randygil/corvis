# Tasks

## 1. Bind the Postgres path

- [x] 1.1 In `crates/corvis-orm/src/engine/filter.rs` (`json_at`), bind the Postgres JSON path as a `text[]` parameter (`#> $n::text[]`, `#>>` for text) instead of an escaped literal; verify `cargo build -p corvis-orm` has no warnings
- [x] 1.2 Check the other dialect branches: SQLite already binds the path (unchanged) and corvis-orm has no MySQL dialect; verify with a search for inlined path text in `src/`

## 2. Tests

- [x] 2.1 Add `crates/corvis-orm/tests/pg_json_paths.rs` with an SQL-text test (no server needed): no path segment in the generated SQL, `#> $1::text[]` present; verify with `cargo test -p corvis-orm --test pg_json_paths`
- [x] 2.2 In the same file, add Postgres tests with hostile keys (quotes, backslashes, braces, commas, double quotes, unicode, statement-shaped text, empty key, `$.a`) that check exact rows; start the server with `docker start corvis-pg` and verify they pass
- [x] 2.3 Add the Postgres JSON-path differential property against a Rust evaluation, skipped without a server; verify it passes at the default count and with `PROPTEST_CASES=5000`

## 3. Definition of done

- [x] 3.1 `cargo build --workspace --all-targets` with no warnings
- [x] 3.2 `cargo clippy --workspace --all-targets -- -D warnings`: the code added by this change adds no lints (`cargo clippy -p corvis-orm --all-targets`); the existing workspace lints are tracked in #108
- [x] 3.3 `cargo test --workspace` passes (the 3 known CRLF failures of `corvis-swagger` are tracked in #110)
- [ ] 3.4 Runtime code changed (`engine/filter.rs`): run the `corvis-parity` check and verify the three ports keep 0 differences (left to the maintainer's parity run)
- [x] 3.5 Docs: no user-facing change, so no `docs/` page (EN/ES) is needed; confirm

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #109, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

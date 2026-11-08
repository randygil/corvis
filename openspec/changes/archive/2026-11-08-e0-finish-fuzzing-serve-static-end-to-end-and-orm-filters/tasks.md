# Tasks

## 1. Parser properties already written

- [x] 1.1 Review `crates/corvis/src/fuzz_tests.rs` and `crates/corvis/src/multer/fuzz.rs` against the `http/request-parsing`, `http/trust-proxy` and `auth/jwt` specs, adding any missing check; verify `cargo test -p corvis --lib fuzz` passes all properties at the default case count
- [x] 1.2 Keep `crates/corvis/proptest-regressions/` (the empty-boundary seed) and verify the seed is replayed by running the multipart properties once more
- [x] 1.3 Verify `cargo clippy -p corvis --all-targets -- -D warnings` is clean for the new test modules

## 2. Static files end to end

- [x] 2.1 Add `crates/corvis/tests/fuzz_static.rs`: served folder with `a.txt`, `sub/b.txt`, `.env`, `.git/config` and an `outside/secret.txt` sibling; an app built once with `serve_static` mounts using the default policy, `deny` and `allow`; a `TestRunner` over generated URI-valid paths; verify with `cargo test -p corvis --test fuzz_static`
- [x] 2.2 Assert the `http/static-files` scenarios per case (no secret text in any body, no dotfile text outside the `allow` mount, every request answered with a status below 500); fix small local issues it finds (for example an I/O error on a malformed path surfacing as a 500 instead of a 4xx) with a regression test, or open an issue for larger ones; verify the test passes
  - Review follow-up: root mount and `ServeStaticModule` (`spa_fallback`, `exclude`) mounts added, raw backslashes generated, and the generator now fails if it produces an invalid URI instead of skipping it.

## 3. ORM filters

- [x] 3.1 Add `crates/corvis-orm/tests/fuzz_filters.rs`: in-memory SQLite with a seeded model, generated `where` arguments (real and unknown fields, operators, injection-shaped values); verify with `cargo test -p corvis-orm --test fuzz_filters`
- [x] 3.2 Assert the `orm/query-filters` scenarios (no panic, no database syntax error, exact `equals` results, table unchanged after the run); fix small local issues with a regression test, or open an issue for larger ones; verify the test passes
  - Review follow-up: fixed SQLite JSON `path` filters (`equals`, `not`, `lt`/`lte`/`gt`/`gte`, `string_*`, `array_contains`, JSON null) in `engine/filter.rs`, which compared `json_extract` output with quoted JSON text; regression tests in `tests/json_filters.rs` and a differential property against a Rust evaluation in `tests/fuzz_filters.rs`. The error oracle now only allows the `P2000` / `P2023` known-request codes and requires a validation error for unknown keys.

## 4. Long run and documentation

- [x] 4.1 Run every property with `PROPTEST_CASES=100000` (corvis lib fuzz modules, `fuzz_static`, `fuzz_filters`); verify all pass and commit any new seeds
- [x] 4.2 Create `CONTRIBUTING.md` at the repository root (it does not exist yet) with a short "Building and testing" section (the definition-of-done commands) and a "Fuzzing" section (what is fuzzed, how to run longer with `PROPTEST_CASES`, what to do with a failing seed in `proptest-regressions/`); verify the documented commands run as written

## 5. Definition of done

- [x] 5.1 `cargo build --workspace --all-targets` with no warnings
- [x] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
  - Result: the code added by this change is clippy-clean; the workspace run still fails on lints that predate it (Rust 1.99), moved to #108, which is the next issue of the same phase (#111).
- [x] 5.3 `cargo test --workspace` passes
- [x] 5.4 If any task changed framework code (not only tests), run the `corvis-parity` check and verify the three ports keep 0 differences; otherwise record that no runtime code changed
- [x] 5.5 Docs: this change adds no user-facing feature, so no `docs/` page (EN/ES) is needed beyond `CONTRIBUTING.md`; confirm

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #2, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

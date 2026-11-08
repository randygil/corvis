# Contributing

## Building and testing

Every change must pass these three commands, with no warnings:

```sh
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use the shared Cargo target directory of the repository; do not create one per task. The Postgres tests of
`corvis-orm` read `CORVIS_ORM_PG` and are skipped when the server is not reachable.

## Fuzzing

Hostile input is checked with property-based tests (`proptest`, stable Rust, run by `cargo test`):

- `crates/corvis/src/fuzz_tests.rs` and `crates/corvis/src/multer/fuzz.rs`: the JSON, query string, body and
  multipart parsers, JWT verification, route paths and trust proxy.
- `crates/corvis/tests/fuzz_static.rs`: static file serving end to end, with generated request paths against a
  served folder that has a secret file next to it.
- `crates/corvis-orm/tests/fuzz_filters.rs`: `where` filters on in-memory SQLite.

Each property runs 2,000 cases by default. Run more with `PROPTEST_CASES`:

```sh
PROPTEST_CASES=100000 cargo test -p corvis --lib fuzz
PROPTEST_CASES=100000 cargo test -p corvis --test fuzz_static
PROPTEST_CASES=100000 cargo test -p corvis-orm --test fuzz_filters
```

On PowerShell, set the variable first: `$env:PROPTEST_CASES = "100000"`.

When a property fails, proptest shrinks the input and writes its seed to a file under `proptest-regressions/` in the
crate. Fix the bug, keep the seed file and commit it: the seed is replayed on every later run. Add a plain
regression test with the shrunk input as well.

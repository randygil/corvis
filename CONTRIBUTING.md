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

### Debug info in dev and test builds

The root `Cargo.toml` trims debug info to keep the target directory small and builds fast (`cargo test` uses the same
settings, because the test profile inherits dev):

```toml
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = false
```

Corvis crates keep line tables, so panics and `RUST_BACKTRACE=1` backtraces still show `file:line`. Local variables
are not available in a debugger, and dependencies have no debug info at all. To get full debug info for the Corvis
crates in one session, override the profile from the environment without editing the file:

```sh
CARGO_PROFILE_DEV_DEBUG=true cargo test -p corvis --test app
```

Dependencies stay without debug info; to step into one, comment out the `[profile.dev.package."*"]` section in a local,
uncommitted change. Full debug info is much larger on Windows (5.8 GB against 2.1 GB after
`cargo build --workspace --all-targets`), so run `cargo clean` afterwards. Linking with `rust-lld` was measured and
does not help on this workspace, so it is not configured.

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

## Naming check

Published surfaces (crate sources and metadata, examples, the root README, the benchmark report and scripts, the
documentation site) do not use the names of the original Node framework and ORM, their packages or their links.
Nominative uses, such as the `schema.prisma` file format, the `_prisma_migrations` table or test oracles that run the
original libraries, are listed in the script's allowlist with a reason. Run it from the repository root:

```sh
node scripts/check-naming.mjs
```

It prints each violation as `path:line` and exits with status 1 when there is one, when an allowlist entry has no
reason, or when an entry no longer allows anything. `node scripts/check-naming.mjs --self-test` checks the matcher on
built-in samples. The documentation site has the same rule in `npm run docs:check` (in `docs/`).

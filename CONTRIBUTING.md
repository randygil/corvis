# Contributing

## Building and testing

Every change must pass these three commands, with no warnings:

```sh
cargo build --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Use the shared Cargo target directory of the repository; do not create one per task.

Code is formatted with rustfmt and the root `rustfmt.toml` (stable options only). Check it with:

```sh
cargo fmt --all --check
```

Install rustfmt once with `rustup component add rustfmt`. The one-time reformat of the workspace is listed in
`.git-blame-ignore-revs`; to keep `git blame` pointing at the real author of each line, run once per clone:

```sh
git config blame.ignoreRevsFile .git-blame-ignore-revs
```

### Postgres tests

The Postgres tests of `corvis-orm` (`pg_features`, `pg_oracle`, `pg_json_paths`, `pg_houseplan` and the Postgres
cases of `security`) connect to `CORVIS_ORM_PG` (default `postgres://postgres:postgres@127.0.0.1:55432`) and create
databases named `orm_test_*`. When the server is not reachable they print `skipping: Postgres not reachable` and
pass. Set `CORVIS_ORM_PG_REQUIRED=1` to make them fail instead, as CI does, so a green run means they really ran. The
failure names the server's host and port, never the password. Start a matching server, bound to the loopback interface,
with:

```sh
docker run -d --name corvis-pg -e POSTGRES_PASSWORD=postgres -p 127.0.0.1:55432:5432 postgres:16
CORVIS_ORM_PG_REQUIRED=1 cargo test -p corvis-orm
```

`pg_houseplan` also needs the original reference application outside this repository (`CORVIS_TESTS_DIR`) and still
skips without it, even with `CORVIS_ORM_PG_REQUIRED=1`.

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

When a property fails in CI, the run's `proptest-regressions` artifact holds the seed files of both crates. Download
it from the run's summary page, copy the new lines into the matching files under `crates/<crate>/proptest-regressions/`,
check that the property now fails locally, then fix the bug and commit the seed with the fix.

## Smoke test of the examples

Close every roadmap phase with a smoke test of the examples. It starts the real `blog-orm`, `tertulia` and `cats-app`
binaries, sends the documented requests and checks status codes and key fields. It builds nothing, so build the
examples first:

```sh
cargo build -p blog-orm -p tertulia -p cats-app
node scripts/smoke-examples.mjs            # or name the examples: node scripts/smoke-examples.mjs cats-app
```

The binaries come from `$CARGO_TARGET_DIR/debug` (`target/debug` when the variable is not set). Each server binds to
`127.0.0.1` on a free port, runs in a temporary directory with a temporary SQLite database and a random `JWT_SECRET`,
and is stopped at the end, when the temporary files are removed too. The script exits with status 1 when a check
fails. It is not part of CI.

## Continuous integration

The workflow `.github/workflows/ci.yml` runs on every pull request to `main` and can be started by hand from the
Actions tab (`workflow_dispatch`). The repository is private and Actions minutes are limited, so there is no push or
schedule trigger; a newer push to the same pull request cancels the run in progress. Every job must pass before
merging. Each one reproduces locally with the command in the table (on PowerShell, set environment variables first,
for example `$env:PROPTEST_CASES = "20000"`):

| Job | Runs on | Local command |
| --- | --- | --- |
| `fmt` | Linux | `cargo fmt --all --check` |
| `clippy` | Linux, Windows | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | Linux (Postgres 16 service, `CORVIS_ORM_PG_REQUIRED=1`), Windows (no Postgres) | `RUSTFLAGS="-D warnings" cargo build --workspace --all-targets`, then `RUSTFLAGS="-D warnings" cargo test --workspace` |
| `fuzz` | Linux | the three commands of [Fuzzing](#fuzzing) with `PROPTEST_CASES=20000` |
| `deny` | Linux | `cargo deny check` |
| `audit` | Linux | `cargo audit` |
| `docs` | Linux | `node scripts/check-naming.mjs`, then `npm ci` and `npm run docs:check` in `docs/` |

CI uses Rust 1.99.0, set once in `RUST_TOOLCHAIN` at the top of `ci.yml` (and repeated in `nightly.yml`), so a new
stable release cannot turn pull requests red without a code change. Bumping it is its own pull request that fixes any
new lints. `cargo deny` (0.20.2) reads `deny.toml` and `cargo audit` (0.22.2) reads `.cargo/audit.toml`; the
workflow passes no ignore flags, so every exception lives in those files with its reason. Install the same versions
with `cargo install --locked cargo-deny@0.20.2 cargo-audit@0.22.2`.

The workflow `.github/workflows/nightly.yml` (Actions > Nightly checks > Run workflow) is manual only. It runs the
fuzzing suites with 100,000 cases per property (the `cases` input changes it) and `cargo audit` against the latest
advisory database. Run it on `main` before a release, after a change to a parser, and every few weeks.

The Rust jobs restore a dependency cache. Pull requests save their own caches, which only later runs of the same pull
request can use; a manual run of `ci.yml` on `main` saves caches that every pull request can restore.

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

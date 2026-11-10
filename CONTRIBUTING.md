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
scratch databases named `orm_test_<name>_<run>` and roles named `orm_<name>_<run>`. `<run>` is
`<unix seconds>_<process id>_<4 hex digits>`, so concurrent runs against one server never share a name; tests get
names from `tests/common/pg.rs` and never hard-code them. A run drops what it created when it exits normally (a run
that fails keeps its databases for inspection), and the next run drops leftovers older than 3 hours. When the server is not reachable they print `skipping: Postgres not reachable` and
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
- `crates/corvis/src/socket_io/engine/fuzz.rs` (feature `socket-io`): the Engine.IO packet and payload codec.
- `crates/corvis/tests/fuzz_static.rs`: static file serving end to end, with generated request paths against a
  served folder that has a secret file next to it.
- `crates/corvis-orm/tests/fuzz_filters.rs`: `where` filters on in-memory SQLite.

Each property runs 2,000 cases by default. Run more with `PROPTEST_CASES`:

```sh
PROPTEST_CASES=100000 cargo test -p corvis --lib fuzz
PROPTEST_CASES=100000 cargo test -p corvis --features socket-io --lib fuzz
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

Work lands on `dev`, the integration branch, and reaches `main` through a pull request. `main` is protected: every CI
job is a required status check (for administrators too), so a change reaches `main` only through a pull request whose
jobs all passed, and force pushes and deletion are blocked. `dev` only blocks force pushes and deletion.

The workflow `.github/workflows/ci.yml` runs on every pull request to `main` or `dev`, on every push to `main` or
`dev`, and can be started by hand from the Actions tab (`workflow_dispatch`). A newer push to the same pull request
cancels the run in progress; runs on `main` and `dev` are never cancelled. Each job reproduces locally with the
command in the table (on PowerShell, set environment variables first, for example `$env:PROPTEST_CASES = "20000"`):

| Job | Runs on | Local command |
| --- | --- | --- |
| `fmt` | Linux | `cargo fmt --all --check` |
| `clippy` | Linux, Windows | `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | Linux (Postgres 16 service, `CORVIS_ORM_PG_REQUIRED=1`), Windows (no Postgres) | `RUSTFLAGS="-D warnings" cargo build --workspace --all-targets`, then `RUSTFLAGS="-D warnings" cargo test --workspace` |
| `fuzz` | Linux | the four commands of [Fuzzing](#fuzzing) with `PROPTEST_CASES=20000` |
| `deny` | Linux | `cargo deny check` |
| `audit` | Linux | `cargo audit` |
| `docs` | Linux | `node scripts/check-naming.mjs`, then `npm ci` and `npm run docs:check` in `docs/` |
| `packaging` | Linux | the commands of [Packaging](#packaging) |

CI uses Rust 1.99.0, set once in `RUST_TOOLCHAIN` at the top of `ci.yml` (and repeated in `nightly.yml`), so a new
stable release cannot turn pull requests red without a code change. Bumping it is its own pull request that fixes any
new lints. `cargo deny` (0.20.2) reads `deny.toml` and `cargo audit` (0.22.2) reads `.cargo/audit.toml`; the
workflow passes no ignore flags, so every exception lives in those files with its reason. Install the same versions
with `cargo install --locked cargo-deny@0.20.2 cargo-audit@0.22.2`.

The workflow `.github/workflows/nightly.yml` runs every day at 07:17 UTC on `dev` and can be started by hand on any
branch (Actions > Nightly checks > Run workflow). It runs the fuzzing suites with 100,000 cases per property (the
`cases` input of a manual run changes it) and `cargo audit` against the latest advisory database. Start it by hand
after a change to a parser and before a release.

The Rust jobs restore a dependency cache. Pull requests save their own caches, which only later runs of the same pull
request can use; runs of `ci.yml` on `main` and `dev` save caches that pull requests to that branch can restore.

## Packaging

Seven crates are published to crates.io: `corvis`, `corvis-macros`, `corvis-orm`, `corvis-orm-macros`,
`corvis-swagger`, `corvis-swagger-macros` and `corvis-cli` (the examples and `bench/corvis-app` have
`publish = false`). Each `.crate` file must contain the license and the third-party notices, so every crate folder
holds a copy of the root `LICENSE` and `THIRD_PARTY_NOTICES.md`. The root files are the only ones edited by hand;
after changing one, refresh the copies:

```sh
for c in corvis corvis-macros corvis-orm corvis-orm-macros corvis-swagger corvis-swagger-macros corvis-cli; do
  cp LICENSE THIRD_PARTY_NOTICES.md crates/$c/
done
```

The check needs no build. It fails when a copy differs from the root, when a published crate lacks `description`,
`license`, `authors`, `repository`, `homepage`, `rust-version`, `readme`, one to five `keywords` or a category, when
those shared fields are not inherited from `[workspace.package]`, or when a dependency on a workspace crate has no
version:

```sh
node scripts/check-crate-files.mjs
```

Dependencies between workspace crates are declared once in `[workspace.dependencies]` with `path` and `version`.
The three proc-macro crates are pinned with `=` (for example `=0.1.0`), because the code they generate calls hidden
items of their parent crate: a release always publishes every crate with the same version. `cargo deny check bans`
rejects any dependency without a version (`wildcards = "deny"`).

To package the crates as crates.io would, name the seven crates (`cargo package --workspace` also tries the
`publish = false` examples and fails on them). Cargo builds each package under `target/package/`, against a temporary
registry that holds the crates packaged before it, so this works before anything is published:

```sh
cargo package --allow-dirty -p corvis-macros -p corvis -p corvis-orm-macros -p corvis-orm -p corvis-swagger-macros -p corvis-swagger -p corvis-cli
cargo package --list -p corvis-swagger --allow-dirty    # the files of one package, without building
```

Delete `target/package/` afterwards: it holds a copy of each crate and its build output. Publishing is a separate,
explicitly approved step; never run `cargo publish` without `--dry-run`. The order is the one above: each proc-macro
crate, then the crate that uses it. `rust-version` in `[workspace.package]` is the lowest toolchain on which
`cargo check --workspace` passes with the committed `Cargo.lock`; it is informative, and CI runs one toolchain only.

## Socket.IO differential harness

`parity/socket-io/` runs the same scenarios against a Node `socket.io` server (the reference) and, once Corvis
implements Engine.IO and Socket.IO, against Corvis, records what each client observes and fails on any difference. It
is Node tooling outside every crate and is not published. The Corvis target is the Engine.IO server of
`crates/corvis/tests/engine_support/harness_app.rs`, started in-process when the test is built with `--features
socket-io`; without the feature, or for scenarios that need Socket.IO, the runner compares Node with itself, which
checks the scenarios and the normalizer on real transcripts.

Prerequisites: Node 22 or later and git. Packages are installed only by an explicit `npm ci` (install scripts are
disabled by the harness's `.npmrc`; `cargo test` never installs anything):

```sh
cd parity/socket-io
npm ci
```

Run it, from the repository root (the test skips with a notice unless `CORVIS_NODE_DIFF=1`, and with it set it fails
when Node 22+ or the install is missing):

```sh
CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_diff     # Bash
$env:CORVIS_NODE_DIFF = "1"; cargo test -p corvis --features socket-io --test socket_io_diff     # PowerShell
```

The upstream `engine.io-protocol` suite runs against the Corvis Engine.IO server, and against the Node reference as a
control, with `node scripts/fetch-suites.mjs` run once and then
`CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test engine_io_conformance`. One test of that suite
("forcefully closes the session", HTTP long-polling) sends a `GET` and a `POST` of the close packet at the same
time and expects the `GET` to arrive first; it fails now and then (one run in twenty on an idle machine, one in eight under load, against the Node reference as well) when the `POST` wins the race.

or directly, in `parity/socket-io/`:

```sh
node --test "test/*.test.mjs"    # self-tests of the normalizer, comparer and gating
node run.mjs                     # one line per scenario and target, then the differences
node run.mjs --scenario handshake    # only the scenarios whose name contains the text
```

Servers bind to `127.0.0.1` on a port chosen by the operating system, so two runs can overlap. The raw transcripts of a
run are in `parity/socket-io/out/<target>/<scenario>.json` (ignored by git). Scenarios that need Redis run only when
`CORVIS_REDIS_URL` is set (`export CORVIS_REDIS_URL=redis://127.0.0.1:6379`, or `$env:CORVIS_REDIS_URL = "..."` in
PowerShell) and are reported as skipped otherwise.

**Reading a diff.** For each scenario that differs the runner prints `DIFFERENCE in <scenario> at <path>` and the value
each target produced; the path points into the normalized transcript (`$.entries[1].data.args[1]` is the second
argument of the acknowledgement, entries being ordered by client name and then by arrival). Session and socket ids are
replaced by `<id:1>`, `<id:2>`, ... by order of first appearance, so only a different id *structure* is a difference. The
exit status is non-zero when there is an unlisted difference, a stale expected difference, or a scenario that failed or
timed out. Nothing is retried and no step sleeps: each step waits for the event it needs, with a 10 second limit.

**Adding a scenario.** Create `scenarios/<area>/<name>.mjs` exporting `{ name, requires, run }`. `requires` lists the
capabilities the scenario needs (`engine-io`, `socket-io`, `redis`). `run({ url, record, step })` drives the target at
`url` with `fetch`, `ws` or `socket.io-client`, wraps each wait in `step("label", promise)` and calls
`record(client, kind, data)` for everything a client observes (events and arguments, acknowledgements, connect errors,
disconnect reasons, HTTP status, `content-type` and `cache-control`, decoded packets). Never record timings; record a
bucket such as `"closed-after-ping-timeout"` instead. The Corvis target declares its capabilities with
`--corvis <url> --corvis-capabilities engine-io,socket-io`; a scenario it lacks runs on Node only and is reported as
skipped on Corvis. Put the id of a session or socket in a field named `sid` so the normalizer recognizes it.

**Adding an expected difference.** Only for a divergence that is intentional (for example a hardening the spec
documents). Add `{ "scenario", "path", "reason", "spec" }` to `expected-differences.json`, where `spec` names the
requirement of a `ws/*` capability that justifies it. An entry without a reason is rejected; an entry whose difference no
longer occurs fails the run as stale, so the list cannot rot.

**Protocol suites.** `node scripts/fetch-suites.mjs` downloads the official `engine.io-protocol` and
`socket.io-protocol` test suites at the commits pinned in `suites.json` into `suites/` (ignored by git) and points them
at `SUITE_URL` instead of `http://localhost:3000`. List them without a server with
`SUITE_URL=http://127.0.0.1:1 npx mocha --dry-run suites/engine.io-protocol` (PowerShell: set `$env:SUITE_URL` first). To
refresh a suite, put the new 40-hex commit in `suites.json`, run the script and the dry run, and keep the lockfile in
step if the suite imports a new package.

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

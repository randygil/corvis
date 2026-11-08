# Proposal

Implements [randygil/corvis#132](https://github.com/randygil/corvis/issues/132) (epic
[#1](https://github.com/randygil/corvis/issues/1)).

## Why

`bench/run.sh` builds the benchmark application with `CARGO_TARGET_DIR="$BENCH_DIR/target" cargo build --release -p
corvis-app` (`bench/run.sh:85`) and runs it from `bench/target/release/corvis-app.exe` (line 39). That is a second,
full Cargo target directory next to the shared one: it compiles the framework and every dependency again in release
mode, and on the maintainer's machine it holds 388 MB today. The project rule is one shared target directory (a disk
filled up once because of per-task target directories). `bench/corvis-app` is already a workspace member
(`Cargo.toml:12`), so the separate directory buys nothing: a release build in the shared target is kept apart from
debug builds by Cargo's `release/` profile folder anyway.

## What Changes

- `bench/run.sh` drops the `CARGO_TARGET_DIR` override and builds with `cargo build --release -p corvis-app` from the
  repository root, so the build goes to the target directory Cargo resolves for the workspace: `CARGO_TARGET_DIR` when
  it is set, otherwise `<repo>/target` (or a `build.target-dir` setting, if one is ever added).
- The script asks Cargo for that directory (`cargo metadata --no-deps --format-version 1`, field
  `target_directory`) and runs `<target>/release/corvis-app.exe`, with and without `SKIP_BUILD=1`. Missing-binary
  errors print the full path and the build command.
- When a `bench/target` folder from older runs exists, the script prints a notice that it is no longer used and can
  be deleted. It does not delete it.
- New `bench/README.md` (English): what the benchmark measures, prerequisites, how to run it, the environment
  variables, where the Corvis binary is built and where the results go. The header comment of `bench/run.sh` points to
  it.
- `.gitignore`: the `bench/target` line is removed (nothing creates it any more, and `**/target` still ignores a
  leftover folder).
- `scripts/check-naming.mjs`: `bench/README.md` joins the published surfaces it scans.

## Capabilities

### New Capabilities

- `bench/synthetic`: how the framework-overhead benchmark (`bench/run.sh`) builds and locates the Corvis application,
  and where that is documented. It sits next to `bench/port-vs-original` (#12).

### Modified Capabilities

None.

## Impact

- Files: `bench/run.sh`, new `bench/README.md`, `.gitignore`, `scripts/check-naming.mjs`.
- No Rust code, no manifest and no `Cargo.lock` change; `bench/corvis-app` stays a workspace member with
  `publish = false`. No parity run is needed.
- Disk: the release build of `corvis-app` and its dependencies now lives in `<shared target>/release/`; the old
  `bench/target` (388 MB here) can be deleted once.
- Builds in the shared target take Cargo's lock on it, so a benchmark build waits while another build in the same
  target runs (it ran in parallel before). The measurement itself never holds the lock.
- Docs site: the pages that mention the benchmark (`docs/http/performance.md`,
  `docs/faq/corvis-vs-fastify-express-hono.md` and their Spanish translations) only say `bash bench/run.sh` and do
  not mention the build folder, so they do not change. The root `README.md` "Performance" section does not either.
- Interaction with #12 (`e0-reproducible-benchmark-port-vs-original`): see design.md, "Interaction with #12".

## Out of scope

- Translating the log and error messages of `bench/run.sh` and the comments of `bench/corvis-app/src/main.rs` to
  English (they are Spanish today): follow-up issue, or part of the repository English pass.
- Making `bench/run.sh` work outside Windows (`taskkill`, `/proc/<pid>/winpid`, `.exe`).
- Changing what or how the benchmark measures, or the published numbers.
- Deleting `bench/target` automatically.

# Design

## Context

Verified on 2026-11-08 by reading `bench/`, the root `Cargo.toml`, `.gitignore`, `scripts/check-naming.mjs`,
`scripts/smoke-examples.mjs` and the #12 change, and by running `cargo metadata`.

**The benchmark** (`bench/`): `run.sh` (Git Bash, Windows only), `report.js`, `results.md` (committed results),
`corvis-app/` (Rust, `publish = false`, depends on `corvis` and `serde` through the workspace) and `node-app/` (the
equivalent Node.js application, built with `npm run build`). There is no `bench/README.md`; the usage and the
environment variables are in the Spanish header comment of `run.sh` (lines 1-18).

**How it builds today** (`run.sh:83-99`): unless `SKIP_BUILD=1`, it runs
`(cd "$ROOT_DIR" && CARGO_TARGET_DIR="$BENCH_DIR/target" cargo build --release -p corvis-app)`. If the build fails and
a previous binary exists, it continues with that binary and writes a warning into `results.md` (`BENCH_NOTES`). The
binary path is hard-coded: `CORVIS_EXE="$BENCH_DIR/target/release/corvis-app.exe"` (line 39), checked with `[ -f ]`
after the build step (line 98).

**Workspace.** `bench/corvis-app` is a member (`Cargo.toml:12`), so `cargo build --workspace` and
`cargo clippy --workspace` already compile it in the shared target (debug profile). There is no `.cargo/config.toml`,
so the target directory is `CARGO_TARGET_DIR` when set, else `<repo>/target`. On the maintainer's machine
`CARGO_TARGET_DIR=C:/Users/Usuario/Devel/nestrs/target`; `bench/target` exists and holds 388 MB, and
`<shared target>/release/` does not exist yet (no other release build uses the shared target).

**Asking Cargo for the directory.** `cargo metadata --no-deps --format-version 1` returns `target_directory` in about
0.1 s without resolving dependencies or touching the network. With `CARGO_TARGET_DIR` set it echoes the variable
(`C:/Users/Usuario/Devel/nestrs/target`); without it, it returns a Windows path with backslashes
(`C:\Users\Usuario\Devel\nestrs\target`), so the script normalizes to forward slashes. `scripts/smoke-examples.mjs`
uses the simpler rule (`CARGO_TARGET_DIR` or `<repo>/target`) for the debug example binaries.

**Ignore rules and naming check.** `.gitignore` has `**/target` and, under "Benchmark", `bench/target`.
`scripts/check-naming.mjs` scans `bench/report.js`, `bench/results.md`, `bench/run.sh` and `bench/corvis-app/**`.

**Docs.** `README.md:708-713`, `docs/http/performance.md:91-95`, `docs/faq/corvis-vs-fastify-express-hono.md:31-56`
and their Spanish translations mention `bench/` and `bash bench/run.sh`, never the build folder.

## Goals / Non-Goals

**Goals:**
- No second target directory: a run leaves no `bench/target` and builds in the shared one.
- The script finds the binary wherever Cargo put it, with and without `SKIP_BUILD=1`.
- One place that says how to run the benchmark and where the binary is built.

**Non-Goals:**
- Porting the script to Linux or macOS, translating its messages, or changing the measurement.

## Decisions

1. **Keep `bench/corvis-app` a workspace member and drop the `CARGO_TARGET_DIR` override.** It is already a member,
   so the build is simply `cargo build --release -p corvis-app` from the repository root, and it inherits whatever
   target directory the caller uses.
   - Alternative: exclude it from the workspace (`exclude = ["bench/corvis-app"]`) as a standalone package. It would
     get its own `Cargo.lock` and, by default, its own `bench/corvis-app/target`: the opposite of the goal, plus a
     second lockfile to keep in sync with the framework's.
   - Alternative: keep a separate target but put it under the shared one (`$CARGO_TARGET_DIR/bench`). Still a second
     full release build of every dependency; it only moves the disk use.
   - Membership already means clippy and the workspace build cover the benchmark app, so it cannot rot unnoticed.
2. **Locate the binary through `cargo metadata`.** A `target_dir` function runs
   `cargo metadata --no-deps --format-version 1` in `$ROOT_DIR`, reads `target_directory` with `node -e` (Node is
   already a prerequisite of the script), converts backslashes to `/`, and the script sets
   `CORVIS_EXE="$TARGET_DIR/release/corvis-app.exe"`. It runs before the build step, so `SKIP_BUILD=1` uses the same
   path. This follows `CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR` and any future `build.target-dir` in a Cargo config,
   which a hand-written `${CARGO_TARGET_DIR:-$ROOT_DIR/target}` would not.
   - Alternative: `cargo build --message-format=json` and read the `executable` field of the `compiler-artifact`
     message for `corvis-app`. Exact, but only available when building, so `SKIP_BUILD=1` would need a second
     mechanism anyway.
   - Alternative: `${CARGO_TARGET_DIR:-$ROOT_DIR/target}` (the rule of `scripts/smoke-examples.mjs`). Simpler, and
     acceptable if the maintainer prefers no `cargo metadata` call; it is wrong only when a Cargo config sets the
     target directory.
3. **Clear messages.** The build log line prints the resolved directory
   (`Building corvis-app (--release) in <target>/release`). A missing binary stops the run with the full path and the
   command that builds it (`cargo build --release -p corvis-app`). The fallback to a previous binary after a failed
   build stays, and its `BENCH_NOTES` warning stays.
4. **Leftover `bench/target`: notice, not deletion.** If the folder exists, the script prints one line saying it is no
   longer used and can be removed with `rm -rf bench/target`. Deleting 388 MB from inside a benchmark script without
   being asked is surprising, and the folder is ignored by git either way. The maintainer deletes it once by hand
   (task 3.1).
5. **`bench/README.md` in English, header comment points to it.** The README covers: what the two applications are,
   prerequisites (Git Bash on Windows, Rust, Node, `oha`, `curl`), `bash bench/run.sh`, the environment variables
   (from the current header), that the Corvis binary is built in release mode in the shared target directory
   (`<target>/release/corvis-app.exe`, where `<target>` is `CARGO_TARGET_DIR` or `<repo>/target`), that `SKIP_BUILD=1`
   reuses the binary found there, that every server binds to `127.0.0.1`, and that `bench/results.md` is overwritten
   by every run (so a short trial run must not be committed). The header comment of `run.sh` becomes a short English
   usage note with a link to the README, so the variable list is not maintained in two places. The rest of the
   script's messages stay as they are (Out of scope in the proposal).
6. **Ignore rules and naming check.** Remove the `bench/target` line from `.gitignore` (`**/target` still covers a
   leftover folder). Add `bench/README.md` to `SURFACES` in `scripts/check-naming.mjs`; the README names Express and
   Fastify and describes the other side as "an equivalent Node.js application", as the published pages already do.

## Interaction with #12

`openspec/changes/e0-reproducible-benchmark-port-vs-original/` (not implemented yet) depends on the benchmark layout
in these places:

- Its proposal lists this exact problem under Out of scope ("one-line follow-up"), and its review notes say it was
  filed separately: this change is that follow-up, and #12 needs no edit for it.
- Its design, Context section, describes `run.sh` building with `CARGO_TARGET_DIR=bench/target`; after this change
  that sentence is historical. Whoever implements #12 second can update it, or leave it (a change's context records
  what was true when it was written).
- Its task 2.4 checks memory readings "on the synthetic `bench/corvis-app` binary if present": after this change that
  binary is at `<shared target>/release/corvis-app.exe`, not in `bench/target/release/`.
- Its orchestrator (`bench/port-vs-original/run.mjs`) never builds and never creates a Cargo target directory; the
  port apps it measures live in the separate `corvis-tests` folder and are built there. No conflict.
- Both changes edit `.gitignore` (#12 adds `bench/port-vs-original/raw/`) and both add a README under `bench/`
  (#12 adds `bench/port-vs-original/README.md`). Whichever lands second resolves the small `.gitignore` overlap and
  adds a link from `bench/README.md` to `bench/port-vs-original/README.md`.

## Risks / Trade-offs

- [Benchmark builds wait for the shared target lock] -> only while another build runs in the same directory; the
  script already tells the maintainer to avoid heavy work during a run.
- [Feature unification changes the binary] -> `cargo build --release --workspace` would unify features across all
  members and overwrite `release/corvis-app.exe` with a binary built with the union of features. The script always
  builds with `-p corvis-app`, so a normal run uses the app's own feature set; the README says that `SKIP_BUILD=1`
  runs whatever binary is in `<target>/release/`. With today's manifests no member enables extra `corvis` features,
  but other members can add features to shared dependencies (`tokio`, `serde_json`), so the README recommends a normal
  run (with the build) for published numbers.
- [A release build appears in the shared target] -> it replaces `bench/target` (net disk use goes down once the old
  folder is deleted); `cargo clean --release` removes it.

## Open questions

1. Locate the target with `cargo metadata` (recommended, follows every way Cargo can be told the directory) or with
   the `CARGO_TARGET_DIR` / `<repo>/target` rule used by `scripts/smoke-examples.mjs` (simpler, same answer today)?
2. Should the script delete a leftover `bench/target` itself instead of printing a notice? Recommendation: notice
   only.
3. Translate the remaining Spanish messages of `bench/run.sh` (and the comments of `bench/corvis-app/src/main.rs`) in
   this change or in a separate issue? Recommendation: separate issue, so this change stays a build-location fix.

## Review notes (2026-11-08)

1. Open question 1: `cargo metadata --no-deps` (`target_directory`) is accepted.
2. Open question 2: the script only warns about an old `bench/target`; it does not delete it.
3. Open question 3: translate the Spanish messages of `run.sh` to English in this change (same file, small, and public
   repository text is English); no separate issue.

## Implementation notes (2026-11-08)

- Messages of `bench/run.sh` (comments, log lines, errors) are translated to English, as the review notes ask. Two
  strings stay Spanish on purpose because they are written into `bench/raw/meta.json` and end up verbatim in
  `bench/results.md`, whose report (`bench/report.js`) is Spanish and out of scope: the `BENCH_NOTES` warning and the
  `hilos` word of the `loadgen` field. Translating them alone would mix two languages inside the report.
- The script prints the resolved target directory in the build log line and a notice (not an error) for a leftover
  `bench/target`.
- The script was not run end to end and no release build was made (disk budget on the maintainer's machine). Checked
  instead: `bash -n`; the `target_dir` function extracted and run with `CARGO_TARGET_DIR` set
  (`C:/Users/Usuario/Devel/nestrs/target`) and unset (`<worktree>/target`, already with forward slashes, so the
  backslash conversion works); `SKIP_BUILD=1 bash bench/run.sh` with an empty `bench/target` and no release binary:
  the notice appears, the folder is left in place and the run stops with the full path and
  `cargo build --release -p corvis-app` before starting any server.
- `docs:check` could not run (`docs/node_modules` is not installed); no page of `docs/` changed.
- The Rust checks (`cargo build`, `clippy`, `test`) were not run: no Rust code, manifest or lockfile changed. Only
  `cargo fmt --all --check` was run.
- Unticked: 3.1 and 3.2 (need a real benchmark build and run), 4.2 to 4.6 (see above).

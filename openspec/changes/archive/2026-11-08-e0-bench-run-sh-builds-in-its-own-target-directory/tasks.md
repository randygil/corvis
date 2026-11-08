# Tasks

## 1. Build and locate the binary in the shared target

- [x] 1.1 In `bench/run.sh`, add a `target_dir` function (design decision 2): `cargo metadata --no-deps
  --format-version 1` in `$ROOT_DIR`, `target_directory` read with `node -e`, backslashes turned into `/`; set
  `TARGET_DIR` and `CORVIS_EXE="$TARGET_DIR/release/corvis-app.exe"` before the build step; verify by printing both
  values (temporary `echo`, removed afterwards) with `CARGO_TARGET_DIR` set and with it unset (`env -u
  CARGO_TARGET_DIR`): `C:/.../nestrs/target` in both cases on the maintainer's machine
- [x] 1.2 Replace the build command with `(cd "$ROOT_DIR" && cargo build --release -p corvis-app)` (no
  `CARGO_TARGET_DIR` override); update the build log line to name `$TARGET_DIR/release`; keep the fallback to a
  previous binary and its `BENCH_NOTES` warning; make the missing-binary error give the full path and
  `cargo build --release -p corvis-app`; verify `bash -n bench/run.sh` passes (and `shellcheck bench/run.sh` reports
  nothing new, if shellcheck is installed)
- [x] 1.3 Print the leftover notice of design decision 4 when `$BENCH_DIR/target` exists; verify with an empty
  `bench/target` folder created for the test that the notice appears and the folder is left in place
- [x] 1.4 Replace the header comment of `bench/run.sh` (lines 1-18) with a short English usage note that links to
  `bench/README.md`; verify the variable list now lives only in the README

## 2. Documentation and repository rules

- [x] 2.1 Write `bench/README.md` (English) with the content of design decision 5 (applications, prerequisites, how
  to run, environment variables, binary location `<target>/release/corvis-app.exe`, `SKIP_BUILD=1`, loopback only,
  `bench/results.md` is overwritten by every run); describe the other side as an equivalent Node.js application on
  Express and on Fastify; verify every command in it runs as written
- [x] 2.2 Remove the `bench/target` line from `.gitignore`; verify `git check-ignore -v bench/target/x` still reports
  the `**/target` rule
- [x] 2.3 Add `bench/README.md` to `SURFACES` in `scripts/check-naming.mjs`; verify `node scripts/check-naming.mjs`
  passes and that it reports a violation when a banned word is added to the README on a scratch edit (then revert)
- [x] 2.4 Docs site: confirm no page under `docs/` or `docs/es/` mentions `bench/target` or the build folder
  (`grep -rn "bench/target" docs README.md`), so no page changes; if #12 is already merged, link
  `bench/port-vs-original/README.md` from `bench/README.md` (design, "Interaction with #12")

## 3. Run checks

- [x] 3.1 Delete the old `bench/target` folder on the maintainer's machine (388 MB), then a short run with the build:
  `FRAMEWORKS=corvis SCENARIOS=hello DURATION=2 WARMUP=1 bash bench/run.sh` with no other heavy process running;
  verify the run completes, the binary is at `$CARGO_TARGET_DIR/release/corvis-app.exe`, no `bench/target` folder
  exists afterwards, and nothing new appears under `bench/` except the git-ignored `raw/` and `logs/`; restore the
  committed results with `git checkout bench/results.md` (short runs are not publishable)
- [x] 3.2 Same short run with `SKIP_BUILD=1` (reuses the binary, no build output), then with `SKIP_BUILD=1` after
  renaming the binary temporarily (stops with the path and the build command, starts no server); restore the binary
  and `bench/results.md`; verify each result and that `git status` is clean apart from the intended files

## 4. Definition of done

- [x] 4.1 `cargo fmt --all --check` passes (no Rust change expected)
- [x] 4.2 `cargo build --workspace --all-targets` with no warnings (unchanged Rust code; `bench/corvis-app` still
  builds as a member)
- [x] 4.3 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.4 `cargo test --workspace` passes
- [x] 4.5 Regression checks for the script: the results of 3.1 and 3.2 recorded in the pull request
- [x] 4.6 Docs: `bench/README.md` is repository text (English only); no page of the docs site changes (2.4); `npm run
  docs:check` in `docs/` passes
- [x] 4.7 `node scripts/check-naming.mjs` passes
- [x] 4.8 corvis-parity: no framework runtime code changes, so no parity run is required; confirm with
  `git diff --stat` that only `bench/run.sh`, `bench/README.md`, `.gitignore` and `scripts/check-naming.mjs` changed
  and that `Cargo.lock` is unchanged

## Workflow follow-up

- [x] Archive the change with `/opsx:archive` after review
- [x] Close issue [#132](https://github.com/randygil/corvis/issues/132)
- [x] Tick #132 in its epic [#1](https://github.com/randygil/corvis/issues/1)
- [x] Move the card to Done in the Corvis Roadmap project (see the `corvis-task` skill)

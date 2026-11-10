# Tasks

Start after #36 (`e2-socket-io-redis-adapter`) is merged into `dev` and archived, and with it #32 to #35: the
coverage check reads their specs from `openspec/specs/ws/`. Node 22, git and a Redis 6 or later on `127.0.0.1` are
needed locally. Install the harness first (`npm ci` in `parity/socket-io/` and in
`parity/socket-io/gateway-reference/`, then `node scripts/fetch-suites.mjs`). The full local command, called "the
strict run" below, is:

```sh
CORVIS_NODE_DIFF=1 CORVIS_NODE_DIFF_STRICT=1 CORVIS_REDIS_URL=redis://127.0.0.1:6379 \
  cargo test -p corvis --features socket-io-redis --test socket_io_diff -- --nocapture
```

Rules for every scenario: waits on events inside `step`, never sleeps; absence is shown by a marker event; no
timing is recorded; no retries. Servers bind to `127.0.0.1` only.

## 1. Reconcile and audit

- [ ] 1.1 Compare design.md "What #32 to #36 add before this change starts" with the merged code (scenario names,
  app commands and control routes, capabilities, how the short-heartbeat configuration is selected, the arguments
  `socket_io_diff.rs` passes); record renamed or missing items in a new "Implementation notes" section of design.md;
  verify the lenient run (`CORVIS_NODE_DIFF=1`, with Redis) passes before any edit
- [ ] 1.2 Write `parity/socket-io/coverage.json` (design.md Decision 1): one entry per requirement of the five
  capabilities in `openspec/specs/ws/`, with `comparable`, `reason` where not comparable, and `verifiedBy`; list in
  Implementation notes every requirement that had no verifier and what is added for it (a scenario in group 3, or a
  Rust test next to the owning layer's tests); verify by reading each spec once more that no requirement is missing
- [ ] 1.3 `scripts/check-coverage.mjs` with the five checks of Decision 1, and `test/coverage.test.mjs` running it on
  the repository and on fixtures for each failure (requirement without an entry, stale entry, comparable without a
  scenario, not comparable without a reason, unknown scenario, unknown suite, missing test function, expected
  difference citing an unknown requirement, exclusion without a covering scenario); verify
  `node --test "test/*.test.mjs"` passes and `node scripts/check-coverage.mjs` exits zero
- [ ] 1.4 Fix the `spec` field of any entry of `expected-differences.json` that the check rejects (form or renamed
  requirement), without changing its scenario, path or meaning; verify the check passes and the lenient run reports
  no unlisted and no stale difference

## 2. Runner: matrix and strict mode

- [ ] 2.1 Transport matrix (Decision 2) in `run.mjs` and `lib/targets.mjs`: `transports: true`, `clientOptions`
  passed to `run`, variant names in transcripts, reports and `--scenario`; `lib/compare.mjs`: an expected difference
  without a variant applies to all three and is stale if missing in any; self-tests for the three spec scenarios of
  "Client scenarios run on every transport" on synthetic transcripts; verify `node --test "test/*.test.mjs"`
- [ ] 2.2 Declare the matrix in the official-client scenarios of `scenarios/socket-io/`, `scenarios/socket-io/rooms/`
  and `scenarios/gateways/` (not `wire/`), spreading `clientOptions` into every `io(...)` call; move the hand-written
  polling and WebSocket clients of `socket-io/binary` to the matrix; verify the lenient run passes, and fix at its
  cause any difference a transport reveals (Decision 9)
- [ ] 2.3 Strict mode (Decision 4): `--require-all` in `run.mjs`, skips caused by a package that is not installed
  counted as skips, the summary line with the skipped count, `CORVIS_NODE_DIFF_STRICT` in
  `crates/corvis/tests/socket_io_diff.rs`; self-tests for "Redis missing in a strict run", "Contributor without
  Redis" and "Summary line"; verify the strict run passes, that it fails without `CORVIS_REDIS_URL`, and that it
  fails when built without `--features socket-io-redis` (spec "Test built without the features")

## 3. Cross-layer scenarios

- [ ] 3.1 App command `drop-transport` in `reference/socket-server.mjs` and in the Corvis scenario app, and scenario
  `socket-io/reconnect` (matrix) as in Decision 3; verify the strict run has no difference in its three variants and
  the transcript holds `transport close`, attempt 1, `recovered: false`, the single room and the acknowledgement
- [ ] 3.2 Scenario `socket-io/heartbeat` (matrix) with the short-heartbeat configuration, waiting for the fifth
  `ping` of the client's engine; verify no difference, and that the scenario takes under five seconds per variant
- [ ] 3.3 Scenario `socket-io/upgrade-order` (default transport): 200 numbered events each way during the upgrade,
  both lists and the final transport recorded; verify no difference in twenty consecutive runs of
  `node run.mjs --scenario upgrade-order` against the Corvis target
- [ ] 3.4 Scenario `socket-io/oversized-emit` (matrix): the client's disconnect reason, the server log and a second
  client's `echo`; verify no difference on the three variants (a difference is a bug of the transport layer: fix it
  with a regression test in `crates/corvis/tests/engine_io.rs` or `socket_io.rs`, Decision 9)
- [ ] 3.5 `--redis-url` / `--redis-key` in the gateway reference app (`@socket.io/redis-adapter` added to
  `gateway-reference/package.json` at the version `parity/socket-io/package.json` pins, lockfile refreshed) and in
  the Corvis gateway app; scenario `cluster/gateways`; verify the strict run compares it and reports no difference
- [ ] 3.6 Add the scenarios that task 1.2 listed as missing; update `coverage.json` for groups 2 and 3; verify
  `node scripts/check-coverage.mjs` exits zero and the strict run passes

## 4. Conformance suites

- [ ] 4.1 Run each upstream suite fifty times against its Node reference server and record in Implementation notes
  every test that failed at least once, with its count; verify the record names the suite commit of `suites.json`
- [ ] 4.2 Suite exclusions (Decision 5): `exclude` in `suites.json`, the dry-run title check and `--grep ... --invert`
  in `crates/corvis/tests/engine_io_conformance.rs` and `socket_io_conformance.rs`, for the Corvis run and the Node
  control run; control route `GET /harness/poll-pending/<sid>` in both Engine.IO apps and scenario
  `engine-io/close-while-polling`; one exclusion per test recorded by 4.1 that is a race of the suite, each with its
  covering scenario; verify both conformance tests pass twenty times in a row with `CORVIS_NODE_DIFF=1`, that an
  exclusion with a misspelled title fails the test, and that the coverage check rejects an unknown `coveredBy`
- [ ] 4.3 If 4.1 recorded no failing test, leave `exclude` empty and keep the mechanism and its tests; verify the
  conformance tests pass twenty times in a row

## 5. CI

- [ ] 5.1 `.github/workflows/ci.yml`: the `socket-io-diff` job of Decision 6 (service, environment, steps, the
  summary check, the artifact on failure); confirm `parity/socket-io/gateway-reference/.npmrc` disables install
  scripts and add it if missing; verify on a pull request to `dev` that the job is green, that its log shows the
  runner's summary with `0 skipped` and both suites' passing counts, and that the other jobs are unchanged
- [ ] 5.2 On throwaway branches, verify the job fails for the right reason in three cases: one acknowledgement
  argument changed in the Corvis scenario app (the log names the scenario and path, the artifact holds both
  transcripts); the Redis service removed (strict mode lists the skipped cluster scenarios); the `npm ci` step of
  the gateway reference removed (strict mode lists the gateway scenarios). Delete the branches
- [ ] 5.3 `.github/workflows/nightly.yml`: the job `socket-io-diff (repeat)` (same setup, `ref` as the other jobs,
  ten runs in a loop that stops at the first failure, artifact of the failing run); verify with a manual run
  (Actions > Nightly checks > Run workflow) that it completes ten runs
- [ ] 5.4 `CONTRIBUTING.md` (Decision 8): the `socket-io-diff` row of the CI table with its local commands, the
  nightly paragraph, and in "Socket.IO differential harness" strict mode, the matrix, the coverage manifest, suite
  exclusions, the transcripts artifact and the usual causes of flakiness; verify every command runs as written in
  Bash and in PowerShell

## 6. Stability

- [ ] 6.1 Ten consecutive strict runs on Linux and ten on Windows; fix the cause of any intermittent result and
  record it in Implementation notes; verify all twenty pass with the same number of scenarios compared
- [ ] 6.2 Verify the nightly repeat job is green on three scheduled runs on `dev` before the job is made a required
  status check (Workflow follow-up)

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features
  socket-io-redis --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --features socket-io-redis --all-targets -- -D warnings`
- [ ] 7.3 `cargo test --workspace` passes (the Node-gated tests skip without `CORVIS_NODE_DIFF`), and the strict run
  plus `--test engine_io_conformance --test socket_io_conformance` pass
- [ ] 7.4 Regression tests: the self-tests of groups 1 and 2, the new scenarios of groups 3 and 4, and a Rust
  regression test for every Corvis bug fixed under Decision 9
- [ ] 7.5 Docs: no page of `docs/` or `docs/es/` changes (contributor tooling, documented in `CONTRIBUTING.md`; the
  user-facing pages are #38); `npm run docs:check` in `docs/` and `node scripts/check-naming.mjs` still pass
- [ ] 7.6 corvis-parity: if framework runtime code changed under Decision 9, run the `corvis-parity` check and verify
  the three port apps keep 0 apidiff differences; otherwise record in the archive that no runtime code changed
- [ ] 7.7 `openspec validate e2-socket-io-differential-tests --strict` passes without warnings

## Workflow follow-up

- Add `socket-io-diff` to the required status checks of `main` (repository settings, or `gh api` on the branch
  protection) once task 6.2 is met, and confirm with a pull request to `main` that the check is listed as required.
- Update `ROADMAP.md` (and the issue body) with the change name `e2-socket-io-differential-tests`.
- Archive the change with `/opsx:archive` (its `ws/socket-io-parity` and `infra/ci` deltas are applied).
- Close issue #37, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).
- Open the follow-up issues of proposal.md "Out of scope" that the maintainer accepts (browser run, upstream drift
  job, labelled paths in `expected-differences.json`).

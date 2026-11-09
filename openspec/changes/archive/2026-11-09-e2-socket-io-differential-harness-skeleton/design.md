# Design

## Context

Design source: the Socket.IO spike #31, `openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`
(design.md Decisions 4 and 7, Review notes 2, 3, 7 and 9; tasks.md group 2; delta `specs/websockets/socket-io-parity`).
State of the repository verified at `6f5ff4d` (`origin/dev`).

**What exists today**

- No `parity/` directory. Node oracles live under `crates/corvis/tests/fixtures/<name>/` with a pinned
  `package.json` + `package-lock.json` and a `generate.mjs` run once by hand; its JSON output is committed and replayed
  by a Rust test. Example: `crates/corvis/tests/fixtures/ws_gateways/package.json` (exact versions, `"type":
  "module"`), `generate.mjs:1-4` ("Run once with `npm install && node generate.mjs` ... The server binds to 127.0.0.1
  only"), replayed by `crates/corvis/tests/ws_enhancers.rs:976`. `.gitignore` lists each fixture's `node_modules`
  one by one. The spike rejected this offline pattern for socket.io (Decision 4, last paragraph).
- Gated external dependencies follow one convention: the Postgres tests skip with `skipping: Postgres not reachable`
  and pass, and fail instead when `CORVIS_ORM_PG_REQUIRED=1` (`CONTRIBUTING.md`, section "Postgres tests"). The
  harness uses the same shape with `CORVIS_NODE_DIFF=1` (fixed by the issue).
- CI (`.github/workflows/ci.yml`) installs Node 22 only in the `docs` job (`actions/setup-node`, `node-version: 22`).
  The Rust `test` jobs have no Node. `parity/` is not one of the published surfaces scanned by
  `scripts/check-naming.mjs` (`SURFACES`, lines 18-30).
- Rust integration tests can start an application in-process on an ephemeral port with
  `CorvisApplication::listen_in_background` (`crates/corvis/src/application.rs:649-668`), which is how #32 will start
  the Corvis side; this change starts no Corvis server.
- Pinned Node versions found by the spike on 2026-11-08: `socket.io` 4.8.4 (depends on `engine.io` ~6.6.0),
  `engine.io` 6.6.11, `socket.io-client` 4.8.4, `@socket.io/redis-adapter` 8.3.0. The upstream protocol suites
  (`socketio/engine.io-protocol/test-suite`, `socketio/socket.io-protocol/test-suite`) are mocha + chai suites that
  target the hard-coded `http://localhost:3000`.

## Goals / Non-Goals

**Goals:**

- A harness that #32 and #33 can extend with their scenarios and their Corvis target without changing its structure.
- Reproducible: exact versions, lockfile, explicit `npm ci`, no machine paths, loopback only.
- A comparer whose behavior (normalization, readable diff, expected and stale differences) is itself tested.
- The suite download script, so #32 runs the engine.io suite from its first task.

**Non-Goals:**

- Engine.IO or Socket.IO code in Corvis, wire scenarios, running the suites (see proposal, Out of scope).
- A CI job (#37).

## Decisions

### 1. Layout of `parity/socket-io/`

```
parity/socket-io/
  package.json, package-lock.json, .npmrc     exact versions; ignore-scripts=true; engines node >= 22
  reference/socket-server.mjs                 Node socket.io reference app (127.0.0.1, port 0, prints {"port":N})
  lib/targets.mjs                             start/stop of targets, capability lists
  lib/normalize.mjs, lib/compare.mjs          normalizer and comparer (no dependencies)
  scenarios/socket-io/handshake-and-ack.mjs   the first scenario
  run.mjs                                     runs scenarios against two targets, writes out/<target>/<scenario>.json
  expected-differences.json                   [] in this change
  suites.json, scripts/fetch-suites.mjs       pinned upstream suites and their download
  test/*.test.mjs                             node:test self-tests of the normalizer, comparer and gating
```

`out/` and `suites/` are generated and ignored by git, as is `node_modules/`.

Alternative considered: one more fixture under `crates/corvis/tests/fixtures/`. Rejected: the spike placed the
harness at `parity/socket-io/` (Decision 4), it serves several crates' tests over the epic (#32-#37), and the
gateway reference package of #35 goes next to it.

### 2. One scenario module per scenario, capabilities instead of hard-coded skips

A scenario is an ES module exporting `{ name, requires, run }`; `run({ url, record, step })` drives raw HTTP (Node's
`fetch`), raw `ws` or the official client and calls `record(client, kind, data)`. `requires` lists capabilities
(`socket-io`, later `engine-io`, `redis`). A target declares its capabilities: the Node reference server has all of
them; the Corvis target (from #32 on) declares what is implemented. The runner runs a scenario on both targets when
both have its capabilities, on Node only when Corvis lacks one (printed as "skipped on corvis"), and not at all when
`redis` is required and `CORVIS_REDIS_URL` is unset.

With no Corvis target (this change), the runner starts two Node reference servers and compares them: this checks
the scenario and the normalizer on real transcripts (two servers generate different sids) instead of only on
synthetic ones.

Alternative considered: marking scenarios as "Node only" by hand and removing the mark in #32/#33. Rejected: every
later change would edit scenario files to enable them; capabilities move that decision to the one place where the
Corvis target is defined.

### 3. Transcripts, normalization and comparison

- A transcript is `{ scenario, entries: [{ client, kind, data }] }`, entries in the order each client observed them.
  Entries of different clients are compared per client (each client's sequence must be equal), so interleavings
  between independent clients never produce a difference. Transcripts hold no timestamps or durations; scenarios that
  need a timing (the heartbeat in #32) record a bucket such as `"closed-after-ping-timeout"` computed with generous
  margins.
- Normalization replaces session ids (`sid` of the open packet and of the Socket.IO CONNECT answer, and any later
  occurrence of the same string anywhere in the transcript) with `<id:1>`, `<id:2>`, ... in order of first
  appearance. Ids are recognized by the field they come from, not by a regular expression over all strings, so an
  application value that happens to look like an id is never rewritten.
- The comparer walks both normalized transcripts and reports differences as JSONPath-like paths
  (`$.entries[3].data.args[0]`) with both values, grouped by scenario; it exits 1 when any difference is not
  expected.
- No dependency for the comparer (a structural walk), so its behavior does not change with a package update.

### 4. Expected differences

`expected-differences.json` is an array of `{ scenario, path, reason, spec }`; `spec` names the requirement of a
`ws/*` capability that justifies the divergence. A difference matches an entry when scenario and path are equal. An
entry with an empty `reason` is rejected at load. An entry is stale when its scenario ran on both targets and the
difference did not occur; a stale entry fails the run. In this change the list is empty and the behavior is covered
by the self-tests (`test/compare.test.mjs`): Node vs Node (no difference), an altered acknowledgement argument (one
unlisted difference), the same difference listed (passes), a listed difference that does not occur (stale), and the
id identity case of the spec.

### 5. Rust entry point

`crates/corvis/tests/socket_io_diff.rs` (no feature requirement, so it compiles in every configuration):

1. `CORVIS_NODE_DIFF` not `1`: print `skipping socket_io_diff: set CORVIS_NODE_DIFF=1 (see CONTRIBUTING.md)` and
   return.
2. Otherwise check `node --version` (major 22 or later) and `parity/socket-io/node_modules/socket.io/package.json`;
   fail with a message naming `npm ci` when missing.
3. Run `node --test parity/socket-io/test/` (self-tests), then `node parity/socket-io/run.mjs`; on a non-zero exit,
   fail with the runner's output. Paths come from `env!("CARGO_MANIFEST_DIR")`, commands are spawned with argument
   lists (no shell), stdout and stderr are captured and printed, and children are killed on drop.
4. From #32 on, the test starts the Corvis server in-process on `127.0.0.1:0` and passes
   `--corvis <url> --corvis-capabilities engine-io` to the runner. In this change the arguments are absent.

The Node reference servers are started by `run.mjs`, not by Rust: the runner also needs them when a contributor runs
it directly (`node run.mjs`), and it can restart one per scenario if a scenario needs a fresh server.

Alternative considered: a Corvis test-only binary started as a child process. Rejected for the diff: an in-process
server needs no extra target, shares the test's build and dies with the test. Review note 9 of the spike (a test-only
server rather than an example) is met, since the server lives in a test target.

### 6. Suite download

`suites.json`:

```json
{
  "engine.io-protocol": { "repo": "https://github.com/socketio/engine.io-protocol.git", "commit": "<40 hex>", "dir": "test-suite" },
  "socket.io-protocol": { "repo": "https://github.com/socketio/socket.io-protocol.git", "commit": "<40 hex>", "dir": "test-suite" }
}
```

The script, for each suite: validates `commit` against `^[0-9a-f]{40}$`; in a fresh temporary directory runs
`git init`, `git fetch --depth 1 <repo> <commit>`, `git checkout --detach FETCH_HEAD` with `execFileSync` (argument
arrays, no shell); checks `git rev-parse HEAD` equals `commit` (git verifies object ids, so the content is the pinned
one); copies `dir` to `parity/socket-io/suites/<name>/`; replaces the URL literal `http://localhost:3000` with
`process.env.SUITE_URL` in the files listed for that suite, failing if a listed file does not contain it exactly once.
The commits are the heads of the default branches on the day the task runs, recorded with that date in `suites.json`
and in the Review notes. The suites' own `package.json` is not installed: the packages they import are added, pinned,
to the harness `package.json`, so one lockfile covers everything that runs.

Alternatives considered: vendoring the suites (rejected by Review note 2 of the spike); downloading GitHub tarballs
by commit (their bytes are not guaranteed stable, so a checksum pin could break without any change; git checks the
commit itself).

`SUITE_URL` uses `127.0.0.1`, not `localhost`: `localhost` can resolve to `::1` while the servers bind IPv4 only.

## Risks / Trade-offs

- The harness is not run by CI until #37 adds its job → contributors of #32/#33 run it locally (documented, and
  required by those changes' tasks); see Open question 1.
- Node and npm become contributor tools for the socket.io work → gated by `CORVIS_NODE_DIFF`; `cargo test` stays
  Node-free; no implicit `npm install`.
- A comparer bug would hide real differences → its self-tests run before every harness run, in the same command.
- Upstream suites may change the URL literal → the replacement fails loudly instead of silently testing
  `localhost:3000`.
- Windows: child processes must not outlive a failed test → the runner kills its servers on exit and on signals, and
  the Rust side kills the runner on drop.

## Open questions

1. Should a nightly-only job run the harness (Node vs Node) before #37 adds the required `socket-io-diff` job, so it
   cannot rot between #131 and #37? Recommendation: no; #32 starts right after this change and runs it in every task.
2. The issue lists `socket.io`, `socket.io-client` and `@socket.io/redis-adapter`; this design also pins `engine.io`
   (so the version behind `socket.io`'s `~6.6.0` range cannot drift), `ws`, `redis`, mocha/chai and whatever the
   suites import. Decided here as part of "pinned"; listed for confirmation.

## Review notes (2026-11-09, maintainer)

1. No nightly job for the harness before #37: #32 runs it in every task, and #37 adds the CI job.
2. Pinning `engine.io`, `ws`, `redis`, mocha/chai and the suites' imports with exact versions is confirmed: "pinned"
   means the whole tree, through the lockfile.
3. The package versions in tasks.md (socket.io 4.8.4, engine.io 6.6.11, socket.io-client 4.8.4,
   @socket.io/redis-adapter 8.3.0) are the drafter's reading of the registry: check them with `npm view` when pinning,
   use the latest patch of those minor lines, and record the final versions in Implementation notes.
4. The #32 change is named `e2-engine-io-handshake-polling-upgrade-and-heartbeat`, as in the issue and ROADMAP.md.

## Implementation notes

- Final versions (checked with `npm view` on 2026-11-09; latest patch of the pinned minor lines): `socket.io` 4.8.4,
  `engine.io` 6.6.11, `socket.io-client` 4.8.4 (resolves `engine.io-client` 6.6.7), `@socket.io/redis-adapter` 8.3.0,
  `redis` 6.3.1, `ws` 8.21.3, `mocha` 12.0.3, `chai` 4.5.0, `chai-string` 1.6.0, `node-fetch` 3.3.2.
- `ws` is 8.21.3, not the latest 8.22.2: `engine.io` 6.6.11 depends on `ws ~8.21.0`, and `npm ls ws` shows a single
  deduped copy. `chai` is 4.5.0, not 6.x: the upstream suites use `import chai from "chai"` (a default export that
  chai 5+ no longer has) and `chai-string`, which target chai 4. The suites also import `node-fetch`, added as a pin.
- Suite commits (heads of the default branches on 2026-11-09): `socketio/engine.io-protocol`
  `f21de7b00ed09b3bbad2807db718ea5d6bc36aba`, `socketio/socket.io-protocol` `4f633dd6a4445492d57af3b27ddd2d336014f1f1`.
  In both, `test-suite.js` holds `"http://localhost:3000"` once; the script replaces the quoted literal with
  `process.env.SUITE_URL`. `mocha --dry-run` lists 24 (engine.io) and 32 (socket.io) tests.
- Self-tests run as `node --test "test/*.test.mjs"`, not `node --test test/`: Node 24 treats the directory argument as a
  module path and fails, while the quoted glob works on Node 22 and 24. The Rust entry point uses the same command.
- The reference server prints `{"address":"127.0.0.1","port":N}` (the design said `{"port":N}`) so the spec scenario
  "Loopback address" can be checked by the runner and by a self-test.
- Normalized transcripts keep `entries` but ordered by client name (stable, so each client's order is kept); paths in
  diffs and in `expected-differences.json` index that ordered array (in the first scenario, `client` entries come
  before `raw` ones: the ack arguments are at `$.entries[1].data.args`).
- Both targets run for the whole run on one server each (not one server per scenario); a scenario that needs a fresh
  server can be given one later. Targets are named `node`, `corvis` and, without Corvis, `node-2`.
- The runner sets `process.exitCode` instead of calling `process.exit`: on Windows `process.exit` while handles were
  closing printed a libuv assertion.
- Verified by hand, reverted afterwards: breaking the `sid` rule of the normalizer fails 8 self-tests and the runner;
  a Node server answering the ack with `43` (started by hand, passed with `--corvis`) makes `node run.mjs` exit 1 with
  `DIFFERENCE in socket-io/handshake-and-ack at $.entries[1].data.args[1]`; `--corvis-capabilities engine-io` prints
  `skipped (lacks capability socket-io)`; the failure paths of `fetch-suites.mjs` (malformed commit, unknown commit,
  listed file without the literal) exit 1 naming the suite or file; with `node_modules` renamed,
  `CORVIS_NODE_DIFF=1 cargo test` fails with the `npm ci` message and creates no `node_modules`.
- No runtime code changed: the `corvis-parity` check is not needed for this change.

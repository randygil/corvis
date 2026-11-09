# Tasks

No Rust runtime code changes. Node 22 and git are needed locally for groups 1-4. Every Node command runs inside
`parity/socket-io/`.

## 1. Pinned harness package

- [ ] 1.1 Create `parity/socket-io/package.json` (`"private": true`, `"type": "module"`, `engines.node` `>=22`) with
  exact versions of `socket.io` 4.8.4, `engine.io` 6.6.11, `socket.io-client` 4.8.4, `@socket.io/redis-adapter` 8.3.0,
  `redis`, `ws` (the version `engine.io` 6.6.11 resolves) and `mocha`/`chai` (versions current on the day, exact);
  add `.npmrc` with `ignore-scripts=true`; run `npm install` once to produce `package-lock.json`; verify `npm ci` from
  a clean checkout succeeds and `npm ls socket.io engine.io` shows exactly the pinned versions
- [ ] 1.2 Add `parity/socket-io/node_modules`, `parity/socket-io/out` and `parity/socket-io/suites` to `.gitignore`;
  verify `git status` shows none of them after a run

## 2. Reference server, runner and first scenario

- [ ] 2.1 `reference/socket-server.mjs`: a Node `socket.io` server with default options on an `http` server bound to
  `127.0.0.1:0`; `connection` handler with `echo` answered through the acknowledgement with the same arguments
  (after `--ack-delay <ms>` when given, for the self-test of spec "Slow machine"); a 404
  fallback for other requests; prints `{"port":N}` as its first stdout line; exits on SIGTERM and when stdin closes
  (so it dies with its parent on Windows); verify by starting it and running `curl
  "http://127.0.0.1:N/socket.io/?EIO=4&transport=polling"`
- [ ] 2.2 `lib/targets.mjs` and `run.mjs`: start two reference servers (or Node plus `--corvis <url>` with
  `--corvis-capabilities <list>`), check the installed `socket.io`/`engine.io` versions against `package.json`, load
  `scenarios/**/*.mjs`, apply the capability and `CORVIS_REDIS_URL` rules (design.md Decision 2), run each scenario on
  each target with a 10 s timeout per step, write `out/<target>/<scenario>.json`, kill the servers on exit and on
  signals; verify `node run.mjs` prints one line per scenario and per target and leaves no `node` process behind
- [ ] 2.3 `scenarios/socket-io/handshake-and-ack.mjs` (spec "The first scenario covers a handshake and an acknowledged
  event"): raw polling handshake recorded with status, `content-type`, `cache-control` and the decoded open packet;
  official client with `auth: { token: "t" }`, `forceNew: true`, `reconnection: false`; `emit("echo", "hello", 42,
  ack)`; record the ack arguments; `disconnect()` and record the reason; waits are on events, never sleeps; verify the
  transcript of a run holds `["hello", 42]` and Node's default open packet values

## 3. Normalizer, comparer, expected differences

- [ ] 3.1 `lib/normalize.mjs`: id placeholders by field of origin and first appearance, per-client sequences
  (design.md Decision 3); `lib/compare.mjs`: structural diff with paths and both values, `expected-differences.json`
  loading (entries without reason rejected), unlisted and stale differences fail; `expected-differences.json` is `[]`
- [ ] 3.2 `test/normalize.test.mjs` and `test/compare.test.mjs` (`node:test`): the spec scenarios "Ids are normalized",
  "A behavior difference", "Id identity is kept", "Stale entry" and "Listed difference" on small synthetic
  transcripts, capability selection ("Corvis target without the capability"), the `redis` gating ("Redis scenario
  without Redis"), and "Slow machine" against a reference server with `--ack-delay 2000`; verify
  `node --test test/` passes, and that it fails after deliberately breaking the normalizer (revert afterwards)
- [ ] 3.3 Wire the comparer into `run.mjs`; verify `node run.mjs` (Node vs Node) reports no difference, and that
  editing the reference server to answer the ack with one argument changed makes it exit 1 with the scenario name
  and path (revert afterwards)

## 4. Protocol suites download

- [ ] 4.1 `suites.json` with the head commits of `socketio/engine.io-protocol` and `socketio/socket.io-protocol` on the
  day, and the list of files holding `http://localhost:3000`; `scripts/fetch-suites.mjs` as in design.md Decision 6;
  add the packages the suites import (exact versions) to `package.json` and refresh the lockfile; verify `node
  scripts/fetch-suites.mjs` writes both suites and `SUITE_URL=http://127.0.0.1:1 npx mocha --dry-run
  suites/engine.io-protocol` (and the socket.io one) lists the tests
- [ ] 4.2 Verify the failure paths of the script: a malformed commit, an unknown commit and a listed file without the
  URL literal each exit non-zero with a message naming the suite or file

## 5. Rust entry point

- [ ] 5.1 `crates/corvis/tests/socket_io_diff.rs` as in design.md Decision 5 (skip notice, Node 22+ and
  `node_modules` checks, self-tests then runner, argument lists, children killed on drop, output printed on failure);
  verify `cargo test -p corvis --test socket_io_diff` prints the skip notice and passes without the variable,
  `CORVIS_NODE_DIFF=1` with the harness installed passes, and `CORVIS_NODE_DIFF=1` after renaming `node_modules`
  fails with the `npm ci` message and creates no `node_modules` (spec "Nothing installed by cargo"; rename it back)

## 6. Contributor docs

- [ ] 6.1 `CONTRIBUTING.md`, new section "Socket.IO differential harness": prerequisites (Node 22, git), `npm ci` in
  `parity/socket-io/`, `CORVIS_NODE_DIFF=1` and `CORVIS_REDIS_URL` (PowerShell syntax too), `node run.mjs` and the
  Rust command, how to read a diff, how to add a scenario (capabilities) and an expected difference (reason and spec
  requirement), how to refresh the suites (`fetch-suites.mjs`, updating `suites.json`); verify every command runs as
  written on Windows and Linux

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 7.3 `cargo test --workspace` passes (the new test skips without `CORVIS_NODE_DIFF`), and
  `CORVIS_NODE_DIFF=1 cargo test -p corvis --test socket_io_diff` passes with the harness installed
- [ ] 7.4 Regression tests: the `node:test` self-tests of group 3 and the Rust entry point's skip and failure paths
- [ ] 7.5 Docs: no page of `docs/` or `docs/es/` changes (contributor tooling, documented in `CONTRIBUTING.md`; the
  user-facing Socket.IO pages are #38); `node scripts/check-naming.mjs` passes
- [ ] 7.6 No framework runtime code changes: record in the archive that the `corvis-parity` check is not needed

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/socket-io-parity` delta is applied; #37 extends it).
- Close issue #131, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).

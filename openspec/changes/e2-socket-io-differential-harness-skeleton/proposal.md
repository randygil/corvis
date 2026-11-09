# Proposal

Implements [randygil/corvis#131](https://github.com/randygil/corvis/issues/131) (epic
[#30](https://github.com/randygil/corvis/issues/30)). First part of #37, split off by the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`, design.md Decision 7 and Review
note 7), so that #32 (Engine.IO) and #33 (Socket.IO) are checked against Node from their first test.

## Why

The spike decided that Corvis implements Engine.IO v4 and Socket.IO v5 itself (design.md Decision 1) and that parity
is earned with the official protocol test suites plus a differential harness that runs the same scenarios against a
Node `socket.io` server and against Corvis (Decision 4). Without the harness, #32 would be built against hand-written
expectations and re-verified later, which is the unverified parity the project's positioning rules out.

The only Node oracle pattern in the repository today records fixtures offline and replays them
(`crates/corvis/tests/fixtures/ws_gateways/generate.mjs:1-4`, replayed by `crates/corvis/tests/ws_enhancers.rs:976`);
the spike rejected it for socket.io, whose behavior is a conversation (handshakes, upgrades, acks), not a fixed
response.

## What Changes

- A Node harness in `parity/socket-io/` with its own `package.json` and committed `package-lock.json` pinning exact
  versions of `socket.io` 4.8.4, `engine.io` 6.6.11, `socket.io-client` 4.8.4, `@socket.io/redis-adapter` 8.3.0, the
  `redis` client, `ws` and the test runner packages the protocol suites import. `npm ci` is the only way packages are
  installed; install scripts are disabled for the harness (`.npmrc`).
- A Node reference server (`reference/socket-server.mjs`) bound to `127.0.0.1` on an ephemeral port, which prints its
  port as one JSON line on stdout.
- A runner (`run.mjs`) that runs every scenario against two targets, records a JSON transcript of what each client
  observes, normalizes values random by design (session and socket ids), compares the transcripts, prints a readable
  diff and exits non-zero on a difference. Targets declare capabilities (`engine-io`, `socket-io`, `redis`); a
  scenario that needs a capability the Corvis target lacks runs on Node only and is reported as skipped on Corvis.
  Until #32/#33 land there is no Corvis target, and the runner compares two independent Node reference servers.
- `expected-differences.json` (empty in this change): an unlisted difference fails, and a listed difference that no
  longer occurs fails too.
- The first scenario, `socket-io/handshake-and-ack`: a raw polling handshake plus the official client connecting,
  emitting one event with arguments and an acknowledgement, and disconnecting.
- Redis scenarios are gated: a scenario that requires `redis` runs only when `CORVIS_REDIS_URL` is set. No Redis
  scenario exists yet (#36 adds them).
- `scripts/fetch-suites.mjs`: downloads the official `engine.io-protocol` and `socket.io-protocol` test suites at a
  commit pinned in `suites.json`, verifies the commit, and writes copies whose hard-coded `http://localhost:3000` is
  replaced by a URL taken from the environment. Nothing is vendored.
- The Rust entry point `crates/corvis/tests/socket_io_diff.rs`: skipped with a printed notice unless
  `CORVIS_NODE_DIFF=1`; with it, fails when Node 22+ or the installed harness is missing, runs the harness self-tests
  and the runner, and fails when the runner fails. It never installs packages.
- `CONTRIBUTING.md`: how to install and run the harness, read a diff, add a scenario and add an expected difference.

## Capabilities

### New Capabilities

- `ws/socket-io-parity`: the pinned Node oracle, the runner with normalization and comparison, expected differences,
  gating, and the suite download script. #37 extends it (full scenario catalog, conformance runs, CI job). The spike
  named it `websockets/socket-io-parity`; the main spec tree groups WebSocket capabilities under `ws/` (`ws/gateways`,
  `ws/write-deadline`).

### Modified Capabilities

None.

## Impact

- New: `parity/socket-io/**` (Node only, outside every crate, not published), `crates/corvis/tests/socket_io_diff.rs`.
- Edited: `.gitignore` (`parity/socket-io/node_modules`, `parity/socket-io/suites`, `parity/socket-io/out`),
  `CONTRIBUTING.md`.
- No runtime code of any crate changes; the port apps cannot be affected. `cargo test --workspace` stays Node-free
  (the new test skips without `CORVIS_NODE_DIFF`).
- No new Rust dependency.

## Out of scope

- Any Engine.IO or Socket.IO code in Corvis, and the Corvis side of the first scenario: #32 and #33.
- Engine.IO wire scenarios (handshake errors, polling framing, upgrade, heartbeat, limits) and the `engine` mode of
  the reference server: #32 adds them together with the code they verify.
- Running the downloaded protocol suites against Node or Corvis: #32 (engine.io suite) and #33 (socket.io suite).
  This change only downloads them and checks that they load.
- Redis scenarios and a Redis service: #36.
- The gateway reference package with the original framework's socket.io platform adapter: #35.
- The `socket-io-diff` CI job and the full scenario catalog: #37.
- Docs site pages: the harness is contributor tooling, documented in `CONTRIBUTING.md`; user-facing Socket.IO docs
  are #38.

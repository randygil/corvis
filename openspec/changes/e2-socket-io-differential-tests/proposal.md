# Proposal

Implements [randygil/corvis#37](https://github.com/randygil/corvis/issues/37) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: design.md Decisions 4 and 7,
Review notes 3 and 7, tasks.md group 8). #37 was split by that spike: #131
(`openspec/changes/archive/2026-11-09-e2-socket-io-differential-harness-skeleton/`) built the harness, and this change
keeps "the full catalog, the conformance runs and the CI job" (`openspec/specs/ws/socket-io-parity/spec.md`, Purpose).
Builds on the approved changes for #32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`), #33
(`e2-socket-io-namespaces-events-and-acks`), #34 (`e2-socket-io-rooms-and-broadcast`), #35
(`e2-socket-io-gateway-adapter`) and #36 (`e2-socket-io-redis-adapter`), referred to **as designed there, including
their Review notes**. The issue and `ROADMAP.md` name the change `e2-differential-tests-against-node-socket-io`; this
change uses `e2-socket-io-differential-tests`, consistent with its siblings (design.md Open question 1).

## Why

Each of #32 to #36 adds the scenarios of its own layer to the harness and runs them by hand: every one of those
changes states that "Node-gated tests stay out of CI until #37". When #36 lands, the parity of the whole epic is
therefore a property of five local runs made at five different times, and four things are still missing:

- **Nothing runs it.** CI has no job with Node and the harness (`.github/workflows/ci.yml`; Node 22 is installed only
  in the `docs` job, `:215-233`). A later change to `ws.rs` or `server.rs` can break Socket.IO parity without any
  check turning red.
- **A skipped scenario passes.** The runner runs a scenario on Node only when the Corvis target lacks a capability,
  and not at all without `CORVIS_REDIS_URL` (`parity/socket-io/lib/targets.mjs:127-138`); both are reported as a
  printed line and exit zero. That is right for a contributor without Redis and wrong for a required check.
- **Nobody checked the catalog as a whole.** The spike asks that every requirement of the `ws/*` deltas name a
  scenario that exercises it (tasks.md 8.1). The five changes wrote scenarios per layer; what crosses layers (the
  official client reconnecting, every transport, gateways over Redis) belongs to none of them.
- **A required check must not be flaky.** #32 found that one test of the upstream `engine.io-protocol` suite races
  against the Node reference as well (its `CONTRIBUTING.md` section, one run in eight under load); the harness
  forbids retries (`ws/socket-io-parity`, "No retries and no speed-dependent assertions").

## What Changes

What is **new** in this change (everything else it uses already exists after #36):

- **Coverage manifest** `parity/socket-io/coverage.json` and `scripts/check-coverage.mjs`: every requirement of
  `ws/engine-io`, `ws/socket-io`, `ws/socket-io-rooms`, `ws/socket-io-gateways` and `ws/socket-io-cluster` names what
  verifies it (a harness scenario, a conformance suite, or a Rust test when no client can observe the behavior, with
  the reason). The script reads the requirement headings from `openspec/specs/ws/` and fails on a requirement without
  an entry, an entry for a requirement that no longer exists, and a named scenario, suite or test that is not in the
  repository. It also checks that every entry of `expected-differences.json` cites an existing requirement.
- **Transport matrix**: scenarios driven by the official client run three times per target (polling only, WebSocket
  only, the default polling-then-upgrade), instead of only with the default.
- **Cross-layer scenarios** that no earlier change owns: automatic reconnection of the official client, the heartbeat
  as the official client sees it, ordering of events exchanged during the upgrade, what the client observes when its
  emit exceeds the payload limit, and a gateway broadcasting across a mixed Node / Corvis cluster. Plus whatever the
  audit of the manifest finds missing.
- **Strict mode** (`--require-all`, `CORVIS_NODE_DIFF_STRICT=1`): a run in which any scenario was skipped fails.
- **Suite exclusions**: `suites.json` may exclude a test of an upstream suite by its exact title, with a reason and
  the deterministic scenario that covers the behavior instead; an exclusion whose title is no longer in the suite
  fails. Both conformance suites run in CI with it.
- **CI**: a `socket-io-diff` job in `ci.yml` (Linux, Node 22, Redis service, strict mode, transcripts uploaded on
  failure), added to the required status checks of `main`; `nightly.yml` runs the harness ten times in a row.
- `CONTRIBUTING.md`: the harness section and the CI table describe the job, strict mode, the manifest and how to
  reproduce the job locally.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ws/socket-io-parity`: added requirements for the coverage manifest, the transport matrix, the cross-layer
  scenarios, the citation rule of expected differences, strict runs, suite exclusions and repeated runs.
- `infra/ci`: one added requirement (the `socket-io-diff` job) and two modified ones (the required status checks of
  `main` gain the job; the long checks gain the repeated harness run).

## Impact

- New: `parity/socket-io/coverage.json`, `parity/socket-io/scripts/check-coverage.mjs`,
  `parity/socket-io/test/coverage.test.mjs`, `parity/socket-io/scenarios/socket-io/{reconnect,heartbeat,upgrade-order,oversized-emit}.mjs`,
  `parity/socket-io/scenarios/cluster/gateways.mjs`,
  `parity/socket-io/scenarios/engine-io/close-while-polling.mjs`.
- Edited: `parity/socket-io/run.mjs` and `lib/targets.mjs` (matrix, strict mode), `lib/compare.mjs` (variant names in
  expected differences), `suites.json` and the conformance tests `crates/corvis/tests/{engine_io,socket_io}_conformance.rs`
  (exclusions), `crates/corvis/tests/socket_io_diff.rs` and the harness applications it starts (strict mode, the
  commands the new scenarios need), `parity/socket-io/reference/*.mjs` and `parity/socket-io/gateway-reference/**`
  (the same commands; the Redis adapter for the gateway reference), the existing client scenarios (they take the
  client options of the matrix), `.github/workflows/ci.yml`, `.github/workflows/nightly.yml`, `CONTRIBUTING.md`.
- GitHub settings: `socket-io-diff` added to the required status checks of `main` (a maintainer action).
- No framework runtime code is planned to change, and no Rust or Node dependency is added. If a scenario of this
  change finds a real difference, the fix is made here with its regression test and the corvis-parity check is then
  run (design.md Decision 9).
- Docs site: no page changes. The harness is contributor tooling (`CONTRIBUTING.md`); the user-facing statement of
  what is verified against Node is written by #38.

## Out of scope

- User guide, migration notes and the example application: #38. That change adds its example's official-client check
  to the job created here.
- New Socket.IO behavior. A difference found by the catalog is fixed here only when the fix is small; otherwise it
  becomes an issue and blocks closing #37 (it is never added to `expected-differences.json` to make the job green).
- Running the harness in CI on Windows or macOS: the job needs a Redis service container (Linux runners only). The
  harness keeps working locally on Windows.
- Browsers: the official client runs in Node. A browser run (Playwright) is a possible later issue.
- Load, soak and latency comparisons with Node: the benchmarks epic, not parity.
- Detecting upstream drift (running the catalog against the newest `socket.io` release instead of the pinned one):
  possible later issue, a nightly job that is allowed to fail.
- Paths of `expected-differences.json` that index transcript entries (`$.entries[9]...`) and therefore change when a
  scenario gains a step: the failure is loud (one unlisted and one stale difference), so it stays; labelled entries
  are a possible later issue.
- Server restart seen by a connected official client: the Corvis target is one in-process server for the whole run;
  the reconnection scenario closes the transport instead. Shutdown behavior stays covered by the Rust tests of #32
  and #33.

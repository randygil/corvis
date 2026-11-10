# Design

## Context

Design source: the Socket.IO spike #31 (design.md Decision 4, "Verification: conformance suites plus a differential
harness", its CI bullet; Decision 7, steps 1 and 7; Review notes 3 and 7; tasks.md group 8). State of the repository
verified at `c837dcb` (`origin/dev`); the #32 implementation was read in its worktree (not merged yet), and #33 to
#36 are read as designed in `openspec/changes/`, including their Review notes.

**What is on `dev` today (#131)**

- `parity/socket-io/`: `package.json` with exact versions (`socket.io` 4.8.4, `engine.io` 6.6.11,
  `socket.io-client` 4.8.4, `@socket.io/redis-adapter` 8.3.0, `redis` 6.3.1, `ws` 8.21.3, mocha, chai), `.npmrc` with
  `ignore-scripts=true`, `run.mjs`, `lib/{targets,normalize,compare}.mjs`, one scenario
  (`scenarios/socket-io/handshake-and-ack.mjs`), `expected-differences.json` (`[]`), `suites.json` with the two
  upstream suites pinned by commit, `scripts/fetch-suites.mjs`, and `node:test` self-tests under `test/`.
- Skipping rules, `lib/targets.mjs:127-138` (`plan`): a scenario that requires `redis` returns `run: []` without
  `CORVIS_REDIS_URL`; a scenario whose capabilities the second target lacks runs on `node` only. `run.mjs:82-84`
  prints the skip and continues; the exit status (`run.mjs:117`) depends only on failures and differences.
  Capabilities are `engine-io`, `socket-io`, `redis` (`lib/targets.mjs:8`); each step has a 10 s limit (`:9`).
- `crates/corvis/tests/socket_io_diff.rs` (102 lines): skips unless `CORVIS_NODE_DIFF=1` (`:79-80`), checks Node 22
  and the install, runs the self-tests and `run.mjs` (`:99`).
- CI: `.github/workflows/ci.yml` has no job with the harness. Node 22 is installed only by the `docs` job
  (`:215-233`, `actions/setup-node@v7` with the npm cache). The Linux test job declares its Postgres service at
  `:81-92` and the per-feature clippy loop is at `:72`. `nightly.yml` has two jobs, `fuzz` and `audit`, both checking
  out `dev` on a schedule (`:45`, `:81`). `openspec/specs/infra/ci/spec.md` lists the required status checks of
  `main` by name ("Changes reach main only through checked pull requests").
- `CONTRIBUTING.md`: "Continuous integration" (`:118`, the job table) and "Socket.IO differential harness" (`:196`).
- `.gitignore:43-45` ignores the harness's `node_modules`, `out` and `suites`. `parity/` is not a published surface
  of `scripts/check-naming.mjs` (`SURFACES`, `:18-30`).

**What #32 to #36 add before this change starts**

| Change | Harness additions | Expected differences | CI |
|---|---|---|---|
| #32 | `reference/engine-server.mjs` (presets by path, `--suite`), `lib/engine-client.mjs`, `scenarios/engine-io/{handshake,errors,polling,cors,upgrade,heartbeat,bounds}.mjs`, the Corvis target with `engine-io`, `crates/corvis/tests/engine_io_conformance.rs` (the suite against Corvis and against the Node reference as a control) | ten entries, all in `engine-io/bounds`: batch bounds, session caps, send buffer, silent polling session, compression | `socket-io` in the clippy loop, the Linux test job and the fuzz jobs |
| #33 | the scenario app (`reference/socket-server.mjs`: namespaces `/` and `/admin`, middleware, handlers, `GET /harness/log`, `POST /harness/reset`), `scenarios/socket-io/{namespaces,middleware,connect-timeout,events,acks,binary,disconnect}.mjs` and `wire/{encoding,malformed,attachments-limit,pending-connect,ack-id,depth}.mjs`, `socket_io_conformance.rs` | `wire/pending-connect`, `wire/ack-id`, `wire/depth` | none |
| #34 | room and broadcast commands, `scenarios/socket-io/rooms/{scoping,socket-id,union,from-socket,except,leave-and-disconnect,bulk,broadcast-ack,limits}.mjs` | room cap, room name length, broadcast ack without `timeout()` | none |
| #35 | `parity/socket-io/gateway-reference/` (its own lockfile with the original framework's packages), capability `gateways`, `scenarios/gateways/{ack,no-ack,events,body-mapping,exceptions,guard,pipe,interceptor,filter,lifecycle,namespaces,server-emit,unknown-event,wire/cors}.mjs` | `gateways/http-exception`, `gateways/binary-body`, the same event in two gateways | none |
| #36 | `--redis-url` / `--redis-key` on both apps, `cluster: 2` scenarios (two Node instances as the reference, one Node and one Corvis instance as the candidate), `scenarios/cluster/{broadcast,bulk,fetch-sockets,broadcast-ack,server-side-emit,fetch-sockets-redacted,wire/messages}.mjs` | the redacted handshake of `fetch_sockets` | a `redis:7` service in the Linux `test` job for the Rust tests only |

Each of them states that Node-gated runs stay out of CI until #37 (#32 design.md Decision 10, #33 Decision 10, #35
Decision 12, #36 Decision 12).

Two facts found in the #32 worktree matter here:

- Its `CONTRIBUTING.md` section records that one test of the upstream `engine.io-protocol` suite ("forcefully closes
  the session", HTTP long-polling) sends a `GET` and a `POST` of the close packet at the same time and expects the
  `GET` to arrive first; it fails one run in twenty on an idle machine and one in eight under load, **against the
  Node reference as well**.
- `expected-differences.json` entries are keyed by scenario and by a path that indexes the transcript
  (`$.entries[9].data.reason`), and their `spec` field already has the shape
  `ws/engine-io: Send buffers are bounded (Slow consumer)`.

## Goals / Non-Goals

**Goals:**

- Socket.IO parity is a required check: a change that breaks it cannot reach `main`.
- A green run means every scenario ran on both targets; a red run tells which scenario and which path.
- The catalog is complete against the five Socket.IO specs, and stays complete: adding a requirement without a
  verifier fails.
- No flakiness, and no retries to hide it.

**Non-Goals:**

- New Socket.IO features, new harness structure, a second runner. Everything is added to `run.mjs`, the existing
  apps and the existing Rust entry points.
- Docs site pages and the example (#38).
- Windows or browser runs in CI, load comparisons, upstream drift detection (proposal, Out of scope).

## Decisions

### 1. The audit is a file that a script checks, not a one-time review

`parity/socket-io/coverage.json`:

```json
{
  "ws/socket-io": {
    "Events match Node": {
      "comparable": true,
      "verifiedBy": [
        { "scenario": "socket-io/events" },
        { "suite": "socket.io-protocol" }
      ]
    },
    "Options are validated": {
      "comparable": false,
      "reason": "Node accepts any value; the check exists only in Corvis",
      "verifiedBy": [{ "test": "crates/corvis/tests/socket_io.rs::zero_connect_timeout" }]
    }
  }
}
```

`scripts/check-coverage.mjs` (no dependency, Node built-ins only):

1. Reads the `### Requirement:` headings of `openspec/specs/ws/{engine-io,socket-io,socket-io-rooms,socket-io-gateways,socket-io-cluster}/spec.md`
   (the main specs, where the archived deltas of #32 to #36 live). A capability file that does not exist is an error:
   this change starts after the five are archived.
2. Fails on: a requirement without an entry; an entry whose requirement is not in the spec; `comparable: true`
   without a `scenario` or `suite`; `comparable: false` without `reason` or without a `test`.
3. Resolves every verifier: `scenario` against the `name` exported by the files under `scenarios/` (read as text with
   the same pattern for all files, `name: "<value>"`, so no scenario module is imported and no package is needed);
   `suite` against the keys of `suites.json`; `test` as `<path>::<function>`, the file must exist and contain
   `fn <function>(`.
4. Validates `expected-differences.json`: the `spec` field of each entry is `<capability>: <requirement>` with an
   optional ` (<text>)` suffix, and the capability and requirement exist. The capability may be any main spec, not
   only the five (an entry could cite `ws/socket-io-parity`).
5. Validates the exclusions of `suites.json` (Decision 5): each `coveredBy` is an existing scenario.

It runs in three places: as a self-test (`test/coverage.test.mjs` calls it on the repository and on small fixtures
for each failure), so the Rust entry point runs it before the scenarios; as its own step in the CI job, so its
output is easy to find; and by hand.

Writing the manifest is the audit of spike task 8.1: for each of the roughly 110 requirements
(25, 24, 14, 22 and 26 in the five deltas as approved), find the scenario or test that the owning change wrote, and
add the missing ones. Missing verifiers found by the audit are added in this
change when they are scenarios; a missing Rust test is added next to the tests of the owning layer.

Alternatives considered: a table in `design.md` (rejected: it is stale at the next change, and the problem this
solves is keeping the catalog complete after #37); annotations inside scenario files (`covers: [...]`, rejected: Rust
tests and suites verify many requirements, so the list would live in three kinds of files); parsing the spec's
scenarios instead of its requirements (rejected: too fine, a scenario title is not a stable key).

### 2. Transport matrix in the runner

A scenario module may export `transports: true`. The runner then runs it once per variant on each target and passes
`clientOptions` to `run`:

| Variant | `clientOptions` |
|---|---|
| `@polling` | `{ transports: ["polling"] }` |
| `@websocket` | `{ transports: ["websocket"] }` |
| `@default` | `{}` (polling first, then upgrade) |

Scenarios spread it into their `io(url, { ...clientOptions, ... })` calls. Transcripts are written to
`out/<target>/<scenario>@<variant>.json`; the comparison, the report and `--scenario <substring>` use the name with
its variant. In `expected-differences.json` an entry whose `scenario` has no `@` applies to every variant of a matrix
scenario and is stale when the difference is missing in any of them; an entry may name one variant.

Declared by: every official-client scenario of `scenarios/socket-io/` (not `wire/`), of `scenarios/socket-io/rooms/`
and of `scenarios/gateways/` (not `wire/`), and the new scenarios of Decision 3 except `upgrade-order` (which is
about the default path only). `socket-io/binary` already runs polling-only and WebSocket clients by hand (#33); it
moves to the matrix. Cluster scenarios run on the default only: they exercise the adapter, not the transport, and
each takes two servers and a Redis key.

Alternative considered: three scenario files per behavior. Rejected: the variants would drift apart, and the matrix
is exactly "the same scenario, unchanged" that `ws/socket-io-parity` asks for.

### 3. Scenarios that cross layers

The issue lists five areas. What the earlier changes cover and what is added:

| Area | Covered by #32 to #36 | Added here |
|---|---|---|
| Events | `socket-io/events`, `binary`, `wire/encoding`, `gateways/events`, `body-mapping` | every one on the three transports; `socket-io/upgrade-order` |
| Acks | `socket-io/acks`, `rooms/broadcast-ack`, `gateways/ack`, `no-ack`, `cluster/broadcast-ack` | every one on the three transports; the ack of an emit buffered across a reconnection |
| Rooms | `socket-io/rooms/*`, `gateways/server-emit`, `cluster/broadcast`, `bulk` | rooms after a reconnection; `cluster/gateways` |
| Disconnects | `socket-io/disconnect`, `rooms/leave-and-disconnect`, `engine-io/heartbeat`, `gateways/lifecycle` | `socket-io/reconnect`, `socket-io/heartbeat` |
| Errors | `engine-io/errors`, `socket-io/middleware`, `namespaces`, `wire/malformed`, `gateways/exceptions`, `filter`, `guard` | `socket-io/oversized-emit` |

The five new scenarios:

- **`socket-io/reconnect`** (matrix). Official client with `reconnection: true`, `reconnectionDelay: 50`,
  `reconnectionDelayMax: 50`, `randomizationFactor: 0`. Steps: connect, `join` a room, ask the server to close the
  transport (new app command `drop-transport`: Node `socket.conn.close()`; Corvis closes the Engine.IO session of
  that socket from the test app), wait for `disconnect`, emit `echo` with an acknowledgement while disconnected, wait
  for the manager's `reconnect` event and the socket's `connect`, call `rooms`. Recorded: the disconnect reason, the
  attempt number of `reconnect`, the new id (the normalizer keeps "different from the first"), `socket.recovered`,
  the rooms, the acknowledgement arguments. No timing is recorded.
- **`socket-io/heartbeat`** (matrix). Uses the short-heartbeat configuration the apps already offer for #32 and #33
  (300 ms / 200 ms). The client counts `ping` events of `socket.io.engine` and the scenario waits for the fifth, then
  records `connected` and that no `disconnect` was observed. Waiting on the fifth event, not on a duration.
- **`socket-io/upgrade-order`** (default only). The client emits `seq` 0..199 from its `connect` handler; the server,
  on connection, emits `seq` 0..199 to the client and acknowledges the client's last event with the list it received;
  the scenario records both lists and `socket.io.engine.transport.name` after the `upgrade` event.
- **`socket-io/oversized-emit`** (matrix). One client emits a 1,000,001-byte string; the scenario records the
  client's `disconnect` reason, then `GET /harness/log` (the handler was not called) and an `echo` from a second
  client. If Node and Corvis differ here it is a bug of #32 or #33, not an expected difference.
- **`cluster/gateways`** (requires `gateways` and `redis`, `cluster: 2`). The gateway reference app and the Corvis
  gateway app take `--redis-url` / `--redis-key` as the scenario apps do since #36 (`@socket.io/redis-adapter` is
  added to the gateway reference's `package.json` at the version the harness pins). A client on instance A sends the
  `broadcast` gateway event of #35; a client on instance B in the room receives it; the marker event of #34 shows who
  did not.

Each scenario follows the existing rules: waits on events inside `step`, never sleeps, absence shown by a marker
event, ids in fields named `sid`.

Alternative considered: writing one large "end to end" scenario per area. Rejected: a failure would point at a
scenario with forty steps; the existing per-behavior files plus the matrix give the same coverage with small diffs.

### 4. Strict mode

`run.mjs --require-all`: after the run, if any scenario or variant was skipped (`plan` returned a skip, or a target
could not be started because its package is not installed), print each with its reason and exit 1. The last line of
every run becomes
`<n> scenario(s) compared, <u> unexpected, <e> expected, <s> stale difference(s), <k> skipped`.

`socket_io_diff.rs` passes `--require-all` when `CORVIS_NODE_DIFF_STRICT=1`. A strict run of a test built without
`--features socket-io-redis` therefore fails by itself: the Corvis target lacks capabilities, scenarios are skipped,
strict mode refuses. The two conformance tests are `#![cfg(feature = "socket-io")]` files, so a run without the
feature would silently run zero tests; the CI step names the features and the three test targets, and the job greps
its own log for the three summary lines (the runner's line with `0 skipped` and mocha's passing counts) as its last
step.

Alternative considered: making skips always fatal under `CORVIS_NODE_DIFF=1`. Rejected: a contributor working on
#33 without Redis would have to start one to run the scenarios of their layer; `ws/socket-io-parity` already
specifies the lenient behavior ("Redis scenarios need CORVIS_REDIS_URL").

### 5. Upstream tests that race are excluded by name, with a replacement

`suites.json` gains, per suite, `"exclude": [{ "title": "...", "reason": "...", "coveredBy": "engine-io/close-while-polling" }]`.
The conformance tests:

1. run `mocha --dry-run` on the suite and check that each excluded title occurs exactly once (a stale or ambiguous
   exclusion fails);
2. run mocha with `--grep <alternation of the escaped titles> --invert`.

The same exclusions apply to the control run against the Node reference. The list starts with the test recorded by
#32, if it still races at implementation time (task 4.1 measures it: fifty runs against the Node reference). Its
behavior is closing a polling session while a `GET` is pending. The "Client close" part of #32's
`scenarios/engine-io/polling.mjs` posts the close packet with no `GET` pending, so this change adds the pending case
as a deterministic scenario, `engine-io/close-while-polling`: it sends the `GET`, asks a new loopback-only control
route (`GET /harness/poll-pending/<sid>`, answered by both apps once the session holds a poll; in Node, when the
polling transport is `writable`) and only then posts the close packet, recording the body of the `GET`, the status
of the `POST` and the close reason.

Alternatives considered: retrying the suite (forbidden by the spec, and it hides real races in Corvis); leaving the
suite out of the required job (it is the official definition of the protocol); patching the fetched test file
(a second kind of rewrite in `fetch-suites.mjs`, harder to review than a title in a JSON file).

### 6. The CI job

Added to `.github/workflows/ci.yml`, after `fuzz`:

```yaml
  socket-io-diff:
    name: socket-io-diff
    runs-on: ubuntu-latest
    timeout-minutes: 45
    services:
      redis:
        image: redis:7
        ports:
          - 56379:6379
        options: >-
          --health-cmd "redis-cli ping"
          --health-interval 5s
          --health-timeout 5s
          --health-retries 20
    env:
      RUSTFLAGS: -D warnings
      CORVIS_NODE_DIFF: "1"
      CORVIS_NODE_DIFF_STRICT: "1"
      CORVIS_REDIS_URL: redis://127.0.0.1:56379
      CORVIS_REDIS_REQUIRED: "1"
```

Steps: checkout (`persist-credentials: false`); the pinned Rust toolchain; `Swatinem/rust-cache` (same pin and
`save-if` as the other jobs); `actions/setup-node@v7` with Node 22 and the npm cache keyed on both lockfiles
(`parity/socket-io/package-lock.json`, `parity/socket-io/gateway-reference/package-lock.json`); `npm ci` in both
directories; `actions/cache` for `parity/socket-io/suites` keyed on the hash of `suites.json` and
`scripts/fetch-suites.mjs`, then `node scripts/fetch-suites.mjs` on a miss; `node scripts/check-coverage.mjs`;
`cargo test -p corvis --features socket-io-redis --test socket_io_diff --test engine_io_conformance --test
socket_io_conformance --no-fail-fast -- --nocapture`; the summary check of Decision 4; on failure,
`actions/upload-artifact@v7` with `parity/socket-io/out/` (14 days, `if-no-files-found: ignore`).

- The Redis service is declared exactly as #36 declares it for the `test` job (image, port, health check), so both
  jobs can be read side by side. The harness uses a key of its own per scenario run (#36), and service containers are
  per job, so the two jobs do not share a server.
- No secret, no write permission (the workflow's `permissions: contents: read` applies), no new third-party action:
  `actions/*` at a major version, as the header of `ci.yml` states.
- `npm ci` runs with `ignore-scripts=true` from the harness's `.npmrc`; task 5.1 checks that
  `gateway-reference/` has the same file, and adds it if #35 did not.
- The job runs on every pull request and push, like the others, without a path filter: a required check that is
  skipped by a path filter stays "expected" and blocks the merge, and the runtime code Socket.IO depends on
  (`server.rs`, `ws.rs`, the router, the macros) is most of the crate anyway.
- Trigger, concurrency and cache rules are inherited from the workflow.

Required status check: `socket-io-diff` is added to the protection of `main` by the maintainer once the job has been
green on `dev` (tasks.md, Workflow follow-up); the `infra/ci` requirement that lists the required checks is modified.

Nightly: `nightly.yml` gains a third job, `socket-io-diff (repeat)`, with the same setup, that runs the
`socket_io_diff` test ten times in a shell loop that stops at the first failure and uploads `out/` of the failing
run. It checks out `dev` on a schedule, like the other two jobs.

Alternative considered: adding the Node steps to the existing `test (ubuntu-latest)` job. Rejected by the spike's
Review note 3 (its own job), and a harness failure would hide the Rust test results.

### 7. Stability is verified before the job becomes required

Before the job is added to the required checks: ten consecutive strict runs locally on Linux and on Windows (the
harness must keep working where contributors work), and the nightly repeat job green on three scheduled runs. Any
intermittent result is traced to its cause. The usual causes and their fixes are written in `CONTRIBUTING.md`: a
wait on time instead of on an event, an order between two independent clients recorded in one client's transcript,
state left by a previous scenario on the shared target (fixed with `POST /harness/reset`, #33).

A deliberate regression test of the job itself: on a throwaway branch, change one acknowledgement argument in the
Corvis harness app and confirm the job fails and names the scenario; stop the Redis service in a throwaway run and
confirm strict mode fails.

### 8. `CONTRIBUTING.md`

- "Continuous integration": a row for `socket-io-diff` (Linux, Redis 7 service; the local commands: `npm ci` twice,
  `node scripts/fetch-suites.mjs`, then the `cargo test` command of Decision 6 with the four variables), and the
  nightly paragraph mentions the repeated run.
- "Socket.IO differential harness": strict mode, the matrix (how to declare it, how variants are named), the
  coverage manifest (how to add an entry when adding a requirement or a scenario), suite exclusions, how to download
  and read the transcripts artifact of a failed job, and the causes of flakiness of Decision 7.

### 9. Differences found by this change

A scenario added here can show that Corvis differs from Node. The rule, stated so that the job cannot be made green
the wrong way:

- If the difference is a bug, it is fixed in this change when the fix is local (with a regression test in the
  owning layer's test file), and the corvis-parity check is then part of the definition of done. A larger fix
  becomes an issue under epic #30, and #37 is not closed until it lands.
- If the difference is intentional, it must already be a stated divergence of a `ws/*` requirement; then the entry
  is added to `expected-differences.json` citing it. A new intentional divergence is not decided here: it needs a
  spec change in the owning capability.

## Risks / Trade-offs

- The job adds ten to fifteen minutes to every pull request (a Rust build of the test targets with the features,
  two `npm ci`, the catalog three times for matrix scenarios) → its own job, in parallel with the others; caches for
  cargo, npm and the suites.
- The suites are fetched from GitHub by commit → cached by the hash of `suites.json`; a fetch failure fails the job
  loudly rather than skipping the conformance run.
- The matrix triples the client scenarios → they take milliseconds each; the heartbeat scenario (five pings at
  300 ms) is the slowest, about two seconds per variant and target.
- The coverage check reads sources as text → a scenario whose `name` is computed, or a test generated by a macro,
  cannot be resolved; both are rejected by the check with a message, and neither exists today.
- The manifest can be satisfied with a weak verifier (a scenario that touches the requirement without asserting
  it) → the audit of task 1.2 is reviewed by the maintainer, and the manifest diff is small and readable in later
  pull requests.
- An excluded upstream test could hide a real Corvis bug in the same area → the exclusion requires a covering
  scenario, and the control run against Node uses the same exclusions, so the list cannot grow to make Corvis pass.
- `socket-io-diff` as a required check makes Node, npm and GitHub's registry availability part of merging to `main`
  → accepted by the spike (Review note 3); `npm ci` uses the lockfile and the npm cache.
- Index-based paths in `expected-differences.json` shift when a matrix scenario gains a step → loud failure; noted
  as a possible later issue.

## Open questions

1. The issue and `ROADMAP.md` name this change `e2-differential-tests-against-node-socket-io`; the draft uses
   `e2-socket-io-differential-tests` as assigned, consistent with `e2-socket-io-gateway-adapter` and
   `e2-socket-io-redis-adapter`. Recommendation: keep the draft's name and update `ROADMAP.md` and the issue body, as
   the Review notes of #34, #35 and #36 did.
2. Should `socket-io-diff` be a required status check of `main` from the day it is merged, or after a trial period?
   Recommendation: required after it has been green on `dev` for the three scheduled nightly runs of Decision 7.
3. The job runs on every pull request, without a path filter. Alternative: run it only when `crates/corvis/**`,
   `crates/corvis-macros/**` or `parity/**` change, with a job-level condition that reports success otherwise.
   Recommendation: always run; the saving is small and a wrong filter is a hole in a required check.
4. Excluding the racy upstream test by title (Decision 5), or keeping the whole suite and accepting an occasional
   red run. Recommendation: exclude, with the covering scenario; a required check that fails one run in eight is
   ignored within a week.
5. `cluster/gateways` requires the gateway reference to take the Redis adapter, which brings
   `@socket.io/redis-adapter` into a second lockfile. Alternative: leave gateways over Redis to the Rust tests.
   Recommendation: keep the scenario; the migration this framework is for runs gateways on a cluster.
6. The nightly repeat count (ten) and whether it should also run on pull requests that touch `parity/`.
   Recommendation: ten, nightly only.
7. Should the coverage manifest also cover `ws/gateways` and `ws/exception-filters` (the `ws`-style adapter, whose
   Node oracle is the offline fixture of `crates/corvis/tests/fixtures/ws_gateways/`)? Recommendation: no; that
   adapter is outside epic #30. A later issue can extend the manifest.

## Review notes

These settle the open questions and override the design where they differ.

1. The change keeps the name `e2-socket-io-differential-tests`; `ROADMAP.md` is updated to match.
2. The `socket-io-diff` job runs on every push to `dev` from the first day. It becomes a required check of `main`
   after three consecutive green nightly runs on `dev`; the maintainer changes the branch protection, so the task
   that does it stays open until then and does not block archiving the rest.
3. The job has no path filter.
4. The flaky upstream test is excluded by exact title, with its reason and its deterministic substitute scenario.
5. `cluster/gateways` keeps `@socket.io/redis-adapter` in the second lockfile.
6. Ten repetitions, in the nightly run only.
7. The coverage manifest covers the five `ws/*` capabilities of epic #30 only.
8. A difference the harness finds is fixed or blocks closing #37; it is never added to `expected-differences.json`
   to turn the job green. A fix that changes runtime code gets the usual security review.
9. Implemented after #36 lands.

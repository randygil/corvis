# Design

Spike for [#31](https://github.com/randygil/corvis/issues/31). Sources were read on 2026-11-08; versions are pinned
below so the findings can be re-checked.

## Context

### What Corvis has today

Gateway protocol (`crates/corvis/src/websockets.rs`):

- Frames are JSON `{ "event", "data" }` over RFC 6455 (`websockets.rs:1-4`). An optional `id` in the incoming frame is
  echoed in the reply, a Corvis-specific ack extension (`dispatch`, `websockets.rs:777-822`;
  `docs/websockets/gateways.md` "Acknowledgements").
- Binary frames are decoded as lossy UTF-8 and parsed as JSON (`run_connection`, `websockets.rs:914-916`): there is
  no binary payload support. Non-JSON frames and unknown events are silently ignored (`websockets.rs:933-940`).
- Gateways are keyed by one URL path (`build_gateways`, `websockets.rs:584-593`; `GatewayInfo.path`,
  `discovery.rs:114-122`). The macro treats `namespace` as a synonym of `path` and has no `cors` option
  (`crates/corvis-macros/src/gateway.rs:13-39`; documented in `docs/websockets/gateways.md` line 93).
- Upgrades are matched only when `Upgrade: websocket` is present and the path equals a gateway path
  (`match_upgrade`, `websockets.rs:601-608`), checked before routing in `handle` (`crates/corvis/src/server.rs:322`).
  An Engine.IO long-polling request (`GET/POST /socket.io/?EIO=4&transport=polling`) is a plain HTTP request and would
  reach the router (404).
- Rooms are a per-client `Mutex<HashSet<String>>` (`websockets.rs:238`, `join`/`leave` at 281-286); broadcast walks
  every client of the server and filters by room (`RoomEmitter::emit`, `websockets.rs:504-512`). `except` takes a
  single client id (`websockets.rs:500`). Rooms are per gateway server; there is no namespace level and no adapter
  abstraction. A client does not join a room named after its own id.
- `WsServer` instances are stored per path in a process-wide map (`SERVERS`, `websockets.rs:444`) and per gateway
  instance (`build_gateways`).
- Handler pipeline: guards then interceptors then pipes then the handler (`run_handler`, `websockets.rs:697-730`); a
  guard returning `false` yields `WsException::new("Forbidden resource")` (`websockets.rs:713`); errors become an
  `exception` event with `exception_payload` (`websockets.rs:127-148`). The context is built with
  `ExecutionContext::for_ws` (`crates/corvis/src/common/context.rs:84`) and read with `switch_to_ws()`
  (`context.rs:111`, `WsArgumentsHost` at 165).
- Hardening already in place and covered by `crates/corvis/tests/ws_security.rs`: bounded send buffer with close 1008
  (`OutQueue`, `crates/corvis/src/ws.rs:80-170`, default 8 MiB at `ws.rs:46`), bounded in-flight handlers per
  connection with TCP backpressure (`max_inflight` 32, `websockets.rs:371-373`, used at 894), close timeout
  (`ws.rs:49`), handshake validation like `ws` (`validate_handshake`, `websockets.rs:625-648`), origin allowlist and
  `verify_client` (`GatewayOptions`, `websockets.rs:363-396`), panic containment (`guarded`, `ws.rs:201`;
  `catch_unwind` in `dispatch`, `websockets.rs:788`).
- Pings are answered by tungstenite (`websockets.rs:913`); there is no application-level heartbeat.
- Dependencies: `tokio-tungstenite` 0.24 (optional, feature `websockets`, on by default:
  `crates/corvis/Cargo.toml:35,46,48`), `hyper` 1 (`Cargo.toml:25`, 1.12.0 in `Cargo.lock`). No `tower`,
  `tower-service` or Redis crate in `Cargo.lock`.
- Existing tests: `crates/corvis/tests/websockets.rs` (round trip, ordering, per-app servers, guards/pipes/
  interceptors), `crates/corvis/tests/ws_security.rs` (limits, handshake, panics). The only Node oracle pattern in the
  repo captures fixtures offline with a hard-coded `node_modules` path
  (`crates/corvis/tests/fixtures/res_express.oracle.mjs:2-3`), which is not reproducible on another machine or in CI.
- Port apps: none of consi-backend, facten-api, houseplan depends on `socket.io`; `facten-api` depends on `ws`
  (`package.json`), which Corvis serves with `corvis::ws`.
- CI: the repository has no `.github/workflows/` yet; the workflow is planned by the open change
  `openspec/changes/e0-ci-in-github-actions` (#6: jobs `fmt`, `clippy`, `test`, `fuzz`, `deny`, `audit`, `docs`). The
  socket.io differential job is added to that workflow by #37.
- Gateway results today: a handler's `Ok(Some(reply))` is sent as a `{event, data}` frame and errors as an `exception`
  frame, both echoing the incoming `id` (`dispatch`, `websockets.rs:798-821`); `exception_payload`
  (`websockets.rs:127-148`) already produces the original base filter's `{status: "error", message}` shape.

### What the official client depends on

Versions: npm `socket.io` 4.8.4 (depends on `engine.io` ~6.6.0, `socket.io-parser` ~4.2.4, `socket.io-adapter`
~2.5.2), `engine.io` 6.6.11, `socket.io-client` 4.8.4 (`engine.io-client` ~6.6.1). Protocol specs:
<https://github.com/socketio/engine.io-protocol> and <https://github.com/socketio/socket.io-protocol>; server source
`packages/engine.io/lib/{server,socket,transports/polling}.ts` and `packages/socket.io/lib/{socket,namespace,client}.ts`
in <https://github.com/socketio/socket.io>.

Engine.IO v4:

- Query: `EIO=4` (required), `transport=polling|websocket`, `sid` after the handshake; `t` is a cache buster. Default
  path `/engine.io`, which socket.io sets to `/socket.io/`.
- Handshake: open packet `0{"sid","upgrades","pingInterval","pingTimeout","maxPayload"}`; defaults pingInterval
  25000, pingTimeout 20000, upgradeTimeout 10000, maxHttpBufferSize 1e6 (sent as `maxPayload`), transports
  `["polling","websocket"]`, `allowUpgrades` true, `cors` off, cookie off, `allowEIO3` false. The sid comes from
  `base64id.generateId()`. A WebSocket-first connection (no polling) receives the open packet as its first frame with
  `upgrades: []`.
- Packets: 0 open, 1 close, 2 ping, 3 pong, 4 message, 5 upgrade, 6 noop. Polling payloads join packets with `\x1e`;
  binary packets are `b` + base64 on polling, raw binary frames on WebSocket.
- Heartbeat (v4): the server sends `2` every pingInterval and closes with reason `ping timeout` if no `3` arrives
  within pingTimeout.
- Upgrade: client sends `2probe` on the WebSocket, server answers `3probe`, the server answers the pending poll with
  `6` (noop) while the upgrade is in progress, client sends `5`, the polling transport is closed.
- Errors: HTTP 400 (403 for Forbidden), `Content-Type: application/json`, body `{code, message}` with 0 "Transport
  unknown", 1 "Session ID unknown", 2 "Bad handshake method", 3 "Bad request", 4 "Forbidden", 5 "Unsupported protocol
  version".
- Polling: a second concurrent GET (or POST) for the same sid gets 400 and closes the session; a POST body above
  maxHttpBufferSize gets 413; a binary POST body in v4 gets 400; a valid POST answers `200 ok` (`text/html`); GET
  responses carry `cache-control: no-store`.

Socket.IO v5:

- Packet: `<type>[<attachments>-][<nsp>,][<ackId>][JSON]`, types 0 CONNECT, 1 DISCONNECT, 2 EVENT, 3 ACK,
  4 CONNECT_ERROR, 5 BINARY_EVENT, 6 BINARY_ACK; `/` is omitted; binary placeholders `{"_placeholder":true,"num":N}`.
- CONNECT carries the optional `auth` object; the answer is `{sid}` (plus `pid` only with connection state recovery).
  Middleware errors send CONNECT_ERROR `{message, data}`; unknown namespaces `{message:"Invalid namespace"}`.
- ACK payloads are always arrays; server ack ids are a per-namespace counter; ack timeouts fail with
  "operation has timed out".
- Reserved event names (cannot be emitted): connect, connect_error, disconnect, disconnecting, newListener,
  removeListener.
- Disconnect reasons: transport error, transport close, forced close, ping timeout, parse error, server shutting down,
  forced server close, client namespace disconnect, server namespace disconnect.
- `connectTimeout` 45000: an Engine.IO connection that joins no namespace in time is closed. A decode error, a
  non-CONNECT packet for an unjoined namespace, or a duplicate CONNECT closes the connection.

### socketioxide (the crate option)

- `socketioxide` 0.18.9 (2026-11-01), MIT, MSRV 1.94, edition 2024; `engineioxide` 0.17.9, `socketioxide-core`
  0.20.0. <https://crates.io/crates/socketioxide>, <https://github.com/Totodore/socketioxide>.
- Built as a Tower service on hyper 1 / http 1: depends on `tower-service`/`tower-layer` 0.3, `hyper` 1.9,
  `tokio-tungstenite` **0.30**, `matchit` 0.9. Entry points `SocketIo::new_svc()`, `build_with_inner_svc(svc)`,
  `new_layer()`; its e2e server serves a raw hyper `http1` connection with upgrades.
- API: `io.ns(path, handler)` (dynamic namespaces too), axum-style extractors (`SocketRef`, `Data<T>`, `TryData<T>`,
  `AckSender`, `Extension`, `State`, `HttpExtension`, `DisconnectReason`), `emit_with_ack` with timeouts (default
  5 s), operators `to/within/except/local/broadcast/timeout/emit/fetch_sockets/disconnect/join/leave`, connect
  middleware `handler.with(mw)`. No per-event middleware. Config defaults: `max_payload` 100 kB (Node 1e6),
  `max_buffer_size` 128 packets, `ws_max_message_size` 64 MiB, `connect_timeout` 45 s, `req_path` `/socket.io`.
- Connection state recovery: not supported (issue #729 open since 2026-05-23).
- Conformance: CI runs in-repo TypeScript ports (`node:test`) of the official engine.io and socket.io protocol test
  suites against e2e servers, for EIO v3/v4 and SIO v4/v5/v5-msgpack
  (<https://github.com/Totodore/socketioxide/blob/main/.github/workflows/github-ci.yml>). It does not run the upstream
  suites directly, and does not compare against a Node server.
- `socketioxide-redis` 0.4.4 states: "Socketioxide-Redis is not compatible with `@socketio/redis-adapter` and
  `@socketio/redis-emitter`" (<https://docs.rs/socketioxide-redis/latest/socketioxide_redis/>); it needs RESP3
  (Redis 7+).
- Maintenance: about 1,500 commits, of which about 1,200 by one author (bus factor 1); 0.x with breaking releases.

Not verified in the spike (to be checked by the harness if the fallback of Decision 1 is ever taken): the exact
CONNECT_ERROR payload of socketioxide (whether `data` is included), 413 vs 400 above `max_payload`, the literal
"Invalid namespace", and whether event handlers are spawned without a per-connection bound.

### `@socket.io/redis-adapter`

Version 8.3.0 (`notepack.io` ~3.0.1 msgpack, peer `socket.io-adapter` ^2.5.4),
<https://github.com/socketio/socket.io-redis-adapter>. Classic adapter: channels `socket.io#<nsp>#`, where `<nsp>`
keeps its leading slash (plus `<room>#` for single-room broadcasts), `socket.io-request#<nsp>#`,
`socket.io-response#<nsp>#` (plus `<uid>#`); node uid
`uid2(6)`; a broadcast is msgpack `[uid, packet, {rooms, except, flags}]`; request types 0 SOCKETS .. 9 BROADCAST_ACK,
JSON except broadcasts with acks (msgpack; the first byte `0x7b` tells JSON apart); `requestsTimeout` 5000.

### Conformance suites

- `socketio/engine.io-protocol/test-suite` and `socketio/socket.io-protocol/test-suite`: mocha + chai, target
  `http://localhost:3000`; the socket.io suite expects pingInterval 300, pingTimeout 200, maxPayload 1e6,
  connectTimeout 1000, `cors: { origin: "*" }`, an `auth` echo on connect, `message` answered as `message-back`,
  `message-with-ack`, and a `/custom` namespace.

## Goals / Non-Goals

**Goals:**

- Decide the implementation strategy with evidence and record it.
- List the protocol behavior the official client depends on, so #32-#34 have a precise target.
- Design the verification (conformance suites plus a Corvis-vs-Node differential harness) that #37 builds and that
  #32-#36 use from their first test.
- Plan #32-#38 as concrete task groups.

**Non-Goals:**

- Writing code or prototypes in the repository (no compilation is allowed in this spike).
- Engine.IO v3, connection state recovery, msgpack parser, compression, sharded Redis (see proposal, Out of scope).

## Decisions

### 1. Implement Engine.IO and Socket.IO in Corvis; do not depend on socketioxide

Options compared:

| Criterion | A. socketioxide behind a Corvis facade | B. Own implementation on `ws.rs`/`websockets.rs` |
|---|---|---|
| Protocol parity evidence | Strong: ports of the official suites in CI | None yet; must be earned with the suites and the Node diff (#37) |
| Redis compatible with `@socket.io/redis-adapter` (#36) | No: `socketioxide-redis` is explicitly incompatible; Corvis would write an adapter against socketioxide's 0.x adapter trait anyway | Written once against Corvis' own adapter trait |
| Corvis hardening (byte-bounded send queue with 1008, bounded in-flight handlers with read backpressure, close timeout, origin allowlist, panic containment) | Must be re-verified or re-built around a black box; queue bound is in packets (128), not bytes; per-event concurrency bound unverified | Reused as is (`OutQueue`, `write_loop`, `guarded`, `max_inflight`), already covered by `ws_security.rs` |
| Guards/pipes/interceptors per event | Wrap every handler; no per-event middleware upstream | Native: `run_handler` already does it |
| Dependency footprint | Adds `tower-service`, `tower-layer`, `matchit` 0.9, `state`, and a second tungstenite (0.30 next to 0.24) | No new crates for #32-#35 |
| Maintenance risk | Bus factor 1, 0.x with breaking releases, MSRV 1.94 moving | Corvis owns about 3-4k lines of protocol code |
| Cost | #32 shrinks to integration; #33/#34 to a facade | #32 is L, #33/#34 M (as sized in ROADMAP.md) |
| Error/edge parity with Node (CONNECT_ERROR `data`, 413, defaults like `max_payload` 100 kB vs 1e6) | Unverified; fixes need upstream PRs | Fixed locally, verified by the diff |

Recommendation: **B**. The positioning is verified parity with security owned by Corvis; the two places where
option A saves most work (Engine.IO transport state machine, rooms) are where Corvis already has hardened building
blocks, and the Redis adapter (#36) has to be written either way. Option A's real advantage, conformance evidence, is
recovered by running the same official suites against Corvis (Decision 4).

Fallback: the public API (Decision 3) names no implementation type, so if #32 overruns its estimate the maintainer can
switch to option A behind the same API without touching gateways, docs or tests. socketioxide's source (MIT) is a
useful reference for edge cases; code copied from it must be credited in `THIRD_PARTY_NOTICES.md`.

### 2. Module layout and feature flag

- `crates/corvis/src/socket_io/` with `engine/` (packet codec, session, polling transport, WebSocket transport,
  upgrade, heartbeat), `parser.rs` (Socket.IO packet codec with binary attachments), `namespace.rs`, `socket.rs`,
  `adapter.rs` (trait plus in-memory adapter), and later `redis.rs` behind its own feature.
- Cargo feature `socket-io` (depends on `websockets`), off by default until #37 passes; `socket-io-redis` for #36.
  Whether `socket-io` joins the default features is an open question.
- Entry point: in `handle` (`server.rs:320-337`), before `match_upgrade`, a request whose path is under a mounted
  Engine.IO path (default `/socket.io/`) goes to the Engine.IO server, for both polling (plain GET/POST/OPTIONS) and
  WebSocket upgrades. Polling POST bodies are read with the `maxHttpBufferSize` cap, not the router's body parser.
- The WebSocket transport reuses `OutQueue`/`write_loop` from `ws.rs` and the RFC 6455 handshake checks of
  `validate_handshake`; Engine.IO's own errors (`{code, message}`) apply before the upgrade.

Alternative considered: a separate crate `corvis-socket-io`. Rejected for now: it needs `pub(crate)` internals of
`ws.rs`, `server.rs` and the gateway pipeline; can be split later.

### 3. Public API shape

- Low level, mirroring the Node server: `IoServer` (namespaces via `of(name)`, `use`-style connect middleware,
  `emit`, `to/in/except`, `sockets()`), `IoSocket` (`id`, `handshake`, `data`, `emit`, `emit_with_ack`, `join`,
  `leave`, `rooms`, `to`, `broadcast`, `disconnect(close)`), `Ack` sender. Names avoid the original framework's
  names; `socket.io` is named as the protocol.
- Gateways (#35): an adapter is chosen per application, like `useWebSocketAdapter(new IoAdapter(app))` in the
  original: `app.use_websocket_adapter(IoAdapter::new())`. The current `ws` protocol remains the default, so existing
  gateways and the Tertulia tests do not change. Under the socket.io adapter `#[websocket_gateway]` separates
  `path` (Engine.IO path, default `/socket.io`) from `namespace` (default `/`) and accepts `cors`; under the `ws`
  adapter `namespace` keeps its current meaning (a synonym of `path`) for compatibility.
- `#[connected_socket]` receives a handle that works with both adapters (`WsClient` gains the socket.io operations,
  or a new `IoSocket` is accepted for socket.io gateways; decided in #35's design).
- Handler results under socket.io follow the original `IoAdapter`: a `WsResponse` (`{event, data}`) is emitted as that
  event; any other non-null value goes to the client's ack callback if it sent one, and is dropped otherwise. The
  Corvis-specific `id` echo of the `ws` protocol does not apply.

Alternatives considered: making socket.io the default adapter (closer to the original model, breaks every existing
gateway client; open question); a per-gateway adapter attribute (not in the original; rejected).

### 4. Verification: conformance suites plus a differential harness

Two layers, both run by the Rust test suite when Node is available and always in CI on Linux:

1. Conformance: the upstream `engine.io-protocol` and `socket.io-protocol` test suites, pinned by commit, run with
   mocha against a Corvis test server configured as the suite expects (pingInterval 300, pingTimeout 200,
   connectTimeout 1000, the `/custom` namespace and the echo handlers). The suites hard-code
   `http://localhost:3000`; the harness passes the URL through a small wrapper or a patched copy, and binds to
   `127.0.0.1` only (local servers must bind to loopback).
2. Differential: a harness directory `parity/socket-io/` with its own `package.json` and `package-lock.json`
   (pinned `socket.io` 4.8.4, `socket.io-client` 4.8.4, `engine.io-client`, `ws`, `@socket.io/redis-adapter` 8.3.0,
   `redis`):
   - `reference-server.mjs`: the scenario app written with Node `socket.io`;
   - a Corvis binary (`examples/socket-io-diff-server` or a test-only bin) with the same app;
   - `client-scenarios.mjs`: scenarios driven by the official client (connect with auth, namespaces, events with
     several arguments, acks both ways, binary, rooms/broadcast with several clients, middleware errors, server and
     client disconnects, heartbeat timeout with short intervals) that print a JSON transcript of everything each
     client observes;
   - `wire-scenarios.mjs`: raw HTTP and `ws` requests for what the client hides (handshake JSON, polling framing,
     error codes and bodies, 400/413, overlapping polls, upgrade probe/noop, header set);
   - a normalizer (sids, ack ids where unspecified, timings, cross-client interleavings sorted) and a comparer that
     prints a diff and exits non-zero.
   - Rust entry point `crates/corvis/tests/socket_io_diff.rs`: skipped with a printed notice unless `node` is on
     `PATH` and `CORVIS_NODE_DIFF=1` (CI sets it and fails if Node is missing); starts both servers on ephemeral
     loopback ports, runs both scenario files against each, compares.
   - Redis scenarios (#36) run only when `CORVIS_REDIS_URL` is set: mixed clusters (one Node and one Corvis instance on
     the same Redis) must deliver the same broadcasts as two Node instances.
   - Gateway scenarios (#35): the reference side is the same gateway written with the original framework and its
     socket.io platform adapter (`@nestjs/platform-socket.io` and its peers, pinned in a separate
     `parity/socket-io/gateway-reference/` package so the protocol harness stays small), against a Corvis example app
     with the same gateway, guards, pipes and exception cases.
   - `expected-differences.json`: one entry per intentional divergence (Decision 5) with the reason and the scenario
     that exercises it; an unlisted difference fails, and a listed difference that no longer occurs fails too.
   - CI: a separate `socket-io-diff` job (Linux, Node 22 like `docs/Dockerfile`, `npm ci` in the harness, a Redis
     service container, `CORVIS_NODE_DIFF=1`, `CORVIS_REDIS_URL` set) added to the workflow of #6.

Alternative considered: offline fixtures captured once from Node, like `res_express.oracle.mjs`. Rejected: socket.io
behavior is a conversation (timing, upgrades, acks), fixtures go stale with the pinned versions, and the existing
fixture script depends on a machine-specific path.

### 5. Security divergences from Node, decided up front

Node `engine.io` has no session cap, no per-connection handler bound and keeps unlimited pending polling buffers. Corvis
keeps its existing protections and documents each divergence; the differential harness lists them as expected
differences so they cannot hide other ones:

- per-session byte-bounded send buffer (default 8 MiB as in `ws.rs:46`); overflow closes the session with
  reason `transport error` (Node: unbounded `writeBuffer`);
- bounded in-flight handlers per socket (`max_inflight`, default 32) with read backpressure;
- an optional cap on concurrent Engine.IO sessions (default off, so the default matches Node);
- `maxHttpBufferSize` 1e6 kept as in Node; WebSocket frame limit derived from it.

### 6. Spec deltas are targets; each follow-up owns its part

The deltas in this change define what #32-#37 must meet. Archiving this spike must not publish requirements that no
code meets yet: archive it without applying specs (`openspec archive <change> --skip-specs`), and each follow-up
change copies its delta from here (adjusting it to what its design finds) and applies it when archived.

### 7. Order of the follow-ups: the harness first

The ROADMAP lists #37 after the features, but every feature needs it as its test oracle. Order:

1. #37, first part: harness skeleton (pinned packages, runner, normalizer, comparer, expected differences, Rust entry
   point, conformance-suite runner) with only the Engine.IO wire scenarios, run against Node alone until #32 lands.
2. #32 Engine.IO, verified by the engine.io conformance suite and the wire scenarios.
3. #33 Socket.IO namespaces/events/acks, verified by the socket.io conformance suite and client scenarios.
4. #34 rooms and broadcast (adapter trait plus in-memory adapter).
5. #35 gateways over socket.io (adds the gateway reference package).
6. #36 Redis adapter (adds mixed-cluster scenarios).
7. #37, closing part: full scenario catalogue reviewed against the spec deltas, CI job required.
8. #38 docs and example.

#37 stays one issue and one change; its change is opened first and archived last (or split in two changes if the
maintainer prefers, see Open questions). #36 and #35 are independent of each other once #34 is done.

Alternative considered: the ROADMAP order (#37 after #36). Rejected: #32-#36 would be built against hand-written
expectations and re-verified later, which is the kind of unverified parity the project's positioning rules out.

## Risks / Trade-offs

- Own implementation cost (Engine.IO polling/upgrade races are subtle) → the official suites from day one of #32,
  and the fallback in Decision 1.
- Timing-dependent differential tests can be flaky → short heartbeat intervals only in dedicated scenarios, explicit
  waits for events instead of sleeps, normalization of interleavings, retries forbidden (a flaky diff is a bug).
- Node becomes a test dependency → gated by `CORVIS_NODE_DIFF`; required only in CI; `package-lock.json` pinned;
  the harness never runs `npm install` implicitly from `cargo test`.
- Mixed `ws`/socket.io semantics of `namespace` in the macro can confuse users → documented per adapter; a compile
  time note in the macro docs; covered in #38.
- Redis compatibility depends on msgpack details of `notepack.io` (e.g. how `undefined` and binary are encoded) →
  mixed-cluster differential scenarios in #36 rather than byte-level assumptions.
- Holding polling GETs open counts against server connection limits and graceful shutdown → #32 must close pending
  polls on shutdown with reason `server shutting down`, verified against Node.
- Second tungstenite version avoided by Decision 1; if the fallback is taken, it appears (0.24 and 0.30).

## Open questions

1. Should the `socket-io` feature become a default feature once #37 passes, and should socket.io eventually become
   the default gateway adapter (closer to the original model) in a future major version?
2. Vendor the upstream protocol test suites (MIT) pinned by commit into `parity/socket-io/`, or fetch them by commit
   in CI? Vendoring is reproducible offline; fetching keeps the repo smaller.
3. Is a Node toolchain acceptable as a CI requirement for the main test job, or should the differential job be a
   separate workflow job (recommended) so `cargo test` stays Node-free locally?
4. Redis client crate for #36: `redis` (redis-rs, BSD-3-Clause, widely used) or `fred` (MIT)? Both are allowed by
   `deny.toml`. Recommendation: `redis`, with RESP2 support so Redis 6 works like the Node adapter.
5. In #35, does `#[connected_socket]` keep the type `WsClient` for both adapters (one type, two transports) or take a
   separate `IoSocket` for socket.io gateways?
6. The original `BaseWsExceptionFilter` payload (whether recent versions add a `cause` field) must be confirmed against
   the pinned Node packages in #35; the current `exception_payload` (`websockets.rs:127-148`) omits it.
7. #37 started first and archived last (Decision 7): keep one change open across the epic, or split #37 into a
   "harness" issue (before #32) and a "full catalogue and CI" issue (after #36)? Recommendation: split, so no change
   stays open for weeks.
8. The gateway reference package (Decision 4) brings the original framework's packages into the repository as test
   dependencies (pinned, harness only, never shipped). Acceptable, or should gateway parity be checked against a
   hand-written Node socket.io server that mimics the platform adapter (less faithful)?
9. The spec delta for the socket.io conformance suites names the upstream suites' fixed configuration
   (pingInterval 300 and so on): should the Corvis test server for them be a test-only binary in `crates/corvis/tests/`
   or an example under `examples/`? Recommendation: a test-only bin, so examples stay user-facing.

## Review notes (2026-11-08)

- Own Engine.IO v4 / Socket.IO v5 implementation accepted, with the documented fallback to `socketioxide` if #32
  overruns; compatibility with `@socket.io/redis-adapter` (mixed Node/Rust clusters during a migration) is the
  deciding requirement.
1. `socket-io` stays an opt-in feature; making it a default feature or the default adapter is decided for 1.0.
2. The official protocol suites are downloaded at a pinned commit by a script (reproducible, nothing vendored).
3. The Node differential harness runs in its own CI job.
4. Redis client: the `redis` crate (same as the queues spike, #49).
5. A separate `IoSocket` type for Socket.IO gateways; `WsClient` keeps its meaning for the plain `ws` adapter.
6. Whether the original's `exception` payload includes `cause` is settled by the differential test in #33.
7. #37 is split: #131 (harness skeleton) is done first; #37 keeps the full catalog and CI.
8. The original framework's socket.io packages are accepted as test-only dependencies of the Node harness.
9. The server for the official suites is a test-only binary, not an example.
- The order with the harness first (Decision 7) is accepted.

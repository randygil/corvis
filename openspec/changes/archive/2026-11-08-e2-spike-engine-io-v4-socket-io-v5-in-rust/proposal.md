# Proposal

Implements [randygil/corvis#31](https://github.com/randygil/corvis/issues/31) (epic
[#30](https://github.com/randygil/corvis/issues/30), follow-ups #32-#38).

## Why

Corvis gateways only speak the `ws`-style protocol: RFC 6455 frames carrying JSON `{ event, data }`
(`crates/corvis/src/websockets.rs:1-4`). The original model's default WebSocket adapter is socket.io, so most Node
backends being migrated have browser and mobile clients built on the official `socket.io-client`. Those clients cannot
talk to Corvis today: there is no Engine.IO handshake, no long-polling transport, no heartbeat, no namespaces, no acks,
no binary attachments and no multi-instance adapter (`docs/websockets/adapter.md` says so explicitly).

Before the epic's seven implementation issues start, a spike has to settle three things with evidence: whether to build
on the `socketioxide` crate or implement the protocols on top of the existing Corvis WebSocket code; which protocol
details the official client actually depends on; and how a differential test against the Node `socket.io` server runs,
so every follow-up can be verified the same way.

## What Changes

This is a research change: it produces no runtime code. Its deliverables are:

- `design.md`: the comparison of `socketioxide` 0.18.9 versus an own implementation (parity evidence, maintenance,
  dependency footprint, integration with the Corvis pipeline and its WebSocket hardening), a recommendation, the
  protocol details the official client relies on, the design of the differential test harness, and the risks.
- Spec deltas stating the behavior the epic's features must meet, each verifiable by a differential test against
  Node `socket.io` 4.8.x / `engine.io` 6.6.x driven by the official `socket.io-client` 4.8.x.
- `tasks.md`: the concrete plan for the follow-up issues, one task group per issue (#32-#38), in dependency order,
  starting with the harness skeleton of #37 so every feature is verified against Node from its first test
  (design.md, Decision 7).
- Archiving this spike does not apply its spec deltas: each follow-up copies its own delta and applies it when its
  code meets it (design.md, Decision 6).

Recommendation recorded in design.md (Decision 1): implement Engine.IO v4 and Socket.IO v5 in Corvis, reusing the
hardened queue, writer and handshake code of `crates/corvis/src/ws.rs` and the gateway pipeline of
`crates/corvis/src/websockets.rs`, behind a public API that does not expose the implementation, and verify it with the
official protocol test suites plus a Corvis-vs-Node differential harness.

## Capabilities

### New Capabilities

- `websockets/engine-io`: Engine.IO v4 server (handshake, HTTP long-polling, WebSocket upgrade, heartbeat, framing,
  limits, error responses) observably equal to Node `engine.io` 6.6.x (#32).
- `websockets/socket-io`: Socket.IO v5 namespaces, events, acknowledgements, binary attachments, connect errors and
  disconnect reasons equal to Node `socket.io` 4.8.x (#33).
- `websockets/socket-io-rooms`: rooms and broadcast operators (`join`, `leave`, `to`, `in`, `except`, `broadcast`)
  equal to the Node in-memory adapter (#34).
- `websockets/socket-io-gateways`: `#[websocket_gateway]` gateways served over socket.io with namespace, path and CORS
  options, handler return values mapped to acks, `WsException`, and guards, pipes and interceptors (#35).
- `websockets/socket-io-cluster`: a Redis adapter wire-compatible with `@socket.io/redis-adapter` 8.x so Corvis and
  Node instances can share broadcasts (#36).
- `websockets/socket-io-parity`: the differential harness and conformance suites every socket.io requirement is
  verified with (#37).

### Modified Capabilities

None. The `ws`-style gateway protocol stays as it is and remains available.

## Impact

- This change: only files under `openspec/changes/e2-spike-engine-io-v4-socket-io-v5-in-rust/`.
- Follow-ups (planned in tasks.md): a new `socket_io` module in `crates/corvis` behind a new Cargo feature; changes to
  the request entry point (`crates/corvis/src/server.rs:322`) so Engine.IO polling and upgrades reach it; the
  `#[websocket_gateway]` macro (`crates/corvis-macros/src/gateway.rs:13-39`) and `GatewayInfo`
  (`crates/corvis/src/discovery.rs:114-122`) gain separate `path`/`namespace`/`cors` options; an optional Redis
  client dependency; a Node-based harness directory with a pinned `package-lock.json`; a CI job with Node; docs pages
  in English and Spanish; the Tertulia example.
- The three port apps do not use socket.io (no `socket.io` dependency in their `package.json`; `facten-api` uses the
  plain `ws` package, served by `corvis::ws`), so apidiff parity is only at risk where follow-ups touch shared code in
  `ws.rs`, `websockets.rs` or `server.rs`.

## Out of scope

- Any runtime code; each follow-up issue implements its part through its own change.
- Engine.IO v3 / Socket.IO v4 (`allowEIO3`, clients older than 3.0): a possible later issue.
- Connection state recovery (`connectionStateRecovery`, `pid`/`offset`): a possible later issue; the Redis adapter
  does not support it either.
- The msgpack parser (`socket.io-msgpack-parser`), `perMessageDeflate` and HTTP compression of polling responses:
  possible later issues.
- The sharded Redis adapter (`createShardedAdapter`), the Redis emitter, and the MongoDB/Postgres adapters.
- `serverSideEmit`, `fetchSockets`/`socketsJoin` across instances beyond what #36 lists, and the admin UI.
- Upgrading `tokio-tungstenite` 0.24 used by `ws.rs` (would be its own maintenance issue).

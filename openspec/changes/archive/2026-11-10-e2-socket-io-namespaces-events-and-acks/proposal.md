# Proposal

Implements [randygil/corvis#33](https://github.com/randygil/corvis/issues/33) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: design.md Decisions 2, 3 and 5 with
its Review notes, delta `specs/websockets/socket-io`, tasks.md group 4). Builds on #32
(`openspec/changes/e2-engine-io-handshake-polling-upgrade-and-heartbeat/`, approved and being implemented in another
worktree: its Engine.IO API is used here **as designed there**, design.md Decision 2 and Review notes, not as merged
code) and on the harness of #131
(`openspec/changes/archive/2026-11-09-e2-socket-io-differential-harness-skeleton/`).

## Why

After #32 Corvis speaks Engine.IO v4, but the official `socket.io-client` still cannot use it: the client sends a
Socket.IO CONNECT packet (`40`) right after the Engine.IO handshake and waits for `40{"sid":...}`, then exchanges
EVENT / ACK / BINARY_EVENT packets per namespace. Nothing in Corvis decodes those packets: #32 only delivers opaque
Engine.IO messages through its hidden `engine` module, and the existing gateways speak a different protocol, JSON
`{ event, data }` frames (`crates/corvis/src/websockets.rs:1-4`), with binary frames read as lossy UTF-8
(`websockets.rs:1051`).

This change adds the Socket.IO v5 layer: the packet codec with binary attachments, namespaces multiplexed over one
Engine.IO session, the connection handshake with `auth`, connect middleware, events and acknowledgements in both
directions, and the disconnect reasons, behind a low-level API (`IoServer`, `IoNamespace`, `IoSocket`, `AckSender`)
that #34 (rooms), #35 (gateways) and #36 (Redis) build on. Everything a client can make the server hold gets a
bounded default.

## What Changes

- New files under `crates/corvis/src/socket_io/`, all behind the opt-in `socket-io` feature added by #32:
  `parser.rs` (Socket.IO v5 packet codec, equal to `socket.io-parser` 4.2.7, the version the harness lockfile
  resolves), `value.rs` (`IoValue`, a JSON tree with binary leaves, and `Binary`), `options.rs` (`IoOptions`),
  `server.rs` (`IoServer`), `client.rs` (one Engine.IO connection: decoder state, joined namespaces, connect timeout,
  in-flight bound), `namespace.rs` (`IoNamespace`, middleware), `socket.rs` (`IoSocket`, `AckSender`, pending
  acknowledgements, disconnect), `fuzz_parser.rs` (proptest properties).
- Public low-level API mirroring the Node server without naming implementation types: `IoServer::new(IoOptions)`,
  `attach(&HttpServer)`, `of(name)`, `close()`; `IoNamespace::use_middleware`, `on_connection`, `sockets()`,
  `socket(id)`; `IoSocket::{id, handshake, data, set_data, insert, get, on, off, emit, timeout, emit_with_ack,
  on_disconnecting, on_disconnect, disconnect, connected}`; `AckSender::send`; `ConnectError`, `DisconnectReason`,
  `EmitError`, `AckError`.
- Wire behavior equal to Node `socket.io` 4.8.4 for the official client, verified by the differential harness: CONNECT
  with `auth` answered `{"sid"}` (a socket id that is not the Engine.IO sid), several namespaces on one session,
  `Invalid namespace`, `connectTimeout` 45 s, middleware errors as CONNECT_ERROR `{message, data}`, events with any
  number of arguments, acknowledgements both ways with `operation has timed out`, BINARY_EVENT / BINARY_ACK with
  placeholders over polling and WebSocket, reserved event names, the disconnect reasons and `disconnect(close)`.
- Bounded defaults, documented, each with a regression test and, where a client can observe a difference from Node,
  an entry in `parity/socket-io/expected-differences.json`: packet size (the Engine.IO `max_http_buffer_size`,
  1,000,000 bytes, per string packet and per attachment), 10 attachments per binary packet (as Node), reassembly
  bytes counted against the session's `max_unread_bytes`, JSON nesting depth 128, one socket per namespace per
  session including connections still in middleware, client acknowledgement ids up to 2^53 - 1, 1,000 pending
  server-side acknowledgements per socket, a default acknowledgement timeout of 30 s, 32 handlers in flight per
  connection with read backpressure, namespace names validated when registered. The full table is design.md
  Decision 9.
- Small additions to the hidden Engine.IO module of #32 (design.md Decision 3): a send that queues the frames of one
  packet together, received bytes that stay accounted until the Socket.IO layer releases them, and a hook that runs
  before the engine closes its sessions.
- Harness: the reference server becomes the scenario app (namespaces, middleware, events, acks, binary, disconnects,
  a loopback-only control route for server-side observations); the Corvis target gains the `socket-io` capability;
  new client and wire scenarios; the upstream `socket.io-protocol` suite runs against Corvis.
- Docs (EN + ES): the "Socket.IO (preview)" section of `docs/websockets/adapter.md` gains the low-level API, the
  limits table and the divergences.

## Capabilities

### New Capabilities

- `ws/socket-io`: the Socket.IO v5 layer on `ws/engine-io`: packet encoding, namespaces and connection, connect
  middleware, events, acknowledgements, binary attachments, disconnect reasons, limits. (The spike named it
  `websockets/socket-io`; the main spec tree groups WebSocket capabilities under `ws/`, as accepted in #32's Review
  note 5.)

### Modified Capabilities

None. `ws/gateways`, `ws/engine-io` and `ws/socket-io-parity` keep their requirements; the harness additions follow
the rules `ws/socket-io-parity` already states.

## Impact

- New: the files listed above, `crates/corvis/tests/socket_io.rs`, `crates/corvis/tests/socket_io_security.rs`,
  `crates/corvis/tests/socket_io_conformance.rs`, `parity/socket-io/scenarios/socket-io/**`,
  `parity/socket-io/scripts/record-parser.mjs`.
- Edited: `crates/corvis/src/socket_io/mod.rs` (modules and re-exports), `crates/corvis/src/socket_io/engine/`
  (the three additions above), `crates/corvis/tests/socket_io_diff.rs` (Corvis scenario app, `socket-io` capability),
  `parity/socket-io/reference/socket-server.mjs`, `parity/socket-io/lib/targets.mjs` (server arguments),
  `parity/socket-io/expected-differences.json`, `docs/websockets/adapter.md`, `docs/es/websockets/adapter.md`,
  `CONTRIBUTING.md` (harness section: the new scenarios and the conformance test).
- No new dependency: `serde`, `serde_json` (with `preserve_order`, workspace `Cargo.toml:31`), `bytes`, `futures`,
  `async-trait`, `base64` and `rand` are already used by `crates/corvis`.
- Public API: new items under `corvis::socket_io`. Nothing existing changes; the `ws`-style gateways are untouched.
- Port apps do not use socket.io and the code is behind a feature they do not enable; the corvis-parity check is still
  run because the definition of done asks for it when framework runtime code changes.

## Out of scope

- Rooms, `to` / `except` / `broadcast`, `IoNamespace::emit` to every socket, `fetch_sockets`, the adapter trait: #34.
- Gateways over socket.io (`#[websocket_gateway]`, `#[subscribe_message]`, guards, pipes, interceptors, exception
  filters, `WsResponse` mapping, route posture): #35. Redis adapter: #36. Full scenario catalog and the CI job: #37.
  User guide and example application: #38 (this change only extends the preview section and writes rustdoc examples).
- Dynamic namespaces (`of` with a regular expression or a function) and `cleanupEmptyChildNamespaces`: possible
  later issue; every namespace is registered by the application, which is what bounds namespaces per session.
- Connection state recovery (`pid`, `offset`), `volatile` and `compress` flags, catch-all listeners (`onAny`,
  `onAnyOutgoing`), per-socket middleware (`socket.use`), `serverSideEmit`: possible later issues.
- Custom parsers (`socket.io-msgpack-parser`), Engine.IO v3 clients (`allowEIO3`), serving the client bundle
  (`serveClient`, `GET /socket.io/socket.io.js`): not planned; documented.
- A supported public Engine.IO API: the `engine` module stays `#[doc(hidden)]` (design.md Decision 3 answers #32's
  Review note 1).

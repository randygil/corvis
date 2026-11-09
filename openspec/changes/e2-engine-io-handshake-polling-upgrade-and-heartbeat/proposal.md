# Proposal

Implements [randygil/corvis#32](https://github.com/randygil/corvis/issues/32) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`, design.md with its Review notes,
delta `specs/websockets/engine-io`, tasks.md group 3). Depends on #131 (`e2-socket-io-differential-harness-skeleton`),
whose harness verifies every requirement here. The issue and `ROADMAP.md` name the change
`e2-engine-io-handshake-polling-upgrade-and-heartbeat`; this change uses `e2-engine-io-handshake-polling-upgrade-and-heartbeat`,
the name it was assigned when drafted.

## Why

The official `socket.io-client` cannot talk to Corvis: Corvis gateways speak RFC 6455 frames carrying JSON
`{ event, data }` (`crates/corvis/src/websockets.rs:1-4`), and an Engine.IO long-polling request
(`GET /socket.io/?EIO=4&transport=polling`) is a plain HTTP request that reaches the router and gets a 404
(`handle`, `crates/corvis/src/server.rs:551-614`, only diverts WebSocket upgrades to gateways, at `:558-563`).
Engine.IO v4 is the transport layer under Socket.IO v5: handshake, HTTP long-polling, upgrade to WebSocket, heartbeat
and packet framing. #33 (namespaces, events, acks) builds on it.

The spike decided to implement it in Corvis on top of the hardened WebSocket code rather than depend on
`socketioxide` (Decision 1), behind an opt-in `socket-io` feature (Decision 2, Review note 1), and to keep Corvis'
protections where Node keeps unbounded state (Decision 5). This change also gives every Engine.IO limit a bounded
default, including the ones the spike left off by default.

## What Changes

- New opt-in Cargo feature `socket-io` (depends on `websockets`, off by default) and module
  `crates/corvis/src/socket_io/engine/`.
- An Engine.IO v4 server, attached to the application's HTTP server like Node's `engine.attach(httpServer)`, that
  takes every request under its path (default `/socket.io/`) before the router, the `ws`-style gateways and the raw
  `upgrade` listeners: handshake, long-polling `GET`/`POST`, CORS preflight, WebSocket-first connections and upgrades
  from polling. Other paths behave exactly as before.
- Wire behavior equal to Node `engine.io` 6.6.11, verified by the #131 harness: open packet and defaults, error codes
  0-5 with their status and JSON body, payload framing (`\x1e`, `b` + base64), `200 ok` answers, `cache-control:
  no-store`, overlapping polls, `413` above `maxHttpBufferSize`, the probe upgrade (`2probe`/`3probe`, noop `6`,
  `5`), the v4 heartbeat (server pings, `ping timeout`), close packets and server close.
- Bounded defaults for every limit, documented, with a regression test each and an entry in the harness's
  `expected-differences.json` where Node has no bound: `maxHttpBufferSize` 1,000,000 bytes (as Node), at most 10,000
  packets per polling `POST`, polling responses capped at `maxHttpBufferSize` bytes, one pending `GET` and one
  pending `POST` per session (as Node), heartbeat 25 s / 20 s (as Node), upgrade timeout 10 s and one upgrade at a
  time (as Node), an 8 MiB send buffer per session, 16 MiB of received bytes waiting for the application per session,
  at most 10,000 open sessions per Engine.IO server, 120-bit random session ids checked for shape before lookup, no
  CORS headers unless configured (as Node), and option values validated when the server is attached.
- Reuse of the existing hardening instead of copies: the byte-bounded `OutQueue` and `write_loop`
  (`crates/corvis/src/ws.rs:77-221`, made generic over the queued item), `guarded` and `protocol_error`
  (`ws.rs:224-237`, `:960-967`), the RFC 6455 checks of `validate_handshake` (`websockets.rs:695-722`, made independent
  of `GatewayOptions`), `Request::for_upgrade` for the handshake request so `ip()` follows `set_trust_proxy`
  (`crates/corvis/src/http/request.rs:65-78`), the server's write deadline and read credit for upgraded connections
  (`server.rs:1245-1330`, `:1839-1879`), and `CorsOptions` with its startup warning (`router/cors.rs:93-118`,
  `:189-194`).
- Engine.IO sessions are closed when the server shuts down (`serve`, `server.rs:2273-2345`) and on
  `CorvisApplication::close` (`application.rs:709`), so pending polls are answered instead of holding the 10-second
  drain.
- The harness gains the Engine.IO reference mode, the Engine.IO wire scenarios and the Corvis target with the
  `engine-io` capability; the upstream `engine.io-protocol` suite runs against Corvis.
- CI: the `socket-io` feature joins the per-feature clippy loop, the Linux test job and the fuzz job.
- Docs (EN + ES): the `socket-io` feature as a preview in `docs/websockets/adapter.md`, with the Engine.IO limits and
  the divergences from Node.

## Capabilities

### New Capabilities

- `ws/engine-io`: the Engine.IO v4 server: path, handshake, session ids, errors, long-polling, CORS, upgrade,
  WebSocket transport, heartbeat, backpressure, limits, close and shutdown. (The spike named it
  `websockets/engine-io`; the main spec tree groups WebSocket capabilities under `ws/`, as `ws/gateways` and
  `ws/write-deadline`.)

### Modified Capabilities

None. The `ws`-style gateways, `corvis::ws` and the HTTP server keep their specified behavior; the refactors of
`ws.rs` and `websockets.rs` change no observable behavior.

## Impact

- New: `crates/corvis/src/socket_io/` (`mod.rs`, `engine/` with packet codec, options, session ids, session store,
  polling and WebSocket transports, upgrade, heartbeat, server, fuzz properties), `crates/corvis/tests/engine_io.rs`,
  `crates/corvis/tests/engine_io_security.rs`, `crates/corvis/tests/engine_io_conformance.rs`.
- Edited: `crates/corvis/Cargo.toml` (feature), `crates/corvis/src/lib.rs` (module), `ws.rs` (`OutQueue<M>`,
  `write_loop` generic), `websockets.rs` (handshake checks taking the origin list), `server.rs` (`handle` entry,
  shutdown), `http_adapter.rs` (attached Engine.IO servers on `HttpServer`), `application.rs` (`close`),
  `crates/corvis/tests/socket_io_diff.rs` (Corvis target), `parity/socket-io/**` (engine mode, wire scenarios, expected
  differences), `.github/workflows/ci.yml`, `docs/websockets/adapter.md`, `docs/es/websockets/adapter.md`.
- No new dependency (`base64`, `rand`, `tokio-tungstenite` 0.24 and `hyper` are already used).
- Public API: `corvis::socket_io::EngineOptions` (documented; #33 embeds it in the Socket.IO server options). The
  Engine.IO server handle is `#[doc(hidden)]` and unstable until #33 (design.md Decision 2).
- Port apps do not use socket.io, but shared runtime code changes (`ws.rs`, `websockets.rs`, `server.rs`): the
  corvis-parity check is required.

## Out of scope

- Socket.IO packets, namespaces, events, acks, connect middleware and disconnect reasons such as
  `server shutting down`: #33. Rooms: #34. Gateways over socket.io and route posture for them: #35. Redis: #36.
- Engine.IO v3 / `allowEIO3`: rejected with code 5, as Node's default.
- HTTP compression of polling responses (`httpCompression`, on in Node above 1 KiB) and `perMessageDeflate`: possible
  later issue; listed as an expected difference.
- The `cookie` option, `allowRequest`, `initialPacket`, custom `generateId`, WebTransport: possible later issues
  (`allowRequest`-style handshake checks are covered by #33's connect middleware).
- Closing `ws`-style gateway connections on shutdown (they are not closed today either): possible later issue.
- Binding a session to the client address that opened it: not done by Node; see design.md Open question 3.
- Rate limiting handshakes per client address: the throttler's job, not Engine.IO's.
- A supported public Engine.IO API for applications (Node's standalone `engine.io`): see design.md Open question 1.

# Proposal

Implements [randygil/corvis#35](https://github.com/randygil/corvis/issues/35) (epic
[#30](https://github.com/randygil/corvis/issues/30)). Design source: the Socket.IO spike #31
(`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`: design.md Decision 3 and Review
notes 1, 5, 6 and 8, delta `specs/websockets/socket-io-gateways`, tasks.md group 6). Builds on three approved changes
that are not merged yet, whose APIs are referred to **as designed there, including their Review notes**:
#32 (`openspec/changes/e2-engine-io-handshake-polling-upgrade-and-heartbeat/`, being finished in another worktree with
its security fixes: `max_sessions_per_ip`, the `allow_request` hook, `first_request_timeout`, `max_total_unread_bytes`),
#33 (`openspec/changes/e2-socket-io-namespaces-events-and-acks/`) and #34
(`openspec/changes/e2-socket-io-rooms-and-broadcast/`). The issue and `ROADMAP.md` name the change
`e2-ioadapter-for-gateways`; this change uses `e2-socket-io-gateway-adapter`, the name it was assigned when drafted
(design.md Open question 1).

## Why

After #33 and #34 Corvis has a Socket.IO server (`IoServer`, `IoNamespace`, `IoSocket`, rooms and broadcast), but
gateways cannot use it: `#[websocket_gateway]` is always served with the `ws`-style protocol, JSON `{ event, data }`
frames on one URL path (`crates/corvis/src/websockets.rs:1-4`, `match_upgrade` at `:671-678`). Applications built on
the original model declare gateways and choose the Socket.IO platform adapter for the whole application; their
browser clients are the official `socket.io-client`. To migrate such a backend, the same gateway code (message
handlers, guards, pipes, interceptors, exception filters, lifecycle hooks) has to be reachable by that client, and what
the client observes (acknowledgements, emitted events, `exception` events, connection and disconnection) has to match
the same gateway written for the original framework with its Socket.IO adapter.

## What Changes

- `CorvisApplication::use_websocket_adapter(adapter)`: the transport of every gateway of an application is chosen
  once, before `init()`. The default stays the current `ws`-style adapter, so existing gateways, their tests and the
  port apps do not change. `corvis::socket_io::IoAdapter` (feature `socket-io`, opt-in) selects Socket.IO and carries
  the server options (`IoOptions`, one per Engine.IO path), CORS, the origin allowlist and the opt-in
  `guard_connections` switch.
- `#[websocket_gateway]` keeps its forms and gains meaning under Socket.IO: `namespace = "/chat"` (or the positional
  literal) is the Socket.IO namespace (default `/`), `path = "/realtime"` is the Engine.IO path (default
  `/socket.io`), `cors = true | "origin" | ["origin", ...]` declares CORS for that path. Under the `ws` adapter
  `namespace`, `path` and the literal keep their current meaning (the URL path). An unknown option becomes a compile
  error instead of being ignored. Gateways that share an Engine.IO path share one server; conflicting declarations
  fail `init()`.
- Message handlers run as Socket.IO event handlers through the existing pipeline (`run_handler`: guards,
  interceptors, pipes, handler, exception filters). The body is built from the event arguments as the original adapter
  does; a `WsResponse` (or a returned object with an `event` field) is emitted as that event; any other non-null
  value answers the client's acknowledgement when it sent one and is dropped otherwise; a failure is emitted as the
  `exception` event and the acknowledgement stays unanswered.
- Handles for Socket.IO gateways (spike Review note 5): `#[connected_socket] socket: IoSocket`,
  `#[websocket_server] server: IoGatewayServer` (the broadcast operators of the gateway's namespace, bound when the
  gateway starts), `#[ip]`, and `ExecutionContext::switch_to_ws().get_client::<IoSocket>()` in guards. The lifecycle
  traits become generic over the handle (`OnGatewayInit<IoNamespace>`, `OnGatewayConnection<IoSocket>`,
  `OnGatewayDisconnect<IoSocket>`), with the current `WsServer` / `WsClient` as defaults so existing impls compile
  unchanged. A gateway whose handles do not match the application's adapter fails `init()` with a message naming the
  gateway, instead of a hook that silently never runs.
- Connection order with a security guarantee: `handle_connection` runs after the connect middleware accepted the
  socket and finishes before any event of that socket is dispatched; events received meanwhile wait, within the
  per-connection in-flight bound. With `IoAdapter::guard_connections(true)` the global and gateway-level guards also
  run when a socket connects to the namespace, and a denial answers `CONNECT_ERROR` instead of connecting (opt-in:
  the original runs guards on messages only).
- Route posture: Socket.IO gateway handlers are checked like today's; message exemptions match the namespace; the
  report lists each gateway's connection under "Not checked" with whether connection guards run.
- Harness: `parity/socket-io/gateway-reference/` (the original framework's packages pinned as test-only dependencies,
  spike Review note 8) with a reference gateway app, the same app written with Corvis gateways, and gateway scenarios
  driven by the official client.
- Docs (EN + ES): the gateway adapter in the "Socket.IO (preview)" section of `docs/websockets/adapter.md` and the
  per-adapter meaning of the `#[websocket_gateway]` options in `docs/websockets/gateways.md`.

## Capabilities

### New Capabilities

- `ws/socket-io-gateways`: gateways served over Socket.IO: adapter selection, gateway options, event dispatch through
  the gateway pipeline, result and exception mapping, handles, lifecycle hooks, connection guards and the limits of
  this layer. (The spike named it `websockets/socket-io-gateways`; the main spec tree uses `ws/`.)

### Modified Capabilities

- `security/route-posture`: one added requirement, the posture of gateways served over Socket.IO (handlers checked,
  exemptions by namespace, the connection reported as an unchecked surface).

`ws/gateways` and `ws/exception-filters` keep their requirements: with the default adapter nothing they describe
changes, and under Socket.IO the same pipeline and filter order apply (stated in `ws/socket-io-gateways`).
`ws/socket-io-parity` keeps its requirements too: the `gateways` capability and the second pinned package of the
harness follow its existing rules and are stated in `ws/socket-io-gateways`.

## Impact

- New: `crates/corvis/src/socket_io/gateway.rs` (`IoAdapter`, `IoGatewayServer`, the dispatcher),
  `crates/corvis/tests/socket_io_gateways.rs`, `crates/corvis/tests/socket_io_gateways_security.rs`,
  `parity/socket-io/gateway-reference/**`, `parity/socket-io/scenarios/gateways/**`.
- Edited: `crates/corvis/src/websockets.rs` (pipeline functions take the socket as a type-erased handle, lifecycle
  traits generic over the handle, adapter-aware `compile_gateways` / `start_gateways`),
  `crates/corvis/src/discovery.rs` (`GatewayInfo`, `WsEnhancers`), `crates/corvis/src/application.rs`
  (`use_websocket_adapter`, init and close), `crates/corvis/src/router/mod.rs` (`RouterConfig`, `AppShared`),
  `crates/corvis/src/posture.rs`, `crates/corvis/src/modules/throttler.rs` (`request_ip`),
  `crates/corvis/src/__private.rs` (`ws_ip_arg`), `crates/corvis/src/socket_io/{mod,namespace,client}.rs` (namespace
  dispatcher hook), `crates/corvis-macros/src/gateway.rs`, `crates/corvis-macros/src/handlers.rs`,
  `crates/corvis/tests/socket_io_diff.rs`, `parity/socket-io/lib/targets.mjs`, `parity/socket-io/run.mjs`,
  `parity/socket-io/expected-differences.json`, `docs/websockets/{adapter,gateways,guards}.md` and their
  `docs/es/` translations.
- No new Rust dependency. New Node packages only in the harness (`parity/socket-io/gateway-reference/`), never
  shipped.
- Public API: additions (`use_websocket_adapter`, `corvis::socket_io::{IoAdapter, IoGatewayServer}`, the type
  parameter of the three lifecycle traits with a default). One compile-time tightening: an unknown
  `#[websocket_gateway]` option is rejected.
- Behavior of existing applications: none without `use_websocket_adapter(IoAdapter::new())`. The corvis-parity check
  is run because `websockets.rs` and the gateway macro change.

## Out of scope

- The Redis adapter and mixed Node / Corvis clusters: #36 (a gateway application gets it through
  `IoOptions::adapter`, nothing gateway-specific).
- The full scenario catalog and the CI job of the differential harness: #37. User guide, migration notes and moving
  the Tertulia example to Socket.IO: #38.
- A public trait for third-party gateway adapters (`use_websocket_adapter` accepts the two built-in adapters through
  a sealed trait): possible later issue.
- A handler parameter for the acknowledgement function (the original's manual acknowledgement) and one for the raw
  argument list: possible later issue. Handlers that need several acknowledgement arguments, binary payloads or
  a deferred answer register a low-level handler with `IoSocket::on` in `handle_connection`.
- Binary values inside `#[message_body]` and binary return values: the pipeline carries JSON; a binary argument is
  seen as `null` (design.md Decision 6).
- Streams or several replies from one handler (the original accepts an observable): one reply per message, as today.
- A process-wide lookup like `WsServer::for_path` for Socket.IO namespaces: other providers inject the gateway.
- Dynamic namespaces, connection state recovery, Engine.IO v3: out of scope of the epic's earlier changes too.
- Running two adapters in one application (some gateways on `ws`, some on Socket.IO): not in the original; raw
  `corvis::ws` servers and `on_upgrade` listeners keep working next to Socket.IO gateways.
- Two gateways on the same path silently replacing each other under the `ws` adapter (`start_gateways`,
  `websockets.rs:634-644`, inserts by path): found while reading; possible new issue.

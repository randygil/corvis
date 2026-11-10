# Tasks

Start after #34 (`e2-socket-io-rooms-and-broadcast`) is merged into `dev` (and with it #32 and #33). New test files
start with `#![cfg(feature = "socket-io")]`. Run the Node-gated checks with the harness installed (`npm ci` in
`parity/socket-io/` and in `parity/socket-io/gateway-reference/`) and `CORVIS_NODE_DIFF=1`. Timing rule for every
test: absence of a reply is observed through a marker event, never by sleeping; timer logic runs on Tokio's paused
clock; no retries. Test servers bind to `127.0.0.1` only.

## 1. Reconcile and verify the reference

- [ ] 1.1 Compare design.md "The layers this builds on" with the code merged by #32, #33 and #34 (`IoOptions` /
  `EngineOptions` builders, `IoNamespace::on_connection` and `use_middleware`, in-flight accounting, deferred operator
  errors, `Handshake`); note renamed items in a new "Implementation notes" section of design.md; verify
  `cargo test -p corvis --features socket-io` passes before any edit
- [ ] 1.2 Create `parity/socket-io/gateway-reference/` (`package.json` with exact versions of the original
  framework's core, WebSocket and Socket.IO platform packages and their peers, `package-lock.json`, a README line
  saying it is test-only) and read the installed sources against the eight items of design.md "Reference behavior";
  record each as confirmed or corrected in Implementation notes, including whether `exception` payloads carry `cause`
  and what `cors: true` sends; verify with a throwaway script run against the reference app that items 4, 5 and 7
  hold, and delete the script
- [ ] 1.3 If 1.2 corrected an item, update the affected decision, spec scenario and task below before continuing;
  verify `openspec validate e2-socket-io-gateway-adapter --strict` passes without warnings

## 2. Adapter selection and macro options

- [ ] 2.1 `GatewayAdapter` (sealed), `WsAdapter`, `GatewayAdapterKind`, `RouterConfig` field and
  `CorvisApplication::use_websocket_adapter` with the warning after `init()` (`crates/corvis/src/application.rs`,
  `router/mod.rs`); verify a unit test for the default and for the late call, and that `cargo build -p corvis` and
  `cargo build -p corvis --no-default-features` have no warnings
- [ ] 2.2 `crates/corvis-macros/src/gateway.rs`: parse `namespace`, `path`, the literal and `cors` (`true`, a string,
  an array of strings) into `GatewayInfo.io` (`IoGatewayDecl`) while keeping `GatewayInfo.path` as computed today;
  validate the namespace literal; reject unknown options and `cors` values of another form; verify
  `cargo test -p corvis-macros` with tests for every row of the table in design.md Decision 2 and for the three
  scenarios of the spec requirement "Gateway options are checked at compile time"
- [ ] 2.3 `#[websocket_server]` through `GatewayServerField` (the field's own type), `#[connected_socket]` through
  `GatewaySocket` and `socket_from_any`, the socket kind recorded in `WsEnhancers`, `ws_ip_arg` accepting both
  handles; verify `cargo test -p corvis-macros`, and that `cargo test -p corvis --test websockets --test ws_enhancers`
  pass unmodified
- [ ] 2.4 Lifecycle traits generic over the handle with defaults (`OnGatewayInit<S = WsServer>`,
  `OnGatewayConnection<C = WsClient>`, `OnGatewayDisconnect<C = WsClient>`), `hooks::detect!` for both kinds,
  hook kinds recorded in `GatewayInfo`; verify the examples (`examples/tertulia`, `examples/cats-app`) and
  `cargo test -p corvis --test websockets --test ws_security --test auth_ws` build and pass unmodified

## 3. Socket.IO gateway runtime

- [ ] 3.1 Pipeline functions take the socket type-erased (`run_handler`, `call_interceptors`, `invoke_handler` in
  `crates/corvis/src/websockets.rs`); no behavior change; verify `cargo test -p corvis --test websockets --test
  ws_security --test ws_enhancers --test throttler_skip_log_ws` pass unmodified
- [ ] 3.2 `crates/corvis/src/socket_io/gateway.rs`: `IoAdapter` (`new`, `options`, `guard_connections`),
  `IoGatewayServer` (unbound and bound states, operator entry points, `namespace()`), the new variant
  `EmitError::NotStarted`, exported from `corvis::socket_io` and the prelude; verify unit tests: operators of an
  unbound handle return `EmitError::NotStarted` and send nothing, and `namespace()` is `None`
- [ ] 3.3 Startup: the adapter / gateway mismatch check in `compile_gateways` (socket parameter, server field, hooks,
  both directions), grouping by Engine.IO path, option and `cors` conflicts, one handler per event per namespace,
  creation of the `IoServer`s and namespaces, binding of `IoGatewayServer`, `after_init`, attach, the startup log
  line of each gateway, close on failure and on `CorvisApplication::close`; verify `tests/socket_io_gateways.rs` has
  one test per scenario of the spec requirements "Gateways that share an Engine.IO path share one server", "Ambiguous
  path configuration fails startup", "One handler per event per namespace", "A gateway written for the other adapter
  fails startup" and "The server handle is bound when the gateway starts"
- [ ] 3.4 Namespace dispatcher (crate-private hook on `IoNamespace`, per-socket `on` handlers first) and
  `dispatch_event`: body mapping of design.md Decision 6, the execution context, panic containment; verify one test
  per scenario of the spec requirements "Message handlers are Socket.IO events", "The message body is built from the
  event arguments" and "Events without a gateway handler"
- [ ] 3.5 Result mapping of design.md Decision 7 (every row of the table, interceptor-replaced results, reserved
  event in a `WsResponse` logged) and exceptions through `exception_data` (filters, empty filter body, `cause` as
  recorded in 1.2); verify one test per scenario of the spec requirements "Handler results map like the reference
  adapter" and "Exceptions are emitted as the exception event"
- [ ] 3.6 Connection gate and hooks of design.md Decision 8 (`handle_connection` in registration order, gate,
  discard on disconnect, panic disconnects, `handle_disconnect` after rooms were left); verify one test per scenario
  of the spec requirements "Lifecycle hooks under Socket.IO" and "Events wait for the connection hook" (the in-flight
  bound with a blocked hook measured as #33's tests do)
- [ ] 3.7 `guard_connections`: connect middleware running global and gateway-level guards with the synthetic
  `connect` handler target, `CONNECT_ERROR` mapping; verify one test per scenario of the spec requirements "Guards
  run per message unless connection guards are enabled" and "A denied connection is refused with CONNECT_ERROR"
- [ ] 3.8 `request_ip` of the throttler (`crates/corvis/src/modules/throttler.rs`) and `#[ip]` for `IoSocket`; verify
  a test with `set_trust_proxy` and a throttled Socket.IO handler

## 4. Route posture

- [ ] 4.1 `posture_gateways` for Socket.IO gateways (namespace as path), `UncheckedSurface::IoConnection` with its
  text and JSON form, report mode, the `connect` pseudo-event under `guard_connections`; verify unit tests in
  `crates/corvis/src/posture.rs` and integration tests for every scenario of the three requirements of the
  `security/route-posture` delta (the `connect` exemption included)
- [ ] 4.2 The two `init()` warnings of design.md Decision 11 (credentials CORS without `allowed_origins`, `cors`
  under the `ws` adapter); verify tests that capture the log for both scenarios of the spec requirement "Risky
  configurations are reported at startup"

## 5. Security tests

- [ ] 5.1 `crates/corvis/tests/socket_io_gateways_security.rs`: mismatch errors; early events (not handled early,
  discarded on disconnect, reads pause above `max_inflight`); connection guards (deny, error, panic, public gateway,
  off by default, denied socket receives no broadcast); undeclared namespace; session cap through the adapter
  options; panics in handlers, hooks, guards and filters never reach the client as text; verify
  `cargo test -p corvis --features socket-io --test socket_io_gateways_security`
- [ ] 5.2 Confirm the existing suites are untouched: `git diff --stat origin/dev -- crates/corvis/tests/websockets.rs
  crates/corvis/tests/ws_security.rs crates/corvis/tests/ws_enhancers.rs crates/corvis/tests/auth_ws.rs` is empty and
  they pass

## 6. Harness

- [ ] 6.1 Reference gateway app in `parity/socket-io/gateway-reference/` (plain JavaScript, decorators applied as
  functions) with the handlers, guard, pipe, interceptor, filter, hooks and control routes of design.md Decision 12;
  capability `gateways` in `lib/targets.mjs` and `run.mjs`, version check for the reference package; verify
  `node --test parity/socket-io/test` and a Node-only run of one gateway scenario
- [ ] 6.2 The same application with Corvis gateways, started in-process by
  `crates/corvis/tests/socket_io_diff.rs` with capabilities `engine-io,socket-io,gateways`; verify it starts on
  `127.0.0.1:0` and answers the control routes
- [ ] 6.3 Scenarios `gateways/ack`, `no-ack`, `events`, `body-mapping`, `exceptions`, `guard`, `pipe`, `interceptor`,
  `filter`; verify `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_diff` reports no
  unlisted and no stale difference
- [ ] 6.4 Scenarios `gateways/lifecycle`, `namespaces`, `server-emit`, `unknown-event`, `wire/cors`; verify no
  unlisted and no stale difference
- [ ] 6.5 Expected differences `gateways/http-exception` and `gateways/binary-body` with their entries in
  `parity/socket-io/expected-differences.json` (reason and `ws/socket-io-gateways` requirement); verify the listed
  differences occur and the full harness run is clean three times in a row

## 7. Docs and review

- [ ] 7.1 Rustdoc for every public item added (`use_websocket_adapter`, `IoAdapter` with each option and its default,
  `IoGatewayServer`, the generic lifecycle traits with one compiling example per adapter, the macro's option table);
  verify `cargo doc -p corvis --features socket-io --no-deps` without warnings and
  `cargo test -p corvis --features socket-io --doc`
- [ ] 7.2 Docs: gateways over Socket.IO in the "Socket.IO (preview)" section of `docs/websockets/adapter.md`
  (selecting the adapter, handles per adapter, body and result mapping, exceptions, hooks and the gate, the limits
  table of design.md Decision 11 and the divergences); the per-adapter option table in
  `docs/websockets/gateways.md`; connection checks and `guard_connections` in `docs/websockets/guards.md`
  ("Authenticating on connect", "Route posture for gateways"); translate the three in `docs/es/websockets/`; verify
  `npm run docs:check` in `docs/`
- [ ] 7.3 Security review of the change with the `security-reviewer` agent (adapter / gateway mismatch, the
  connection gate and early events, connection guards and their context, CORS versus the origin allowlist, what
  `exception` and `CONNECT_ERROR` payloads can leak, posture reporting, panics, the namespace dispatcher as an
  extension of #33); repeat rounds until no open finding; record findings and fixes in design.md under "Security
  review"; verify every finding has a regression test or a recorded reason

## 8. Definition of done

- [ ] 8.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features socket-io`
  with no warnings
- [ ] 8.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings`
- [ ] 8.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io` passes, and with
  `CORVIS_NODE_DIFF=1` the `socket_io_diff` and `socket_io_conformance` tests pass
- [ ] 8.4 Regression tests: one per spec scenario (macro, unit, integration, security or harness scenario), as listed
  in groups 2-6
- [ ] 8.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 8.6 Framework runtime code changed (`crates/corvis/src/websockets.rs`, the gateway macro): run the
  `corvis-parity` check and verify the three port apps keep 0 apidiff differences
- [ ] 8.7 The security review of 7.3 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/socket-io-gateways` and `security/route-posture` deltas are
  applied).
- Close issue #35, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).
- Open the follow-up issues noted in proposal.md "Out of scope" that the maintainer accepts (manual acknowledgement
  parameter, binary in the gateway pipeline, duplicate `ws` gateway paths).

# Tasks

This spike ships no runtime code. Group 1 closes the spike itself; groups 2-9 are the plan for the epic's follow-up
issues, in the order of design.md Decision 7. Each follow-up opens its own OpenSpec change (named in the issue), copies
its spec delta from `specs/websockets/` here, and refines these tasks in its own `tasks.md`.

## 1. This spike (#31)

- [x] 1.1 Maintainer review of design.md Decisions 1-7 and the Open questions; record the answers in design.md; verify
  no open question that blocks #37 or #32 (1, 2, 3, 7, 9) is left unanswered
- [x] 1.2 Run `openspec validate e2-spike-engine-io-v4-socket-io-v5-in-rust --strict`; verify it reports no errors
- [x] 1.3 If Open question 7 is answered "split", note the new issue numbers in design.md Decision 7 and in the groups
  below; verify the epic #30 task list matches

## 2. #37 Differential tests against Node socket.io, part 1: harness skeleton

- [ ] 2.1 Create `parity/socket-io/` with `package.json` and a committed `package-lock.json` pinning `socket.io` 4.8.4,
  `socket.io-client` 4.8.4, `engine.io-client` 6.6.x and `ws`; add `node_modules/` to `.gitignore`; verify `npm ci`
  works from a clean clone
- [ ] 2.2 Add the upstream `engine.io-protocol` and `socket.io-protocol` test suites pinned by commit (vendored or
  fetched, per Open question 2) with a wrapper that passes the server URL instead of `localhost:3000`; verify both
  suites pass against a small Node reference server bound to `127.0.0.1`
- [ ] 2.3 Write `reference-server.mjs` (Node scenario app: echo, acks, namespaces `/` and `/custom`, middleware
  errors, configurable ping intervals; port and options from arguments) and `wire-scenarios.mjs` for the Engine.IO
  requirements (handshake, error codes 0-5, polling framing, overlap, 413, upgrade probe/noop, heartbeat, shutdown);
  verify each scenario prints a JSON transcript against the Node server
- [ ] 2.4 Write the normalizer (ids, timings, interleavings), the comparer (readable diff, non-zero exit) and
  `expected-differences.json` handling (unlisted difference fails, stale entry fails); verify with a self-test that
  compares Node against Node (no difference) and against a deliberately altered transcript (one difference)
- [ ] 2.5 Add `crates/corvis/tests/socket_io_diff.rs`: skips with a notice unless `CORVIS_NODE_DIFF=1`, fails if set
  and Node or `node_modules` is missing, starts both servers on ephemeral loopback ports, runs the scenario files and
  the comparer; until #32 lands it runs Node-vs-Node only; verify `cargo test -p corvis --test socket_io_diff` skips
  without the variable and passes with it
- [ ] 2.6 Document how to run the harness in `CONTRIBUTING.md` (install, environment variables, reading a diff, adding
  a scenario, adding an expected difference); verify the commands run as written

## 3. #32 Engine.IO: handshake, polling, upgrade and heartbeat

- [ ] 3.1 Add the `socket-io` Cargo feature (depends on `websockets`, off by default) and the `socket_io::engine`
  module with the Engine.IO packet and payload codec (`\x1e` separator, `b` + base64 binary); unit and property tests
  for round trips and malformed input (no panic); verify `cargo test -p corvis --features socket-io --lib engine`
- [ ] 3.2 Session store and handshake: `EIO`/`transport`/`sid` validation, the error responses with codes 0-5, the open
  packet with Node's defaults, unguessable sids; route requests under the Engine.IO path in `handle`
  (`crates/corvis/src/server.rs:320-337`) before `match_upgrade`; verify the handshake and error wire scenarios have no
  difference
- [ ] 3.3 Long-polling transport: pending `GET` answered on flush, `POST` with the `maxHttpBufferSize` cap (413), the
  `200 ok` answer, overlap handling (400 and close), `cache-control` and CORS preflight; verify the polling wire
  scenarios have no difference
- [ ] 3.4 WebSocket transport reusing `OutQueue`/`write_loop` (`crates/corvis/src/ws.rs`) and the RFC 6455 checks of
  `validate_handshake` (`crates/corvis/src/websockets.rs:625-648`); WebSocket-first connections; verify with the
  WebSocket wire scenarios
- [ ] 3.5 Upgrade from polling (`2probe`/`3probe`, noop `6`, `5`, `upgradeTimeout`) without loss or reordering of
  buffered packets; verify the upgrade wire scenarios, including events emitted during the upgrade
- [ ] 3.6 Heartbeat (`pingInterval`/`pingTimeout`, reason `ping timeout`), client close, and graceful shutdown closing
  every session with `server shutting down` and answering pending polls; verify the heartbeat and shutdown scenarios
- [ ] 3.7 Security bounds of design.md Decision 5 (byte-bounded send buffer, in-flight bound, optional session cap) with
  regression tests in the style of `crates/corvis/tests/ws_security.rs` and entries in `expected-differences.json`;
  verify the slow-consumer scenario is reported as expected and memory stays bounded
- [ ] 3.8 Run the engine.io conformance suite against the Corvis test server (test-only bin, per Open question 9);
  verify every test passes

## 4. #33 Socket.IO: namespaces, events and acks

- [ ] 4.1 `socket_io::parser`: Socket.IO v5 packet codec with binary attachments (placeholders, reconstruction), parse
  errors as `parse error`; unit and property tests including round trips against byte strings captured from the Node
  parser by the harness; verify `cargo test -p corvis --features socket-io --lib parser`
- [ ] 4.2 Namespaces: CONNECT with `auth`, `{sid}` answer, socket id distinct from the session id, multiplexing,
  `Invalid namespace`, `connectTimeout`, connect middleware with `{message, data}` errors; verify the matching client
  scenarios have no difference
- [ ] 4.3 Events and acks in both directions (`emit`, `emit_with_ack` with timeout and `operation has timed out`),
  reserved event names, binary events over polling and WebSocket; verify the client scenarios
- [ ] 4.4 Disconnect reasons and `disconnect(close)` semantics, `disconnecting`/`disconnect` hooks; verify the
  disconnect scenarios
- [ ] 4.5 Public low-level API (`IoServer`, `IoSocket`, `Ack`) with rustdoc examples, no implementation types exposed;
  verify `cargo doc -p corvis --features socket-io --no-deps` builds without warnings
- [ ] 4.6 Run the socket.io conformance suite against the Corvis test server; verify every test passes

## 5. #34 Rooms and broadcast

- [ ] 5.1 Adapter trait (add, delete, broadcast, broadcast with acks, sockets, socket rooms, fetch sockets, add/remove
  sockets to rooms, disconnect sockets) and the in-memory adapter; each socket in its own id room; empty rooms deleted;
  unit tests; verify `cargo test -p corvis --features socket-io --lib adapter`
- [ ] 5.2 Broadcast operators `to`/`in`/`except` (several rooms, chaining), `socket.broadcast`/`socket.to` excluding
  the sender, broadcast with acks and timeout; verify the rooms client scenarios with several clients have no
  difference
- [ ] 5.3 Rooms during `disconnecting` and after `disconnect`, namespace scoping of rooms; verify the matching
  scenarios
- [ ] 5.4 Regression test: broadcasting to many slow clients keeps each one's send buffer bounded and does not block
  the emitter; verify with a test in the style of `ws_security.rs`

## 6. #35 IoAdapter for gateways

- [ ] 6.1 Application-level adapter selection (`use_websocket_adapter`-style API, `ws` stays the default); verify
  `crates/corvis/tests/websockets.rs` and `ws_security.rs` pass unmodified
- [ ] 6.2 `#[websocket_gateway]` options under socket.io (`namespace`, `path`, `cors`, Engine.IO options) in
  `crates/corvis-macros/src/gateway.rs` and `GatewayInfo` (`crates/corvis/src/discovery.rs:114-122`), keeping
  `namespace` as a synonym of `path` under `ws`; gateways sharing a path share one Engine.IO server; macro tests;
  verify `cargo test -p corvis-macros` and a gateway test with two namespaces
- [ ] 6.3 Dispatch socket.io events through `run_handler` (`websockets.rs:697-730`): `#[message_body]` is the first
  argument, `#[connected_socket]` per Open question 5, `WsResponse` emitted, other values to the ack, exceptions as
  `exception` with `exception_payload` (resolve Open question 6 first); verify gateway tests for each case
- [ ] 6.4 Lifecycle hooks (`on_init`, `on_connection`, `on_disconnect`) and `#[websocket_server]` injection under
  socket.io; verify with tests
- [ ] 6.5 Add `parity/socket-io/gateway-reference/` (per Open question 8) and a Corvis gateway app with the same
  gateway (guards, validation pipe, interceptor, `WsException`, CORS); verify the gateway scenarios have no difference

## 7. #36 Redis adapter for multiple instances

- [ ] 7.1 Add the `socket-io-redis` feature with the Redis client chosen in Open question 4; check `deny.toml` and
  `THIRD_PARTY_NOTICES.md`; verify `cargo deny check` passes
- [ ] 7.2 Encoders/decoders for the `@socket.io/redis-adapter` 8.3.x messages (msgpack broadcasts, JSON/msgpack
  requests and responses, channel names, uid) with tests against messages captured from the Node adapter; verify the
  unit tests
- [ ] 7.3 Redis adapter implementing the adapter trait: publish/subscribe, local delivery of remote broadcasts with
  rooms and except, cluster requests with `requestsTimeout`; verify mixed-cluster broadcast scenarios (one Node and one
  Corvis instance) have no difference with `CORVIS_REDIS_URL` set
- [ ] 7.4 Cluster operations (`fetch_sockets`, `sockets_join`/`sockets_leave`, `disconnect_sockets`, broadcast with
  acks across instances); verify the cluster scenarios
- [ ] 7.5 Reconnection and resubscription after a Redis restart, cluster requests failing with the timeout error
  meanwhile; verify with a test that restarts the Redis container or drops the connection

## 8. #37 Differential tests against Node socket.io, part 2: full catalogue and CI

- [ ] 8.1 Review every scenario of the `websockets/*` spec deltas against the harness and add the missing ones; verify
  each requirement names at least one scenario that exercises it
- [ ] 8.2 Add the `socket-io-diff` job to the CI workflow of #6 (Linux, Node 22, `npm ci`, Redis service,
  `CORVIS_NODE_DIFF=1`, `CORVIS_REDIS_URL`); verify it is green on a branch and fails on a deliberately broken scenario
- [ ] 8.3 Run the full harness ten times in a row; verify there are no flaky results (fix the cause, do not retry)

## 9. #38 Socket.IO docs and example

- [ ] 9.1 Rewrite `docs/websockets/adapter.md` (today says there is no socket.io adapter) and extend
  `docs/websockets/gateways.md` with the socket.io adapter, options per adapter (the different meaning of `namespace`),
  rooms, acks, binary, Redis adapter and the documented divergences; mirror in `docs/es/websockets/`; verify
  `npm run docs:check` in `docs/`
- [ ] 9.2 Move Tertulia's chat to socket.io (or add a new example, per the issue) with a browser client using the
  official `socket.io-client`, keeping its plain `corvis::ws` firehose; verify the example's tests and a manual run
  bound to `127.0.0.1`
- [ ] 9.3 A migration note: what changes for a Node app moving from `socket.io` (and its Redis adapter) to Corvis;
  verify every code sample in the page compiles as a doc test or is taken from the example

## 10. Definition of done

Applies to this spike as a check that no code changed, and to every follow-up change in full.

- [ ] 10.1 `cargo build --workspace --all-targets` with no warnings (also with `--features socket-io,socket-io-redis`
  once they exist)
- [ ] 10.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 10.3 `cargo test --workspace` passes, and `CORVIS_NODE_DIFF=1 cargo test -p corvis --test socket_io_diff`
  passes once #37 part 1 exists
- [ ] 10.4 Regression tests for every new behavior (unit, conformance suites and differential scenarios)
- [ ] 10.5 Docs pages updated in English (`docs/`) and Spanish (`docs/es/`), then `npm run docs:check` in `docs/`;
  for this spike, confirm no docs change is needed (#38 owns the docs)
- [ ] 10.6 When framework runtime code changes (`ws.rs`, `websockets.rs`, `server.rs`, macros), run the
  `corvis-parity` check and verify the three port apps keep 0 apidiff differences; for this spike, record that no
  runtime code changed

## Workflow follow-up

- Archive this change with `/opsx:archive` using `--skip-specs` (design.md Decision 6): the spec deltas are targets
  that each follow-up applies when its code meets them.
- Close issue #31, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).
- Each follow-up (#32-#38) is archived the same way with its own specs applied, then its issue is closed, ticked in
  epic #30 and its card moved to Done.

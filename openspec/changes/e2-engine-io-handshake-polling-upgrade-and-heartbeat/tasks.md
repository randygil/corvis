# Tasks

Start after #131 (`e2-socket-io-differential-harness-skeleton`) is archived. New test files start with
`#![cfg(feature = "socket-io")]`. Run the Node-gated checks with the harness installed (`npm ci` in
`parity/socket-io/`) and `CORVIS_NODE_DIFF=1`. Timing rule for every test: "not before" bounds use the configured
value, "within" bounds are generous, unit tests of timers use paused Tokio time, no retries.

## 1. Feature, options and codec

- [x] 1.1 Add the `socket-io` feature (`["websockets"]`, not default) to `crates/corvis/Cargo.toml`, the module skeleton
  of design.md Decision 1 and `EngineOptions` with the defaults of Decision 8 and validation (spec "Options are
  validated"); unit tests for every default and every rejected zero value; verify
  `cargo test -p corvis --features socket-io --lib socket_io` and `cargo build -p corvis` (feature off) with no warnings
- [x] 1.2 `packet.rs`: packet types 0-6, text and `b` + base64 binary, payload split on `\x1e` without allocating every
  packet first, packet counting with the `max_payload_packets` bound, lossy UTF-8 input; unit tests with byte strings
  taken from the Node reference (`engine.io-parser` output recorded by a harness script); verify the unit tests
- [x] 1.3 `fuzz.rs`: properties (no panic on any input, encode/decode round trip, decoded size proportional to input,
  count bound never exceeded); verify `cargo test -p corvis --features socket-io --lib fuzz` and with
  `PROPTEST_CASES=20000`
- [x] 1.4 `sid.rs` (design.md Decision 4): `OsRng` 15 bytes, base64url, shape check; unit tests for spec "Ids differ"
  and the shape check; verify the unit tests

## 2. Reuse refactors (no behavior change)

- [x] 2.1 Make `OutQueue` generic (`OutQueue<M: QueueItem = Message>`) and add the encoder variant of `write_loop` in
  `crates/corvis/src/ws.rs` (design.md Decision 3); verify `cargo test -p corvis --test ws_security --test websockets
  --test send_timeout_ws --test http_adapter` pass unmodified
- [x] 2.2 Split `validate_handshake` in `crates/corvis/src/websockets.rs` into the origin-list form plus the gateway
  wrapper; verify `tests/ws_security.rs` (`gateway_handshake_is_validated`) and `tests/tls_ws.rs` (with `--features
  tls`) pass unmodified

## 3. Server, entry point and handshake

- [x] 3.1 `EngineServer` with the session store and `attach(&HttpServer)` (engines list in `http_adapter.rs`, overlap
  check); the entry in `handle` (`crates/corvis/src/server.rs:551`) before the `websockets` block (design.md
  Decision 2); first tests in `crates/corvis/tests/engine_io.rs` for spec "Polling request is not routed" and "Other
  paths are unchanged"; verify them and `cargo test -p corvis --test websockets --test http_adapter`
- [x] 3.2 Handshake and errors: `EIO`/`transport`/`sid` validation in Node's order, duplicate parameters, codes 0-5
  with status and JSON body, open packet, handshake request through `Request::for_upgrade` with `ip()` resolved once;
  tests for "Polling handshake", "Oversized sid", "Address behind a trusted proxy" and each error code; verify
  `cargo test -p corvis --features socket-io --test engine_io`

## 4. Harness: Engine.IO mode and Corvis target

- [x] 4.1 `parity/socket-io/reference/engine-server.mjs` and the Corvis-side app (design.md Decision 10: echo,
  `flood:<bytes>`, `GET /harness/close-reason/<sid>`, options from arguments); `socket_io_diff.rs` starts the Corvis
  app in-process on `127.0.0.1:0` under `#[cfg(feature = "socket-io")]` and passes `--corvis-capabilities engine-io`;
  verify `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_diff` runs the #131 scenario
  on Node only and reports it skipped on Corvis
- [x] 4.2 Wire scenarios `scenarios/engine-io/handshake.mjs` and `errors.mjs` (spec "Polling handshake",
  "WebSocket-first handshake" once group 6 lands, "Error codes", "Duplicate and mismatched parameters", "Rejected
  upgrade"); verify no unlisted difference

## 5. Long-polling transport

- [x] 5.1 Pending `GET`, `POST` reading (cap, `413`, `request_timeout`, `drain`), `ok` answer, headers, overlap rules,
  octet-stream `400`, method handling on an existing session as recorded from Node (design.md Decision 5); tests and
  wire scenarios for "Several packets in one poll", "Overlapping polls", "Oversized POST"; verify the tests and no
  unlisted difference
- [x] 5.2 Batch bounds: `max_payload_packets` and the per-`GET` byte cap; tests for "Packet flood in one POST" and
  "Large backlog" in `crates/corvis/tests/engine_io_security.rs`; wire scenarios with entries in
  `expected-differences.json`; verify the tests and that the listed differences occur (none stale)
- [x] 5.3 CORS and `allowed_origins` (spec "CORS of the polling transport is off unless configured"): reuse of
  `CorsOptions`, the attach-time warning, `enable_cors` not applied to the engine path; tests and wire scenarios for
  "No CORS by default", "Configured origin", and a test for "Origin allowlist on WebSocket" after group 6; verify both

## 6. WebSocket transport and upgrade

- [x] 6.1 WebSocket transport (design.md Decisions 3 and 6): WebSocket-first sessions, outbox drained through the
  encoder `write_loop`, tungstenite limits from `max_http_buffer_size`, close deadline, `protocol_error` codes, unread
  gauge; tests for "WebSocket-first handshake", "Oversized WebSocket message" and "Peer that never reads"
  (`ServerOptions::send_timeout(1 s)`); verify the tests and the wire scenario
- [x] 6.2 Upgrade: probe sequence, noop every 100 ms, `5`, single outbox switch, one upgrade at a time,
  `upgrade_timeout`, non-probe packet on the candidate; unit tests of the upgrade states under paused time, tests for
  "Upgrade never completed" and "Second upgrade", wire scenario "Upgrade with queued packets" with the official
  `engine.io-client`; verify no message lost or reordered in 20 consecutive local runs of that scenario

## 7. Heartbeat, backpressure, bounds, close

- [x] 7.1 Heartbeat (design.md Decision 7): ping schedule, `ping timeout`, client ping as `transport error`; unit test
  "Timer logic" under paused time; integration test and wire scenario "Client stops answering pings" with 300/200 ms;
  verify all three
- [x] 7.2 `max_unread_bytes` (WebSocket read pause, `POST` held) and `max_buffered_amount` (send fails, outbox
  discarded, `transport error`); tests "Application that does not read" and "Slow consumer" measuring queue sizes;
  slow-consumer wire scenario with its expected difference; verify
- [x] 7.3 `max_sessions` (`503` before any state or upgrade, warning at most once per minute); test "Session cap" on
  both transports; verify
- [x] 7.4 Close and shutdown (design.md Decision 9): client close packet, `EngineServer::close`, call from `serve`
  after it stops accepting and from `CorvisApplication::close`; tests "Graceful shutdown with a pending poll" (answered
  within 5 s) and "Client close"; wire scenario for the client close; verify

## 8. Conformance, CI, docs

- [x] 8.1 `crates/corvis/tests/engine_io_conformance.rs` (design.md Decision 10) running the upstream
  `engine.io-protocol` suite fetched by #131's script; verify every test passes with `CORVIS_NODE_DIFF=1`, and that the
  same suite passes against `reference/engine-server.mjs` with the suite's options
- [x] 8.2 Review `expected-differences.json`: one entry per observable bound of design.md Decision 8 plus compression,
  each with reason and `ws/engine-io` requirement; verify the full harness run has no unlisted and no stale difference
- [x] 8.3 `.github/workflows/ci.yml`: `socket-io` in the per-feature clippy loop, `cargo test -p corvis --features
  socket-io --no-fail-fast` in the Linux test job, `cargo test -p corvis --features socket-io --lib fuzz` in the fuzz
  job; update the CI table and the Fuzzing section of `CONTRIBUTING.md`; verify the commands locally
- [x] 8.4 Docs: rustdoc of `EngineOptions` (every option, default, Node's value, divergence); a "Socket.IO (preview)"
  section in `docs/websockets/adapter.md` (feature flag, not yet usable from gateways, the bounds table of design.md
  Decision 8, `forced close` on shutdown, engine path takes precedence, no WebSocket over HTTP/2) and its translation
  in `docs/es/websockets/adapter.md`; verify `npm run docs:check` in `docs/` and
  `cargo doc -p corvis --features socket-io --no-deps` without warnings

## 9. Definition of done

- [x] 9.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features socket-io`
  with no warnings
- [x] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings`
- [x] 9.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io` passes, and with
  `CORVIS_NODE_DIFF=1` the `socket_io_diff` and `engine_io_conformance` tests pass
- [x] 9.4 Regression tests: one per spec scenario (unit, integration, security or wire), as listed in groups 1-8
- [x] 9.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 9.6 Framework runtime code changed (`ws.rs`, `websockets.rs`, `server.rs`, `http_adapter.rs`,
  `application.rs`): run the `corvis-parity` check and verify the three port apps keep 0 apidiff differences
- [ ] 9.7 Security review of the change (`security-reviewer`) before archiving, with its findings fixed or recorded in
  design.md

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/engine-io` delta is applied).
- Close issue #32, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).

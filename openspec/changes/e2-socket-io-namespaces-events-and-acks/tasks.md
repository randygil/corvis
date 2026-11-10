# Tasks

Start after #32 (`e2-engine-io-handshake-polling-upgrade-and-heartbeat`) is merged into `dev`. New test files start
with `#![cfg(feature = "socket-io")]`. Run the Node-gated checks with the harness installed (`npm ci` in
`parity/socket-io/`) and `CORVIS_NODE_DIFF=1`. Timing rule for every test (#32 Review note 7): "not before" bounds use
the configured value, "within" bounds are generous, timer logic runs on Tokio's paused clock, no retries.

## 1. Engine.IO additions, options and values

- [ ] 1.1 Reconcile design.md Decision 3 with the Engine.IO module as merged by #32 and add what is missing in
  `crates/corvis/src/socket_io/engine/`: `EngineSocket::send_all` (group queued whole or refused whole), messages that
  keep their unread accounting until released, `EngineServer::on_closing`; unit tests for each (a group is never
  interleaved by 32 concurrent senders; the gauge falls only on release; the hook runs before sessions close); verify
  `cargo test -p corvis --features socket-io --lib socket_io::engine` and that `tests/engine_io.rs` and
  `tests/engine_io_security.rs` pass unmodified
- [ ] 1.2 `options.rs`: `IoOptions` (embedding `EngineOptions`) with the defaults of design.md Decision 9 and the
  validation of spec "Options are validated" and "Inconsistent options"; unit tests for every default and every
  rejected value; verify `cargo test -p corvis --features socket-io --lib socket_io::options` and
  `cargo build -p corvis` (feature off) with no warnings
- [ ] 1.3 `value.rs`: `IoValue`, `Binary`, `Args`, `IntoArgs`, conversions from and to `serde_json::Value`, the
  serializer and deserializer bridge of design.md Decision 2, depth bound 128; unit tests (struct with a `Binary`
  field both ways, tuples as arguments, insertion order kept, depth 129 rejected without recursion beyond the bound);
  verify the unit tests

## 2. Packet codec

- [ ] 2.1 `parity/socket-io/scripts/record-parser.mjs`: prints, as JSON, the output of `socket.io-parser`'s `Encoder`
  for a fixed list of packets (every type, custom namespace, ack ids, nested binary, several attachments, strings with
  control characters and `U+001E`) and the `Decoder`'s result or error message for a fixed list of inputs, with the
  parser version; commit its output as `crates/corvis/src/socket_io/fixtures/parser.json`; verify it runs with
  `node scripts/record-parser.mjs` after `npm ci` and that the version in the file is 4.2.7
- [ ] 2.2 `parser.rs`: `Packet`, `encode`, the streaming `Decoder` with the checks and divergences of design.md
  Decision 4 (`max_attachments`, ack id range, depth, no allocation from the declared count, placeholders sharing one
  `Bytes`); unit tests replaying the fixture of 2.1 in both directions and one per decode error; verify
  `cargo test -p corvis --features socket-io --lib socket_io::parser`
- [ ] 2.3 `fuzz_parser.rs`: properties (round trip; arbitrary text and arbitrary text/binary interleavings never
  panic; decoded size proportional to input; attachment count never above the limit; encoded text never contains
  `U+001E`); verify `cargo test -p corvis --features socket-io --lib fuzz` and with `PROPTEST_CASES=20000`

## 3. Server, namespaces and connection

- [ ] 3.1 `server.rs`, `namespace.rs`, `client.rs`: `IoServer::new` / `attach` / `of` (name validation, `/` always
  present), the `Client` task per Engine.IO connection, CONNECT handling (`{"sid"}` answer with an id from
  `engine/sid.rs`, `Handshake`, `Invalid namespace`), `IoNamespace::on_connection`, `sockets()`, `socket(id)`; first
  tests in `crates/corvis/tests/socket_io.rs` for spec "Auth is visible to the server", "Unknown namespace", "Two
  namespaces on one session", "Ids differ", "Name with a comma"; verify
  `cargo test -p corvis --features socket-io --test socket_io`
- [ ] 3.2 Connect middleware (design.md Decision 6): order, `ConnectError` with and without data, panic containment,
  socket not usable for emits before it connects, pending socket dropped when the connection closes; tests for
  "Middleware rejects with data", "Middleware rejects without data", "Middleware panics"; verify the tests
- [ ] 3.3 One socket per namespace per connection including pending ones, protocol violations and `connect_timeout`
  (design.md Decisions 5 and 8); tests for "Duplicate CONNECT", "CONNECT flood during slow middleware" (middleware
  runs once), "Event before CONNECT", and "No namespace joined in time" on the paused clock; verify the tests

## 4. Harness: scenario app and Corvis target

- [ ] 4.1 Extend `parity/socket-io/reference/socket-server.mjs` into the scenario app of design.md Decision 10
  (arguments, namespaces, middleware, handlers, server-side log, `GET /harness/log`, `POST /harness/reset`,
  `serveClient: false`) and pass the arguments through `lib/targets.mjs`; add a scenario helper for the control
  route; keep `socket-io/handshake-and-ack` passing; verify `node run.mjs` (Node against Node) and
  `node --test "test/*.test.mjs"` in
  `parity/socket-io/`
- [ ] 4.2 Write the same app with `IoServer` in `crates/corvis/tests/socket_io_diff.rs`, started in-process on
  `127.0.0.1:0` under `#[cfg(feature = "socket-io")]`, and pass `--corvis-capabilities engine-io,socket-io`; verify
  `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_diff` runs
  `socket-io/handshake-and-ack` on both targets with no difference
- [ ] 4.3 Scenarios `scenarios/socket-io/namespaces.mjs`, `middleware.mjs`, `connect-timeout.mjs` and
  `wire/pending-connect.mjs` (with its entry in `expected-differences.json`: reason and requirement "One socket per
  namespace per connection"); verify no unlisted and no stale difference

## 5. Events and acknowledgements

- [ ] 5.1 `socket.rs`: `on` / `off`, dispatch in arrival order with the `max_inflight` bound and read backpressure,
  handler panic containment, `emit` with its errors, `AckSender` (design.md Decision 7); tests for "Several
  arguments", "Reserved name", "Client ack with several values", "Order", "Handler panics", and "Blocked handlers" in
  `crates/corvis/tests/socket_io_security.rs` (measuring running handlers and queue sizes); verify both test files
- [ ] 5.2 `emit_with_ack` and `timeout`: per-namespace id counter, per-socket pending table, deadline, cap, failure
  on disconnect; unit tests on the paused clock for "Default deadline" and "Server ack timeout", tests for "Pending
  acknowledgement cap", "Client answers", "Socket disconnects with a pending acknowledgement" and "ACK from another
  socket"; verify the tests
- [ ] 5.3 Scenarios `events.mjs`, `acks.mjs`, `wire/encoding.mjs` (byte-equal payloads, the `U+001E` string) and
  `wire/ack-id.mjs` (with its expected-difference entry); verify no unlisted and no stale difference

## 6. Binary attachments

- [ ] 6.1 Binary events and acks end to end over polling and WebSocket using `send_all`; reassembly accounted in
  `max_unread_bytes`; tests for "Binary echo over polling", "Binary acknowledgement over WebSocket", "Concurrent
  binary emits"; verify `cargo test -p corvis --features socket-io --test socket_io`
- [ ] 6.2 Security tests in `socket_io_security.rs`: "Too many attachments", "Declared count without data",
  "Repeated placeholder" (memory measured as buffer references, not RSS), "Incomplete packets on many connections",
  "Nesting above the limit", "Extreme nesting", "Oversized ack id", emit above `max_attachments`; verify the tests
- [ ] 6.3 Scenarios `binary.mjs` (polling-only and WebSocket clients), `wire/malformed.mjs`,
  `wire/attachments-limit.mjs` and `wire/depth.mjs` (with its expected-difference entry); verify no unlisted and no
  stale difference

## 7. Disconnect and shutdown

- [ ] 7.1 `DisconnectReason`, `disconnect(false)` / `disconnect(true)`, client DISCONNECT, Engine.IO close reasons,
  `on_disconnecting` / `on_disconnect` order and exactly-once, pending acks failed (design.md Decision 8); tests for
  "Client leaves one namespace", "Server leaves one namespace", "Server closes the connection", "Transport closed by
  the client", "Handler order", "Malformed packet" (`forced close`); verify the tests
- [ ] 7.2 `IoServer::close()` and the `on_closing` hook: `server shutting down` before the engine closes, on
  `IoServer::close`, on graceful shutdown and on `CorvisApplication::close`; test "Shutdown with connected sockets";
  verify the test and that `tests/engine_io.rs` "Graceful shutdown with a pending poll" still passes
- [ ] 7.3 Scenario `disconnect.mjs` (the four client-visible cases, with the server-side reasons read from
  `/harness/log`); verify no unlisted and no stale difference

## 8. Conformance, docs, review

- [ ] 8.1 `crates/corvis/tests/socket_io_conformance.rs` (design.md Decision 10) running the upstream
  `socket.io-protocol` suite fetched by `scripts/fetch-suites.mjs`; verify every test passes with
  `CORVIS_NODE_DIFF=1`, and that the same suite passes against the Node reference server started with the suite's
  options
- [ ] 8.2 Review `parity/socket-io/expected-differences.json`: exactly the three client-observable divergences of
  design.md Decision 9, each with reason and `ws/socket-io` requirement; verify the full harness run has no unlisted
  and no stale difference, three times in a row
- [ ] 8.3 Rustdoc for every public item of `corvis::socket_io` added here (each option with its default, Node's value
  and the divergence; a compiling example on `IoServer`, `IoSocket::on`, `emit_with_ack` and `use_middleware`);
  verify `cargo doc -p corvis --features socket-io --no-deps` without warnings and
  `cargo test -p corvis --features socket-io --doc`
- [ ] 8.4 Docs: extend the "Socket.IO (preview)" section of `docs/websockets/adapter.md` (low-level API with a short
  example, namespaces are registered by the application, the limits table of design.md Decision 9, the divergences,
  what is not supported: dynamic namespaces, state recovery, `volatile`, the client bundle) and translate it in
  `docs/es/websockets/adapter.md`; update the harness section of `CONTRIBUTING.md` (scenario app, control route,
  conformance test); verify `npm run docs:check` in `docs/`
- [ ] 8.5 Security review of the change with the `security-reviewer` agent (parser, reassembly, ack tracking,
  CONNECT handling, limits of design.md Decision 9, the three Engine.IO additions); repeat rounds until no open
  finding; record findings and their fixes in design.md under "Security review"; verify every finding has a regression
  test or a recorded reason

## 9. Definition of done

- [ ] 9.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features socket-io`
  with no warnings
- [ ] 9.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings`
- [ ] 9.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io` passes, and with
  `CORVIS_NODE_DIFF=1` the `socket_io_diff`, `engine_io_conformance` and `socket_io_conformance` tests pass
- [ ] 9.4 Regression tests: one per spec scenario (unit, integration, security or harness scenario), as listed in
  groups 1-8
- [ ] 9.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 9.6 Framework runtime code changed (`crates/corvis/src/socket_io/`): run the `corvis-parity` check and verify
  the three port apps keep 0 apidiff differences
- [ ] 9.7 The security review of 8.5 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/socket-io` delta is applied).
- Close issue #33, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).

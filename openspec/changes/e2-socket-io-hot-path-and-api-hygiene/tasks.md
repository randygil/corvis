# Tasks

Start after #35 (`e2-socket-io-gateway-adapter`) is merged into `dev`. Shared Cargo target directory only (never a target
directory per task). Run the Node-gated checks with the harness installed (`npm ci` in `parity/socket-io/`) and
`CORVIS_NODE_DIFF=1`. Timing rule for every test: wait for a condition with a deadline that only fails the test, observe
absence through a marker event or on the paused clock, never `sleep` to synchronize, never compare durations between runs.
Test servers and the benchmark bind to `127.0.0.1` only. Behavior must not change except where design.md "Behavior visible
to applications and clients" says so.

## 1. Reconcile and reproduce

- [x] 1.1 Re-check every file and line of design.md "The findings, confirmed against the code" against `dev` with #35
  merged (in particular `socket_io/client.rs`, which #35 edits, and `gateway.rs`, which calls the connect path); write the
  moved lines and any conflict with #35 in a new "Implementation notes" section of design.md before any other task; verify
  `cargo test -p corvis --features socket-io` passes on the untouched tree
- [x] 1.2 Reproduce #184: run `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_conformance`
  8 times and record the failures (count and the failing test) in Implementation notes; verify the failure is the one the
  issue describes (`expected '2' to deeply equal '42["auth",{}]'`) or record what differs

## 2. Benchmark

- [x] 2.1 `bench/socket-io/` workspace member (`Cargo.toml` with `publish = false`, feature `socket-io` of `corvis`, added
  to the workspace `members`): binary `corvis-sio-bench` with `server` (an `IoServer` with default options on
  `127.0.0.1:<port>`, a global counting allocator, a `/stats` route that resets and reads allocations and bytes) and `load`
  (raw Engine.IO / Socket.IO clients over WebSocket and polling, scenarios `emit`, `server-emit`, `broadcast`, parameters
  for connections, messages, payload size, a 4 KiB `cookie` header on every client); verify
  `cargo build --release -p corvis-sio-bench` without warnings and a manual run of each scenario with 10 connections
- [x] 2.2 `bench/socket-io/run.sh` (matrix of the spec's default run, three runs per measurement, median with min and max,
  `SKIP_BUILD`, the missing-binary message, one server at a time) and `bench/socket-io/README.md` (prerequisites, variables,
  where the binary is built); add `bench/socket-io/raw/` to `.gitignore`; verify the two scenarios of the spec requirement
  "The Socket.IO benchmark drives emit and broadcast" by hand
- [x] 2.3 Before run: the full matrix plus `server-emit` and `broadcast` with a 16 KiB payload; copy the results table into
  Implementation notes ("Benchmark before"); verify `git status` shows no `bench/socket-io/raw/` file

## 3. Release wakeups (design.md Decision 2)

- [x] 3.1 `crates/corvis/src/ws.rs`: `UnreadGauge::waiters`, `register()` returning an allocation-free future (three
  `Option<Notified>`, a guard that decrements the counts), chain depth asserted at construction, `release` and `roll_back`
  waking a level only when its count is not zero; unit tests in `ws.rs::gauge_tests` for the three scenarios of the spec
  requirement "Waiting for room costs nothing below the limits" (a test-build counter of `wake` calls) and an interleaving test
  with the waiter steps; verify the existing gauge tests, including
  `observers_paused_by_the_pool_are_not_woken_by_rollbacks_at_their_address`, pass unchanged
- [x] 3.2 Callers check, register, recheck: `socket_io/client.rs` (the loop's `unread_changed` only when parked or over the
  limit), `socket_io/engine/websocket.rs` (reader), `socket_io/engine/polling.rs` (held `POST`); remove
  `EngineSocket::unread_changed` if nothing else uses it; verify `cargo test -p corvis --features socket-io --test
  socket_io_accounting --test engine_io_security --test socket_io_transit`
- [x] 3.3 `OutQueue::watchers` with the same count (`wait_ready` registers, `wake` notifies watchers only when counted); unit
  test: 10,000 pushes with no pending poll send no watcher notification, and a poll that registers between a push and its
  check gets the item; verify `cargo test -p corvis --features socket-io --lib ws` and `--test engine_io`

## 4. One handshake request per session (Decision 3)

- [x] 4.1 `engine/session.rs` and `engine/mod.rs`: `Session.handshake: Arc<Request>`, `EngineSocket::handshake()` unchanged,
  crate-internal `handshake_arc()`; `socket.rs`: `HandshakeData.request: Arc<Request>`; `client.rs:521` shares instead of
  cloning; verify `cargo test -p corvis --features socket-io --test socket_io --test socket_io_rooms`
- [x] 4.2 `EngineServer::handle` / `dispatch`: the request built once and passed by reference; `Query` with three fields;
  `Request::for_engine_poll` for requests with a `sid` (no query parsing, never handed to application code, debug assertion);
  tests: the three scenarios of the spec requirement "The handshake request is kept once per session" (counting allocator in
  `tests/socket_io_memory.rs`, `testing::same_request`); verify `cargo test -p corvis --features socket-io --test engine_io
  --test engine_io_security --test socket_io_memory` and that the transport-confusion, origin and shape-check tests pass
  unchanged
- [x] 4.3 Correct the docs that claimed otherwise: `HandshakeInfo` rustdoc (`adapter.rs`, `info.rs` after group 11),
  `budget.rs` module docs, `Handshake` rustdoc: one request per session, bounded by `max_header_size`, not charged, and why;
  verify `cargo doc -p corvis --features socket-io --no-deps` without warnings

## 5. Fewer copies (Decision 4)

- [x] 5.1 `parser.rs`: non-allocating binary count bounded by `MAX_DEPTH`, header then JSON into one buffer with one leading
  byte reserved for the Engine.IO type; `value.rs`: `contains_binary` without allocation for values with no container;
  verify `cargo test -p corvis --features socket-io --lib socket_io::parser` and `--lib fuzz` (the fixtures of
  `fixtures/parser.json` unchanged)
- [x] 5.2 `Encoded` by value through `IoSocket::send` and `ClientShared::send_encoded`; hidden `EngineMessage::Framed`, and
  `EngineMessage::Shared` holding the framed text as `Bytes`; `len`, `polling_len`, `encode_polling`, `encode_ws` for both;
  unit tests: lengths equal the encoded packet's for both variants, polling output byte for byte as before; verify
  `cargo test -p corvis --features socket-io --lib` and `--test socket_io_rooms_memory`
- [x] 5.3 `OutQueue::push_group` without the size vector; verify the group tests in `ws.rs` and `engine/session.rs`
- [ ] 5.4 Upgrade `tokio-tungstenite` (crate and dev-dependency in `crates/corvis/Cargo.toml`, `examples/tertulia/Cargo.toml`)
  to the latest release with `Bytes` payloads (0.26 or later; record the version and why in Implementation notes after
  checking MSRV 1.89, `cargo deny check` and `cargo audit`); adapt `ws.rs`, `websockets.rs`, `server.rs`,
  `engine/websocket.rs` (`Message::Text` from `Utf8Bytes` without copying, `Message::Binary` from the attachment's `Bytes`),
  the tests that build `tungstenite` messages and the example; the limits keep their values; if the change needs files beyond
  these, stop and open a separate issue (design.md Decision 4 fallback); verify `cargo test -p corvis --features websockets
  --test websockets --test ws_security --test ws_enhancers --test auth_ws --test send_timeout_ws --test tls_ws` and the
  socket-io suites
- [ ] 5.5 Pointer-identity tests for the three scenarios of the spec requirement "An emit is encoded once"; verify
  `a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches` passes unchanged

## 6. Connect sequence (Decision 5, #183)

- [x] 6.1 `socket.rs`: `Phase::Joining`; `is_live` true for it; emits, `AckSender::send` and `register_broadcast_ack` refuse;
  `disconnect_with` on a joining socket tears it down silently; `namespace.rs`: `add_socket` inserts only while `Joining`
  (phase read under the table's write lock); `LocalNamespace::sockets_by_ids` leaves joining sockets out; rustdoc of
  `socket`, `sockets`, `socket_count` (Review note 2); verify `cargo test -p corvis --features socket-io --lib`
- [x] 6.2 `client.rs` `connect`: steps 1-6 of Decision 5 (no `nsps()` across the adapter or a send; CONNECT queued and the phase
  set under the socket's state lock; an own-room failure answers CONNECT_ERROR `Internal server error` and removes the slot);
  `begin_connect` keeps the id-check failure as it is; the connect step hook in `testing` (parks `connect` after a chosen
  step, test builds of the integration suites only through the hidden module); verify `cargo test -p corvis --features
  socket-io --test socket_io`
- [x] 6.3 `InMemoryAdapter::add_all`: the other-socket-id test against `sids` under the write lock, one `is_live` for the
  joining socket; unit test: an `add_all` of 1,000 names makes one liveness lookup (a test-build counter in `LocalNamespace`);
  a test that joins a joining socket's id between steps 2 and 3 and finds itself expelled; verify the id-room tests of
  `socket_io/tests/` and `--lib fuzz`
- [x] 6.4 `crates/corvis/tests/socket_io_connect_order.rs` (multi-thread runtime, 4 workers): one test per scenario of the spec
  requirements "A connecting socket is registered before its CONNECT" and "No connection lock is held across the adapter", and
  the teardown at each of steps 1-5; update the CONNECT failure tests to the MODIFIED requirement "A panicking adapter does not
  leave a connecting socket behind" (both scenarios of the own room); replace `socket_of` in
  `tests/socket_io_rooms_security.rs` with direct lookups; verify the new suite passes 5 runs in a row and
  `--test socket_io_rooms_security`
- [x] 6.5 The lock order of Decision 5 in the `adapter/mod.rs` module docs and in Implementation notes; verify no
  `nsps()` guard is alive across a call into the adapter, a handler or the engine (read the diff of `client.rs`)

## 7. CONNECT in a write of its own (Decision 6, #184)

- [x] 7.1 `ws.rs` / `engine`: acknowledgement on the last item of `push_group`, `take_batch` acknowledging taken items,
  `EngineSocket::send_all_written` returning a future of `bool`; unit tests: acknowledged when written (WebSocket writer),
  when taken (polling), `false` when discarded or closed; verify `cargo test -p corvis --features socket-io --lib`
- [x] 7.2 `connect` queues CONNECT with `send_all_written`, sets `joined_once` when the socket is `Connected`, waits for the
  write or the close before the connection handlers and `mark_ready`; `IoNamespace::on_connection` rustdoc ("after the CONNECT
  packet was written"); verify `cargo test -p corvis --features socket-io --test socket_io`
- [x] 7.3 Tests for the scenarios "Polling", "WebSocket" and "Connection closes before the write" of the spec requirement
  "CONNECT leaves in a write of its own" (unit tests over the session outbox and the in-process transport of
  `socket_io/tests.rs`); verify they pass
- [ ] 7.4 Run the upstream suite 20 times in a row (`CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test
  socket_io_conformance`) and `socket_io_diff` once; record the 20 results in Implementation notes; verify 20 of 20 pass

## 8. Address key (Decision 7)

- [x] 8.1 `EngineSocket::address_key()`; `IoSocket::resolve_ack` passes it; test for the scenario of the spec requirement "The
  answer budget keys answers by the session's address"; verify
  `one_address_cannot_fill_the_answer_budget_and_make_the_calls_of_others_fail` passes unchanged

## 9. `Connection` struct (Decision 8)

- [x] 9.1 `client.rs`: `Connection` owning the loop state, methods for `handle_message`, `admit`, `handle_packet`, one method for
  the outcomes of a `Flow`; no `#[allow(clippy::too_many_arguments)]` left in the file; verify
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings` and the suites `socket_io`,
  `socket_io_accounting`, `socket_io_lingering`, `socket_io_security`, `socket_io_transit`

## 10. One place for each duplicated rule (Decision 9)

- [x] 10.1 `RoomLimits` with `check` and `check_name`, used by `IoSocket::join`, `InMemoryAdapter::add_all`,
  `BroadcastOperator::valid_rooms` and `with_rooms`; verify the room-limit tests of `socket_io_rooms`,
  `socket_io_rooms_security` and `fuzz_adapter` pass unchanged
- [x] 10.2 `SocketState::register_ack` used by `emit_with_ack_within` and `register_broadcast_ack`; verify the pending-ack tests
  of `socket_io` and `socket_io/tests/`
- [x] 10.3 The engine's three limiters replaced by a `WarnLimiter` (static kinds, empty namespace); `WarnLimiter` with the nested
  map and lazy messages (`impl FnOnce() -> String`), "since the last one" in the text; unit tests: a dropped line allocates
  nothing (a test-build counter of message closures called), the cap counts both levels; verify the `warn.rs` tests and
  `--test socket_io_log`

## 11. API hygiene and the nice-to-have items (Decisions 10, 11, 13, 14)

- [x] 11.1 `socket_io/testing.rs` (`#[doc(hidden)]`) with the hooks of Decision 10; remove them from the trait, from
  `InMemoryAdapter`, `IoSocket`, `IoNamespace` and `socket_io/mod.rs`; `Membership.examined` under `cfg(test)`; update
  `tests/socket_io_rooms.rs`, `socket_io_rooms_memory.rs`, `socket_io_memory.rs`, `socket_io_accounting.rs`; verify the suites
- [x] 11.2 `#[non_exhaustive]` on the seven enums with a `compile_fail` doctest each; verify
  `cargo test -p corvis --features socket-io --doc`
- [x] 11.3 Dead code (`ServerInner.closed`, `IoSocket::send_encoded`, the unused parameter of `handshake_refused`) and the two
  comments; verify `cargo build -p corvis --features socket-io` without warnings
- [x] 11.4 `parse_json` without matching error text, tests at `MAX_DEPTH` and `MAX_DEPTH + 1`; `begin_connect` returning
  `Result<(), ProtocolViolation>`; verify `--lib socket_io::parser` and `--lib fuzz`
- [x] 11.5 Weak connection callback in `IoServer::new`, strong one installed by `attach`; test: an unattached `IoServer` dropped
  is freed; verify the lifetime and shutdown tests of `socket_io` pass unchanged
- [x] 11.6 `WriteEnd::Refused` as a peer close in `engine/websocket.rs`; test of design.md Decision 13 (200 iterations, reason
  always `transport close`); verify `--test engine_io_ws`
- [x] 11.7 `HandshakeData` stores the session's resolved address; `ip()` returns a copy of it; the `value.rs` sentence on the
  CI job of the counting-allocator tests; verify `--test trust_proxy` and the `ip()` tests
- [x] 11.8 Split `adapter.rs` into `adapter/mod.rs`, `adapter/memory.rs`, `ack.rs`, `info.rs` with the same public paths; split
  `socket_io/tests/review1.rs` into files named after behavior and rename tests that name a review round; verify
  `cargo test -p corvis --features socket-io -- --list | wc -l` gives the same count before and after the split and no
  import outside `socket_io/` changed

## 12. Tests that wait for conditions (Decision 12)

- [ ] 12.1 `eventually(deadline, condition)` in `tests/engine_support/mod.rs`; `settle_until(condition)` replacing `settle()` in
  `socket_io/tests.rs` and `advance` in `engine/session.rs` tests; verify the unit suites pass 5 runs in a row
- [ ] 12.2 `tests/socket_io_accounting.rs` (lines 184, 202, 256, 277 and 437-554 of design.md's table): sleeps replaced by
  conditions or markers, wall-clock upper bounds removed, "not before" bounds kept against the configured value; verify 5 runs
  in a row
- [ ] 12.3 `tests/socket_io_lingering.rs` (153, 204), `tests/engine_io.rs` (242, 275, 293), `tests/engine_io_security.rs`
  (105, 125, 193, 218, 312-316, 332, 409, 431, 465, 566-571, 591); verify 5 runs in a row each
- [ ] 12.4 Grep the socket-io and engine-io suites for `sleep(` and `elapsed()` once more and justify each one left (a
  "not before" bound, a peer that must be slow, a load generator) in a comment beside it; verify the grep result in
  Implementation notes

## 13. Benchmark after

- [x] 13.1 After run on the same machine and matrix as 2.3; "Benchmark after" table in Implementation notes with, per decision,
  the figure it had to move (Decision 2: allocations per received message down by at least six, `emit` rate not lower;
  Decision 3: bytes per polling request and per CONNECT down by about the 4 KiB of headers; Decision 4: bytes per
  `server-emit` message from about five payloads to about two, one payload and one allocation less per broadcast recipient);
  any decision that missed its figure is explained there; publish `bench/socket-io/results.md`

## 14. Docs and review

- [x] 14.1 Rustdoc of every item whose behavior or place changed (`on_connection`, `socket`, `sockets`, `socket_count`,
  `Handshake`, the seven enums, `testing`, `Adapter` lock rules); verify `cargo doc -p corvis --features socket-io --no-deps`
  without warnings and `cargo test -p corvis --features socket-io --doc`
- [x] 14.2 Docs: in the "Socket.IO (preview)" section of `docs/websockets/adapter.md`, when connection handlers run, the
  CONNECT_ERROR on an adapter failure at connect, lookup by id right after connecting, the handshake memory (one request per
  session), the non-exhaustive enums; translate in `docs/es/websockets/adapter.md`; `bench/socket-io/README.md` linked from
  `bench/README.md`; verify `npm run docs:check` in `docs/`
- [ ] 14.3 Security review with the `security-reviewer` agent (the connect sequence: phases, teardown at each step, lock order,
  reentrant adapters; the wakeup protocol and lost wakeups; the engine's request handling for `sid` requests; the
  `tokio-tungstenite` upgrade and the message limits; the answer budget key); repeat rounds until no open finding; record
  findings and fixes in design.md under "Security review"; verify every finding has a regression test or a recorded reason
- [x] 14.4 Add design.md "What the Redis adapter (#36) needs from this change" to the review notes of
  `openspec/changes/e2-socket-io-redis-adapter/design.md`, and correct its file list for the split of Decision 14; verify
  `openspec validate e2-socket-io-redis-adapter --strict`

## 15. Definition of done

- [x] 15.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features socket-io` with no
  warnings
- [x] 15.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings`
- [ ] 15.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io` passes, and with
  `CORVIS_NODE_DIFF=1` the `socket_io_diff` and `socket_io_conformance` tests pass
- [x] 15.4 Per-feature checks: `cargo check -p corvis --tests --features <f>` for websockets, compression, tls, auth-local,
  auth-api-key, auth-session, auth-oauth2 and socket-io, with no warnings
- [ ] 15.5 Regression tests: one per spec scenario (unit, integration, counting allocator or harness), as listed in groups 3-12
- [x] 15.6 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 15.7 Framework runtime code changed (`ws.rs`, `websockets.rs`, `server.rs`, `socket_io/`): run the `corvis-parity` check and
  verify the three port apps keep 0 apidiff differences
- [x] 15.8 `cargo deny check` and `node scripts/check-naming.mjs` pass
- [ ] 15.9 The security review of 14.3 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (deltas of `ws/socket-io`, `ws/socket-io-rooms`, `ws/engine-io` and the new
  `bench/socket-io` applied).
- Close issues #185, #183 and #184, tick #185 in epic #30 and move the cards to Done (see the `corvis-task` skill).
- Open the follow-ups the maintainer accepts: a `&str` accessor for the socket address, a Node comparison in the Socket.IO
  benchmark, and the `tokio-tungstenite` upgrade as its own issue if task 5.4 used the fallback.

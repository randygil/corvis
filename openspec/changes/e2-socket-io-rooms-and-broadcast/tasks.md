# Tasks

Start after #33 (`e2-socket-io-namespaces-events-and-acks`) is merged into `dev`. New test files start with
`#![cfg(feature = "socket-io")]`. Run the Node-gated checks with the harness installed (`npm ci` in
`parity/socket-io/`) and `CORVIS_NODE_DIFF=1`. Timing rule for every test: absence of delivery is observed through a
marker event, never by sleeping; "not before" bounds use the configured value; timer logic runs on Tokio's paused
clock; no retries.

## 1. Adapter interface and options

- [x] 1.1 Reconcile design.md "The layers this builds on" with the code merged by #32 and #33 (grouped send,
  per-socket pending acks and the namespace id counter, disconnect order, `IoSocket::data`); note any renamed item at
  the top of design.md; verify `cargo test -p corvis --features socket-io --test socket_io` passes before any edit
- [x] 1.2 `crates/corvis/src/socket_io/adapter.rs`: the `Adapter` trait of design.md Decision 2, `Room`, `SocketId`,
  `BroadcastOptions`, `BroadcastFlags` (`#[non_exhaustive]`), `AckSink`, `SocketInfo`, `AdapterError`, `RoomError`,
  `LocalNamespace`, and `IoOptions::adapter(factory)`; make `Packet` public as an opaque type; rustdoc stating the
  trait is unstable until #36; verify `cargo build -p corvis --features socket-io` and `cargo build -p corvis`
  (feature off) with no warnings
- [x] 1.3 `IoOptions`: `max_rooms_per_socket`, `max_room_name_length`, `max_broadcast_ack_bytes` with the defaults
  and validation of design.md Decision 8; unit tests for every default and every rejected value (spec "Zero rooms");
  verify `cargo test -p corvis --features socket-io --lib socket_io::options`

## 2. In-memory adapter

- [x] 2.1 `InMemoryAdapter` membership (design.md Decision 3): `add_all` (connected check and limits under the write
  lock, all-or-nothing), `del`, `del_all`, `socket_rooms`, empty rooms deleted; unit tests including the room cap and
  the name length; verify `cargo test -p corvis --features socket-io --lib socket_io::adapter`
- [x] 2.2 Selection and delivery: recipients computed as Node's `apply` under the read lock, delivery after releasing
  it, one shared encoding queued with `send_all`, `sockets`, `fetch_sockets`, `add_sockets`, `del_sockets`,
  `disconnect_sockets`, `server_count`; unit tests from a table of cases derived from Node's `apply` (rooms, unions,
  except rooms, except ids, unknown rooms, ids without a local socket); verify the unit tests
- [x] 2.3 `socket_io/fuzz_adapter.rs`: proptest properties (selection equals a naive model for random memberships and
  options; after any sequence of join / leave / disconnect the two maps mirror each other and hold no empty room and
  no disconnected socket); verify `cargo test -p corvis --features socket-io --lib fuzz` and with
  `PROPTEST_CASES=20000`
- [x] 2.4 Test-only adapter in `crates/corvis/tests/socket_io_rooms.rs` wrapping `InMemoryAdapter`: yields on every
  cluster operation, can fail a chosen operation, reports two servers with a simulated second node for
  `broadcast_with_ack`; used by tasks 3.x through a test parameter; verify it compiles against the public API only
  (no `pub(crate)` item needed)

## 3. Rooms on sockets and operators

- [x] 3.1 `IoSocket::join` / `leave` / `rooms`, the own-id room on connect, and the disconnect order of design.md
  Decision 4 (`del_all` between `on_disconnecting` and `on_disconnect`, join after disconnect a no-op); tests in
  `crates/corvis/tests/socket_io_rooms.rs` for spec "Rooms right after connecting", "Emit to a socket id" (after 3.2),
  "Leave", "Rooms during disconnecting", "Farewell from the disconnecting handler"; verify
  `cargo test -p corvis --features socket-io --test socket_io_rooms` and that `--test socket_io` still passes
- [x] 3.2 `broadcast.rs`: `BroadcastOperator` (`to`, `within`, `except`, `local`, `timeout`, deferred room errors,
  `emit`) and the entry points on `IoServer`, `IoNamespace` and `IoSocket` (sender excluded), `IoNamespace::emit`;
  tests with both adapters for "Same room name in two namespaces", "Whole namespace", "Union without duplicates",
  "Broadcast from a socket", "Except a room", "Except a socket id inside a room", "Empty room", "Binary broadcast",
  "Order", "Long name in to", "Adapter failure"; verify the tests
- [x] 3.3 Bulk operations and `RemoteSocket` (design.md Decision 7): `fetch_sockets`, `sockets_join`,
  `sockets_leave`, `disconnect_sockets`, a socket at its room cap skipped and logged; tests for "Make a room join
  another", "Disconnect a room", "Fetch sockets"; verify the tests
- [x] 3.4 Broadcast with acknowledgements (design.md Decision 6): shared id, per-recipient pending entries,
  completeness with `server_count`, deadline, cleanup at the deadline, `max_broadcast_ack_bytes`; unit tests on the
  paused clock for the deadline ("Default deadline"), tests with both adapters for "Every recipient answers", "One
  recipient silent",
  "Answer from outside the broadcast", "Answers too large", "Replaced adapter"; verify the tests

## 4. Security tests

- [x] 4.1 `crates/corvis/tests/socket_io_rooms_security.rs`, in the style of `tests/ws_security.rs`: "Room cap",
  "Long name in join", "Many slow clients" (queue sizes and the shared encoding's reference count measured, not RSS;
  every emit returns without waiting), "Join during a broadcast"; verify
  `cargo test -p corvis --features socket-io --test socket_io_rooms_security`
- [x] 4.2 "Join racing a disconnect": 1,000 iterations on a multi-thread runtime, asserting the adapter's maps after
  each; verify it passes 5 runs in a row

## 5. Harness scenarios

- [x] 5.1 Add the room and broadcast commands of design.md Decision 9 to
  `parity/socket-io/reference/socket-server.mjs` and to the Corvis app in `crates/corvis/tests/socket_io_diff.rs`
  (`join`, `leave`, `rooms`, `broadcast` with the `done` marker, `broadcast-ack`, `sockets-join`, `sockets-leave`,
  `disconnect-sockets`, `fetch-sockets`, rooms in the `disconnecting` / `disconnect` log entries) and a scenario
  helper that opens several named clients and collects events up to the marker; verify `node run.mjs` (Node against
  Node) and `node --test "test/*.test.mjs"` in `parity/socket-io/`
- [x] 5.2 Scenarios `scenarios/socket-io/rooms/scoping.mjs`, `socket-id.mjs`, `union.mjs`, `from-socket.mjs`,
  `except.mjs`, `leave-and-disconnect.mjs`; verify
  `CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_diff` reports no unlisted and no
  stale difference
- [x] 5.3 Scenarios `rooms/bulk.mjs` and `rooms/broadcast-ack.mjs` (all answer; one silent with a 200 ms timeout,
  answers sorted before recording); verify no unlisted and no stale difference
- [x] 5.4 Scenario `rooms/limits.mjs` (1,000 rooms in one join; a 257-byte room name) with its two entries in
  `parity/socket-io/expected-differences.json`, each with reason and `ws/socket-io-rooms` requirement; verify the
  listed differences occur and the full harness run is clean three times in a row

## 6. Docs and review

- [x] 6.1 Rustdoc for every public item added (operators with a compiling example of `to(...).except(...).emit(...)`
  and `socket.broadcast()`, each option with its default, Node's value and the divergence, the `Adapter` trait with
  what #36 will implement); verify `cargo doc -p corvis --features socket-io --no-deps` without warnings and
  `cargo test -p corvis --features socket-io --doc`
- [x] 6.2 Docs: add rooms and broadcast to the "Socket.IO (preview)" section of `docs/websockets/adapter.md` (join /
  leave, the own-id room, operators and `within` as Node's `in`, terminal operations are `async`, broadcast
  acknowledgements, the limits table of design.md Decision 8 and the divergences, the adapter option with Redis
  announced for #36) and translate it in `docs/es/websockets/adapter.md`; verify `npm run docs:check` in `docs/`
- [ ] 6.3 Security review of the change with the `security-reviewer` agent (room limits, operator errors that must
  not widen a broadcast, fan-out memory, lock discipline and the join / disconnect race, broadcast acknowledgement
  tracking and its cap, the adapter trait as an extension point); repeat rounds until no open finding; record
  findings and their fixes in design.md under "Security review"; verify every finding has a regression test or a
  recorded reason

## 7. Definition of done

- [ ] 7.1 `cargo build --workspace --all-targets` with no warnings, and `cargo build -p corvis --features socket-io`
  with no warnings
- [ ] 7.2 `cargo clippy --workspace --all-targets -- -D warnings` passes, and
  `cargo clippy -p corvis --no-default-features --features socket-io -- -D warnings`
- [ ] 7.3 `cargo test --workspace` passes, `cargo test -p corvis --features socket-io` passes, and with
  `CORVIS_NODE_DIFF=1` the `socket_io_diff` and `socket_io_conformance` tests pass
- [ ] 7.4 Regression tests: one per spec scenario (unit, integration, security or harness scenario), as listed in
  groups 1-5
- [ ] 7.5 Docs pages updated in English and Spanish (`docs/` and `docs/es/`), then `npm run docs:check` in `docs/`
- [ ] 7.6 Framework runtime code changed (`crates/corvis/src/socket_io/`): run the `corvis-parity` check and verify
  the three port apps keep 0 apidiff differences
- [ ] 7.7 The security review of 6.3 is closed before archiving

## Workflow follow-up

- Archive the change with `/opsx:archive` (its `ws/socket-io-rooms` delta is applied).
- Close issue #34, tick it in epic #30 and move its card to Done (see the `corvis-task` skill).

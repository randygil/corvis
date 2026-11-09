# Tasks

Start after #157 (`e0-write-deadline-for-stalled-clients`) is merged into `main` and archived: this change edits its
`TimedIo::poll_write_deadline`, rewrites its `tests/send_timeout_ws.rs` and removes a requirement of its
`http/send-timeout` spec.

## 1. Failing tests

- [x] 1.1 Rewrite `crates/corvis/tests/send_timeout_ws.rs` (feature `websockets`): keep the `/feed` gateway; the
  existing test becomes `..._keeps_its_connection_without_a_send_timeout` with `send_timeout(None)`; add a gateway
  client that never reads (connection ends within a few seconds, `handle_disconnect` observed through a static
  counter, `server.clients()` empty), the `max_connections(1)` slot probe, a client that pings every 200 ms and never
  reads, an idle connection for five seconds, a slow reader every 400 ms, and `client.disconnect()` on a client that
  does not read with `send_timeout` (1 s) shorter than `close_timeout` (30 s); verify with
  `cargo test -p corvis --features websockets --test send_timeout_ws` that the stall tests fail and the idle,
  slow-reader and `None` tests pass
- [x] 1.2 Add the plain `WebSocketServer` cases to the same file: an `upgrade` listener handing `/raw` to a
  `WebSocketServer` that sends 3 MiB and one `send_and_wait` to a peer that never reads; assert `on_close` reports
  `1006`, `send_and_wait` returns an error and the socket is closed; verify it fails on the current code

## 2. Write deadline on upgraded connections

- [x] 2.1 Remove the `upgraded` early return from `TimedIo::poll_write_deadline` in `crates/corvis/src/server.rs`
  (design.md, Decision 1); keep the watchdog and `poll_idle` exemptions; update the rustdoc of `send_timeout` (it now
  applies to upgraded connections, writes only) and of `max_connections` (heartbeat still needed for idle dead
  peers); verify the gateway never-read and pinging tests of 1.1 pass and `tests/send_timeout.rs`,
  `tests/websockets.rs` and `tests/ws_security.rs` still pass

## 3. Runtimes

- [x] 3.1 Make `write_loop` in `crates/corvis/src/ws.rs` return `Closed` or `Failed(io::ErrorKind)` (design.md,
  Decision 4); verify existing unit tests of `OutQueue` still pass
- [x] 3.2 In `crates/corvis/src/websockets.rs::run_connection`, break on `Failed` even while closing and log one
  warning for `TimedOut`; verify the gateway tests of 1.1, including the closing-handshake one, pass
- [x] 3.3 In `ws.rs::run`, add a writer arm that ends the loop on `Failed` (and on `Closed` once a close was received
  or sent), log one warning for `TimedOut`; verify the tests of 1.2 pass and `tests/http_adapter.rs` still passes

## 4. Docs

- [x] 4.1 `docs/websockets/gateways.md` and `docs/websockets/adapter.md`: a paragraph after the options tables on
  peers that stop reading (`send_timeout` of the server, what counts as progress, pings, `1006`, `handle_disconnect`,
  idle connections unaffected, heartbeat for dead idle peers, comparison with `ws`, which has no such limit); same in
  `docs/es/websockets/`
- [x] 4.2 `docs/http/performance.md`, section `send_timeout`: replace "neither is an established WebSocket connection"
  with the write-deadline rule for upgraded connections and link the WebSocket pages; same in
  `docs/es/http/performance.md`
- [x] 4.3 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes, including the regression tests of 1.1 and 1.2
- [ ] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences

## 5b. Security review round (2026-11-09)

- [x] 5b.1 `ws.rs::run`: do not poll the writer `JoinHandle` again after the writer arm consumed it (guard with
  `writer_done`); regression test for a protocol error during the closing handshake (on_close once, no panic)
- [x] 5b.2 Read backpressure on upgraded connections: `TimedIo` stops reading while a write is blocked, so pongs cannot
  pile up in tungstenite's write buffer; tests for a ping flood without reading, gateway and plain `WebSocketServer`
- [x] 5b.3 `ws.rs::run`: a refused frame (not a socket failure) no longer ends the connection; only socket errors do
  (`WriteEnd::Refused`)
- [x] 5b.4 Run the reviewer's PoCs against the branch and record the results in design.md

## 5c. Security review round 2 (2026-11-09)

- [x] 5c.1 `TimedIo::read_paused` must not take the wake-up of the task whose write is pending: no waker is registered
  in the socket's write slot or the write deadline by the reader; the parked reader has its own timer, and the timeout
  wakes both halves and fails later reads of the upgraded connection; test with a split reader/writer protocol
- [x] 5c.2 Bounded read budget (64 KiB per blocked period) so half-duplex clients can write before reading; test and
  docs (EN + ES)
- [x] 5c.3 Correct the Implementation notes (`Other` became `WriteEnd::Refused`) and note the TLS constraint for #117
- [x] 5c.4 Re-run the reviewer's `split` PoC

## 5d. Security review round 3 (2026-11-09)

- [x] 5d.1 The read allowance becomes a credit tied to bytes written (`ReadCredit`): earned by bytes the socket accepts,
  spent by bytes read, reads while a write is blocked limited to the credit left, the writable probe never refills it;
  unit test with small write slices and an integration test with a trickle reader and a ping flood
- [x] 5d.2 `read_paused` unpauses on any probe result except `WouldBlock`, so a socket error reaches the next read
- [x] 5d.3 Extend the #117 note in design.md (credit measured in bytes accepted by TCP)
- [ ] 5d.4 Cross-check the Linux PoC (`pocl`): not run here (see design.md); the CI Linux job is the check

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after #157's change is archived).
- Close issue #161, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

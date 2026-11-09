# Tasks

Start after #157 (`e0-write-deadline-for-stalled-clients`) is merged into `main` and archived: this change edits its
`TimedIo::poll_write_deadline`, rewrites its `tests/send_timeout_ws.rs` and removes a requirement of its
`http/send-timeout` spec.

## 1. Failing tests

- [ ] 1.1 Rewrite `crates/corvis/tests/send_timeout_ws.rs` (feature `websockets`): keep the `/feed` gateway; the
  existing test becomes `..._keeps_its_connection_without_a_send_timeout` with `send_timeout(None)`; add a gateway
  client that never reads (connection ends within a few seconds, `handle_disconnect` observed through a static
  counter, `server.clients()` empty), the `max_connections(1)` slot probe, a client that pings every 200 ms and never
  reads, an idle connection for five seconds, a slow reader every 400 ms, and `client.disconnect()` on a client that
  does not read with `send_timeout` (1 s) shorter than `close_timeout` (30 s); verify with
  `cargo test -p corvis --features websockets --test send_timeout_ws` that the stall tests fail and the idle,
  slow-reader and `None` tests pass
- [ ] 1.2 Add the plain `WebSocketServer` cases to the same file: an `upgrade` listener handing `/raw` to a
  `WebSocketServer` that sends 3 MiB and one `send_and_wait` to a peer that never reads; assert `on_close` reports
  `1006`, `send_and_wait` returns an error and the socket is closed; verify it fails on the current code

## 2. Write deadline on upgraded connections

- [ ] 2.1 Remove the `upgraded` early return from `TimedIo::poll_write_deadline` in `crates/corvis/src/server.rs`
  (design.md, Decision 1); keep the watchdog and `poll_idle` exemptions; update the rustdoc of `send_timeout` (it now
  applies to upgraded connections, writes only) and of `max_connections` (heartbeat still needed for idle dead
  peers); verify the gateway never-read and pinging tests of 1.1 pass and `tests/send_timeout.rs`,
  `tests/websockets.rs` and `tests/ws_security.rs` still pass

## 3. Runtimes

- [ ] 3.1 Make `write_loop` in `crates/corvis/src/ws.rs` return `Closed` or `Failed(io::ErrorKind)` (design.md,
  Decision 4); verify existing unit tests of `OutQueue` still pass
- [ ] 3.2 In `crates/corvis/src/websockets.rs::run_connection`, break on `Failed` even while closing and log one
  warning for `TimedOut`; verify the gateway tests of 1.1, including the closing-handshake one, pass
- [ ] 3.3 In `ws.rs::run`, add a writer arm that ends the loop on `Failed` (and on `Closed` once a close was received
  or sent), log one warning for `TimedOut`; verify the tests of 1.2 pass and `tests/http_adapter.rs` still passes

## 4. Docs

- [ ] 4.1 `docs/websockets/gateways.md` and `docs/websockets/adapter.md`: a paragraph after the options tables on
  peers that stop reading (`send_timeout` of the server, what counts as progress, pings, `1006`, `handle_disconnect`,
  idle connections unaffected, heartbeat for dead idle peers, comparison with `ws`, which has no such limit); same in
  `docs/es/websockets/`
- [ ] 4.2 `docs/http/performance.md`, section `send_timeout`: replace "neither is an established WebSocket connection"
  with the write-deadline rule for upgraded connections and link the WebSocket pages; same in
  `docs/es/http/performance.md`
- [ ] 4.3 Run `npm run docs:check` in `docs/`; verify it passes

## 5. Definition of done

- [ ] 5.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 5.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 5.3 `cargo test --workspace` passes, including the regression tests of 1.1 and 1.2
- [ ] 5.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 5.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences

## Workflow follow-up

- Archive the change with `/opsx:archive` after review (after #157's change is archived).
- Close issue #161, tick it in epic #13 and move its card to Done (see the `corvis-task` skill).

# Tasks

Start after #150 (`e0-http2-limits-follow-server-options`) is merged: this change edits the same structures in
`crates/corvis/src/server.rs` and reuses its `tests/http2_limits.rs` raw-frame client.

## 1. Failing tests

- [ ] 1.1 Create `crates/corvis/tests/send_timeout.rs` with the helpers of design.md, Decision 9: a raw HTTP/1 client,
  the HTTP/2 `RawClient` of `tests/http2_limits.rs` (move it to `tests/common/h2.rs` and use it from both files, or
  copy it) extended with SETTINGS parameters, HEADERS with a literal HPACK block and PING, and a "second client served
  after" probe; routes: `/big` (1 MiB `Full`), `/huge` (8 MiB), `/stream` (4 MiB in 64 KiB stream chunks), `/sse`
  (one event, a three-second pause, another event), `/slow` (sleeps three seconds); verify
  `cargo test -p corvis --test send_timeout --no-run` builds and `cargo test -p corvis --test http2_limits` still passes
- [ ] 1.2 Add one test per scenario of the spec delta (the default as a unit test in `server.rs` asserting
  `ServerOptions::default().send_timeout == Some(60 s)`, not a 60-second integration test; disabled, HTTP/1 pipelined, HTTP/2 PING flood, large response then stop, zero window, one-byte window, idle SSE over
  HTTP/1 and h2c, slow download over HTTP/1 and h2c, slow handler, WebSocket client that stops reading); verify with
  `cargo test -p corvis --test send_timeout` that the stall tests fail (or fail to compile for lack of
  `send_timeout`) and the live-client tests pass where they compile

## 2. Option

- [ ] 2.1 Add `send_timeout: Option<Duration>` (default `Some(60 s)`) and the setter
  `send_timeout(impl Into<Option<Duration>>)` to `ServerOptions` in `crates/corvis/src/server.rs`, with rustdoc: what
  it measures (no response byte accepted, restarted by every byte), what it never touches (slow handlers, idle SSE,
  idle keep-alive, upgraded connections), the comparison with Node's `server.timeout` (default 0, inactivity in both
  directions) and nginx's `send_timeout`; mention it in the type-level rustdoc next to the Node defaults; verify
  `cargo doc -p corvis --no-deps` has no broken links

## 3. Write deadline

- [ ] 3.1 In `TimedIo`, add `send_timeout`, `write_stalled_since` and `write_sleep` and a helper
  `poll_write_deadline(&mut self, cx) -> Poll<io::Result<()>>` used on the `Pending` path of `poll_write`,
  `poll_write_vectored` and `poll_flush`; clear the stall on progress; skip when upgraded or `None`; on expiry set
  `no_linger` and return `TimedOut` (design.md, Decision 2); verify the HTTP/1 pipelined, HTTP/2 PING flood and large
  response tests pass and `tests/transport_security.rs` still passes

## 4. Stream watchdog

- [ ] 4.1 Add `BodyClock`, `ConnState::epoch`, `ConnState::clocks` and `ConnState::clocks_changed`; register and
  remove the clock in `GuardedBody` (constructor and `Drop`) and update it in `poll_frame` (design.md, Decision 3);
  verify unit tests in `server.rs` for the clock transitions (created → waiting, `Pending` → not waiting,
  `Ready(Some)` → waiting, end → not waiting)
- [ ] 4.2 Add the `watchdog(state, send_timeout)` future and run it in `finish_connection` with `tokio::select!`
  against the connection; on expiry set `no_linger` and drop the connection future; skip when upgraded or `None`;
  verify the zero-window test passes and the idle SSE, slow handler and slow download tests still pass
- [ ] 4.3 Slice HTTP/2 data frames to 16 KiB in `GuardedBody` (version passed from the `service_fn` closure in
  `serve`), keeping the remainder between polls (design.md, Decision 4); verify the one-byte-window test passes and
  the HTTP/2 tests of `tests/http2_limits.rs` and the h2c tests of `tests/transport_security.rs` still pass

## 5. Docs

- [ ] 5.1 `docs/http/performance.md`, "Timeouts and connections": add `.send_timeout(...)` to the example with its
  default, a paragraph on what it measures for HTTP/1 and HTTP/2 (blocked socket, closed flow-control window), what it
  does not touch (slow handlers, SSE, WebSockets), how it differs from Node's `server.timeout` and how to disable it;
  same in `docs/es/http/performance.md`
- [ ] 5.2 `docs/security/http-parsing.md` (slow clients: slowloris covered by the read timeouts, slow readers by
  `send_timeout`) and `docs/deployment.md` (server options paragraph); same in `docs/es/`
- [ ] 5.3 Run `npm run docs:check` in `docs/`; verify it passes

## 6. Definition of done

- [ ] 6.1 `cargo build --workspace --all-targets` with no warnings
- [ ] 6.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] 6.3 `cargo test --workspace` passes, including the regression tests of 1.2
- [ ] 6.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [ ] 6.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences
- [ ] 6.6 Re-run the proofs of concept of the security review (`window`, `ping`, `h1pipe`, `pinglong`, `mem`) against
  the branch and record in design.md that the second client is served and the memory is released after
  `send_timeout`

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #157, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

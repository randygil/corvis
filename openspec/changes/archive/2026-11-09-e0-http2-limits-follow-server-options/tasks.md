# Tasks

## 1. Failing tests

- [x] 1.1 (h2 dev-dependency not needed, see design.md Implementation notes) Add `h2 = "0.4"` to `[dev-dependencies]` of `crates/corvis/Cargo.toml` (same version as the lockfile's,
  0.4.20) and a small raw-frame helper for the misbehaving clients of design.md, Decision 7 (write the preface and an
  empty SETTINGS frame, read frame headers); verify `cargo test -p corvis --test transport_security --no-run` builds
  and `Cargo.lock` gains no new crate
- [x] 1.2 Add failing tests over h2c, one per scenario of the spec delta (streams cap of two, default advertised 100,
  oversized header list with `max_header_size(8 KiB)` then a small request on the same connection, 24 KiB with the
  default, PING not answered, PING answered with a two-second stream, no PING by default, `max_connections(1)` with
  four streams, per-stream `request_timeout`, idle HTTP/2 connection, HTTP/1 unaffected); verify with
  `cargo test -p corvis --test transport_security h2` (or the new test file) that the new ones fail to compile or fail
  for the expected reason and the per-stream timeout and idle tests already pass (they document current behavior)

## 2. Server options

- [x] 2.1 Add `http2_max_concurrent_streams: u32` (default 100), `http2_keep_alive_interval: Option<Duration>`
  (default `None`) and `http2_keep_alive_timeout: Duration` (default 20 s) to `ServerOptions` in
  `crates/corvis/src/server.rs`, with setters (`http2_max_concurrent_streams` clamps to at least 1) and rustdoc that
  states each default and what it bounds; rewrite the rustdoc of `max_header_size` (both protocols, how HTTP/2 counts),
  `h2c` (remove "200 concurrent requests" and "16 KiB", point to the new options) and `max_connections` (sockets, not
  streams; the product bound); verify `cargo doc -p corvis --no-deps` has no broken links
- [x] 2.2 Move the builder setup out of `serve` into `connection_builder(&ServerOptions)` and add the HTTP/2 settings
  (`max_header_list_size`, `max_concurrent_streams`, `keep_alive_interval`, `keep_alive_timeout`) next to the existing
  timer; verify every test of 1.2 passes and the existing tests of `crates/corvis/tests/transport_security.rs` (h2c,
  framing, drain, `max_connections`) still pass

## 3. Docs

- [x] 3.1 `docs/http/performance.md`: in "Protocols", replace the 200-stream and 16 KiB text with the new defaults and
  options; in "Timeouts and connections", add the three `http2_*` options to the example, a paragraph on how each
  timeout applies to HTTP/2 (connection idle while no stream is in flight, `request_timeout` per stream, PING
  keep-alive for long streams), and the bound `max_connections × http2_max_concurrent_streams`; same in
  `docs/es/http/performance.md`
- [x] 3.2 `docs/security/http-parsing.md` (the h2c and header-size bullets: `max_header_size` now covers both
  protocols, `431` on an oversized HTTP/2 header list, the stream cap) and `docs/deployment.md` (the server options
  paragraph); same in `docs/es/`
- [x] 3.3 Run `npm run docs:check` in `docs/`; verify it passes

## 4. Definition of done

- [x] 4.1 `cargo build --workspace --all-targets` with no warnings
- [x] 4.2 `cargo clippy --workspace --all-targets -- -D warnings` passes
- [x] 4.3 `cargo test --workspace` passes, including the regression tests of 1.2
- [x] 4.4 Docs updated in English and Spanish (`docs/` and `docs/es/`) and `npm run docs:check` passes in `docs/`
- [x] 4.5 Framework runtime code changed: run the `corvis-parity` check and verify the three port apps keep 0 apidiff
  differences (they speak HTTP/1 only)
- [x] 4.6 Leave a note on #117 (or its change, if drafted) that its TLS accept path must use `connection_builder` so
  ALPN `h2` connections get these limits

## Workflow follow-up

- Archive the change with `/opsx:archive` after review.
- Close issue #150, tick it in epic #1 and move its card to Done (see the `corvis-task` skill).

# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state, verified in the code at `af99b71`
(paths under `crates/corvis/` unless stated) and in the locked dependencies (`Cargo.lock`: hyper 1.12.0, hyper-util
0.1.21, h2 0.4.20; sources read from `~/.cargo/registry/src`).

**Corvis server**

- `ServerOptions` (`src/server.rs:45-53`): `headers_timeout`, `request_timeout`, `keep_alive_timeout`,
  `max_connections`, `max_header_size`, `h2c`, `insecure_http_parser`; defaults at `:55-67` (60 s, 300 s, 5 s, none,
  32 KiB, `false`, `false`). `max_header_size` is clamped to at least 8 KiB (`:103-106`) and documented as "HTTP/1"
  (`:101-102`). The `h2c` rustdoc (`:107-115`) states the 200-stream and 16 KiB limits this change removes.
- `serve` (`src/server.rs:811-885`) builds one `hyper_util::server::conn::auto::Builder` per listener (`:819`). The
  HTTP/1 side gets `ignore_invalid_headers`, a timer, `header_read_timeout(headers_timeout)` and
  `max_buf_size(max_header_size)` (`:823-828`); the HTTP/2 side only gets a timer (`:829`). `max_connections` is a
  semaphore acquired per accepted socket (`:830`, `:833-839`); the permit lives in `TimedIo` until the socket is closed
  (`:481-482`, `:525-532`).
- Each request runs through `service_fn` (`:852-870`), which creates an `InflightGuard` (`:416-435`) incrementing
  `ConnState::inflight` for the connection until the response body is dropped (`GuardedBody`, `:438-457`). For HTTP/2
  that means one guard per stream, all on the same `ConnState`.
- `TimedIo::poll_idle` (`:617-648`) applies `headers_timeout` (phase `Head`) or `keep_alive_timeout` (phase
  `KeepAlive`) only while `inflight == 0` (`:622-629`); `on_read` (`:598-615`) moves `KeepAlive` to `Head` on any byte,
  which for HTTP/2 includes PING, SETTINGS and WINDOW_UPDATE frames. So an HTTP/2 connection with one stream in flight
  has no connection timer, and an idle one is closed after `keep_alive_timeout`, or after `headers_timeout` once it
  sends any frame.
- `request_timeout` becomes a per-request deadline in `from_hyper` (`:266-274`, called from `handle` at `:367`), so it
  already applies to each HTTP/2 stream's body read on its own.
- With h2c off, `TimedIo::on_read` closes a connection that sends the full preface (`:603-611`); the #146 framing check
  switches to passthrough on the preface (`src/server/h1_framing.rs:19`, `:46-47`, `:288-294`). Neither depends on the
  HTTP/2 builder's settings.
- `finish_connection` (`:549-562`) drains (`linger_close`, up to 2 s / 1 MiB) unless the server is punishing the peer
  (`no_linger`, upgraded, destroyed, or a hyper timeout per `is_hyper_timeout`, `:535-546`).
- Existing h2c tests: `h2c_can_be_enabled` (`tests/transport_security.rs:634-648`) uses
  `hyper::client::conn::http2::handshake` with prior knowledge; `silent_and_slow_header_connections_are_closed`
  (`:563-581`) checks an idle h2c connection is closed.
- Docs: `docs/http/performance.md:17-22` (h2c limits: 200 streams, 16 KiB), `:30-51` (timeouts and
  `max_connections`); `docs/security/http-parsing.md:99-102`; `docs/deployment.md:94-96`; copies under `docs/es/`.

**hyper / h2 behavior**

- hyper's HTTP/2 server config defaults (`hyper-1.12.0/src/proto/h2/server.rs:36-41`, `:63-79`):
  `max_concurrent_streams: Some(200)`, `max_header_list_size: 16 KiB`, `keep_alive_interval: None`,
  `keep_alive_timeout: 20 s`, `max_send_buffer_size: 400 KiB`, `max_local_error_reset_streams: 1024`,
  `max_pending_accept_reset_streams: None` (h2's own default, 20). The server sets `keep_alive_while_idle: true`
  (`:169`): with an interval, PINGs are sent whether or not streams are open.
- On a missing PING ack, hyper calls `abrupt_shutdown(NO_ERROR)` (`server.rs:350-353`); the error is not a
  `TimedOut`, so `is_hyper_timeout` does not see it.
- hyper-util's `auto::Http2Builder` exposes `max_concurrent_streams`, `keep_alive_interval`, `keep_alive_timeout`,
  `max_header_list_size`, `max_send_buf_size` and the reset-stream limits
  (`hyper-util-0.1.21/src/server/conn/auto/mod.rs:953-1080`).
- h2 applies `max_header_list_size` to the codec at the handshake (`h2-0.4.20/src/server.rs:395-397`), before the
  peer acknowledges SETTINGS; `max_concurrent_streams` bounds remotely initiated streams from the start
  (`proto/connection.rs:149-151`). A header block at or over the limit is still decoded (to keep HPACK in sync), and the
  server answers that stream `431` and resets it (`proto/streams/recv.rs:221-238`, `proto/streams/streams.rs:555-563`);
  a block over 4× the limit ends the connection with `COMPRESSION_ERROR` (`frame/headers.rs:17`, `:873-884`;
  `codec/framed_read.rs:350-366`). The size counts name + value + 32 per field (`frame/headers.rs:1038-1039`).
- A HEADERS frame without END_HEADERS must be followed only by its CONTINUATION frames on that connection, and h2
  caps the number of CONTINUATION frames per block (`codec/framed_read.rs:130`); a client that stalls a block stalls
  only its own connection.

## Goals / Non-Goals

**Goals:**
- Every `ServerOptions` bound that has an HTTP/2 counterpart applies to HTTP/2, with defaults no looser than hyper's
  except where the option is already documented (header size).
- Dead peers behind long-lived HTTP/2 streams can be detected.
- One place configures HTTP/2 for every listener, so #117 inherits it.

**Non-Goals:**
- Server-wide request caps, per-stream header timeouts, flow-control or send-buffer tuning (proposal, Out of scope).

## Decisions

1. **Map `max_header_size` onto `max_header_list_size`.** `http2().max_header_list_size(max_header_size)` (after the
   8 KiB clamp, saturating to `u32::MAX`). HTTP/1 counts raw head bytes and HTTP/2 counts decoded fields plus 32 each,
   so the same number is comparable, not byte-identical; the docs say so. The HTTP/2 default rises from 16 KiB to
   32 KiB, the value already documented for the option.
   Alternatives considered: a separate `http2_max_header_list_size` option (one more knob for the same concern, and
   the issue asks for the mapping); keeping 16 KiB for HTTP/2 unless the option is set (a default that depends on
   whether the setter was called).
2. **`http2_max_concurrent_streams(n: u32)`, default 100, clamped to at least 1.** RFC 9113, section 6.5.2,
   recommends a value no smaller than 100 so as not to limit parallelism; nginx uses 128, Go 250, hyper 200. 100 halves
   the per-connection fan-out while staying within the RFC's advice. A cap of 0 would refuse every request, so it is
   clamped like `max_connections(0)` is (`:830`).
   Alternatives considered: keep 200 and only expose the option (the issue asks for a lower default); `Option<u32>`
   with `None` for no limit (an unbounded fan-out per socket is what the issue warns about).
3. **PING keep-alive as two options, off by default.** `http2_keep_alive_interval(impl Into<Option<Duration>>)`
   (default `None`) and `http2_keep_alive_timeout(Duration)` (default 20 s, hyper's), named after hyper's builder
   methods with an `http2_` prefix so they do not collide with `keep_alive_timeout` (HTTP/1 idle time). Off by
   default because Node's `http` has no equivalent and PINGs cost a frame per interval per connection; the docs
   recommend it with SSE or other long streams over HTTP/2.
   Alternatives considered: on by default (for example every 30 s). Left as Open question 1. Reusing
   `keep_alive_timeout` as the PING interval. Rejected: it means idle time between requests, a different thing.
4. **`max_connections` stays a socket count.** Node's `server.maxConnections` counts sockets for HTTP/2 servers too,
   and the stream cap of Decision 2 already bounds requests per socket; the docs give the product
   `max_connections × http2_max_concurrent_streams`.
   Alternative considered: count streams against a shared semaphore. Rejected: a stream waiting for a permit would
   hold its connection's flow-control window and the h2 accept loop, and hyper has no hook to queue streams before the
   service is called; a server-wide request cap is a separate feature (Out of scope).
5. **Timeouts: document, test, and add nothing per stream.** The connection timers keep their meaning ("no stream in
   flight" is HTTP/2's idle). `request_timeout` already applies per stream (Context); a test proves it over h2c. A
   per-stream headers timeout is not needed: header blocks cannot interleave, a stalled block blocks only the
   connection that sent it, and while that connection has no other stream in flight `headers_timeout` closes it; with
   a stream in flight the client only delays its own requests, and the PING keep-alive covers a dead peer.
   Alternative considered: parse HTTP/2 frames in `TimedIo` to time each header block. Rejected: a second frame
   parser in the IO layer for a case that harms only the sender.
6. **One builder function.** `fn connection_builder(options: &ServerOptions) -> auto::Builder<TokioExecutor>` holds
   both protocols' settings (moved out of `serve`, `:819-829`); `serve` calls it once. #117's TLS accept path calls the
   same function, so ALPN `h2` connections get the same limits. The function sets the HTTP/2 options even when h2c is
   off, since TLS will negotiate HTTP/2 without the h2c flag.
7. **Tests over h2c, one per limit.** In `tests/transport_security.rs` next to the h2c tests (or a new
   `tests/http2_limits.rs` if that file grows too much), with hyper's HTTP/2 client and, where a client must misbehave,
   raw frames over a `TcpStream`:
   - streams: a handler that increments an `AtomicUsize`, records the maximum, and waits on a `Notify`; three
     concurrent requests with a cap of 2 never see more than 2;
   - default stream cap: read the server's SETTINGS with an `h2` client (`h2::client::Connection::
     max_concurrent_send_streams()` after the handshake), so `h2` becomes a dev-dependency (already in the graph);
   - header list: 12 KiB header with `max_header_size(8 KiB)` → `431`, then `200` on the same connection; 24 KiB with
     the default → `200`;
   - PING: a raw client that sends the preface and an empty SETTINGS frame and never acks; expect a PING frame, then
     end of stream within interval + timeout + the 2 s drain bound;
   - PING answered: hyper client, streamed response held two seconds, connection stays open;
   - `max_connections(1)` with 4 streams on one connection served concurrently, a second connection queued;
   - per-stream `request_timeout`: unfinished `POST` body → `408`, sibling `GET` → `200`.

## Risks / Trade-offs

- [Clients that open more than 100 streams wait longer] → only h2c users are affected (opt-in, documented); the
  option raises it.
- [Header limit rises from 16 KiB to 32 KiB for HTTP/2] → it matches the documented default; operators who want less
  set `max_header_size`, which now bounds both protocols.
- [A connection closed by the PING timeout is drained for up to 2 s] → hyper ends it with `abrupt_shutdown`, which
  `finish_connection` does not recognize as a punishment, so the socket and its permit are held for at most
  `LINGER_MAX_TIME` (2 s) more. Bounded; the test allows for it. Marking it `no_linger` would need hyper to expose the
  cause.
- [PINGs on idle connections] → `keep_alive_while_idle` is fixed to `true` in hyper's server; idle connections are
  closed by `keep_alive_timeout` (5 s by default) long before a typical interval, so the extra frames only appear when
  the operator disables the idle timeout.
- [Memory per connection with slow readers] → up to 100 streams × hyper's 400 KiB send buffer; unchanged by this change
  apart from the lower stream cap, and listed as a possible issue (write-progress timeout) in the proposal.

## Open questions

1. **PING default.** Keep the PING keep-alive off by default (proposed), or turn it on (for example 30 s interval,
   20 s timeout) so SSE over HTTP/2 detects dead peers without configuration?
2. **Stream cap default.** 100 (proposed, RFC 9113's floor) or another value such as 128 (nginx)?
3. **Option names.** `http2_max_concurrent_streams`, `http2_keep_alive_interval`, `http2_keep_alive_timeout` on
   `ServerOptions` (proposed), or a nested `Http2Options` passed with `ServerOptions::http2(..)`?

## Review notes (2026-11-09)

- Q1: PING keep-alive off by default; opt in with `http2_keep_alive_interval`.
- Q2: 100 concurrent streams per connection.
- Q3: flat `http2_*` methods on `ServerOptions`.

## Implementation notes

- The tests are in the new `crates/corvis/tests/http2_limits.rs`. The `h2` dev-dependency of task 1.1 was not
  needed: the advertised stream cap is read from the server's SETTINGS frame with the raw-frame client the PING tests
  already use, so `Cargo.toml` and `Cargo.lock` are unchanged.
- Tests that open streams concurrently first complete one request, so the client has the server's SETTINGS: a stream
  opened before them is refused with `REFUSED_STREAM` (correct HTTP/2 behavior, not a server fault).
- The tests were written together with the implementation, not run red first.
- `connection_builder` is `pub(crate)` in `server.rs`; the HTTP/2 settings are applied whether or not h2c is on.
- Rapid reset (security review, finding 3): hyper starts a handler for every accepted stream, so a client that
  resets stream N-cap before opening stream N keeps the peer's own count under the cap while handlers pile up. Fix, as
  Go does: a per-connection semaphore with `http2_max_concurrent_streams` permits, acquired in the service before the
  handler runs and held until the service future is dropped; and a per-connection count of service futures dropped
  before producing a response. More than 500 such cancellations within a 10 s window close the connection without a
  drain (`CANCEL_LIMIT`, `CANCEL_WINDOW`). The threshold is far above ordinary client aborts on one connection (a
  gateway aborting a few requests per second) and far below an attack (the PoC reached about 2,900 per second).
  Measured by the regression test: the connection was closed at stream 505 with at most 2 handlers at once (cap 2).
- Zero durations (finding 4): a zero `http2_keep_alive_interval` is stored as `None`; `http2_keep_alive_timeout` is
  clamped to at least 1 s.
- Known limitations moved to #157 and documented, not fixed here: a peer that never reads holds the connection (no
  write deadline, and `inflight > 0` disables the timers), and an unread PING flood blocks writing so neither the idle
  timeouts nor the PING timeout fire.
- Task 4.5 (parity check) is left to the maintainer's session; the note on #117 (task 4.6) is posted.

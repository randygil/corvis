# Design

## Context

See proposal.md for motivation; the behavior is in the spec delta. Current state verified at `510576f` (paths under
`crates/corvis/` unless stated), in the #150 branch (`worktree-agent-a5dad9b32ed26a489`, commit `a98c60f`), in the
locked dependencies (`Cargo.lock`: hyper 1.12.0, hyper-util 0.1.21, h2 0.4.20, tokio 1.54.1; sources read from
`~/.cargo/registry/src`) and in Node.js v22.11.0 (`lib/` and `doc/api/` at tag `v22.11.0`).

**Corvis server (main)**

- `ServerOptions` (`src/server.rs:45-53`, defaults `:55-67`): `headers_timeout` 60 s, `request_timeout` 300 s,
  `keep_alive_timeout` 5 s, `max_connections`, `max_header_size`, `h2c`, `insecure_http_parser`. No option bounds
  writing.
- `request_timeout` becomes a deadline in `from_hyper` (`:274`) that bounds only the body read (`:292-296`) and the
  multipart/drain reads (`PendingBody::deadline`, `:190-194`, `:324-338`).
- `ConnState` (`:393-413`) counts `inflight` requests; `InflightGuard` (`:416-435`) increments it in the `service_fn`
  closure (`:852-853`) and decrements it when `GuardedBody` (`:438-456`), the response body handed to hyper (`:867-869`),
  is dropped. `GuardedBody::poll_frame` (`:447-449`) only delegates.
- `TimedIo` (`:478-498`) wraps the `TcpStream`. `poll_idle` (`:617-648`) returns `Pending` while the connection is
  upgraded (`:619-621`) or `inflight > 0` (`:622-629`); otherwise it applies `headers_timeout` or `keep_alive_timeout`
  and, when one expires, sets `no_linger` and fails the read with `TimedOut` (`:640-644`). It is called only from the
  read path (`:707`, `:717`, `:723`, `:765`).
- `AsyncWrite for TimedIo` (`:771-808`): `poll_write` and `poll_write_vectored` delegate and, on progress, move the
  `KeepAlive` start forward (`:775-783`, `:795-802`); `poll_flush` and `poll_shutdown` delegate. No write deadline.
- `finish_connection` (`:549-562`) awaits the connection, then closes the socket left by `TimedIo::drop`
  (`:525-532`) with `linger_close` (`:510-523`, up to 2 s / 1 MiB) unless `no_linger`, `destroyed`, `upgraded` or a
  hyper timeout (`is_hyper_timeout`, `:535-544`).
- `serve` (`:811-885`): one task per connection, `tokio::spawn(finish_connection(conn_state, conn))` (`:874`), the
  connection wrapped by `graceful.watch` (`:873`). On shutdown, `graceful.shutdown()` is awaited for at most 10 s
  (`:880-883`); connection tasks still running after that are not cancelled by `serve`.

**#150 branch (`a98c60f`), which this change builds on**

- Adds `http2_max_concurrent_streams` (default 100), `http2_keep_alive_interval` (default off) and
  `http2_keep_alive_timeout` (20 s) to `ServerOptions` (`src/server.rs:53-55`, `:68-70`), moves the builder into
  `connection_builder(&ServerOptions)` (`:847-866`) and adds `tests/http2_limits.rs` with a raw-frame `RawClient`
  (`:110-147`). The structures this change touches move by about 35 lines (`ConnState` `:429`, `GuardedBody` `:473`,
  `TimedIo` `:513`, `finish_connection` `:584`, `poll_idle` `:652`, `AsyncWrite` `:806`, `serve` `:869`) but are not
  modified by #150. Its design lists "a write-progress timeout for peers that stop reading responses" as a possible
  issue; this is that issue.

**hyper / h2 behavior relevant to the design**

- h2's connection loop calls `poll_ready` before reading the next frame (`h2-0.4.20/src/proto/connection.rs:385`);
  `FramedWrite::poll_ready` flushes and returns `Pending` while its buffer is full (`codec/framed_write.rs:110-120`).
  So with blocked writes h2 never reads again, which is problem 2 of the issue. When the task is woken it flushes
  again through `TimedIo::poll_write` (`framed_write.rs:138-174`), so a timer registered by `TimedIo` reaches the write
  path.
- hyper's h1 dispatcher polls writes in `poll_loop` (`hyper-1.12.0/src/proto/h1/dispatch.rs:166-210`) and flushes in
  `poll_flush` (`:450`).
- hyper's HTTP/2 body pump (`hyper-1.12.0/src/proto/h2/mod.rs:142-245`) polls the body first (`:195`), then reserves
  one byte of capacity and waits for stream capacity (`:233`, `:160-172`) before `send_data` of the **whole** chunk
  (`:178-181`), then polls the body again. With `SETTINGS_INITIAL_WINDOW_SIZE=0` the capacity never comes: the body
  has produced one frame and is never polled again (problem 1). With a window of one byte the whole chunk is handed to
  h2's send buffer, the body ends and is dropped (so `inflight` falls) while the data stays buffered.
- Each HTTP/2 stream's pump runs as its own task (`hyper-1.12.0/src/proto/h2/server.rs:316`). Dropping the h2
  connection notifies every stream (`h2-0.4.20/src/proto/connection.rs:686-689`, `recv_eof(true)`), so the stream
  tasks end and drop their bodies.

**Node.js (the reference)**

- `server.timeout`: "The number of milliseconds of inactivity before a socket is presumed to have timed out",
  **default 0 (no timeout)** since v13.0.0, when it changed from 120 s (`doc/api/http.md`, `server.timeout`,
  nodejs/node#27558). It is applied with `socket.setTimeout(server.timeout)` on each connection
  (`lib/_http_server.js:672-674`), and on expiry the socket is destroyed unless a `timeout` listener exists
  (`socketOnTimeout`, `:785-794`).
- The socket timer measures inactivity in both directions: it is refreshed on every read (`onStreamRead`,
  `lib/internal/stream_base_commons.js:172`), when a write is dispatched (`Socket.prototype._writeGeneric`,
  `lib/net.js:951`) and when a write completes (`onWriteComplete`, `stream_base_commons.js:101`). A write the kernel
  never accepts produces no completion, so the timer expires `server.timeout` after the last progress. This is
  "lack of progress", not total time.
- `requestTimeout` (300 s) and `headersTimeout` (60 s) only cover receiving the request: `checkConnections`
  (`_http_server.js:640-654`, every `connectionsCheckingInterval`, 30 s) expires connections whose request is still
  incomplete. `keepAliveTimeout` (5 s) is armed only when a response finished and nothing else is queued
  (`:1010-1015`) and reset to `server.timeout || 0` when the next request arrives (`:1156-1161`).
- So with Node's defaults a client that stops reading is never timed out: responses written with `res.write` keep
  buffering in the socket's writable queue (backpressure is advisory), and only a non-zero `server.timeout` (or
  `res.setTimeout`) closes the socket.
- HTTP/2 (`lib/internal/http2/core.js`): `Http2SecureServer` and `Http2Server` set `timeout = 0` (`:3158`, `:3205`); when
  set, each session gets `session.setTimeout` (`:3087-3088`) and is destroyed on expiry (`sessionOnTimeout`,
  `:3026-3033`). Stream activity refreshes the session timer (`Http2Stream[kUpdateTimer]`, `:2000-2007`).
- nginx's `send_timeout` (default 60 s): "a timeout for transmitting a response to the client. The timeout is set only
  between two successive write operations, not for the transmission of the whole response. If the client does not
  receive anything within this time, the connection is closed." That is the semantics wanted here.

**Proofs of concept** (scratchpad `h2poc/src/main.rs`, run against the #150 branch): `window_stall` (`:173-211`,
problem 1), `ping_stall` / `ping_stall_long` (`:143-171`, `:480-496`, problem 2 over HTTP/2), `h1_pipeline_stall`
(`:463-477`, problem 2 over HTTP/1), `window_mem` (`:599-621`, the 7 → 314 MiB measurement).

## Goals / Non-Goals

**Goals:**
- A client that accepts no response byte for a configurable time loses its connection, over HTTP/1 and HTTP/2, whether
  the socket is blocked or the HTTP/2 flow-control window is closed, and whether or not the connection reads.
- The deadline measures lack of progress: slow but live downloads, idle SSE streams and slow handlers are unaffected.
- On by default with a value that legitimate clients do not reach.

**Non-Goals:**
- A minimum transfer rate, a cap on buffered response bytes, per-stream resets, WebSocket write timeouts, a shorter
  timeout during shutdown (proposal, Out of scope).

## Decisions

1. **One option, `send_timeout`, default 60 s.** `ServerOptions::send_timeout(impl Into<Option<Duration>>)`, same
   shape as the other timeouts. It bounds both mechanisms below because both answer the same question: how long the
   client may refuse response bytes. The name and default are nginx's, whose semantics match (Context). Rustdoc and
   docs compare it with Node's `server.timeout`.
   Alternatives considered:
   - *Reuse `request_timeout`.* Rejected: in Node and in Corvis it bounds receiving the request (408), and 300 s is
     long for a socket that is known to be stuck; changing its meaning would surprise users of the Node option.
   - *A Node-compatible `timeout` (inactivity in both directions, default 0).* Rejected as the fix: off by default it
     fixes nothing, and on it also cuts slow handlers that write nothing for a while, which is why Node turned it off
     in v13. Corvis already has the read-side timers (`headers_timeout`, `request_timeout`, `keep_alive_timeout`).
   - *Default off, like Node.* Rejected: the issue is a remote resource exhaustion with ~70 bytes from the client
     ("security over parity"). Open question 1 confirms the value.
2. **Write deadline in `TimedIo`.** New fields `send_timeout: Option<Duration>`, `write_stalled_since:
   Option<Instant>` and `write_sleep: Option<Pin<Box<Sleep>>>` (separate from the read `sleep`, which `poll_idle`
   resets). In `poll_write`, `poll_write_vectored` and `poll_flush`:
   - `Ready(Ok(n))` with `n > 0` (or `Ready(Ok(()))` for flush) clears `write_stalled_since`: progress.
   - `Pending` sets `write_stalled_since` if unset, then polls `write_sleep` at `since + send_timeout`, so the task is
     woken at the deadline even if the socket never becomes writable. On expiry: `no_linger = true` and
     `Ready(Err(TimedOut, "send timeout"))`. hyper ends the connection with that error; `TimedIo` is dropped, and
     `finish_connection` drops the socket without `linger_close` (no GOAWAY or response tail is attempted, the socket
     is unwritable anyway).
   - Skipped when `state.upgraded` is set (decision 7) or `send_timeout` is `None`.
   This covers problem 2 for both protocols: h2's flush and hyper's h1 flush both go through `TimedIo::poll_write`
   when woken (Context). It also covers TLS once #117 lands, where `poll_flush` of the TLS stream can pend.
   Alternatives considered: evaluate `poll_idle` from the write path too. Rejected: `poll_idle` is about idleness with
   nothing in flight and is disabled exactly when a response is being written.
3. **Per-stream progress watchdog.** For a stream whose socket is not blocked (problem 1) the write deadline never
   fires, so the response bodies report their own progress:
   - `GuardedBody` gets an `Arc<BodyClock>` with `waiting_since: AtomicU64` (milliseconds since the connection's
     `epoch: Instant` plus one; `0` = not waiting for hyper). It is set at creation (the body has not been polled), set
     again when `poll_frame` returns `Ready(Some(_))`, and cleared when it returns `Pending` (the body waits for the
     application: SSE, streams) or `Ready(None)`. The clock is registered in `ConnState::clocks:
     Mutex<Vec<Arc<BodyClock>>>` on creation and removed on drop; `poll_frame` itself only touches the atomic.
   - The watchdog is a future polled by `finish_connection` next to the connection: `tokio::select!` over the
     connection and `watchdog(state, send_timeout)`. It waits on `ConnState::clocks_changed: Notify` while no clock is
     registered; otherwise it sleeps until the earliest `waiting_since + send_timeout`, or `now + send_timeout` when no
     body is waiting, and re-scans on wake. A body that starts waiting after a scan has a deadline later than the
     scheduled wake, so detection is never late; the cost is one timer per connection with responses in flight.
   - When a deadline has passed: `no_linger = true`, the connection future is dropped (that drops hyper's connection,
     `TimedIo` and, through h2's `recv_eof`, every stream task and its body), and `finish_connection` closes the
     socket without the drain. The `graceful` watcher is released by the drop as usual.
   "Waiting for hyper" is the backpressure signal for both protocols: over HTTP/1 hyper polls the body only when its
   write buffer has room, over HTTP/2 only after stream capacity is granted (Context). A body that waits for the
   application is never stalled, so idle SSE streams and slow sources are safe.
   Alternatives considered: a timer per `GuardedBody` (one `Sleep` per response, and it cannot wake the connection
   task); a spawned watchdog task per connection (a second task and a handle to abort the connection); deciding at the
   connection level ("no body made progress for `send_timeout`"). The last one is kinder to clients that pause one
   HTTP/2 stream while reading others, but a client can keep it alive by opening one stream's window a byte at a time,
   so it buys no security over the per-stream rule while making the rule harder to explain. Open question 3.
4. **HTTP/2 bodies in slices of at most 16 KiB.** hyper hands a whole data chunk to h2 as soon as one byte of capacity
   exists (Context), so a 1 MiB `Full` body would finish, drop its `GuardedBody` and leave the watchdog's sight while
   1 MiB sits behind a one-byte window (scenario "window of one byte"). `GuardedBody` therefore yields data frames of
   at most 16 KiB (the default `SETTINGS_MAX_FRAME_SIZE`) for HTTP/2 responses, slicing `Bytes` without copying and
   keeping the remainder for the next poll. The body then finishes only when the client has granted window for all but
   the last slices, and while it waits it is "waiting for hyper". The request version is known in the `service_fn`
   closure (`req.version()` before `handle`), which passes it to `GuardedBody`. HTTP/1 bodies are not sliced: hyper's
   h1 writer is bounded by the socket, which the write deadline already covers, and slicing would change chunk
   boundaries of chunked responses.
   Alternatives considered: lower hyper's `max_send_buf_size` (it limits granted capacity, not what `send_data`
   buffers); count h2's buffered bytes (not exposed by hyper).
5. **Close the connection, not the stream.** The issue asks to mark the connection for close. Resetting only the
   stalled stream would be gentler for its siblings, but the body is exactly what hyper is not polling, so an error
   from it is never seen, and hyper exposes no handle to the stream's `SendStream`. A stalled client loses the whole
   connection, which is what nginx and Node (`server.timeout`, `Http2Server.timeout`) do too.
6. **HTTP/1 and HTTP/2 share both mechanisms.** HTTP/1: the write deadline is the main path (a stalled reader blocks
   the socket); the watchdog also sees the body hyper stops polling, with the same deadline, so whichever notices first
   closes. HTTP/2: the write deadline catches blocked sockets (PING floods, many streams), the watchdog catches closed
   windows while the socket still flows.
7. **Upgraded connections are exempt.** Both mechanisms check `ConnState::upgraded`, like `poll_idle` (`:619-621`).
   After a `101` the connection carries WebSocket frames whose send queue is bounded by `max_buffered_amount`
   (`src/ws.rs:45-46`, `src/websockets.rs:371`), and pausing reading is legitimate there. The `101` response's own
   `GuardedBody` is empty and ends at once.
8. **Graceful shutdown and drain.** Both mechanisms keep running while `serve` waits for `graceful.shutdown()`
   (`:880-883`): a stalled connection that blocks hyper's graceful close (h1 waiting to finish a response, h2 waiting
   to write its GOAWAY or for its streams) ends after `send_timeout` instead of never. The 10 s cap of `serve` is not
   changed; with the default 60 s the cap still decides when `serve` returns, and the stalled tasks it leaves behind
   now end within `send_timeout` instead of living until the process exits. A connection closed by either mechanism
   skips `linger_close`: the client is not reading, so there is nothing to protect.
9. **Tests in `tests/send_timeout.rs`, timing-tolerant.** `send_timeout(1 s)` with assertion windows of several
   seconds (the transport tests' practice on Windows). Raw clients for the misbehaving cases: HTTP/1 over a
   `TcpStream`; HTTP/2 with the `RawClient` of #150's `tests/http2_limits.rs` (moved to a shared `tests/common/h2.rs`
   module or copied), extended to send SETTINGS with parameters, HEADERS with a literal HPACK block (as in the proof
   of concept, `req_block`, `:85-92`) and PING. Live clients: hyper's h1 and h2 client reading the body slowly with
   `BodyExt::frame` and sleeps. Each case of the spec delta is one test; the `max_connections(1)` probes reuse the
   `queued_client_served_after` pattern of `tests/transport_security.rs:454-464`.

## Risks / Trade-offs

- [Legitimate clients that pause reading for over 60 s lose the connection] → for example a media player that stops
  reading a progressive download, or a client in a debugger. They reconnect (browsers re-request with `Range`); nginx
  has the same default; `send_timeout(None)` or a larger value restores the old behavior. Docs say so.
- [An HTTP/2 client that pauses one stream for 60 s while reading others loses all its streams] → Decision 5; rare
  outside media, documented, and configurable. Open question 3.
- [Trickling clients defeat any progress timeout] → about 16 KiB per stream (HTTP/2) or tens of KiB (HTTP/1) per `send_timeout` keeps the
  connection; the memory a client can pin is bounded by `max_connections × http2_max_concurrent_streams` responses,
  as for a slow legitimate client. A minimum rate is out of scope.
- [Slicing relies on h2 not granting capacity while more than the window is buffered] → if a future h2 version granted
  capacity differently, the "window of one byte" scenario would regress; its regression test guards it.
- [Overhead] → one atomic store per body poll, a mutex on the clock list per response start/end, one `Sleep` per
  connection with responses in flight, one extra `Sleep` per connection whose writes pend. The HTTP/2 slicing adds a
  body poll per 16 KiB, which h2 splits into 16 KiB DATA frames anyway.
- [Timing tests flaky on Windows] → 1 s timeouts with wide windows; no assertion on an upper bound tighter than a few
  seconds.

## Open questions

1. **Default.** 60 s (proposed, nginx), 120 s (Node's pre-v13 `server.timeout`) or 300 s (same as `request_timeout`)?
2. **Name.** `send_timeout` (proposed, nginx) or `write_timeout` / `response_timeout`?
3. **Scope of the watchdog rule.** Close when any one stream is stalled for `send_timeout` (proposed, as the issue
   says), or only when no stream of the connection made progress for that time (Decision 3, alternatives)?
4. **WebSockets.** Keep upgraded connections exempt (proposed), or open an issue for a WebSocket write-progress timeout
   (for example tied to the gateway heartbeat)?

## Review notes (2026-11-09)

- Q1/Q2: the option is `ServerOptions::send_timeout`, 60 s by default (like nginx); `None` turns it off. A deliberate
  divergence from Node, whose `server.timeout` is 0.
- Q3: one stalled stream closes the whole connection, as the issue asks.
- Q4: established WebSocket connections stay exempt here; their own write deadline is filed as a separate issue.
- Implement after #150 is merged, reusing its raw-frame client.

## Review notes (2026-11-10)

- Supersedes Q3 of the 2026-11-09 notes. The connection is closed only when no response body of it has made progress
  for `send_timeout` (some body waits for hyper, and no body of the connection has had a frame taken for that long),
  not when one stream stalls. The security review measured that the per-stream rule cuts legitimate HTTP/2 clients on
  slow links with many streams (100 streams at 20 KB/s were cut at 65 s), and that it does not bound memory better: an
  RST cycle and a 16 KiB WINDOW_UPDATE trickle hold memory under either rule. Memory is tracked in #163 (a cap on
  pending response bytes). Decision 3 (per-stream clocks) and Decision 5 (close the connection, not the stream) stand;
  only the rule over the clocks changed. Progress is a poll of a body after its first one, and the stall counts from
  the later of the oldest wait and the last progress.
- Idle HTTP/2 connections: only DATA, HEADERS and CONTINUATION written by the server refresh the keep-alive phase (an
  outgoing frame scanner advanced by the bytes the socket accepted), so PONGs, SETTINGS acknowledgements and the
  server's own PING keep-alive do not keep an idle connection open. Without this a client PING every half second kept
  a connection open past 12 s where the base closed it at 2 s.
- The docs and rustdoc say what `send_timeout` does not bound: about 16 KiB per stream per period over HTTP/2, about
  64 KiB per period over HTTP/1 on a Windows loopback connection, nothing for an RST cycle; they point to #163 and
  recommend `max_connections`.
- Timeouts that cannot be added to an `Instant` (`Duration::MAX`) are off: `checked_add` in the write deadline, the
  watchdog, `poll_idle` and the request deadline, and the options handed to hyper (`header_read_timeout`, the HTTP/2
  keep-alive interval and timeout) are filtered the same way, because hyper panics on the overflow itself.

## Implementation notes

- Implemented as designed: `send_timeout` (60 s, `None` off), the write deadline in `TimedIo` (`poll_write_deadline`, a
  second `Sleep`), the per-body `BodyClock` with the connection watchdog in `finish_connection`, and 16 KiB slicing of
  HTTP/2 data frames in `GuardedBody`. `finish_connection` takes the `send_timeout` as a new parameter.
- `poll_flush` clears the stall only when the last write was not pending (`write_blocked`): a TCP flush completes
  trivially and must not restart the clock of a blocked write.
- Keep-alive phase fix (comment on #157): done, because `on_read` was reworked anyway. `TimedIo` now matches the HTTP/2
  preface on every connection (rejecting it when h2c is off, as before) and, with h2c on, walks the client's frame
  boundaries (`H2Frames`); only a HEADERS frame moves an idle connection from the keep-alive phase to the headers
  phase, so WINDOW_UPDATE, PING, SETTINGS and the like keep `keep_alive_timeout`. HTTP/1 is unchanged (any byte starts
  the next head). Test: `stray_http2_frames_do_not_end_the_keep_alive_phase`.
- Tests are in `tests/send_timeout.rs` and, for the WebSocket case (feature `websockets`), `tests/send_timeout_ws.rs`.
  The raw HTTP/2 client was copied and extended (SETTINGS parameters, PING answers) instead of moved to a shared module,
  to leave `tests/http2_limits.rs` untouched.
- Stalled HTTP/1 clients use `/t/forever` (an endless stream), not the 8 MiB route of the spec: on a Windows loopback
  connection the kernel buffers swallowed a whole 8 MiB response while the client did not read, so the server never
  blocked and the test proved nothing. HTTP/2 stalls are application-level (flow control) and keep `/t/huge` and
  `/t/big`. Each stall test has a twin with `send_timeout(None)` that shows the slot is not released, and the
  WebSocket test was checked to fail when the upgraded exemption is removed.
- Slow live reader: the server sees progress when the OS reports the socket writable. With a tiny receive buffer a
  reader that takes 4 KiB every 400 ms was cut by a 1 s timeout (the socket became writable only after tens of KiB were
  read), while 128 KiB per 400 ms was not; HTTP/2 readers (flow control) are exact. The HTTP/1 test therefore reads what
  its socket holds every 400 ms, and the docs say that progress is observed in steps for very slow readers (the 60 s
  default is far above the time such a reader needs for a step).
- Compression: the test uses `max_concurrent(1)`; while a stalled client holds the slot a second client gets an
  uncompressed response, and after `send_timeout` it is compressed again.
- Mutation checks done: without slicing the one-byte-window test fails; without the write deadline the HTTP/1
  pipelined and PING flood tests fail; without the watchdog the zero-window test fails.
- Not done here: task 6.5 (parity) and 6.6 (security review proofs of concept re-run) belong to the maintainer's
  session; the regression tests reproduce both scenarios of the review (zero window, never-read PING flood, and the
  HTTP/1 pipelined variant).

### Implementation notes, review of 0c659e1

- The new rule is implemented with `ConnState::progress` (set when hyper polls a body that is waiting for it, after the
  body's first poll) and a watchdog that computes `max(oldest wait, last progress) + send_timeout`. Unit tests with
  paused time cover one body moving next to a stalled one, all stalled, bodies waiting for the application, and
  `Duration::MAX`; integration tests cover the h2c scenarios.
- The outgoing scanner reuses `H2Frames`, which now returns the kinds of frame that own the bytes that pass (a frame
  header counts once complete, then its payload bytes). Mutation check: counting every write as response progress
  makes the three idle-connection tests fail.

### Implementation notes, Linux CI (`http1_slow_but_live_reader_is_not_cut`)

- Failure: on ubuntu-latest a reader taking 128 KiB per 400 ms was cut by a 1 s timeout after 1.79 MB. Cause: for a TCP
  socket `tcp_poll` reports `EPOLLOUT` through `__sk_stream_is_writeable(sk, 1)`, which needs
  `sk_stream_wspace >= sk_stream_min_wspace` (free space of at least half of what is queued) and
  `tcp_stream_memory_free`. Autotuning had grown the send buffer to MiB and the server had filled it, so the socket was
  writable again only after about half of it drained: no visible progress for many seconds although the client read
  steadily. With the 60 s default a 30 KB/s client draining a 2 MB buffer would have been cut while reading.
- Decision: while `send_timeout` is on, the server sets `TCP_NOTSENT_LOWAT` to 64 KiB on accepted sockets (Linux and
  Android, through `socket2` with the `all` feature, because the crate forbids `unsafe`). `tcp_sendmsg` then stops
  copying once the unsent bytes reach the mark, so the queue holds at most the receiver's window plus the mark and the
  half-of-queue rule is satisfied at once; `tcp_stream_memory_free` with the wake-up shift (`tcp_poll` passes `wake = 1`)
  reports writable when the unsent bytes fall under half the mark (32 KiB). Progress therefore shows in steps of about
  32 KiB, independent of buffer autotuning. 64 KiB rather than 128 KiB because of that halving.
- HTTP/2 shares the socket and gets the option too. Its flow control bounds what h2 writes (and the watchdog measures
  window progress, not the socket), but a client with large windows (hyper's client accepts 8 MiB) can still fill the
  kernel buffers and trip the write deadline in the same way.
- SIOCOUTQ (unsent plus unacked bytes) in the watchdog was not needed and was not added: the low-water mark restores
  writability as the progress signal, which keeps one mechanism for all platforms and avoids a Linux-only ioctl.
- macOS: `socket2` 0.6 exposes the option on Linux and Android only, so macOS and the BSDs keep the platform's autotuned
  buffers (granularity possibly MiB; documented). Windows keeps the measured behavior, about 64 KiB per period.
- The test now uses `read_exact` of a per-platform piece every 400 ms, so its rate does not depend on how much the
  client socket holds per `read`: 256 KiB per period on Linux, Android and Windows (guaranteed there: about 32 KiB on
  Linux, 64 KiB measured on Windows loopback), 8 MiB per period elsewhere. It prints the rate it exercises. A unit test
  checks that accepted sockets carry the mark (Linux and Android). Nothing was run on Linux from this branch: the
  reasoning above is from the kernel semantics, so the Linux CI run is the check.

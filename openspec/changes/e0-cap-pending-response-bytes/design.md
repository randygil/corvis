# Design

## Context

See proposal.md for motivation; the behavior is in the spec deltas. Current state verified on branch `batch/phase07-o`
(merge `da5f508`, which carries #157, #158 and #115 on top of `main` `a353e1c`); paths under `crates/corvis/` unless
stated. Locked dependencies (`Cargo.lock`): hyper 1.12.0, hyper-util 0.1.21, h2 0.4.20, bytes 1.12.1; sources read from
`~/.cargo/registry/src`.

**Corvis server**

- `ServerOptions` (`src/server.rs:47-60`, defaults `:62-79`): `send_timeout` 60 s, `max_connections` `None`,
  `http2_max_concurrent_streams` 100, `http2_max_cancelled_streams` 2000. The `send_timeout` rustdoc (`:104-131`) says
  it does not bound the response bytes held for slow clients and points to #163 (`:121-123`).
- `into_hyper` (`:233-284`) turns `Body::Json`, `Text`, `Html` and `Bytes` into `Full` bodies (exact size hint) and
  `Body::Stream` into a `StreamBody` (unknown size). `finish_response` (`:288-305`) applies compression after it
  (`:296-299`), so the server sees a `CompressedBody` whose `size_hint` is `SizeHint::default()`
  (`src/http/compression/body.rs:240-242`) while it still owns the uncompressed `Full` body (`inner`, `:118-133`).
- `handle` (`:464-522`) runs the handler and returns the finished wire response.
- The `service_fn` closure in `serve` (`:1453-1498`): `InflightGuard` and `CancelWatch` are created when hyper calls
  the service (`:1461-1462`); a flooded connection refuses the request (`:1472-1475`); the handler permit of the
  connection (`handlers`, `stream_cap` permits, `:1449`, `:1476`) is taken; the flood check runs again (`:1477-1480`);
  the handler runs (`:1482`); `watch.outcome` becomes `Completed` (or `ClientAborted`) as soon as the handler returns
  (`:1483`); the response body is wrapped in `GuardedBody` (`:1494-1496`).
- Rapid reset (`:614-691`): `CancelWatch::drop` (`:685-691`) calls `ConnState::record` (`:660-665`), which tallies the
  outcome in the 10 s window (`tally`, `:636-658`; floor 500, a quarter of completed as credit, ceiling
  `http2_max_cancelled_streams`) and on a verdict sets `flooded` and notifies `reset_flood`, which
  `finish_connection` turns into a GOAWAY and a drop after 1 s (`:1024-1032`). A stream reset by the client after its
  handler returned is never recorded as cancelled: its `CancelWatch` was already dropped with `Completed`.
- `GuardedBody` (`:718-807`) keeps the `send_timeout` clock and, over HTTP/2, hands data frames to hyper in slices of
  `H2_SLICE` = 16 KiB (`:716`, `:774-784`) split with `Bytes::split_to`, so every slice shares the allocation of the
  whole frame.
- `watchdog` (`:956-993`) closes a connection when some body waits for hyper and no body made progress for
  `send_timeout`; one moving stream keeps all others alive (#157 review notes of 2026-11-10).
- `connection_builder` (`:1391-1414`) sets the HTTP/2 header list size, concurrent streams and keep-alive; hyper's
  per-stream send buffer keeps its default (`DEFAULT_MAX_SEND_BUF_SIZE` = 400 KiB,
  `hyper-1.12.0/src/proto/h2/server.rs:39`). The HTTP/1 buffer is `max_buf_size(max_header_size)` (32 KiB by default,
  `:1401`), which hyper uses for reading and writing.
- Compression (#118): a slot is taken in `compression::apply` (`src/http/compression/mod.rs:379-421`), per connection
  (`ConnectionSlots`, `:77-107`, default 4, `max_per_connection`, `:228-237`) and globally (`max_concurrent`,
  `:215-226`), and held by the `CompressedBody` until it ends or is dropped (`body.rs:126-131`).

**hyper and h2**

- HTTP/2 body pump (`hyper-1.12.0/src/proto/h2/mod.rs:142-250`): it polls the body (`:195`), computes `is_eos` from
  `is_end_stream()` right after the frame (`:199`), reserves one byte (`:233`), waits for stream capacity
  (`:160-174`), then `send_data` hands the whole slice to h2 (`:178-181`) and, at the end of stream, returns, which
  drops the body. h2 keeps the slice in the stream's queue until the window lets it out.
- h2's stream capacity is `min(available, max_send_buffer_size) - buffered` (`h2-0.4.20/src/proto/streams/stream.rs:275-280`),
  so h2 holds at most `max_send_buf_size` plus the slice handed when capacity was positive, per stream.
- A stream with buffered data is not closed (`stream.rs:239-251`: `pending_send` empty and `buffered_send_data == 0`
  are required), so it keeps counting against `SETTINGS_MAX_CONCURRENT_STREAMS` (`counts.rs:311-323`) until its data
  is written.
- A service error resets the stream with `err.h2_reason()` (`hyper-1.12.0/src/proto/h2/server.rs:462-467`), which
  finds an `h2::Error` in the error's source chain or defaults to `INTERNAL_ERROR` (`src/error.rs:353-359`).
- A client `RST_STREAM` reaches the pump through `poll_reset` (`mod.rs:146-155`) and the pump drops the body.
- HTTP/1: the server dispatcher calls the service for the next request as soon as the previous response was taken
  for writing (`hyper-1.12.0/src/proto/h1/dispatch.rs:602-643`, `in_flight`), and polls a body only while its write
  buffer is below `max_buf_size` (`io.rs:586-593`). A `Full` body hands its whole `Bytes` in one frame; the body
  ends at the next poll, when less than `max_buf_size` of it is left to write.

**References**

- nginx: `http2_max_concurrent_streams` default 128 (`src/http/v2/ngx_http_v2_module.c:346-347`),
  `http2_chunk_size` 8 KiB (`:380`), `send_timeout` 60 s (`src/http/ngx_http_core_module.c:3977`). Response bodies go
  through bounded `output_buffers` / proxy buffers, so an upstream or file is read only as the client takes data:
  per-stream memory is bounded by buffer configuration, not by response size.
- Go `net/http2` (`golang/net` `http2/server.go`): a handler's writes go through a 4 KiB chunk writer
  (`handlerChunkWriteSize`, `:61`) and `writeDataFromHandler` (`:1013-1045`) blocks the handler until its frame is
  written, so each stream holds about one frame; `defaultMaxStreams` is 250 (`:62`); `MaxUploadBufferPerConnection`
  is the receive window (`:800`), not response memory. A handler that builds a large `[]byte` before writing still
  holds it while blocked: Go does not bound that.
- Node `http2` (`doc/api/http2.md`, `maxSessionMemory`, default 10 MB): "a credit based limit, existing Http2Streams
  may cause this limit to be exceeded, but new Http2Stream instances will be rejected while this limit is exceeded";
  queued outgoing data counts. Node's `http` module has no limit: `res.write` returns `false` above `highWaterMark`
  but keeps buffering.

**Proof of concept** (`st157b/src/main.rs`): `mixed` (`:289-322`), `rstcycle` (`:325-363`), `trickle` (`:242-286`),
`h1trickle` (`:503-542`); the route `/t/big` returns a 1 MiB `String` (`:22-24`), i.e. a `Full` body.

The key fact for the design: an in-memory response is fully allocated when the handler returns. Not polling its body
saves nothing; only not running the handler (admission) or dropping the response (reset) bounds it.

## Goals / Non-Goals

**Goals:**
- Bound the response memory one connection holds to about the cap (or one response, if larger), whatever the client
  reads: stalled, trickling, one-progressing-stream, RST cycle.
- Keep legitimate clients working: clients that read get their responses; bursts of responses up to the reserved
  share are never reset.
- Make streams reset after their handler finished cost the client in the rapid-reset rule.
- No retained allocations after a body ends (zero-copy tail).

**Non-Goals:**
- A global cap, a default `max_connections`, application-side buffers, request-side memory, a minimum rate
  (proposal, Out of scope).

## Decisions

1. **One per-connection option, `max_pending_response_bytes`, default 8 MiB.** `ServerOptions::
   max_pending_response_bytes(impl Into<Option<usize>>)`, same shape as the other options. 8 MiB is the power of two
   nearest Node's `maxSessionMemory` (10 MB); with 100 streams it reserves about 82 KiB per handler, above typical JSON
   API responses, so ordinary bursts are never reset (Decision 3). Applies to HTTP/1 and HTTP/2 alike, through the
   same accounting.
   Alternatives considered:
   - *Global cap.* Rejected for now: above it one client's stalled responses would block every connection's handlers,
     turning memory exhaustion into a server-wide stall. The global bound is `max_connections ×` the cap, which the
     docs spell out (Out of scope).
   - *Two options (soft wait, hard reset).* Rejected: the reservation of Decision 3 gives the soft behavior without a
     second number to tune.
   - *Node's unit (megabytes).* Rejected: every Corvis size option is in bytes.
2. **What counts.** `GuardedBody` gets a `charge`, accounted in a new `ConnState::budget: Mutex<Budget { pending:
   usize, running: usize, warned: bool }>` plus `budget_freed: Notify`:
   - an in-memory body (exact `size_hint` at creation, or the uncompressed length recorded by `finish_response`)
     charges its whole size until it ends or is dropped: the allocation lives that long whatever part has been sent
     (slices share it);
   - a body of unknown size (stream, SSE, compressed stream) charges the length of `rest`, the part of a cut frame not
     yet handed to hyper (0 over HTTP/1, which does not slice);
   - `finish_response` inserts a crate-private `PlainLen(u64)` extension into the hyper response before compression
     (from `size_hint().exact()` of the wire body); the service closure reads it, so compression (#118) does not hide
     the size.
   Not counted, bounded elsewhere and documented: HTTP/2 tails after the body ended (Decision 7, at most ~48 KiB per
   open stream, i.e. ~4.7 MiB with 100 streams), compression encoders (at most `max_per_connection` slots), HTTP/1's
   write buffer (Decision 5), request heads and bodies (`max_header_size`, hyper's 1 MiB receive window), and memory
   the application holds outside the body.
   Alternatives considered: count bytes handed to hyper minus bytes the socket accepted (from `TimedIo`'s outgoing
   frame scanner). Rejected: h2 discards the queue of a reset stream (`prioritize.rs:667-683`) without telling us, so
   the counter would drift upwards; and it would not see that a sent slice keeps its whole allocation alive.
3. **Admission before the handler, with a reserved share.** In the `service_fn` closure, after the handler permit and
   before the second flood check, `budget.admit(share)` waits until `pending + running × share + share <= cap`, or
   `pending == 0 && running == 0`, then increments `running`. `share = cap / limit` with `limit =
   http2_max_concurrent_streams` for HTTP/2 and 1 for HTTP/1 (the version is already known there, `:1460`). When the
   handler returns, `running` is decremented and the response's charge is added in the same critical section. Waiters
   wait on `budget_freed`, notified whenever `pending` or `running` drops. A response whose charge fits in its share
   never takes the connection above the cap, so it is never reset; only responses larger than their share can
   overshoot (Decision 4).
   Alternatives considered:
   - *REFUSED_STREAM for new streams above the cap* (Node's `maxSessionMemory` model). Rejected: a slow but legitimate
     client with many requests would get failures where waiting costs nothing (a waiting stream holds its head and its
     slot of `http2_max_concurrent_streams`, already bounded), and refusing does not bound the burst that starts the
     problem: all 100 handlers run before any response exists.
   - *Not polling bodies above the cap.* Rejected as the mechanism: in-memory bodies are already allocated; hyper
     already polls a streamed body one frame at a time.
   - *No reservation, admit whenever pending < cap.* Rejected: at the start of a burst pending is 0 for every request,
     so all handlers would run at once and the cap would only act through resets.
   - *Reserve the largest response seen on the connection.* Rejected: it can be gamed in neither direction usefully
     and limits concurrency of clients that once downloaded one large file.
4. **Overshoot is reset with `ENHANCE_YOUR_CALM` (HTTP/2).** When the handler returns a response with charge `c` and
   the connection holds `pending > 0` with `pending + c > cap`, the closure drops the response (its `Full` allocation
   and its compression permit go at once) and returns an error whose source is
   `h2::Error::from(h2::Reason::ENHANCE_YOUR_CALM)`, so hyper resets the stream with that code (Context). Its outcome is
   cancelled. The first such reset per connection logs a warning (`budget.warned`) naming the peer and the cap; later
   ones are silent so a client cannot flood the log. A response arriving when `pending == 0` is always kept.
   `h2 = "0.4"` becomes a direct dependency of `corvis`; it adds no crate (hyper already depends on h2 0.4.20).
   Alternatives considered:
   - *Keep the overshoot.* Rejected: it is exactly the burst of the RST cycle (100 × 1 MiB); a trickle then holds it.
   - *Close the connection (GOAWAY).* Rejected: it bounds memory as well, but a legitimate burst of large responses
     would lose every stream, including those already being sent.
   - *Replace it with a `503`.* Rejected: the handler already ran (a `503` invites a retry of non-idempotent requests
     just like a reset, but looks like an application answer), and an error body in the application's format needs
     flow-control window the client may not grant.
   - *REFUSED_STREAM.* Rejected: RFC 9113 §8.7 reserves it for streams with no application processing; the handler
     ran, so a client could safely retry something that already had effects.
   - *`INTERNAL_ERROR` through the existing `SocketDestroyed`.* Rejected: it blames the server; `ENHANCE_YOUR_CALM`
     is RFC 9113's code for a peer that generates excessive load, and needs no new crate.
5. **HTTP/1: one response at a time.** With a share equal to the cap, a pipelined request's handler waits until the
   previous response body ended (hyper polls it again once less than `max_buf_size` is left), so an HTTP/1 connection
   holds the response being finished and the next one, not three (today: the one in hyper's write buffer, the next
   body and the response of the request after it waiting in hyper's `in_flight`). The overshoot path is unreachable
   over HTTP/1 (`pending == 0` when the only handler returns) and is guarded by a `debug_assert!`. Total HTTP/1 memory
   stays `connections ×` response size; the docs recommend `max_connections`. Node runs pipelined handlers
   concurrently; port apps and browsers do not pipeline, so parity is unaffected (verified by the parity check).
6. **Outcome recorded when the body ends.** `CancelWatch` no longer records `Completed` when the handler returns:
   it hands the outcome to `GuardedBody` (a new field `outcome: Option<Arc<ConnState>>`), which records completed when
   the body reported its end (`poll_frame` returned `Ready(None)`, or `is_end_stream()` is true when hyper drops it,
   which covers empty, HEAD and `101` bodies and the last slice sent with end of stream) and cancelled when dropped
   before. `ClientAborted`, `Refused`, `Pending` and the error path (`SocketDestroyed`) keep their current recording.
   Same counter as #150, not a separate one: the economics are the same (the server did work nobody received), and
   the floor of 500 per 10 s and the quarter credit keep legitimate seeks and navigations far from it. A body dropped
   because the whole connection ends also records cancelled; harmless, the connection is gone.
   Alternatives considered: a separate counter for post-handler resets (one more threshold to tune, no different
   verdict); counting only resets with unsent bytes above a size (a small-response RST cycle would then be free).
7. **No zero-copy tail.** In `GuardedBody`'s HTTP/2 slicing, a slice is copied into its own allocation
   (`Bytes::copy_from_slice`) when fewer than `H2_TAIL` = `H2_SEND_BUF + H2_SLICE` bytes of the frame remain after it,
   with `H2_SEND_BUF` = 32 KiB set through `connection_builder`'s `http2().max_send_buf_size(H2_SEND_BUF)`. After the
   pump sends the last slice with end of stream (Context), h2 can still hold at most `max_send_buf_size` plus one
   slice of that stream; all of it is then a copy, so the response allocation is released when the body ends. Frames
   of at most 16 KiB are not cut and pass as they are. Cost: at most 48 KiB copied per response larger than that.
   Lowering hyper's per-stream buffer from 400 KiB to 32 KiB also removes up to 400 KiB per stream that h2 would
   buffer for a client with a large window and a blocked socket. Task 4.2 measures single-stream h2c throughput
   before and after; if it drops by more than 10 %, `H2_SEND_BUF` goes to 64 KiB (and `H2_TAIL` follows).
   Alternatives considered: copy every slice (a copy of every byte of every HTTP/2 response); keep charging the body
   until the stream closes (not observable through hyper); copy only the very last slice (h2 may hold two).
8. **Interaction with `send_timeout` (#157).** Unchanged mechanisms, complementary roles: `send_timeout` decides how
   long a stalled connection lives; this cap decides how much it holds meanwhile. Waiting requests have no body, so
   they neither feed nor block the watchdog; the kept bodies of a stalled client stall and the watchdog closes the
   connection after `send_timeout` as today. A reset overshoot has no body either. The `send_timeout` rustdoc and
   docs drop the "does not bound memory, see #163" text and point to `max_pending_response_bytes`.
9. **Interaction with compression slots (#118).** A waiting request holds no slot (slots are taken in
   `finish_response`, after the handler). A reset overshoot releases its slot at once with its body. Kept compressed
   bodies hold their slot until they end, as today, and are charged their uncompressed size (Decision 2). The
   encoders' own memory is bounded by `max_per_connection` and not charged.
10. **`None` turns off admission and resets only.** With `max_pending_response_bytes(None)` no reservation is made,
    no request waits and no response is reset. Decisions 6 and 7 (outcome at body end, tail copy and 32 KiB send
    buffer) stay on: they fix accounting and retention bugs, not policy.
11. **Tests.** `tests/response_memory.rs` with a raw h2c client (copied from `tests/send_timeout.rs`, `:161-246`, as
    #157 did) and small caps so that the scenarios run in seconds: `http2_max_concurrent_streams(20)`,
    `max_pending_response_bytes(1 MiB)`, a 256 KiB in-memory route and a route that counts its handler runs. Memory is
    asserted through behavior (which streams get HEADERS, which get `RST_STREAM(ENHANCE_YOUR_CALM)`, handler counts)
    and unit tests of `Budget`, not through RSS. Re-running `mixed`, `rstcycle` and `trickle` of the proof of concept
    with the defaults (task 6.6) records the RSS numbers in this design.

## Risks / Trade-offs

- [A legitimate HTTP/2 burst of large in-memory responses gets some streams reset] → only responses larger than the
  share (about 82 KiB with defaults) can overshoot, and only when more than 8 MiB of them are pending at once on one
  connection; large payloads should be streamed (static files are). The warning names the option to raise.
  Configurable, documented.
- [HTTP/1 pipelined handlers no longer overlap the previous response] → pipelining is rare (browsers do not use it);
  the order of responses is unchanged.
- [Admission adds a mutex per request] → one short critical section at handler start and end; no allocation.
- [Memory not counted] → HTTP/2 tails (~48 KiB per open stream), encoders, request side and application buffers:
  bounded by other limits and listed in the docs, so the per-connection bound is stated honestly.
- [A smaller h2 send buffer lowers throughput] → measured in task 4.2 with a fallback value.
- [Counting post-handler resets closes a proxy that cancels many downloads] → the #150 rule still requires 500
  cancellations in 10 s and more than a quarter of the completed requests; a proxy with mostly finished downloads is
  not affected.
- [Timing-dependent tests] → small caps and counts instead of RSS, wide time windows as in `tests/send_timeout.rs`.

## Open questions

None that needs a maintainer: default (8 MiB), behavior above the cap (wait, then reset with `ENHANCE_YOUR_CALM`),
counter (the #150 one) and the global cap (out of scope) were settled above in favor of security, then parity.

## Review notes (2026-11-09)

- Approved as drafted. The default of 8 MiB per connection, admission before the handler and the
  `ENHANCE_YOUR_CALM` reset above the cap stand; security wins over the Node `maxSessionMemory` behavior.
- The per-stream send buffer goes down to 32 KiB only if task 7 shows a throughput loss of 10 % or less;
  otherwise use 64 KiB and record the numbers here.
- Implement on top of `batch/phase07-o` (it needs `send_timeout` and the #150 counter); do not change the
  watchdog rule of #157.

## Implementation notes

Implemented on top of `batch/phase07-o` (`f09eb10`). Decisions and deviations:

- **HTTP/1 accounting differs from Decision 5.** hyper's HTTP/1 dispatcher takes a whole `Full` frame in one poll and
  the body ends right then (`dispatch.rs`, `poll_write`: the frame is queued in the write buffer and the body is
  cleared as soon as `is_end_stream()` is true). Charging "until the body ends" would release an 8 MiB response while
  all of it still sits in hyper's write queue, and the next pipelined handler would start at once. Over HTTP/1 a
  body's charge is therefore kept until the socket has accepted what was handed to hyper: `ConnState` gets `handed`
  (data bytes of response bodies given to hyper), `written` (bytes the socket accepted, fed by
  `TimedIo::write_progress`) and `deferred` (`(handed mark, charge)` in order); `GuardedBody::drop` defers the release
  to the mark and `ConnState::wrote` releases what the socket has passed. `written` also counts response heads and
  chunk framing, so a mark can be reached a few hundred bytes early per request, irrelevant against the sizes this
  bounds. Streamed HTTP/1 bodies stay uncharged (hyper pulls frames as the socket drains; an application frame of
  several MiB is queued whole, which is application memory and is listed in the docs). HTTP/2 is as designed.
- **Waiting versus resetting.** Admission runs before the handler, so a burst of fast (synchronous) handlers on one
  connection is serialized by the reservation: the responses that fill the cap are kept and the later requests wait
  without a handler; they are not reset. Resets (`ENHANCE_YOUR_CALM`) happen when handlers run concurrently (slow
  ones: each handler of the burst was admitted because its share fits) and return together, which is what the burst
  scenarios of the spec describe. The memory bound is the same in both cases. The integration tests that expect
  resets use routes that sleep 500 ms (`/t/slow-quarter`, `/t/slow-big`); the one that expects waiting uses a
  synchronous route.
- **Outcomes.** New `Outcome::Dropped` records the dropped response as cancelled; `Outcome::Body` hands the record to
  `GuardedBody` (completed when the body ends or `is_end_stream()` is true at drop, cancelled when it is dropped
  earlier). Requests the client aborted while their body was read (`ClientAborted`) keep the old recording and their
  response body records nothing.
- **Error type.** The service closure returns `ServiceError { Destroyed, OverCap(h2::Error) }`; the `source()` of
  `OverCap` is the `h2::Error`, so hyper resets the stream with `ENHANCE_YOUR_CALM`. `Cargo.lock` only gains `"h2"` in
  the dependencies of `corvis` (no new package). The warning goes through `Logger::warn_with(.., "HttpServer")`, once
  per connection (`Budget::warned`).
- **Tests.** `tests/response_memory.rs` (11 tests; the compression one needs the `compression` feature), two tests in
  `tests/http2_limits.rs`, and unit tests in `server.rs` (budget, charges, tail copy, outcome recording). They were
  written after the code, so "fails before" was not run; the scenarios that depend on the change (burst, waiting, RST
  cycle closing the connection, reset after HEADERS closing it) cannot pass with the limit off or the old outcome rule.
  Not covered by an integration test: "large window, blocked socket" and "window update trickle after the body ended"
  (they need h2's internal queue; unit tests cover the slicing, the copies and the 32 KiB builder option), and an
  HTTP/1 pipelined client that stalls the socket (a Windows loopback socket took more than 128 MiB without the client
  reading), so the HTTP/1 test asserts that handlers never overlap and the wait for a response still being written
  is covered by `an_http1_body_stays_charged_until_the_socket_accepted_its_bytes`.
- A waiting request reset by the client is shown to count as cancelled through `http2_max_cancelled_streams(3)`: four
  such resets close the connection and none of the waiting handlers ran.
- **Existing test changed.** `send_timeout.rs::a_stalled_stream_next_to_a_moving_one_is_not_cut_but_all_stalled_is`
  held an unread 8 MiB response (`/t/huge`) and then opened a second stream. With the default 8 MiB cap that unread
  response holds the whole budget, so the second request waits (by design) and `send_timeout` cut the connection
  before it was answered. The stalled stream now uses `/t/big` (1 MiB); the assertion about progress is unchanged.
  The same head-of-line effect applies to a real client that downloads one very large in-memory response while
  opening other streams: they wait until it ends (documented under "Large payloads belong in streamed bodies").
- The parity check (task 7.5) was not run: the implementation brief leaves it to the maintainer's session.

### Measurements

Task 4.2: single-stream h2c download of 64 MiB on loopback (release build, h2 client with 16 MiB and 64 MiB windows),
median of five runs, three series each:

| | MiB/s |
|---|---|
| before (hyper's 400 KiB send buffer, no tail copy) | 327, 333, 315 |
| after (32 KiB send buffer, tail copy) | 308, 306, 300 (298 and 307 in two earlier series) |

The drop is about 6 %, below the 10 % threshold, so `H2_SEND_BUF` stays at 32 KiB.

Task 7.6: the proof of concept (`st157b`, run against this branch with the default options, `send_timeout` 60 s,
release build; RSS before the scenario 7 MiB):

| Scenario | Before (issue #163) | After, peak RSS |
|---|---|---|
| `mixed` (100 streams of 1 MiB, window 0, 16 KiB per 1.5 s to stream 1, 10 s) | about 103 MiB | 18 MiB, connection alive, 8 responses kept |
| `rstcycle` (100 streams per round, reset and reopened every 1.5 s, 10 s) | 114 MiB | 20 MiB; connection closed after 10.6 s by the rapid-reset rule |
| `trickle` (16 KiB per stream per second, 100 streams, 10 s) | about 100 MiB | 18 MiB |
| `h1trickle` (64 KiB per second, `/t/big` of 1 MiB) | one response kept | 8 MiB |

That is 11 to 13 MiB above the idle baseline against 103 to 114 MiB: the 8 MiB the cap allows plus transient
allocations of handlers whose responses were dropped.

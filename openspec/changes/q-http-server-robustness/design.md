# Design

## Context

Source: the engineering-quality review of the HTTP layer, the server and the multipart module (issue filed from
`quality/q-http-server-robustness.issue.md`, epic #187). Code read on `dev` at `d46758a` (2026-11-11): `tokio` 1.54,
`hyper` 1.x, `hyper-util` 0.1, `tokio-util` 0.7.20 (already in `Cargo.lock` through `h2`). The review was made by
reading only; every finding below was checked against the code, and the ones that need a running server get a failing
test as their first task.

### Findings as checked

Line numbers are those of `d46758a`. The findings of the review that belong to the other three changes are listed in
their own design.md; this table holds the ones of this change plus the review items this change drops.

| # | Finding | Where now | Verdict |
|---|---|---|---|
| 1 | Accept loop spins on `accept` errors | `server.rs:2301-2306` (`Err(_) => continue`), no log, no pause | Confirmed. `tokio::net::TcpListener::accept` returns `EMFILE` / `ENFILE` at once while descriptors are exhausted; the loop retries in a tight loop on one worker |
| 3 | Fixed shutdown deadline, untracked tasks | `server.rs:2347-2353` (10 s `select!`), spawns at `server.rs:2323` (plain), `2328` (TLS handshake then connection), `607` (body drain); `application.rs:648-655` and `685-691` run the hooks right after `serve` returns; `close_with_signal` at `application.rs:749-761` | Confirmed. After 10 s `serve` returns while connection tasks still run handlers; `on_module_destroy` runs next. No test drives a request in flight across the shutdown (searched `tests/` for `listen_until_in_background`: only `engine_io_ws.rs`, a polling session). Found while confirming: the drain tasks (`server.rs:607`) are spawned the same way and can outlive `serve` by up to 30 s (`server.rs:537`) |
| 10 | `@Res()` handler that never ends the response hangs | `res.rs:857-866` (`wait_end`, no bound), `router/mod.rs:730-737`; clones of `Res` live in the request extensions (`router/mod.rs:865`) and in the router (`router/mod.rs:719`); `HttpArgumentsHost::get_response` (`common/context.rs:387-389`) and the `#[res]` parameter (`common/param.rs:45`) hand out more | Confirmed that the wait has no bound and that nothing can end it once every outside handle is gone. Narrowed: the original framework waits too (the request stays open until the client leaves); what Corvis adds is that the stream keeps its handler permit (`server.rs:2174`) and its budget share meanwhile. The fix of the review (`Arc::strong_count == 1`) is wrong as written: the extension and the router each hold a clone, so the count never drops to one. Decision 3 |
| 13 | `from_hyper` never returns `Err` | `server.rs:471-529` (both paths end in `Ok(r)`), `server.rs:612` (`Err(resp) => resp`), `#[allow(clippy::result_large_err)]` at `server.rs:471` | Confirmed |
| 14 | Test client duplicates the request path and diverges | `testing.rs:383-399` against `server.rs:551-615`: no `read_limit` / `body_limit`, no 413 on `Content-Length`, no pending multipart body (`multer/mod.rs:489-493` falls back to `raw_body`), no drain, no `client_aborted` | Confirmed. Engine.IO and upgrade routing are out of reach of an in-memory client and stay out |
| 16 | `server.rs` too large | 2,897 lines; `ConnState` `server.rs:621-667`, 19 fields: activity (`inflight`, `generation`, `finished`, `waker`), connection end (`upgraded`, `destroyed`, `no_linger`, `closing`), rapid reset (`window`, `reset_flood`, `flooded`, `cancel_ceiling`), send-timeout clocks (`epoch`, `clocks`, `progress`, `clocks_changed`), response budget (`budget`, `budget_freed`, `deferred`) | Confirmed |
| — | `Res::finish` notifies twice | `res.rs:766-767` (`notify_waiters` then `notify_one`) | Confirmed. `wait_end` creates its `Notified` before it checks `headers_sent`, so `notify_waiters` suffices; the `notify_one` leaves a stored permit nobody needs. Fixed with Decision 3 |
| — | `take_response` clones the headers | `res.rs:869-875` | Confirmed (`s.headers.clone()` of a response that is never read again). Fixed with Decision 3 |
| — | Watchdog creates a timer per response | `server.rs:1055-1059` (`notify_one` per `GuardedBody`), `server.rs:1390-1430` (new `sleep_until` per wake), `retain` on drop `server.rs:1104-1105` | Confirmed as described, not acted on: no heap allocation (the `Sleep` lives in the watchdog's future), its CPU cost is not shown, and it is the mechanism of `send_timeout`, whose security review closed two rounds. Left for a follow-up if a profile shows it |
| — | HTTP/1 framing check copies the head three times and parses it twice | `server.rs:1819-1828`, `server/h1_framing.rs:506,525` | Dropped: the deliberate design of `e0-reject-ambiguous-http1-framing` (Decision 1 there: the check must see every byte before hyper does), bounded by `max_header_size`; no measurement says it matters |
| — | `headers_timeout` implemented twice | hyper's `header_read_timeout` (`server.rs:2063`) and `TimedIo`'s `Phase::Head` (`server.rs:1499,1576,1716`) | Dropped: two layers on purpose. `TimedIo` also covers HTTP/2 (a HEADERS frame starts the phase, `e0-write-deadline-for-stalled-clients` implementation notes) and the TLS path, where hyper's HTTP/1 timer does not apply; removing either would weaken the slowloris defenses (`transport_security.rs` `silent_and_slow_header_connections_are_closed`, `slowloris_on_keep_alive_connection_released_without_drain`) for no measured gain |
| — | `Gate` takes a mutex per TCP write | `server/transport.rs:152-166` | Dropped: one uncontended lock per write next to a TLS encryption, guarding four fields that must change together (`accepted`, `blocked`, the read credit, the reader's waker) |
| — | Integration tests synchronize with `sleep` | 17 in `response_memory.rs`, 13 in `http2_limits.rs`, 14 in `send_timeout.rs`, 6 in `transport_security.rs`, 2 in `http1_framing.rs` | Confirmed; a follow-up issue (proposal "Out of scope"). New tests here follow the timing rule |

Found while confirming, outside this change's files: upgraded WebSocket sessions are spawned untracked
(`websockets.rs:758`) and keep running after the shutdown hooks; `q-core-lifecycle-and-cache-correctness` (Decision 6
there) closes them from `close()` and is written against this change's `shutdown_timeout`.

## Goals / Non-Goals

**Goals:**

- A server that does not burn a core when the process runs out of descriptors, and says so in its log.
- A shutdown whose deadline the application chooses, after which nothing the server started is still running.
- No request that waits for a response nothing can produce.
- One request path, so a test written with `TestClient` checks what the server does.
- `server.rs` in files a reviewer can hold in mind, with the security bounds of epic #1 untouched.

**Non-Goals:**

- New limits or defaults: the shutdown deadline keeps 10 s, the accept backoff only applies to errors that today spin.
- Closing upgraded connections at shutdown (`q-core-lifecycle-and-cache-correctness`).
- Changing the timing of the existing integration suites (follow-up).

## Decisions

### 1. Accept errors are classified, backed off and logged

The accept step moves to `server/accept.rs`:

- `is_connection_error(&io::Error)`: `ConnectionAborted`, `ConnectionReset`, `ConnectionRefused` and `Interrupted`. These
  concern one connection that died between the kernel's queue and `accept`; the loop retries at once, as today and as
  hyper's former `AddrIncoming` did.
- Any other error: the loop waits before the next `accept`, 5 ms after the first error, doubling up to 1 s, reset by the
  next accepted connection. The wait is a `select!` with the shutdown future, so a server out of descriptors still stops
  on its signal. While it waits it holds no connection permit (the permit acquired for the failed attempt is dropped).
- Logging: `Logger::new("CorvisServer").warn(..)` with the error kind and text, at most once a minute, carrying the number
  of errors since the last line ("accept failed: Too many open files (os error 24); 1,532 errors since the last line"). The
  first error of a quiet period is always logged.
- The loop takes the listener through a small trait (`trait Acceptor { fn poll_accept(..) }`, implemented for
  `TcpListener`) so a unit test can inject errors. The trait is crate-private.

Test (unit, `server/accept.rs`, paused clock): an acceptor that fails with `EMFILE` 20 times then accepts: the attempt
times follow the schedule 5, 10, 20, ... ms capped at 1 s (read from `Instant::now()` on the paused clock, never from wall
time), one warning line for the burst (a test log sink), and after the success the next error waits 5 ms again; an
acceptor that fails with `ConnectionAborted` is retried with no wait; a shutdown signalled during a wait returns at once.

Alternative considered: a fixed 1 s pause (hyper's former behavior). Rejected: it delays the first accept after a
transient error by a full second.

### 2. A shutdown deadline the application sets, and nothing left running after it

- **Option.** `ServerOptions::shutdown_timeout(Option<Duration>)`, default `Some(10 s)`; `None` waits for every
  connection without limit; a duration that cannot be added to an `Instant` is treated
  as `None`, the rule `send_timeout` set. Rustdoc says what is cancelled and what is not (below).
- **Tracking.** `serve` owns a `tokio_util::task::TaskTracker` and a `tokio_util::sync::CancellationToken`. Every task it
  spawns goes through one helper, `spawn_tracked(fut)`, which spawns `tracker.track_future(async move { select! { _ =
  fut => {}, _ = cancel.cancelled() => {} } })`: the connection task (plain), the handshake-then-connection task (TLS)
  and the body drains of `handle` (the helper reaches `handle` through `ConnCtx`). A finished task leaves the tracker on
  its own, so the tracker holds only live tasks and needs no reaping.
- **Sequence** after the shutdown future resolves: drop the listener; close the Engine.IO sessions (as today); signal
  the graceful stop to the connections (the `watch` channel, as today); `tracker.close()`; wait for `tracker.wait()` up to
  the deadline; at the deadline, `cancel.cancel()`, which drops every remaining connection future at its next poll (hyper
  closes the socket, handler futures are dropped at their current `.await`), then wait for `tracker.wait()` again, which
  ends as soon as each task is polled once more. One warning line names the number of connections cut. Then `serve`
  returns and the application runs its hooks.
- **What is not covered**, written in the rustdoc and in `docs/fundamentals/lifecycle-events.md`: tasks the application
  spawns itself (`tokio::spawn` inside a handler), upgraded WebSocket sessions (closed by `close()` once
  `q-core-lifecycle-and-cache-correctness` lands), and code that blocks a worker
  without awaiting (it cannot be cancelled; `spawn_blocking` work runs to its end).

Tests (`tests/server_lifecycle.rs`, current-thread runtime with `start_paused`, server on `127.0.0.1`; no sleep: the paused
clock advances only when every task waits, which is exactly when the deadline should fire):

- A handler that records "started", then awaits a future that never resolves while holding a guard whose `Drop` records
  "dropped"; an `on_application_shutdown` hook that records "hook". With `shutdown_timeout(1 s)`: the order is
  started, dropped, hook (before the fix: started, hook, and "dropped" never or later). This is the failing test of task 3.1.
- A handler that finishes on a signal the test sends after the shutdown began, with the default deadline: the client gets
  its full response and the hooks run after it.
- `shutdown_timeout(None)` with the never-ending handler: after the paused clock passed 1 hour, `serve` has not returned
  (the join handle is not finished); the test then lets the handler finish and `serve` returns.
- A body drain in progress (a client sends a large body to a route that answers without reading it, then stops sending):
  it does not keep `serve` past the deadline.
- TLS: a client that opens a TLS connection and never sends a ClientHello does not keep `serve` past the deadline
  (`tests/tls.rs`).

Alternatives considered: a `JoinSet` (rejected: it keeps finished tasks until joined, so the accept loop would need a
reaping arm, and drains spawned inside `handle` would need a shared `&mut`); aborting with `JoinHandle::abort` (rejected:
the cancellation token gives the same effect at the next poll and keeps one code path for all tracked tasks).

### 3. An `@Res()` response nobody can end is answered with `500`

Handles of a `Res` are clones of one `Arc<ResInner>`. The framework holds two: the request extension (`router/mod.rs:865`)
and the router's own (`router/mod.rs:719`). Everything else is a handle the application holds: the `#[res]` parameter,
`get_response()` in an enhancer, a clone moved into a spawned task.

- After the pipeline returned `Ok` in `ResMode::Handled` and the response is not ended, the router removes the extension
  (`ctx.request().remove::<Res>()`); from then on the router's clone is the only one the framework holds.
- `impl Drop for Res`: when a handle is dropped and the response is not ended, it calls `done.notify_waiters()`.
- `wait_end` becomes `wait_end_or_orphaned`: loop { create the `Notified`; if ended, return `Ended`; if
  `Arc::strong_count(&inner) == 1`, return `Orphaned`; await }. Creating the `Notified` before both checks keeps the
  lost-wakeup rule of today's loop: a drop or an end between the check and the await wakes it.
- On `Orphaned` the router logs one error line through `ExceptionsHandler` ("a #[res] handler returned without ending the
  response, and nothing holds the response any more") and answers the generic internal-error `500`, with the headers
  already set on the `Res` merged as for a thrown error.
- `finish` notifies once (`notify_waiters`); `take_response` moves the headers with `mem::take`.

Consequence, documented in the rustdoc of `Res` and in `docs/controllers.md`: after the handler returned, an enhancer or a
task that kept the `ExecutionContext` but not a `Res` can no longer obtain the response through `get_response()` (the
extension is gone); it must keep the `Res` handle itself.

Tests (`tests/res.rs`): the failing test first (task 4.1): a handled handler that returns without ending: today the
request never completes (the test asserts a response within its deadline and fails); after the fix, `500` with the generic
body and one error line. Then: a handler that moves its `Res` into a spawned task which ends it after the test signals:
the client gets that response, not a `500`; a spawned task that drops its handle without ending: `500` once it is dropped;
the existing `later` case of the recorded fixture (`fixtures/res_express.json`, ended from a task after the handler
returned) still matches; passthrough mode is unaffected.

Alternatives considered: a deadline after the handler returns (rejected: a legitimate handler may end the response from a
long task, and the original framework imposes no deadline); keeping the extension and counting two (rejected: any code
holding the `Request` could fetch a new handle from the extension after the count was read).

### 4. `from_hyper` returns `Request`

`from_hyper(..) -> Request`; `handle` matches on the request only; the `allow` goes. No behavior change; the clippy job is
the check.

### 5. One request path for the server and the test client

- `PendingBody.body` becomes `BoxBody` (`http_body_util::combinators::UnsyncBoxBody<Bytes, Box<dyn Error + Send + Sync>>`)
  instead of `hyper::body::Incoming`. `from_hyper` is generic over the body (`B: Body<Data = Bytes> + Send + 'static`) and
  boxes it only when it keeps it pending (multipart, or a declared length over the limit); `multer::BodySource::Stream`
  holds the boxed body. The `Buffered` source stays for requests built in memory with `Request::new`.
- `server::respond(shared, req: hyper::Request<B>, remote, options, &mut client_aborted) -> hyper::Response<ResBody>`:
  the `RequestInfo` and compression setup, `from_hyper`, the HTTP/2 aborted-body rule, `dispatch`, the drain of an unread
  body and `finish_response`. `handle` keeps the Engine.IO, gateway and upgrade routing and calls `respond`.
- `TestRequest::send` builds a `hyper::Request<Full<Bytes>>` (its method, URI, headers and body) and calls `respond` with
  no remote address and the application's `ServerOptions` (copied into the `TestClient` when it is created). The
  multipart body therefore reaches the file interceptor through the same pending-body path as over a socket.

Tests (`tests/app.rs` or a new `tests/test_client_parity.rs`): a JSON body over the parser limit gets `413` from the test
client as from a listening server; a declared `Content-Length` over the limit gets `413` without the body being parsed; a
multipart upload through the test client reaches `read_body` as a stream (`is_stream()`); the same request through both
clients gives the same status, headers (except `Date`) and body for the routes of `tests/express_parity.rs`.

Fallout: tests that sent bodies over the default limits through the test client start failing. Each is either fixed to
set the limit it needs or reported as a finding in "Implementation notes" (task 6.4).

### 6. `server.rs` split by concern, `ConnState` grouped

Pure moves, in this order, each verified by building and running the server suites:

| File | Holds |
|---|---|
| `server/options.rs` | `ServerOptions` and its builder methods |
| `server/request.rs` | `PendingBody`, `PendingBodySlot`, the 400 / 408 / 413 errors, `content_length`, `read_limit`, `from_hyper`, `drain`, `respond` |
| `server/response.rs` | `full`, `empty_body`, `into_hyper`, `PlainLen`, `finish_response` |
| `server/budget.rs` | `Budget`, `Verdict`, `Admission`, `Waiting`, the deferred HTTP/1 charge |
| `server/rapid_reset.rs` | `CancelWindow`, the cancel constants, `Outcome`, `CancelWatch`, the tally |
| `server/guarded_body.rs` | `GuardedBody`, `cut_slice`, the HTTP/2 slice constants, `InflightGuard` |
| `server/timed_io.rs` | `TimedIo`, `Phase`, `Proto`, `H2Frames`, `ReadCredit`, `linger_close`, `limit_unsent_bytes` |
| `server/watchdog.rs` | `Epoch`, `BodyClock`, `watchdog`, `finish_connection` |
| `server/accept.rs` | Decision 1 |
| `server.rs` | `ResBody`, `ConnState`, `connection_builder`, `ServiceError`, `Transport`, `ConnCtx`, `run_connection`, `handshake`, `serve`, `handle` |

The reviewer's suggestion to move `from_hyper`, `read_limit` and `drain` to `http/` is not taken: they handle hyper bodies,
deadlines and the server's limits, which `http/` does not know about; `server/request.rs` keeps them next to the code that
calls them.

Then `ConnState` becomes five fields, one struct per concern (`activity: Activity`, `end: EndFlags`, `cancels:
CancelState`, `clocks: Clocks`, `budget: ResponseBudget`), each in the file of its concern, with the methods that touch
only one concern moved onto it. Atomic orderings and lock scopes are copied as they are; no lock is merged or split.

Verification: `cargo test -p corvis -- --list` gives the same count before and after (task 7.1 records it), every unit
test of `server.rs` moves with its code, and the security suites of the table below pass unchanged.

## Security invariants that must hold

From the "Security review" sections of the archived changes `e0-reject-ambiguous-http1-framing`,
`e0-cap-pending-response-bytes`, `e0-http2-limits-follow-server-options`, `e0-write-deadline-for-stalled-clients`,
`e13-tls-termination-in-the-built-in-server` and `e13-response-compression`, and the transport tests of the first security
audit. Decision 6 moves all of this code; Decisions 2 and 5 change who owns the connection and body tasks.

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| A framing rejection is answered `400` and ends the connection; empty lines are dropped before hyper | framing change, security follow-up 1 | 6 | `tests/http1_framing.rs` (all), `pipelined_streams_cut_at_every_byte_behave_the_same`, `a_rejection_after_empty_lines_is_still_answered_with_400`, the mutated-stream property test in `server/h1_framing.rs` |
| `Content-Length` must be `1*DIGIT` for the check; lists go to hyper | framing change, follow-up 2 | 6 | `content_length_lists_are_answered_by_hyper_with_400` |
| HTTP/1 over an `h2` TLS connection gets no HTTP bytes; the preface over `http/1.1` is closed | TLS change, security round | 2, 6 | `tests/tls.rs` `an_http1_request_over_an_h2_connection_gets_no_http_bytes`, `a_websocket_upgrade_over_an_h2_connection_is_not_upgraded`, `the_http2_preface_over_an_http_1_1_connection_is_closed` |
| Handshake timeout and `max_connections` count handshakes | TLS change | 2, 6 | `a_stalled_handshake_does_not_block_a_second_client_and_is_closed_at_the_timeout`, `half_a_client_hello_is_closed_at_the_timeout`, `handshakes_count_against_max_connections` |
| At most `http2_max_concurrent_streams` handlers per connection; rapid reset closes the connection (proportional rule, ceiling 2000, refusal after the verdict) | HTTP/2 limits change, rounds 1-3 | 6 | `tests/http2_limits.rs` `rapid_reset_never_exceeds_the_stream_cap_and_closes_the_connection`, `interleaved_completed_requests_do_not_hide_a_flood`, `reset_post_bodies_count_as_cancelled`, `the_cancelled_streams_ceiling_applies_whatever_is_completed`, `a_client_completing_more_than_it_cancels_is_not_cut`, `stream_cap_limits_concurrent_handlers`; the `tally` unit tests |
| Pending response bytes capped per connection; HTTP/1 charge released on flush; HTTP/2 tail copies; waiters woken only when room | pending-bytes change, security round 1-6 | 6 | `tests/response_memory.rs` (all), unit tests `an_http1_charge_is_released_even_when_hyper_wrote_fewer_bytes_than_it_took`, `the_last_slices_of_an_http2_body_are_copies`, the `Budget` unit tests |
| `max_pending_response_wait` refuses with `REFUSED_STREAM`, HTTP/2 only | pending-bytes change, round 2 | 6 | `a_request_that_waits_too_long_is_refused_and_not_counted_as_cancelled`, `pipelined_http1_requests_run_one_handler_at_a_time_and_are_never_cut_by_the_wait` |
| `send_timeout` closes a connection none of whose bodies progressed; idle HTTP/2 frames do not refresh keep-alive; `TCP_NOTSENT_LOWAT` on Linux | write-deadline change, rounds 1-2 | 6 | `tests/send_timeout.rs` (all), the watchdog unit tests, the `limit_unsent_bytes` unit test |
| Header, keep-alive and request timeouts; slowloris; bounded drain in bytes and time; early error responses survive unread input | first audit (transport) | 2, 5, 6 | `tests/transport_security.rs` `silent_and_slow_header_connections_are_closed`, `slowloris_on_keep_alive_connection_released_without_drain`, `drain_after_an_early_error_response_is_bounded_in_bytes`, `drain_after_an_early_error_response_is_bounded_in_time`, `early_error_response_survives_unread_input`, `incomplete_body_gets_408` |
| A body over the parser limit is refused before it is read; multipart bodies are read only by a file interceptor, after the guards | first audit (http-input, multipart) | 5 | `json_over_the_parser_limit_is_rejected_before_reading`, `multipart_to_unknown_route_is_answered_without_reading_the_body`, `multipart_body_limit_rejects_a_declared_length`, `multipart_body_limit_applies_while_streaming`, `read_body_reads_the_pending_multipart_body` |
| An HTTP/2 body reset by the client is not dispatched and counts as cancelled | HTTP/2 limits change, round 2 | 5 | `reset_post_bodies_count_as_cancelled` |
| Compression slots released when a body ends, fails or is dropped; per-connection cap | compression change, security fixes | 5, 6 | `a_stalled_client_releases_its_compression_slot_when_it_is_cut`, the compression slot unit tests |
| `max_connections` queues extra connections | first audit | 1, 2 | `max_connections_queues_extra_connections`, `max_connections_counts_sockets_not_streams` |

Decision 2 adds one: after `serve` returns, no connection task, handshake or drain of that server runs (the new lifecycle
tests). Decision 5 must not let the test client weaken a limit: the limits are now applied to it, never relaxed for the
server.

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| A handler still running at the shutdown deadline | Left running while the shutdown hooks run | Cancelled before the hooks run; one warning line | Hooks no longer race handlers; a handler that must finish needs a longer `shutdown_timeout` or `None` |
| Shutdown deadline | Fixed 10 s | `shutdown_timeout`, default 10 s | Configurable |
| `accept` failing with `EMFILE` | Silent busy loop | Backoff up to 1 s, a warning a minute | New log line |
| `#[res]` handler that returns without ending the response and keeps no handle | Waits until the client leaves | `500`, generic body, one error line | Matches no original behavior: the original waits; nothing can end such a response, so answering it is strictly better |
| `get_response()` after a handled handler returned | Returns the `Res` | Returns `None` | Code that needs the response later keeps the `Res` handle |
| Test client with a body over the parser or transport limit | Parsed whole | `413`, as the server | Tests see what clients see |
| Test client with a multipart body | Read from `raw_body` | Read through the pending-body stream, as the server | Same path as over a socket |

## Order with the other changes

- `e3-authentication-hot-path-and-consolidation` (being implemented): touches `http/request.rs` (poisoned locks,
  `typed_user`) and `http/res.rs` (`has_cookie_prefix`); this change touches `res.rs` only in `finish`, `wait_end`,
  `take_response` and a `Drop` impl. Independent; either order.
- `e2-socket-io-hot-path-and-api-hygiene`: edits `server.rs` only for the `tokio-tungstenite` upgrade (its task 5.4).
  If e2 lands first, Decision 6 moves its lines; if this change lands first, e2's task 5.4 edits `server/timed_io.rs` or
  `server.rs` as the split leaves them. Either order; this change should land first because its fixes are operational.
- `q-http-response-consistency`, `q-http-request-hot-path`, `q-http-multipart-streaming`: land after this change;
  `q-http-multipart-streaming` relies on the boxed pending body of Decision 5.
- `q-core-lifecycle-and-cache-correctness` (core review, same epic) lands after this change: it restructures
  `listen_with` and `close_with_signal` in `application.rs` (this change does not edit that file), stops the background
  servers through `serve`'s shutdown future, bounded by `shutdown_timeout`, and closes gateway connections.
  `q-core-request-hot-path` measures through `server::respond` (Decision 5).

## Risks / Trade-offs

- Cancelling handlers at the deadline can interrupt a database transaction mid-way → a transaction whose future is
  dropped is never committed (with sqlx, which the ORM uses, it is rolled back when its connection goes back to the
  pool); documented in `docs/fundamentals/lifecycle-events.md`; `None` restores waiting.
- Removing the `Res` extension after the handler changes `get_response()` for late callers → documented; no test of the
  repository or of the three port apps calls it after the handler (checked with a grep in task 4.2).
- The test client applying limits breaks tests that relied on it not doing so → each is fixed or reported (task 6.4).
- The split moves every line of the security-critical server code → pure moves, test count compared, every security suite
  run, and the security review reads the diff with `git diff --color-moved`.

## Open questions

1. Default of `shutdown_timeout`: keep 10 s, or wait without limit like a Node server? Recommendation: keep 10 s; a
   process manager usually kills after 10 to 30 s anyway, and the old fixed value is what applications already see.
2. Should the orphaned `#[res]` response be `500` or close the connection without a response? Recommendation: `500`; it
   goes through the same path as a thrown error, so the client and the logs see an ordinary failure.
3. Should the test client set a remote address (`127.0.0.1:0`)? Recommendation: no; `Request::ip()` already answers
   `127.0.0.1` without one, and `remote_addr()` returning `None` tells a test it is in memory.

## Review notes

These settle the open questions and override the design where they differ.

1. `shutdown_timeout` defaults to 10 s.
2. An orphaned `#[res]` response is answered `500` through the exception path, with one error line.
3. The test client keeps `remote_addr() == None`.
4. The security review is mandatory before the merge: the accept backoff, the shutdown sequence and cancellation, the
   boxed pending body (limits and drain), and the split read with moved-line highlighting.

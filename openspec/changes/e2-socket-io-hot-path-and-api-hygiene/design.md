# Design

## Context

Source: the engineering-quality review recorded in issue #185, plus #183 and #184 (absorbed, see proposal.md). Code read
on `dev` at `db5b4f6` (2026-11-11), `tokio` 1.54.1, `tokio-tungstenite` / `tungstenite` 0.24.0.

The gateway adapter (#35) is being finished on a branch and edits `socket_io/client.rs` (a namespace dispatcher hook),
adds `socket_io/gateway.rs` and touches `ExecutionContext`. This change is implemented after #35 merges and does not rely
on anything that exists only on that branch. **Task 1.1 re-checks every line cited below against `dev` at implementation
time** and records moved lines and any conflict in "Implementation notes"; the decisions depend on behavior (what is
copied, what is locked, in which order), not on line numbers.

### The findings, confirmed against the code

Every finding of the issue was read in the code. Line numbers are those of `db5b4f6`.

| Finding | Where now | Verdict |
|---|---|---|
| Wakeup registration allocates per message | `ws.rs:531-541` (`changed`: `Vec` + one `Box::pin(notified)` per level + `select_all`), called from `client.rs:231` (every loop turn), `engine/websocket.rs:123` (every frame), `engine/polling.rs:161` (every `POST`) | Confirmed. A Socket.IO server has three levels by default (session, address, server: `options.rs:462` sets `max_address_unread_bytes`), so two calls per received WebSocket message cost eight allocations |
| `notify_waiters` on every level of every release | `ws.rs:674-680` (`release` → `wake` → `notify_waiters` when `wake_all`) | Confirmed: tokio 1.54.1 `Notify::notify_waiters` takes the waiter-list mutex unconditionally (`sync/notify.rs:740`). Same pattern in `OutQueue::wake` (`ws.rs:117-120`, `watchers.notify_waiters()` on every push), found while confirming |
| Handshake deep-copied per socket and per polling request | `client.rs:521` (`handshake().clone()`), `engine/mod.rs:500` (`dispatch(handshake.clone(), ..)`), second query parse at `engine/mod.rs:527` (`Query::parse` keeps every key) | Confirmed. `Request::clone` copies headers, parsed query and body (`http/request.rs:14-17` says so) |
| Comments that say the opposite | `adapter.rs:175-177`, `budget.rs:3-12` | Partly. The `adapter.rs` sentence is accurate for what it describes (a `HandshakeInfo` shares the socket's `Handshake`); what it omits is that the `Handshake` copies the session's request. `budget.rs` claims everything kept from the client is charged; the request is not. Both corrected in Decision 3 |
| An emit copies the payload about four times | `parser.rs:300-301` (JSON buffer copied behind the header), `client.rs:80` (`encoded.text.clone()`), `engine/packet.rs:150-158` (`encode_ws` builds a new `String` for every text, per recipient for `Shared`), `engine/websocket.rs:46` (`b.to_vec()` per binary frame), `ws.rs:148` (a `Vec<usize>` per group) | Confirmed: encoder (2), clone (1), frame (1), plus the WebSocket library's write buffer (1). `tungstenite` 0.24 takes `String` / `Vec<u8>` |
| Client mutex across the adapter and the send | `client.rs:554-571` | Confirmed. `add_all` → application adapter → `IoSocket::disconnect(false)` → `disconnect_with` (the socket is already `Connected`) → `ClientShared::forget` → `nsps()` on the same thread: a deadlock (or a poisoned-lock panic, depending on the platform) |
| `add_all` looks names up under the write lock | `adapter.rs:1053-1069`, `is_socket_id` (`adapter.rs:388`) → `is_live` → namespace table read lock and socket state lock, once per name | Confirmed: up to `max_rooms_per_socket` (1,000) table and state locks under the membership write lock |
| Answer budget address key differs | `socket.rs:630` (`address_key(engine.ip(), None)`) vs `engine/session.rs:343` (`address_key(&ip, handshake_peer)`) | Confirmed. With a trusted proxy and a forwarded address that does not parse, the session falls back to the peer, the answer budget keys the raw string: one fresh share of the answer budget per connection |
| Eight parameters, triplicated flow handling | `client.rs:342`, `client.rs:380` (`#[allow(clippy::too_many_arguments)]`, nine and eight parameters), `match flow` at `client.rs:233-246` and `293-306`, and the same three outcomes written again in the `select!` arms | Confirmed |
| Room validation four times | `socket.rs:494-501`, `adapter.rs:1038-1052`, `broadcast.rs:237-254`, `broadcast.rs:418-429` | Confirmed |
| Ack registration twice | `socket.rs:581-594`, `socket.rs:640-652` | Confirmed |
| Hand-written once-a-minute limiters | `engine/mod.rs:106-122` (overflow), `449-454` (hook timeout), `460-467` (session cap) | Confirmed |
| `WarnLimiter` allocates before it knows | `warn.rs:41` (`namespace.to_string()` on every call), `warn.rs:74` and callers (`format!` before `allow`) | Confirmed |
| Test hooks in the public API | `adapter.rs:749-766`, `adapter.rs:986-993` (`size()` with `assert_eq!`), `adapter.rs:777-790` (`examined` written on every `add`), `mod.rs:71-114`, `socket.rs:604-609`, `namespace.rs:115-119` | Confirmed (all `#[doc(hidden)]`, all public) |
| Public enums without `#[non_exhaustive]` | `socket.rs:31,94,135`, `adapter.rs:54,92`, `broadcast.rs:98`, `engine/session.rs:38` | Confirmed, all seven |
| Tests synchronized with `sleep` or wall-clock bounds | `socket_io_accounting.rs:184,202,256,277,437-554`, `socket_io_lingering.rs:153,204`, `engine_io.rs:242,275,293`, `engine_io_security.rs:105,125,193,218,591` (and 312-316, 332, 409, 431, 465, 566-571, found while confirming), `socket_io/tests.rs:22-26`, `engine/session.rs:1026-1032` | Confirmed |
| Dead code | `server.rs:39,207` (`closed` only stored), `socket.rs:422-429` (`send_encoded`, `#[allow(dead_code)]`), `engine/mod.rs:350,356` (`let _ = handshake;`) | Confirmed |
| Comments that contradict the code | `engine/session.rs:292` (`Polled::Packets` has no "closing" flag), `warn.rs:66` (says "in the last minute"; the count is since the last line written, which can be longer ago) | Confirmed |
| Errors typed by text | `parser.rs:603`, `client.rs:500` (`Result<(), ()>`) | Confirmed |
| Strong reference cycle | `server.rs:142-151` | Intentional (#33 "Lifetimes": the HTTP server keeps an attached `IoServer` alive, `close` and shutdown break it). The defect is narrower: a server that is never attached and never closed leaks. Decision 13 |
| `adapter.rs` mixes four concerns, tests named after rounds | `adapter.rs` (1,192 lines), `socket_io/tests/review1.rs` | Confirmed |
| Cost model of `value.rs:33-93` and CI | `tests/socket_io_memory.rs` and the other counting-allocator suites | Confirmed they run in CI: Linux `test` job, `cargo test -p corvis --features socket-io` (`ci.yml:126`), glibc, which is what `ALLOC_OVERHEAD` models. The Windows job does not run the `socket-io` feature. No code change (Decision 13) |
| `WriteEnd::Refused` treated as an orderly end | `engine/websocket.rs:187-193` | Reachable: after the peer's close frame, `tungstenite` refuses further writes with a protocol error, which `write_loop_with` reports as `Refused` (`ws.rs:330-339`). The outcome (session closed with `transport close`) is right; the comment ("the outbox ended") is wrong. Decision 13 |
| `ip()` recomputed and allocated per call | `socket.rs:187-189`, `socket.rs:332-334` | Confirmed (trust-proxy resolution and a `String` per call) |
| `contains_binary` allocates, and is redundant | `value.rs:350-361`, `parser.rs:250-253` | Confirmed |

No finding is dropped. Two are narrowed: the `adapter.rs:175` sentence (accurate, incomplete) and the reference cycle
(intentional, leaks only for a server never attached nor closed). One needs no code: the counting-allocator tests run in
CI.

### #183 and #184

- #183: the connect block queues CONNECT, inserts `Slot::Joined`, marks the socket connected, then `add_socket` and
  `join_own_room`, all under `nsps()`. A client on another worker can read CONNECT and send its id back by another channel
  before `add_socket` runs. The #34 tests that run on several workers wait for the registration
  (`tests/socket_io_rooms_security.rs`, `socket_of`, commit `b2c3c5a`).
- #184: the upstream suite (`parity/socket-io/suites/socket.io-protocol/test-suite.js:398-413`) waits for CONNECT with a
  `once` listener and registers the listener for the next message after an `await`; a client that receives both frames in
  one read loses the second. Corvis queues CONNECT and runs the connection handlers at once, so CONNECT and
  `42["auth",{}]` are written back to back (two `sink.send` calls of the writer, `ws.rs:320-343`, with `TCP_NODELAY`,
  `server.rs:2307`) and often arrive in one read. The reference server writes CONNECT from inside the connect call, before
  the handler runs, and writes what the handler emitted only after the transport drains: a later write.

## Goals / Non-Goals

**Goals:**

- Measure first, then remove the allocations and copies the review found on the per-message paths, each against a number.
- One connect sequence that holds no connection lock across application code, registers the socket before its client can
  know the id, and writes CONNECT apart from what follows.
- Leave the `Adapter` trait and the public enums in the shape #36 publishes.
- Tests that wait for conditions, never for time.

**Non-Goals:**

- New features, new limits or new options.
- Changing what a slow recipient holds (#175, #176).
- A Node comparison in the Socket.IO benchmark.

## Decisions

### 1. A Socket.IO benchmark, run before and after

`openspec/specs/bench` has two capabilities: `bench/synthetic` (HTTP throughput with `oha` against a Corvis app in
`bench/corvis-app/`) and `bench/port-vs-original`. Neither drives Socket.IO, and `oha` cannot. New capability
`bench/socket-io`, following `bench/synthetic`'s rules (release build in the shared target directory, servers on
`127.0.0.1`, results file overwritten per run, raw output ignored by git):

- `bench/socket-io/` is a workspace member (`publish = false`) with one binary, `corvis-sio-bench`, and two subcommands.
  `server` starts an `IoServer` on `127.0.0.1:<port>` with default options and a global counting allocator (allocations
  and bytes, reset and read through a plain HTTP route `/stats` on the same server). `load` opens N connections with raw
  Engine.IO / Socket.IO framing over `tokio-tungstenite` (the shape of `tests/engine_support/io_client.rs`) or over HTTP
  long-polling, and runs one scenario.
- Scenarios: `emit` (each client sends M events with an acknowledgement, the server answers from the handler), `server-emit`
  (the server emits M events to each socket from a timer task), `broadcast` (the server emits M events to the whole
  namespace). Payload size is a parameter (default 64 bytes; runs at 16 KiB for the copy decisions). Transports `websocket`
  and `polling`. Every client sends a 4 KiB `cookie` header so the handshake copies of Decision 3 are visible.
- Reported per run: messages per second, p50 / p99 / max latency of `emit` acknowledgements and of `broadcast` delivery
  (send time stamped in the payload), and the server's allocations and bytes allocated per message (from `/stats` around
  the measured window), peak resident memory. Each measurement runs three times, the median is reported with min and max.
- `bench/socket-io/run.sh` builds and runs the matrix; `bench/socket-io/README.md` documents it; results go to
  `bench/socket-io/results.md`.

The before run is made at the start (task 2.3) on `dev` with #35 merged, the after run at the end (task 13.1), same
machine, same matrix; both tables go to "Implementation notes". Benchmarks are not tests: no test compares timings.

Alternative considered: Criterion microbenchmarks of the gauge and the encoder. Rejected as the first measurement: the
costs at stake (mutex contention across connections, allocations per message, copies per recipient) only show with many
connections on the multi-thread runtime; microbenchmarks can be added later for regressions.

### 2. Release wakeups that cost nothing without waiters

`UnreadGauge` gains a waiter count per level (`waiters: AtomicUsize`). Registration and release:

- **Register only after seeing the limit.** `changed()` is replaced by `register(&self) -> Release<'_>`, which increments
  the waiter count of every level of the chain and creates each level's `Notified` (creation is what registers a waiter for
  `notify_waiters`). The returned future is an `async` block over `tokio::select!` with three `Option<Notified>` (the chain
  is at most three deep: session, address, server; asserted when a gauge is built with a parent that already has a parent
  with a parent), and a guard that decrements the counts when the future is dropped or completes. No allocation.
- **Callers check, register, recheck.** The connection loop, the WebSocket reader and the polling `POST` loop read the
  limit; only when it is reached (or a packet is parked) do they call `register()`, then read the limit again and wait only
  if it is still reached. A reader under its limits does no registration at all.
- **Release notifies only where someone waits.** `release` subtracts first and then, at each level, calls `wake()` only if
  that level's waiter count is not zero. `roll_back` keeps its rule (wake only if a refusal was counted since the charge)
  and adds the same test. The gateway gauge (`wake_all == false`, `notify_one`) is unchanged: its fast path takes no lock
  and its stored permit is what protects its single waiter.
- **Why no wakeup is lost.** The waiter increments the count (sequentially consistent), creates the `Notified`, then reads
  the bytes; the releaser subtracts the bytes (sequentially consistent), then reads the count. At least one sees the other:
  either the waiter's read sees the lowered bytes and it does not wait, or the releaser sees the count and calls
  `notify_waiters`, which wakes the `Notified` created before it. The refusal-counting protocol of the namespaces change
  (round 4, item 2) and the stampede fix (round 5: an observer counts a refusal only at the level whose own reading decides
  the pause) are not touched.
- The same rule for `OutQueue::watchers` (the long-poll that waits for the outbox, `ws.rs:212-225`): `wait_ready`
  registers through a count, `wake` calls `watchers.notify_waiters()` only when the count is not zero. The writer's
  `notify_one` is unchanged.

Numbers it must move (`emit`, WebSocket, 1,000 connections × 100 messages, 64-byte payload): server allocations per
received message fall by at least six (two `changed()` calls per message, each a vector and three boxes); messages per
second at 1,000 connections do not fall and are recorded. Tests (unit, `ws.rs::gauge_tests`): a reader under the limit
registers nothing (`waiters == 0` at every level after it reads); 10,000 releases with no waiter send no notification (a
test-build counter of `wake` calls stays at 0); a waiter registered between a release's subtraction and its count check is
woken or sees the release (the interleaving harness `try_add_up_to_with` already uses, extended with the waiter steps);
the existing interleaving, stress and stampede tests pass unchanged.

Alternative considered: keep `select_all` with a `SmallVec`. Rejected: it removes the vector, not the boxes or the mutex.

### 3. One handshake request per session

- `Session.handshake` becomes `Arc<Request>`; `EngineSocket::handshake()` keeps returning `&Request`; a crate-internal
  `handshake_arc()` hands out the `Arc`. `HandshakeData.request` (the socket's `Handshake`) holds the same `Arc`, so every
  socket of a connection shares the session's request. `Handshake::request()` and `headers()` keep their signatures.
- `EngineServer::handle` builds the request once and passes `&Request` to `dispatch`; only `open_session` takes it (as
  `Arc`). The `handshake.clone()` at `engine/mod.rs:500` goes.
- `Query` becomes three fields (`sid`, `transport`, `eio`), filled from `form_urlencoded::parse` without keeping other keys:
  bounded work and memory whatever the query holds.
- For a request that carries a `sid` (a poll or a `POST` of an existing session, not an upgrade), the engine builds the
  request without the application's query parser (`Request::for_engine_poll(parts, remote)`: method, URI, headers moved,
  peer, empty query and body). It is used only for the origin check, CORS and the method, and is never handed to
  application code (the session keeps the handshake's request; `allow_request` runs only for handshakes). A comment and a
  debug assertion say so.
- What is held, stated where the comments were wrong (`adapter.rs` on `HandshakeInfo`, `budget.rs` module docs, the
  `Handshake` rustdoc): one request per session, bounded by the server's `max_header_size` (32 KiB by default), so
  `max_sessions × max_header_size` at worst; not charged to the unread budget, like the session's other fixed state. This
  is the figure the engine change's security review recorded (round 1, item 2); the per-socket copies broke it by a factor
  of the namespaces joined.

Not charged, deliberately: a charge that lives as long as the session would permanently occupy part of its fair share and
its address quota, and the request is bounded by the HTTP server's own limit before any session exists.

Numbers it must move (polling, 200 connections × 100 messages, 4 KiB cookie): bytes allocated per polling request fall by
at least the size of the request's headers (about 4 KiB); bytes allocated per CONNECT fall by the same. Tests (counting
allocator, `tests/socket_io_memory.rs`): a connection that joins ten namespaces with a 16 KiB cookie grows the heap by
less than 16 KiB per extra socket; a `POST` of an existing session with a 16 KiB cookie allocates less than 16 KiB beyond
its body; `Handshake::request()` of two sockets of one connection point to the same request (`Arc::ptr_eq` through the
`testing` module).

### 4. Fewer copies per emit and per recipient

- **Header first, one buffer.** `parser::encode` counts the binary values in a first pass that does not allocate (a
  recursive count bounded by `MAX_DEPTH`; deeper values stop the count and the writer reports `TooDeep` as today), writes
  the header (type, attachment count, namespace, id) and then the JSON into one `Vec`, and returns it. `IoValue::contains_binary`
  (public) stops allocating for values without containers and is no longer called by the encoder.
- **Encoded by value.** `ClientShared::send_encoded` and `IoSocket::send` take `Encoded` by value; the text moves into the
  queue (`client.rs:80` clone gone).
- **The Engine.IO message type travels with the text.** The encoder reserves one leading byte for the Engine.IO `4`, so the
  buffer it produces is the Engine.IO message. Two hidden Engine.IO variants carry such buffers: `EngineMessage::Framed`
  (one recipient) and `EngineMessage::Shared` (a broadcast; it now holds the framed text). Their `len` and `polling_len`
  count the text without the prefix, so `max_buffered_amount`, the polling batch bound and every length the engine reports
  are unchanged; `encode_polling` writes them as they are; `encode_ws` turns them into a frame without copying.
- **No vector per group.** `OutQueue::push_group` sums the sizes in one pass over the items and pushes in a second; no
  `Vec<usize>`.
- **Frames without copies.** `tokio-tungstenite` moves from 0.24 to the latest release at implementation time (0.26 or later,
  the first with `Bytes` payloads; task 5.4 pins it after checking the MSRV 1.89 and `cargo deny`). `Message::Text` takes
  `Utf8Bytes`, built from the framed `String` (one recipient) or from the shared buffer, now a `Bytes` (a broadcast), without
  copying; `Message::Binary` takes the attachment's `Bytes` as it is (`websocket.rs:46` `to_vec` gone). The upgrade touches
  `ws.rs`, `websockets.rs`, `server.rs`, `engine/websocket.rs`, the tests that build `tungstenite` messages and
  `examples/tertulia`; it changes no public Corvis type (`WsMessage` is Corvis' own) and no limit (`max_message_size`,
  `max_frame_size` keep their values).

After this, an emit is copied once by the encoder and once by the WebSocket library's write buffer; a broadcast is encoded
once and each WebSocket recipient is copied only by the library while its frame is written.

Numbers it must move (16 KiB payload): bytes allocated per `server-emit` message fall from about five payloads to about two;
bytes allocated per `broadcast` recipient fall by one payload (the per-recipient frame copy); allocations per broadcast
recipient fall by one (the size vector). Tests: pointer identity, no allocator needed: the frame built for a `Framed` and for
a `Shared` message points into the same buffer the encoder produced (`Utf8Bytes::as_str().as_ptr()`, `Bytes::as_ptr()`); the
existing `a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches`, the parser fixtures
(`fixtures/parser.json`) and the `EngineMessage::Shared` length tests pass unchanged.

Alternative considered: leave `tungstenite` at 0.24 and stop at the encoder. Rejected: the per-recipient copy of a broadcast
is the largest cost (16 KiB × 10,000 recipients is 160 MB written through fresh buffers), and #35 will have merged, so the
upgrade does not race it. Fallback, recorded here so nobody has to decide it again: if the upgrade turns out to need changes
outside the files listed, task 5.4 is split into its own issue and the other three items still land.

### 5. The connect sequence: register, join, announce

A new phase, `Phase::Joining`, between `Pending` (middleware) and `Connected`. A joining socket is in the namespace table
and may be in rooms (`is_live` is true), but it is not announced: `send_shared` skips it (phase check, as today),
`check_emittable` and `AckSender::send` refuse with `NotConnected`, `register_broadcast_ack` refuses, `disconnect(false)`
does nothing (as for any socket that is not connected), and `LocalNamespace::sockets_by_ids` (the selection of broadcasts
and bulk operations) leaves it out. `IoNamespace::socket(id)`, `sockets()` and `socket_count()` include it: that is the
point of #183.

After the middleware passes, `connect` does, with no connection lock held across any step:

1. Under `nsps()` only: check `done` and `Slot::Pending`, insert `Slot::Joined(socket)` (so `terminate` finds it), set the
   phase to `Joining`. Release.
2. `namespace.add_socket(&socket)`, which inserts only if the phase is still `Joining`, under the table's write lock.
3. `socket.join_own_room()`: the adapter's `add_all` with no lock of the connection held. An adapter may call anything,
   including `disconnect(true)` on the connection: nothing deadlocks.
4. If 3 failed (panic or `Err`): the socket is torn down silently (step "Teardown" below), its slot is removed (not `Left`:
   the client never joined), and the client receives CONNECT_ERROR `Internal server error`, as for a panic in the id check.
5. Under the socket's state lock: if the phase is still `Joining`, queue CONNECT with a write acknowledgement (Decision 6)
   and set the phase to `Connected`; otherwise (a teardown ran) do nothing. A refused queue (closed, or full, which closes the
   session) leaves the phase `Joining` and the socket is torn down after the lock is released.
6. `joined_once`, then Decision 6.

Teardown of a joining socket (`disconnect_with` on a `Joining` socket: from `terminate`, from server shutdown, or from step 4):
phase `Disconnected`, `adapter.del_all`, `remove_socket`, `forget`; no `disconnecting` / `disconnect` handlers and no
DISCONNECT packet, because the client never saw the socket connected (the same treatment as a socket refused by its
middleware). Whichever of `add_socket` / `add_all` and the teardown takes the table or membership lock first, nothing
survives: both inserts test the phase under the lock the teardown's removal takes after setting the phase, the pattern #34
uses for `add_all` and `del_all`.

Why the order is right: everything another thread can do with the socket before step 5 is either refused (emits) or skips
it (broadcasts, bulk operations); CONNECT is queued and the phase set inside one critical section of the state lock that
every phase reader takes, so an emit that sees `Connected` queues after CONNECT, and a lookup made after the client read its
CONNECT finds the socket (it was inserted in step 2). This is Node's order (`_doConnect`: `sockets.set`, then `_onconnect`:
`join(id)`, then CONNECT), without its single thread.

**`add_all` without lookups under the lock.** The test "is this name the id of another live socket" reads the adapter's own
`sids` map under the write lock instead of calling `LocalNamespace::is_socket_id` per name: a live socket is always in the
room of its own id (it cannot leave it, and `del_all` runs only after the phase left `Connected`). The only live socket
missing from `sids` is one between steps 2 and 3, and step 3's own-room join expels whoever joined its id meanwhile
(`expel_others`, round 2 of #34). The single `is_live(id)` check of the joining socket itself stays. Result: no namespace
table or socket state lock is ever taken under the membership write lock, which simplifies the lock order.

**Lock order after this change**, outermost first: a socket's state lock (only around the CONNECT step, with the engine's
session state and outbox locks inside it, which are leaves); otherwise the adapter's membership lock, the namespace's socket
table, a socket's state. The connection's namespace table (`nsps`) is a leaf: no other lock is taken while it is held. Written
in `adapter/mod.rs` and in "Implementation notes".

Tests (`tests/socket_io_connect_order.rs`, multi-thread runtime with 4 workers): 1,000 clients connect and each looks its id
up as soon as it read CONNECT, with no wait: never `None` (the #183 scenario); a task broadcasting in a loop while 200
clients connect: every client's first packet for the namespace is CONNECT; a test adapter whose `add_all` calls
`disconnect(true)` on the joining socket's connection: `connect` returns, the socket is gone, the connection closed, within
the test's deadline (no deadlock); an adapter whose own-room `add_all` panics, and one that returns `Err`: the client gets
CONNECT_ERROR `Internal server error`, the namespace has no socket, the adapter has no membership, the session's unread gauge
is back to its value before the attempt; the connection closing during each of steps 1-5 (driven through a test hook that
parks `connect` after each step): no socket, no membership, no handler ran. The `socket_of` waits of
`tests/socket_io_rooms_security.rs` are replaced by direct lookups.

Alternatives considered: keep the lock and only reorder (rejected: the deadlock stays); release `nsps` but keep "CONNECT, then
register" (rejected: that is #183); register in the table but keep the phase `Connected` from the start (rejected: an emit by
a thread that found the socket by enumeration could precede CONNECT).

### 6. CONNECT in a write of its own

The connection handlers run only after CONNECT was handed to the transport. Mechanism:

- `EngineSocket::send_all_written(messages) -> Result<Written, SendError>` queues the group like `send_all` and returns a
  future that resolves `true` when the group's last message was handed to the transport and `false` when the session
  discarded it or closed first. The outbox already carries an optional acknowledgement per item (`OutQueue::push`, `Ack`);
  `push_group` gains it for the last item. WebSocket: `write_loop_with` sends it after `sink.send` returned (it already does).
  Polling: `take_batch` sends it for the items it takes into a response (today it drops them).
- In `connect`, step 5 queues CONNECT with `send_all_written`; after the state lock is released, `connect` waits for
  `Written` or for the connection to close, then runs the connection handlers and `mark_ready`. If the write did not happen
  the connection is ending: `terminate` disconnects the socket, and `connect` returns.
- Packets of the client that arrive meanwhile wait for `ready` as today; the wait is bounded by the connection itself (a
  WebSocket write that never completes is cut by the server's write deadline; a polling client that never polls is closed by
  the heartbeat). `connect_timeout` no longer applies once the socket is `Connected` (`joined_once` is set in step 6).

This is the reference order: CONNECT is written, then the handler runs, and what it emits goes in a later write (WebSocket:
a separate frame written after the CONNECT frame was flushed; polling: the next response, since a response that carries
CONNECT was taken before the handler could emit). Visible to applications: connection handlers start one write later than
today, at most one poll round trip on polling. #35's `handle_connection` hook runs among the connection handlers and keeps
its guarantee ("finishes before any event of that socket is dispatched").

Number it must move: the upstream suite (`CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test
socket_io_conformance`) passes 20 runs in a row (task 1.2 records the failure rate before: about 2 to 3 in 8). Tests,
deterministic: polling, the client posts `40` with no `GET` pending and the connection handler emits `auth`: the first `GET`
answers exactly the CONNECT packet and the second the event (unit test over the session's outbox: the handler has not run
until the CONNECT item is taken); WebSocket, a unit test with the in-process transport of `socket_io/tests.rs`: the
connection handler does not run until the test drains the CONNECT item, and the first emit is a separate item.

Alternatives considered: stop `take_batch` at a CONNECT and yield in the writer after it (rejected: a yield orders nothing
across threads, and the WebSocket writer already flushes per frame; the handler emitting before the CONNECT write is the
cause); delay only the first emit (rejected: any emit of any task would need the same rule).

### 7. The answer budget uses the session's address key

`EngineSocket::address_key() -> &str` returns `Session::addr_key` (crate-internal); `IoSocket::resolve_ack` passes it to
`AckSink::respond_charged`. One key for the session cap, the unread budget and the answer budget. Test
(`socket_io/tests/`, renamed per Decision 14): with `TrustProxy::All` and two connections from one peer sending different
malformed `X-Forwarded-For` values, the two share one address share of the answer budget (the second refused at the
share). This strengthens the round 3 fix of #34; no behavior changes for well-formed addresses.

### 8. One `Connection` for the connection task

`client::run` becomes `Connection::new(server, shared, messages).run()`. The struct owns the decoder, the in-flight set, the
reassembly and parked state, the deadlines, `stream_done` and the close future (`Pin<Box<dyn Future + Send>>` over an owned
`EngineSocket`, built once per connection). `handle_message`, `admit` and `handle_packet` become methods; the three outcomes
of a `Flow` are handled by one method that returns whether the loop ends and whether the client dropped the connection. No
`#[allow(clippy::too_many_arguments)]` is left in `client.rs`. No behavior change: the existing `socket_io`,
`socket_io_accounting`, `socket_io_lingering` and `socket_io_security` suites are the check, plus the handler-cancellation
rule of round 4 (item 3) of the namespaces change, which `tests/socket_io_accounting.rs` pins.

### 9. One place for each duplicated rule

- **Room validation.** `RoomLimits { max_rooms, max_name_length }` in `adapter/mod.rs` with `check(&[Room]) ->
  Result<(), RoomError>` (count first, then each name) and `check_name(&str)`. Used by `IoSocket::join` (the guarantee for any
  adapter), `InMemoryAdapter::add_all` (its own contract: it is also called by `add_sockets` and, with #36, by remote
  requests), `BroadcastOperator::valid_rooms`, and `with_rooms` (names only, keeping the deferred-error rule). Messages and
  error values unchanged.
- **Ack registration.** `SocketState::register_ack(id, target, deadline, max) -> Result<(), Refused>` (phase check, lazy purge
  of expired entries, cap) used by `emit_with_ack_within` and `register_broadcast_ack`.
- **Rate-limited lines of the engine.** `ServerInner` of the engine gets a `WarnLimiter`; the overflow line, the hook timeout
  and the session cap use it with static kinds and the empty namespace (the engine has none), so the table stays bounded and
  keyed by nothing a client sends. The hook-timeout and session-cap lines gain the count of lines left out (operator-visible
  text, documented in "Behavior visible").
- **`WarnLimiter` without allocations when it drops a line.** The state becomes `HashMap<&'static str, HashMap<String, (Instant,
  u64)>>` (lookup by `&str`, allocation only on first insert, `MAX_KEYS` counted over both levels), and `line`, `warn` and
  `panicked` take the message as `impl FnOnce() -> String`, called only when the line is written. The text "in the last
  minute" becomes "since the last one".

### 10. Test hooks in one hidden module

`corvis::socket_io::testing` (`#[doc(hidden)]`, module docs: "not part of the API; may change in any release"). It holds:
`membership_size(&IoNamespace)` and `check_invariants(&IoNamespace)` (they downcast to `InMemoryAdapter`, returning `None` for
other adapters), `MembershipSize`, `has_socket`, `adapter(&IoNamespace)`, `buffered_amount(&IoSocket)`, `probe_decode`,
`probe_auth`, `DecodeProbe`, `same_request(&IoSocket, &IoSocket)` (Decision 3), and the connect step hook of Decision 5. The
trait loses `membership_size`, `check_invariants` and `has_socket`; `InMemoryAdapter::size` (with its `assert_eq!`) becomes the
module's `membership_size`, returning an error instead of panicking when the maps disagree; `Membership.examined` exists only
under `cfg(test)`. The test-only adapter of `tests/socket_io_rooms.rs` forwards the three hooks today; it drops them.

Alternative considered: a Cargo feature for test hooks enabled through a self dev-dependency. Rejected: it adds public surface
under a feature name, and the integration tests need the hooks only through the public crate anyway, which the engine change
settled the same way (`listen_until_in_background` stays `#[doc(hidden)]`).

### 11. Public enums that can grow

`#[non_exhaustive]` on `EmitError`, `DisconnectReason`, `AckError`, `RoomError`, `AdapterError`, `BroadcastAckErrorKind` and
`engine::CloseReason`. #35 adds `EmitError::NotStarted` and #36 adds `AdapterError` variants: after this they are not breaking.
Compatibility: code outside the crate that matches one of them exhaustively needs a wildcard arm; the crate's own matches are
unaffected. A `compile_fail` doctest per enum pins it.

### 12. Tests that wait for conditions

- Integration tests: a helper `eventually(deadline, condition)` in `tests/engine_support/mod.rs` that re-checks a condition
  until it holds and fails with a message at a generous deadline (30 s); the re-check cadence does not decide any outcome.
  Absence of an effect is observed through a marker (an event sent after the effect would have happened, as the room tests do)
  or on the paused clock. "Not before" bounds keep using the configured value; upper bounds on wall-clock time are removed.
- The cited lines and the ones found while confirming (table above) are rewritten; the list is in tasks.md group 12.
- Unit tests on the paused clock: `settle()` (40 `yield_now`) and the session tests' `advance` (5 `yield_now`) become
  `settle_until(condition)`: yield until the condition holds, failing loudly after 10,000 yields. Where a test only needs the
  timer task to have run, the condition is that task's observable effect (a packet queued, a phase changed).

### 13. The nice-to-have items

- Dead code removed: `ServerInner.closed`, `IoSocket::send_encoded`, the unused `handshake` parameter of
  `handshake_refused`.
- Comments: `Polled::Packets` says what it is (packets to send, at least one); `warn.rs` text per Decision 9.
- Errors typed by text: `parse_json` maps errors only through its own `fail` cell; the seed records `Depth` when its bound is hit
  before `serde_json` can reach its own limit (the measure pass already rejects anything deeper than `MAX_DEPTH`). Tests pin
  `MAX_DEPTH` nested containers (decoded) and `MAX_DEPTH + 1` (`TooDeep`, never `InvalidPayload`). `begin_connect` returns
  `Result<(), ProtocolViolation>` (a unit struct).
- Reference cycle: `IoServer::new` installs a connection callback that holds a `Weak<ServerInner>` (a session that arrives for a
  server that is gone is closed); `IoServer::attach` replaces it with the strong one, so an attached server is kept by its HTTP
  server as before and an unattached one is freed when its last handle drops. Test: an `IoServer` created and dropped without
  `attach` is freed (a `Weak` of its inner does not upgrade).
- `WriteEnd::Refused` gets its own arm: the closing handshake began (the peer's close frame), so the loop ends as
  `Exit::Peer { close_frame: true }`, with `transport close`. Test: the peer sends a close frame while the server has 1 MiB queued;
  over 200 iterations the session's reason is always `transport close`.
- `ip()`: `HandshakeData` stores the address the session resolved when it opened (`Arc<str>`, `EngineSocket::ip()`); `Handshake::ip()`
  and `IoSocket::ip()` keep returning `String` (a copy of that value, no proxy resolution). See Review note 4.
- `contains_binary`: Decision 4.
- Cost model and CI: no change; a sentence in `value.rs` says the counting-allocator tests run in the Linux CI job (glibc) and
  measure requested sizes.

### 14. `adapter.rs` in four files, tests named after behavior

- `socket_io/adapter/mod.rs`: the trait, `BroadcastOptions`, `BroadcastFlags`, `Room`, `SocketId`, the errors, `RoomLimits`,
  `LocalNamespace`, the factory.
- `socket_io/adapter/memory.rs`: `Membership`, `InMemoryAdapter`.
- `socket_io/ack.rs`: `AckBudget`, `AckHold`, `AckShared`, `AckSink`, `AckProgress`.
- `socket_io/info.rs`: `HandshakeInfo`, `SocketInfo`, the plain form and `rooms_serde`.

`corvis::socket_io::adapter::*` and `corvis::socket_io::*` re-export the same names as today: no import changes outside the
module. `socket_io/tests/review1.rs` becomes files named after what they check (`ack_budget.rs`, `connect_failures.rs`,
`remote_socket.rs`, `debug_output.rs`, as the contents decide), and test functions that name a review round are renamed after
their behavior. Pure moves: the test count is the same before and after (task 11.8 compares `cargo test -- --list`).

## What the Redis adapter (#36) needs from this change

#36's proposal makes the trait public "adjusted where the Redis implementation showed a need" (its Decision 9) and says its
task 2.1 compares the trait with the merged code. What it will find, and what it must take into account:

1. The trait has no test hooks (`membership_size`, `check_invariants`, `has_socket` are gone; Decision 10). #36's simulated
   second node and its tests use `corvis::socket_io::testing`.
2. `AdapterError` and `RoomError` are already `#[non_exhaustive]` (#36 Decision 9 planned it for `AdapterError`); #36 only adds
   its variants.
3. A socket's own-room `add_all` runs while the socket is `Joining` (Decision 5): `LocalNamespace::is_live` is true for it and an
   adapter must accept the join; `LocalNamespace::sockets_by_ids` leaves joining sockets out of every selection. `del_all` may be
   called for a socket that never reached `Connected` (teardown of a joining socket).
4. No connection lock is held when the adapter runs, but the adapter's synchronous methods are still called from the connect
   path and must not block (#34's rule). `add_all` decides "id of another socket" from its own membership map; a Redis adapter
   that wraps `InMemoryAdapter` gets that for free and must not call back into the namespace while holding its own lock.
5. A failure of the own-room `add_all` refuses the CONNECT (CONNECT_ERROR `Internal server error`) instead of disconnecting a
   connected socket: a Redis adapter that fails there is visible to the client as a refused connection.
6. `AckSink::respond_charged` receives the session's address key; `respond` (answers from other nodes) is unchanged.
7. `Packet` reaches the adapter before encoding, unchanged; the shared encoding a broadcast queues is now the Engine.IO message
   with its type byte. #36 keeps the wire encoding private, as its Decision 9 says.
8. The module layout is Decision 14's: #36's file list (`socket_io/{mod,adapter,...}.rs`) reads `adapter/{mod,memory}.rs`,
   `ack.rs`, `info.rs`.
9. `IoNamespace::adapter()` is no longer public (moved to `testing`). If #36 wants Node's `namespace.adapter`, it publishes it
   with the trait.

Task 14.4 adds these points to #36's design.md as a review note.

## Security invariants that must hold

From the "Security review" sections of the archived engine (#32), namespaces (#33) and rooms (#34) changes. Each refactor that
comes near one has a test that pins it (existing, kept passing, or new):

| Invariant | Source | Decisions near it | Pinned by |
|---|---|---|---|
| No lost wakeup: register before reading the limit; refusal counted before re-reading; rollback subtracts before reading the count | #32 fix 5; #33 round 4 item 2 | 2 | existing `ws.rs::gauge_tests` interleavings and stress test; new waiter-count interleaving test |
| No stampede: an observer counts a refusal only at the level whose own reading decides the pause | #33 round 5 | 2 | `observers_paused_by_the_pool_are_not_woken_by_rollbacks_at_their_address` |
| A session paused by pressure is not closed for it; admission timeout bounds a parked packet | #33 rounds 2-4 | 2, 8 | `socket_io_accounting.rs` admission and pressure tests (rewritten without sleeps, same assertions) |
| Handlers are cancelled when the client failed to make progress; the server's own closes keep the grace | #33 round 4 item 3 | 8 | `a_client_that_goes_away_while_its_packet_waits_for_room_loses_its_handlers_at_the_admission_timeout`, lingering tests |
| Session ids checked for shape before lookup; transport confusion refused; origin allowlist on handshakes and upgrades; `allow_request` only for handshakes | #32 fixes 2-3 | 3 | `engine_io_security.rs` (unchanged assertions); new test: a poll request's query with 10,000 keys keeps only three |
| The handshake request is bounded by the header limit, one per session | #32 round 1 item 2 | 3 | new counting-allocator tests of Decision 3 |
| `auth` charged for the socket's life; handshake `Debug` prints only allow-listed headers | #33; #34 round 2 lead | 3 | `socket_io_accounting.rs` `auth` charge tests, `debug_output_hides_credentials` |
| One shared encoding per broadcast; each recipient bounded by `max_buffered_amount`; lengths of shared messages | #34 round 1 | 4 | `a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches`, `many_slow_clients_never_hold_the_broadcast_back_and_each_is_cut_at_its_own_bound`, length unit tests |
| WebSocket message limits (`max_message_size`, `max_frame_size`) | #32 | 4 (upgrade) | `engine_io_security.rs`, `ws_security.rs` size tests |
| Nothing the application emits precedes CONNECT; a packet behind CONNECT waits for the connection handlers | #33 Implementation notes | 5, 6 | new connect-order tests; existing `socket_io.rs` ordering tests |
| No client in another socket's id room; own room expels; no id that is already a room | #34 round 1 F1, round 2 lead | 5 | `a_client_cannot_join_the_room_of_another_sockets_id`, `the_own_room_of_a_connecting_socket_expels_whoever_joined_its_id_during_the_middleware`, `no_socket_is_given_an_id_that_is_already_a_room`, plus a new test joining a joining socket's id between steps 2 and 3 |
| No membership left behind by a socket that went away | #34 Decision 4 | 5 | `a_join_racing_a_disconnect_leaves_no_membership_behind`, `churn_leaves_the_maps_empty`, the fuzz properties, new teardown-during-joining tests |
| Adapter panics and errors at CONNECT leave nothing behind | #34 round 2 L1/L2, round 3 | 5 | the three CONNECT tests, updated to the CONNECT_ERROR outcome (spec delta) |
| Work under the membership lock bounded by `max_rooms_per_socket` | #34 round 1 B1 | 5, 9 | `a_join_naming_more_rooms_than_the_cap_is_refused_before_the_lock`, `a_join_stops_at_the_first_name_that_passes_the_cap` |
| Answers held by broadcasts are charged to the answer budget, with a per-address share | #34 rounds 2-3 | 7 | `one_address_cannot_fill_the_answer_budget_and_make_the_calls_of_others_fail`, new malformed-address test |
| Log lines a client provokes are rate limited, keyed by static kinds and registered namespaces, never carrying client data; the table is capped | #33; #34 round 1 notes | 9 | `warn.rs` tests (`the_table_cannot_grow_past_its_cap`), the spec scenario "Handler that panics on bad input" |

## Behavior visible to applications and clients

| What | Before | After | Consequence |
|---|---|---|---|
| Adapter fails joining the own room | CONNECT sent, then the socket disconnected and the connection closed | CONNECT_ERROR `Internal server error`, the connection stays | The client sees a refused connection instead of a dropped one (spec delta of `ws/socket-io-rooms`) |
| CONNECT and the first emit of a connection handler | Often one read on WebSocket, one response on polling | Separate writes, separate responses | Matches the reference server; the upstream suite stabilizes |
| When connection handlers run | Right after CONNECT was queued | After CONNECT was written (polling: taken by a poll) | Up to one poll round trip later on polling; events wait for the handlers as before |
| Lookup by id | Could miss a socket whose client knew the id | Never misses; may also return a socket that is joining (its `emit` fails with `NotConnected`) | Applications that look sockets up from another channel stop needing retries |
| Seven public enums | Exhaustive | `#[non_exhaustive]` | Downstream exhaustive matches need a wildcard arm |
| Test hooks | On the trait and on public types (hidden) | In `corvis::socket_io::testing` (hidden) | Only code that used hidden items changes |
| Engine warnings for the session cap and the hook timeout | One line a minute | One line a minute with the count left out | Log text only |

## Risks / Trade-offs

- #35 changes `client.rs` meanwhile → task 1.1 re-checks and records; the decisions are about behavior, and the new
  `Connection` struct (Decision 8) is done after the connect sequence so #35's dispatcher hook moves once.
- The `tokio-tungstenite` upgrade touches the gateway layer and its tests → isolated task, all WebSocket suites run, fallback in
  Decision 4.
- Waiting for the CONNECT write delays connection handlers → bounded by the connection itself; documented; matches the reference.
- The `Joining` phase adds a state every phase check must consider → one place lists who skips it (Decision 5), and the tests
  drive a teardown at each step.
- Wall-clock tests turned into condition waits can hang on a regression → every wait has a deadline that fails with a message.
- The benchmark measures one machine → before and after on the same machine and matrix; the numbers named per decision are
  relative (allocations and bytes per message), not absolute rates.

## Open questions

1. Upgrade `tokio-tungstenite` in this change, or split it into its own issue? Recommendation: in this change, with the
   fallback of Decision 4.
2. Should a socket that is joining appear in `IoNamespace::sockets()` and `socket_count()`, or only in `socket(id)`?
   Recommendation: in all three, documented; filtering would lock every socket's state for an enumeration.
3. Name of the hidden module: `testing` or `__testing`? Recommendation: `testing`, `#[doc(hidden)]`, like `engine` and `adapter`.
4. `IoSocket::ip()` and `Handshake::ip()`: return `&str` (no allocation, breaking) or keep `String` (one copy per call)?
   Recommendation: keep `String` now; #35 uses `ip()` for `#[ip]` and the throttler.
5. Wait for the CONNECT write on polling too, or only on WebSocket? Recommendation: both; it is the reference behavior on both and
   keeps one rule.

## Review notes

These settle the open questions and override the design where they differ.

1. `tokio-tungstenite` is upgraded in this change (task 5.4), with the fallback of Decision 4 if it grows beyond the listed files.
2. A joining socket appears in `socket(id)`, `sockets()` and `socket_count()`; the rustdoc of the three says that a socket returned
   there may still be joining or already leaving, and that `emit` then fails with `NotConnected`.
3. The module is `corvis::socket_io::testing`, `#[doc(hidden)]`.
4. `ip()` keeps returning `String`; it stops resolving the proxy chain on every call. A `&str` accessor is a possible later issue.
5. Connection handlers wait for the CONNECT write on both transports.
6. Implemented after #35 lands; task 1.1 is mandatory and its findings are written in "Implementation notes" before any other
   task starts.
7. The security review is mandatory before the merge: the connect sequence (phases, teardown, lock order), the wakeup protocol,
   the request handling of the engine, the `tokio-tungstenite` upgrade.

## Implementation notes

### 1.1 The findings against `dev` with #35 merged (`bd398aa`)

Every cited line was read again on `dev` at `bd398aa` (the merge of #35, the gateway adapter). Moved lines and what #35
changed:

| Finding | Where now | What changed |
|---|---|---|
| Wakeup registration allocates | `ws.rs:531-541` (`changed`), callers `client.rs:236`, `engine/websocket.rs:123`, `engine/polling.rs:161` | Unchanged |
| `notify_waiters` on every release | `ws.rs:654-680` (`wake`, `roll_back`, `release`); `OutQueue::wake` at `ws.rs:117-120` | Unchanged |
| Handshake copied per socket and per poll | `client.rs:551` (`shared.engine.handshake().clone()` into `Handshake::new`, which wraps it in an `Arc` of its own: one copy per socket); `engine/mod.rs:536` (`dispatch(handshake.clone(), ..)`); `Query::parse` at `engine/mod.rs:563` | #35 made `HandshakeData.request` an `Arc<Request>` (`Handshake::request_arc`, used by the gateway's execution contexts) and detached the session's request from the read buffer when the session opens (`Request::detached`, `engine/mod.rs:415`). The per-socket copy remains, so Decision 3 still applies; the `Arc` it asks for replaces the per-socket one. `Request::detached` stays: the session keeps the detached request, once |
| Comments that say the opposite | `adapter.rs:175-177`, `budget.rs:3-12`, `socket.rs:154` (`Handshake`) | Unchanged |
| Copies per emit | `parser.rs:286-304`, `client.rs:80`, `engine/packet.rs:150-158`, `engine/websocket.rs:43-47`, `ws.rs:148` | Unchanged |
| Client mutex across the adapter | `client.rs:584-601` | Unchanged. #35 added the share check of pending disconnect hooks to `begin_connect` (`client.rs:535-543`) under the same `nsps()` lock; it only reads the lingering table (a leaf lock) |
| `add_all` lookups under the write lock | `adapter.rs:1035-1071`, `is_socket_id` at `adapter.rs:388` | Unchanged |
| Answer budget address key | `socket.rs:705` | **Already done by #35**: the socket keeps the session's `address_key` (`SocketInner::address`, `socket.rs:257,341`) and `resolve_ack` passes it to `respond_charged`; `EngineSocket::address_key()` exists (`engine/mod.rs:905`). Decision 7 reduces to its test (task 8.1) |
| Eight parameters, triplicated flow handling | `client.rs:347`, `client.rs:385`, `client.rs:238-251`, `298-311` | Unchanged |
| Room validation four times | `socket.rs:565-577`, `adapter.rs:1038-1052`, `broadcast.rs:232-254`, `broadcast.rs:418-429` | Unchanged |
| Ack registration twice | `socket.rs:656-669`, `socket.rs:714-726` | Unchanged |
| Hand-written limiters | `engine/mod.rs:121-140` (overflow), `485-491` (hook timeout), `496-503` (session cap) | #35 added a fourth caller of the session-cap limiter (`OpenError::PendingHooks`, `engine/mod.rs:462-468`) |
| `WarnLimiter` allocates first | `warn.rs:41`, `warn.rs:74` and callers | Unchanged; #35 added callers (`IoSocket::warn`, `gateway.rs`) that `format!` before calling |
| Test hooks in the public API | `adapter.rs:749-766`, `adapter.rs:979-1006`, `adapter.rs:774-790`, `mod.rs:80-123`, `socket.rs:679-684`, `namespace.rs:127-131` | Unchanged |
| Enums without `#[non_exhaustive]` | `socket.rs:31,94,138`, `adapter.rs:54,92`, `broadcast.rs:98`, `engine/session.rs:38` | #35 added `EmitError::NotStarted` |
| Tests with `sleep` or wall-clock bounds | as cited | Re-checked in group 12 |
| Dead code | `server.rs:39,139,207`, `socket.rs:488-495`, `engine/mod.rs:376` | Unchanged |
| Contradicting comments | `engine/session.rs:292`, `warn.rs:66` | Unchanged |
| Errors typed by text | `parser.rs:594-607`, `client.rs:521` | Unchanged |
| Reference cycle | `server.rs:142-151` | Unchanged |
| `WriteEnd::Refused` | `engine/websocket.rs:187-194` | Unchanged |
| `ip()` per call | `socket.rs:207-210`, `socket.rs:378-380` | #35 resolves it once per `Handshake` (a `OnceLock`): once per socket, still a `String` per call. Decision 13 still moves it to the session's resolved address |
| `contains_binary` | `value.rs`, `parser.rs:250-253` | Unchanged |

What #35 put on the paths this change redesigns, kept as invariants of this change, each with the tests that pin it:

| Invariant (#35) | Where | Pinned by |
|---|---|---|
| An address at its share of pending disconnect hooks gets no new session (`503`) and no CONNECT to a namespace with disconnect hooks (`CONNECT_ERROR` `Too many pending disconnects`), checked before any socket or slot exists | `engine/mod.rs::caps_ok`, `client.rs::begin_connect` | `an_address_at_its_share_of_pending_hooks_gets_no_session_until_they_drain`, `one_session_joining_and_leaving_in_a_loop_is_refused_at_its_addresss_share`, `twenty_victims_hooks_run_while_one_address_keeps_rejoining` |
| Disconnect hooks run exactly once on every path | `gateway.rs::on_connected` (`once`), `gateway/disconnect.rs` | lifecycle tests of `socket_io_gateways.rs` and `socket_io_gateways_security.rs`, `disconnect_hooks_run_in_a_bounded_number_of_tasks_and_every_one_runs_once` |
| The connection hooks finish before any event of that socket is dispatched; events start in arrival order (tickets taken by the dispatcher, which the connection calls in arrival order) | `gateway.rs` (`Gate`, `Ticket`), `client.rs::dispatch` | `events_written_with_the_connect_start_in_arrival_order_while_the_gate_opens`, `events_held_behind_a_slow_connection_hook_start_in_arrival_order`, `tickets_start_in_the_order_they_were_taken_whatever_the_order_they_are_polled_in` |
| The body copies of a gateway handler are charged at their real size, the packet's charge as credit, before any guard runs | `gateway.rs::dispatch_event` (`IoSocket::try_charge`) | `the_copies_of_a_body_stay_within_their_charge_for_one_and_two_body_parameters`, `an_event_that_finds_no_room_for_its_copies_is_answered_with_exception_and_its_handler_does_not_run`, `the_heap_of_events_in_flight_stays_within_the_unread_budget_times_a_constant` |
| Global connection guards run once per gateway of the namespace | `gateway.rs::run_connection_guards` | `the_global_guards_run_for_every_gateway_of_a_namespace_whatever_the_provider_order` |
| A socket holds its connection weakly; the session's request is detached from the read buffer | `socket.rs` (`SocketInner::client`), `engine/mod.rs::open_session` | `a_socket_queued_for_its_disconnect_hooks_gives_its_auth_charge_back_and_keeps_no_session`, `the_sockets_of_pending_disconnect_hooks_pin_at_most_the_byte_cap` |

Interactions with the decisions:

- Decision 3: the session keeps an `Arc<Request>` of the detached request and `HandshakeData.request` takes that `Arc`
  (no second `Arc::new`), so `Handshake::request_arc` hands out the session's request. `LEFT_SOCKET_HEAP` counts
  `size_of::<Request>()` for a request allocation: a socket a disconnect hook keeps can outlive its session and then
  holds the session's request alone, so the constant keeps counting it (an upper bound), and `request_cost` in
  `gateway/disconnect.rs` keeps charging the request's contents per pending socket. That over-counts only when several
  sockets of one connection are pending at once, bounded by the namespaces with disconnect hooks.
- Decisions 5 and 6: #35's gate opens after the connection hooks, which run among the connection handlers; the
  dispatcher's tickets are taken in arrival order by the connection loop, after `wait_ready`. The new order (handlers
  after the CONNECT write, then `mark_ready`) keeps both: events still wait for `ready`, and `ready` is set after the
  handlers. The CONNECT-time share check stays in `begin_connect`, before any socket exists.

The untouched tree passes `cargo test -p corvis --features socket-io --no-fail-fast`: 1,646 passed, 0 failed, 56
ignored (112 test binaries).

### 1.2 The upstream suite before the change

`CORVIS_NODE_DIFF=1 cargo test -p corvis --features socket-io --test socket_io_conformance`, 8 runs on `bd398aa`. The
file has two tests that run at the same time: the suite against Corvis and, as a control, against the Node reference.

| Run | Against Corvis | Against the Node reference |
|---|---|---|
| 1 | pass | fail: `should connect then disconnect from a custom namespace` (timeout) |
| 2 | **fail: `should allow connection to the main namespace`, `expected '2' to deeply equal '42["auth",{}]'`** | fail: `should connect then disconnect from a custom namespace` (timeout) |
| 3 | pass | pass |
| 4 | pass | fail: `should close the session upon ping timeout` (`expected 200 to deeply equal 400`) |
| 5 | pass | pass |
| 6 | **fail: `should connect then disconnect from a custom namespace` (timeout)** | fail: `should close the session upon ping timeout` |
| 7 | pass | pass |
| 8 | pass | fail: `should close the session upon ping timeout` |

Corvis failed 2 runs of 8. Run 2 is the failure #184 describes. Run 6 is the same cause in another test: that test also
waits for CONNECT and then registers the listener for `auth` after an `await` (`test-suite.js:539-540`), so it loses
the `auth` frame when both arrive in one read and times out. The control failed 5 runs of 8, with the same `custom
namespace` timeout and with the ping-timeout test, whose 300/200 ms heartbeat is tight while two suites, two servers
and the build share the machine: the suite itself is timing-sensitive under load, whatever the server. Decision 6
removes the cause on Corvis' side (CONNECT and what the connection handler emits leave in separate writes, as in the
reference), but separate writes can still reach a slow client in one read, as they do from the reference. For the
acceptance runs of task 7.4 the two tests are therefore run one at a time (`-- --test-threads=1`), so the control
measures the suite and not the contention between the two runs; the result of each test is recorded separately.

### 2.3 Benchmark before

`LARGE_SIZE=16384 bash bench/socket-io/run.sh` on `7aab240` (the code of `dev` with the benchmark added), Windows 11,
the release binary in the shared target directory. Each figure is the median of three runs (minimum to maximum).

| Scenario | Transport | Connections | Messages | Payload | Runs | Messages/s | p50 (ms) | p99 (ms) | Max (ms) | Allocations/msg | Bytes/msg | Peak RSS (MiB) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| broadcast | polling | 200 | 100 | 64 B | 3 | 1175668 (1083183 to 1242491) | 7.23 (5.11 to 9.09) | 13.47 (12.88 to 14.49) | 15.49 (15.08 to 16.31) | 3.3 (3.3 to 3.7) | 864 (846 to 954) | 50.3 (49.2 to 50.8) |
| broadcast | polling | 200 | 100 | 16384 B | 3 | 41012 (40004 to 41732) | 392.48 (342.09 to 401.11) | 469.00 (427.62 to 469.18) | 483.59 (474.78 to 493.13) | 3.4 (3.4 to 3.8) | 42694 (38126 to 43231) | 77.7 (72.2 to 78.9) |
| broadcast | websocket | 1000 | 100 | 64 B | 3 | 127411 (117981 to 227488) | 7.86 (7.58 to 9.18) | 33.17 (27.43 to 33.41) | 53.34 (49.66 to 57.05) | 3.0 (3.0 to 3.0) | 181 (181 to 181) | 76.8 (74.8 to 77.5) |
| broadcast | websocket | 1000 | 100 | 16384 B | 3 | 65566 (45432 to 65579) | 9.90 (9.61 to 18.95) | 118.64 (99.21 to 119.55) | 205.43 (202.69 to 283.91) | 3.0 (3.0 to 3.0) | 16764 (16764 to 16764) | 93.5 (90.4 to 94.8) |
| emit | polling | 200 | 100 | 64 B | 3 | 26319 (22535 to 28580) | 6.18 (5.47 to 7.40) | 24.01 (18.15 to 24.98) | 38.63 (26.35 to 45.99) | 133.0 (133.0 to 133.0) | 25757 (25757 to 25757) | 48.1 (47.2 to 48.5) |
| emit | websocket | 1000 | 100 | 64 B | 3 | 55823 (46848 to 76890) | 13.91 (11.75 to 14.65) | 60.65 (31.32 to 89.74) | 109.48 (50.69 to 145.16) | 39.0 (39.0 to 39.0) | 1968 (1968 to 1969) | 78.8 (75.7 to 81.5) |
| server-emit | polling | 200 | 100 | 64 B | 3 | 599402 (502452 to 618867) | 6.80 (5.20 to 9.57) | 16.91 (15.81 to 17.29) | 21.83 (20.39 to 22.73) | 19.3 (18.7 to 20.1) | 1929 (1842 to 2146) | 51.1 (47.4 to 51.7) |
| server-emit | polling | 200 | 100 | 16384 B | 3 | 36668 (36048 to 38487) | 4.37 (4.18 to 4.42) | 28.37 (27.78 to 30.91) | 53.47 (49.79 to 56.60) | 44.3 (44.0 to 45.0) | 141330 (140979 to 141487) | 60.2 (59.5 to 60.4) |
| server-emit | websocket | 1000 | 100 | 64 B | 3 | 230050 (123227 to 232466) | 3.16 (1.91 to 3.18) | 15.74 (12.40 to 19.19) | 48.53 (43.50 to 189.28) | 17.3 (17.3 to 17.3) | 864 (863 to 866) | 78.3 (73.7 to 78.9) |
| server-emit | websocket | 1000 | 100 | 16384 B | 3 | 36285 (29786 to 53455) | 0.48 (0.26 to 0.92) | 23.08 (7.76 to 23.70) | 152.21 (85.67 to 428.29) | 17.3 (17.3 to 17.3) | 115431 (115430 to 115431) | 114.9 (111.2 to 118.8) |

Two things found while making the run: the benchmark first ran out of ephemeral ports over the matrix (connections the
client closes stay in `TIME_WAIT`, and a run opens thousands), so its client sockets are reset on close (zero linger);
and the polling `emit` figure (25,757 bytes per message for a 64-byte payload) is dominated by the per-request copy of
the 4 KiB cookie that Decision 3 removes.

### Group 3: release wakeups

- `UnreadGauge::register` replaces `changed`: three `Option<Notified>` in an `async` block over `tokio::select!` and a
  guard (`Registered`) that gives the waiter counts back; no allocation. Each level's notification is created before its
  count is incremented, so a release that reads the count wakes a notification that already exists. `wake` returns at
  once on a chained gauge with no registered waiter; the gateway's gauge keeps `notify_one`. `new` asserts a chain of
  at most three.
- The callers look, register only when blocked, and look again: the WebSocket reader and the held `POST` in place; the
  connection loop registers on the turn after one that found it blocked (`blocked`), and a turn that finds it blocked
  without a registration goes round once more, registered, before it waits.
- `OutQueue::watching` counts the watchers in `wait_ready`; a push notifies `watchers` only when one is counted.
- Tests: `releases_with_nobody_waiting_send_no_notification` (10,000 releases, a rollback, 0 notifications at every
  level), `a_registration_counts_one_waiter_per_level_until_its_wait_ends_or_is_dropped`,
  `a_registration_that_lands_around_a_release_never_misses_it` (registration before the release, after its
  subtraction at each of the three levels, and after it: never paused and unwoken),
  `pushes_with_no_pending_poll_notify_no_watcher_and_a_poll_never_misses_an_item`, and
  `socket_io::tests::a_connection_under_its_limits_never_registers_for_a_release` (10,000 events: the peak of waiters
  on the session's chain stays 0). The interleaving, stampede and stress tests of `gauge_tests` pass with `changed`
  renamed to `register` and nothing else changed.

### Group 4: one handshake request per session

- **The finding was narrower than the review said.** `Request::clone` does not copy the header bytes: a header value
  is a reference-counted `Bytes`, so a clone copies the header map's structure, the URI, the parsed query and the body
  value. Measured with a 16 KiB cookie (`tests/socket_io_memory.rs`): each socket after the first grew the heap by
  about 3.4 KB before and 1.7 KB after (the rest is the socket itself); a `POST` of a session peaked at 55,440 bytes
  before and 53,951 after, of which 51,385 are the HTTP server's own for the same bytes (a request to a route that does
  not exist). The copies are gone, but the figures of Decision 3 ("about 4 KiB less per request") were not there to
  remove. The test that failed before the change is the identity one (`std::ptr::eq` of `Handshake::request()` of two
  sockets, which needs no `testing` hook); the heap bounds of the spec scenarios hold before and after, and are kept as
  regression bounds: per extra socket under the cookie size, and a `POST` under the cookie size beyond the HTTP
  server's own allocation for the same request (`a_post_of_an_existing_session_copies_nothing_of_its_request`; the
  spec's "less than 16 KiB beyond its body" cannot hold literally, since the HTTP server alone peaks at three times
  the headers).
- `Session.handshake` is `Arc<Request>` (the detached request of #35), `Session.ip` is `Arc<str>`;
  `EngineSocket::handshake_arc` and `ip_arc` hand them to `Handshake::new`, which no longer wraps a copy. `Handshake::ip`
  returns a copy of the session's resolved address (Decision 13, task 11.7) instead of resolving `X-Forwarded-For` once
  per socket.
- `EngineServer::handle` parses `Query` (three fields) from the URI, builds the request once and passes it by reference;
  a request with a `sid` that is not an upgrade is built by `Request::for_engine_poll` (moved parts, no query parsing).
  `reserve` debug-asserts that the request it gets carries no `sid`. `handshake_refused` lost its unused parameter and
  became a free function; `open_refused` and `open_session` take what they use.
- Unit test `the_query_keeps_only_the_three_parameters_the_engine_reads` (10,000 extra parameters).
- `cargo doc -p corvis --features socket-io --no-deps`: no warning in `socket_io/` or `ws.rs` (the warnings it prints
  are in `z/regex.rs` and `posture.rs`, untouched here).

### Group 5: fewer copies, and the `tokio-tungstenite` upgrade (fallback taken)

- **Task 5.4 took the fallback of Decision 4.** Every `tungstenite` release with `Bytes` payloads (0.26.2, 0.27, 0.28,
  0.29, 0.30, the latest) depends on `rand` 0.9 while the workspace uses `rand` 0.8, and `deny.toml` denies a second
  generation of `rand` (and, for 0.30, of `digest` through `sha1` 0.11): `cargo deny check` fails with every one of them.
  Allowing it is a decision on the dependency policy of the authentication threat model, outside the files Decision 4
  lists, so the upgrade goes to its own issue and the other three items landed. The upgrade itself was done to the
  point of compiling the whole workspace with 0.30 (MSRV 1.85, below the workspace's 1.89): about forty call sites in
  `ws.rs`, `websockets.rs`, `engine/websocket.rs` and the tests that build `tungstenite` messages; `CloseFrame` lost its
  lifetime and `WebSocketConfig` became non-exhaustive (built with its setters). One thing the follow-up must keep:
  from 0.26 the library allocates a 128 KiB read buffer for every connection when it starts (0.24 read through a
  4 KiB chunk), so every server WebSocket must set `read_buffer_size` back to 4 KiB, or an idle connection costs 32
  times more. The patch of that work is handed over with the implementation report.
- With 0.24 what Decision 4 removes: the encoder writes the Engine.IO type, the header and the JSON into one buffer
  (`count_binary`, bounded by `MAX_DEPTH`, counts the attachments first, allocating nothing; slack over an eighth is
  shrunk); `Encoded` moves by value into the queue as the hidden `EngineMessage::Framed`, which a WebSocket frame takes
  without a copy (`Packet::into_ws`, consuming); a send without attachments queues one message (no group vector);
  `push_group` computes sizes without a vector; `contains_binary` allocates nothing for a value without containers.
  `EngineMessage::Shared` holds the framed text too, so its length is the same and polling writes it as it is.
- What stays until the upgrade: a broadcast's text is still copied into a `String` per WebSocket recipient when its
  frame is written, and a binary attachment into a `Vec<u8>` (0.24 takes owned buffers). The spec scenarios
  "Broadcast" and "Binary attachment" of "An emit is encoded once" are therefore not met; "One recipient" is
  (`a_framed_message_is_its_own_websocket_frame_without_a_copy`).
- Tests: `framed_and_shared_messages_are_the_same_bytes_as_a_text_message` (lengths, polling bytes and frames equal to
  those of the plain message), `a_framed_message_is_its_own_websocket_frame_without_a_copy` (pointer identity),
  `an_emit_queues_the_encoder_s_buffer_and_a_broadcast_one_shared_buffer`; the parser fixtures of the npm encoder pass
  unchanged (they now compare `Encoded::packet`, the text without the type).

### Groups 6 and 7: the connect sequence and the CONNECT write

- `Phase::Joining` (between `Pending` and `Connected`): live for the adapter (`is_live`), found by
  `IoNamespace::socket(id)`, `sockets()` and `socket_count()`, skipped by `sockets_by_ids` (broadcasts and bulk
  operations) and refused by every emit (all of them already required `Connected`). `IoSocket::disconnect(false)`
  does nothing on it (it now checks `connected()` first, as its rustdoc always said); `disconnect_with` from the
  server's own paths (`terminate`, shutdown, a failed step) tears it down silently through `leave_unannounced`:
  `del_all`, out of the table, its slot of the connection removed (not `Left`: the client never joined, so a new
  CONNECT is a first one and a packet for that namespace is one for a namespace it has not joined), its `auth` charge
  given back, no handlers, no DISCONNECT.
- `connect` after the middleware: (1) under `nsps()`, the slot and the phase `Joining`; (2) `add_socket`, which inserts
  only while the phase is `Joining`, read under the table's write lock; (3) `join_own_room` with no lock of the
  connection held; a failure tears the socket down and answers CONNECT_ERROR `Internal server error`; (4)
  `IoSocket::announce`: under the socket's state lock, CONNECT queued with `EngineSocket::send_written` and the phase
  set to `Connected`; a refused queue tears it down; (5) `joined_once`, then wait for the write or the close, then the
  connection handlers and `mark_ready`.
- Deviation from Decision 5, step 1: the phase is set while `nsps()` is held, as the design's step 1 says, so `nsps()`
  is not quite a leaf: a socket's state lock is taken inside it there. Nothing takes `nsps()` while holding a state lock
  (`disconnect_with` releases the state lock before `forget`; `terminate` releases `nsps()` before it disconnects), so
  the order holds. Setting the phase after releasing `nsps()` would let a `terminate` in between find the socket still
  `Pending`, skip it, and leave it to join a closed connection.
- Deviation from Decision 6: `EngineSocket::send_written` queues one message with an acknowledgement (CONNECT has no
  attachments) instead of a `send_all_written` for groups; `push_group` did not need an acknowledgement. `take_batch`
  answers the acknowledgements of the items it takes (a poll); the WebSocket writer already answered after
  `sink.send` (written and flushed); a discarded or closed queue drops them, which resolves the wait with `false`.
- `InMemoryAdapter::add_all` tests the names against its own `sids` map; `LocalNamespace::is_socket_id` is gone. The
  only liveness lookup left under the membership lock is the joining socket's own (`is_live(id)`).
- Test hooks for the sequence: `corvis::socket_io::testing::{pause_connect, ConnectStep, ConnectPause}` (a pause at
  one step, `reached`, `release`, and `finished`, reported by a guard when the attempt ends however it ends). The
  namespace holds the pause in a `OnceLock`, so outside tests every step costs one atomic load.

### Group 10: one place for each duplicated rule

- `RoomLimits { max_rooms, max_name_length }` (with `check`, count first and then each name, and `check_name`) is used
  by `IoSocket::join`, `InMemoryAdapter::add_all` (through `LocalNamespace::room_limits`), `valid_rooms` and
  `with_rooms` (names only, the deferred error kept). Messages and error values unchanged.
- `SocketState::register_ack(target, deadline, max, id)`: phase check, purge of expired entries, cap, then the id (taken
  only once the registration is certain, so a refused call takes no id, as before). Used by `emit_with_ack_within` and
  `register_broadcast_ack`.
- `WarnLimiter`: a two-level table (`&'static str` kind, then the namespace, looked up by `&str`; a name is copied
  only on first insert), `MAX_KEYS` counted over the pairs of every kind, messages built lazily
  (`impl FnOnce() -> String`), the text "since the last one", and a logger context (`SocketIO`, `EngineIO`). The
  engine's three hand-written limiters are one `WarnLimiter` with static kinds and the empty namespace: the session
  caps share one kind (one line a minute for all of them, as before), the `allow_request` timeout and the send-buffer
  overflow have their own. Operator-visible text: the overflow line's suffix was "(N more were closed in the last
  minute without a line)" and is now "(N more of this kind were not logged since the last one)"; the cap and timeout
  lines now carry that suffix too.
- Left as it was: the `allow_request` hook's panic line is not rate limited (it was not one of the three, and a panic
  of the application's own hook is not something this change was asked to bound); listed as a follow-up.

### Group 7, task 7.4: the upstream suite after the change (acceptance not met)

The suite was run after the rebase on `ed1e480`, against Corvis alone (`the_protocol_suite_passes_against_corvis`,
`--test-threads=1`), 10 runs on this branch and, for comparison in the same hour on the same machine, 10 runs on `dev`
(`ed1e480`, the code without this change):

| | Runs passed | Failing tests seen |
|---|---|---|
| `dev` | 1 of 10 | `should allow connection to the main namespace` (and `... with a payload`), `... to a custom namespace` (and `... with a payload`), `should connect then disconnect from a custom namespace`, `should disconnect from the main namespace` |
| this branch | 1 of 10 | the same tests, and once `should ignore HTTP requests with same sid after upgrade` (also seen on `dev` in a separate run) |

The control (the suite against the Node reference) failed most runs on `dev` in that hour too (timeouts), where it had
passed 3 of 8 in the morning. The failures are the suite's own pattern: it waits for one frame with a `once` listener and
registers the listener for the next one after an `await`, so it loses a frame whenever two arrive in one read, and its
300/200 ms heartbeat turns any delay into a ping in place of the expected frame. Decision 6 is implemented (the connection
handlers run after the CONNECT write; over WebSocket the write is the `sink.send` of the writer, flushed; over polling the
take of a poll), and the unit and integration tests show CONNECT and the handlers' first emit in separate writes
(`the_connection_handlers_wait_for_the_connect_write`, `connection_handlers_run_after_connect_and_before_the_first_event`,
`over_polling_connect_is_answered_alone_and_the_handlers_event_in_the_next_poll`,
`over_websocket_the_handlers_event_follows_connect_as_its_own_frame`), but two separate writes on loopback still reach a
client that is slower than the server in one read: the gap between them is the time the CONNECT write's acknowledgement
takes to wake the connect task (microseconds), shorter than Node's, whose next write waits for the previous write's
callback on a later turn of its event loop. Making the gap longer (a timer, a yield) would be the "yield in the writer"
the design rejected, and orders nothing. The acceptance (20 of 20) is therefore not met and #184 is not closed by this
change; what would close it is a fix of the suite's listener pattern (a buffered `waitFor`) applied by
`fetch-suites.mjs`, or keeping the conformance test advisory: a decision for the maintainer, listed as a follow-up.
`socket_io_diff` was not run 20 times for the same reason (it shares the client pattern).

### Group 11

- `corvis::socket_io::testing` (hidden) holds: `pause_connect`/`ConnectPause`/`ConnectStep`, `adapter`, `in_memory`,
  `membership_size`, `check_invariants`, `has_socket` (on a namespace), `adapter_size`, `adapter_invariants`,
  `adapter_has_socket` (on an `InMemoryAdapter` a test adapter wraps), `buffered_amount`, `same_request`,
  `poll_pending`, `post_active` (engine sessions, for group 12), `probe_decode`, `probe_auth`, `DecodeProbe`. The trait
  lost its three hooks and gained `Any` as a supertrait (the tests find the in-memory adapter by downcast; no burden on
  implementors, every `'static` type is `Any`); `InMemoryAdapter::size` is crate-private and returns an error instead of
  panicking; `Membership.examined` exists only under `cfg(test)`; `IoNamespace::adapter` and `IoSocket::buffered_amount`
  are crate-private. The test adapter of `tests/socket_io_rooms.rs` keeps the `Arc<InMemoryAdapter>` it wraps in its
  knobs and asks `testing::adapter_size` for it.
- `#[non_exhaustive]` on the seven enums, each with a `compile_fail` doctest that matches every variant without a
  wildcard (they pass: the match does not compile).
- Dead code: `ServerInner.closed`, `IoSocket::send_encoded`, the parameter of `handshake_refused` (now a free function).
  Comments: `Polled::Packets`, `warn.rs`.
- `parse_json` maps errors only through the seed's cell (its depth bound, 127, is below `serde_json`'s 128); the
  existing `nesting_is_bounded_without_recursing_beyond_the_limit` pins 127 levels decoded and 128 `TooDeep`.
  `begin_connect` returns `Result<(), ProtocolViolation>`.
- `IoServer::new` installs a callback that holds the server weakly; `attach` installs the strong one (not after a
  close). Test `a_server_that_is_never_attached_is_freed_when_its_last_handle_drops` (fails with the old strong callback).
- `WriteEnd::Refused` ends the WebSocket loop as a peer close with a close frame. Test
  `a_close_frame_while_the_server_writes_ends_the_session_as_a_transport_close` (200 rounds, 1 MiB queued); the old arm
  also ended as `transport close` in practice (the session's default reason), so the test pins the outcome rather than
  failing before the change.
- The split: `adapter/mod.rs` (with the lock order in its module docs), `adapter/memory.rs`, `ack.rs`, `info.rs`; the
  public paths are re-exported unchanged, no file outside `socket_io/` changed for it. `tests/review1.rs` became
  `tests/review/{probe,id_rooms,ack_budget,adapter_contract,debug_output,connect_failures}.rs` (no test named a round).
  `cargo test -p corvis --features socket-io -- --list` gives 1,755 tests before and after.
- The lint ratchet (from #191) found two counts above the baseline after the split and the encoder change: one
  `expect_used` (the encoder wrote numbers with `write!(..).expect(..)`; it now appends decimal digits itself) and four
  `missing_docs` (the fields of `SocketInfo`, now in a private module re-exported publicly, where the hidden `adapter`
  module had kept them out of the count; documented). The baseline only moves the file paths of the split; on Windows
  `expect_used` reads one below the Linux baseline because of a `cfg(unix)` line in `application.rs`, untouched.

### Group 8

The code was already there (#35). Test `malformed_forwarded_addresses_from_one_peer_share_one_share_of_the_answer_budget`
(three sessions of one peer behind `TrustProxy::All` with forwarded addresses that do not parse: one address entry in
the answer budget, held answers at most its share, the calls past it end with `ResponsesTooLarge`). It passes on `dev`
as well, since #35 made the change; it pins it.

### Group 12 (partly done)

- `eventually(what, condition)` and `EVENTUALLY` (30 s) in `tests/engine_support/mod.rs`; `testing::poll_pending` and
  `testing::post_active` give the engine tests a condition for "the request reached the server".
- Rewritten: `engine_io.rs` (the three sleeps that waited for a pending poll or a held `POST`), `engine_io_security.rs`
  (the held-`POST` waits at 105, 193 and 409, and the hoarders at 465). Left with a comment that says why: the waits that
  observe that reads stay paused while the peer keeps writing (an absence nothing else shows) and the "not before" wait
  past `first_request_timeout`. Each rewritten suite passed 5 runs in a row.
- Finished after the report: `socket_io_accounting.rs`, `socket_io_lingering.rs`, the wall-clock upper bounds of
  `engine_io_security.rs` and `engine_io_ws.rs`, and the sweep of task 12.4 over every socket-io and engine-io suite
  (below). One hook was added under `corvis::socket_io::testing`: `unread_bytes(&IoSocket)` (what the socket's session
  holds against its unread budget), so the accounting test can wait for a packet to reach the server before its client
  leaves. The synchronous `auth` test counts per thread (a thread-local in its allocator), so it no longer waits for the
  heap of the test before it to go quiet.
- `settle_until(condition)` was not done. `settle()` (40 `yield_now`) and the session tests' `advance` (5) stay: they
  count turns of the single-threaded runtime on the paused clock, which is neither a sleep nor wall-clock time. The
  generic replacement tried, a timer on the paused clock (which only completes when no task is ready), breaks the
  exact-time tests: the timer wheel's 1 ms resolution moves the clock under them (13 tests failed). Writing a condition
  at each of the ~150 call sites is what remains; `settle()` says so in its rustdoc.

### 13.1 Benchmark after

The same command and matrix as 2.3, on the same machine, on the final code of this change (`bench/socket-io/results.md`
is this table):

| Scenario | Transport | Connections | Messages | Payload | Runs | Messages/s | p50 (ms) | p99 (ms) | Max (ms) | Allocations/msg | Bytes/msg | Peak RSS (MiB) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| broadcast | polling | 200 | 100 | 64 B | 3 | 2257744 (2120778 to 2413826) | 3.43 (3.32 to 3.55) | 5.85 (5.71 to 6.12) | 7.07 (7.05 to 7.37) | 0.9 (0.8 to 1.0) | 848 (836 to 913) | 52.5 (48.9 to 54.0) |
| broadcast | polling | 200 | 100 | 16384 B | 3 | 51954 (50480 to 67371) | 253.81 (198.72 to 271.95) | 371.93 (280.61 to 373.45) | 379.61 (288.26 to 391.37) | 0.8 (0.8 to 0.8) | 42936 (42717 to 43089) | 81.5 (74.6 to 81.6) |
| broadcast | websocket | 1000 | 100 | 64 B | 3 | 219039 (209138 to 223443) | 8.20 (8.17 to 8.25) | 28.08 (24.71 to 28.82) | 53.02 (44.84 to 56.22) | 1.0 (1.0 to 1.0) | 133 (133 to 133) | 77.3 (74.0 to 78.8) |
| broadcast | websocket | 1000 | 100 | 16384 B | 3 | 47360 (46243 to 54073) | 19.56 (18.25 to 19.89) | 73.88 (73.23 to 87.11) | 190.41 (142.01 to 222.62) | 1.0 (1.0 to 1.0) | 16716 (16716 to 16716) | 93.5 (89.8 to 95.6) |
| emit | polling | 200 | 100 | 64 B | 3 | 50070 (48573 to 50342) | 3.77 (3.73 to 3.92) | 7.90 (7.87 to 7.96) | 12.54 (11.44 to 12.83) | 51.0 (51.0 to 51.0) | 20125 (20125 to 20126) | 47.1 (46.7 to 47.9) |
| emit | websocket | 1000 | 100 | 64 B | 3 | 98017 (96494 to 100814) | 9.25 (8.91 to 9.43) | 24.00 (23.58 to 25.26) | 42.46 (39.16 to 44.19) | 19.0 (19.0 to 19.0) | 1077 (1077 to 1077) | 84.4 (81.3 to 87.5) |
| server-emit | polling | 200 | 100 | 64 B | 3 | 1439947 (1089461 to 1447744) | 3.18 (2.10 to 3.19) | 6.75 (5.96 to 7.11) | 8.87 (8.78 to 10.09) | 8.2 (8.1 to 8.8) | 1375 (1361 to 1646) | 51.7 (47.3 to 53.8) |
| server-emit | polling | 200 | 100 | 16384 B | 3 | 48231 (47917 to 50664) | 3.58 (3.49 to 3.63) | 15.48 (14.46 to 17.61) | 29.39 (26.33 to 42.43) | 18.9 (18.8 to 19.0) | 122715 (122618 to 122953) | 59.9 (59.4 to 61.1) |
| server-emit | websocket | 1000 | 100 | 64 B | 3 | 215966 (209138 to 227486) | 3.03 (2.81 to 3.13) | 19.51 (15.27 to 22.47) | 46.08 (44.63 to 50.07) | 7.2 (7.2 to 7.2) | 444 (443 to 445) | 77.7 (75.1 to 79.1) |
| server-emit | websocket | 1000 | 100 | 16384 B | 3 | 46872 (43056 to 56675) | 0.63 (0.63 to 0.74) | 15.57 (14.53 to 17.51) | 135.19 (85.45 to 183.35) | 8.2 (8.2 to 8.2) | 82507 (82507 to 82508) | 111.0 (108.7 to 112.9) |

Per decision, the figure it had to move:

| Decision | Figure | Before | After | Met |
|---|---|---|---|---|
| 2 (release wakeups) | allocations per message, `emit` over WebSocket (1,000 x 100, 64 B) | 39.0 | 19.0 | yes (20 fewer; at least 6 asked), and the rate rose from 55,823 to 98,017 messages/s |
| 3 (one request per session) | bytes per message, `emit` over polling (one `POST` and one `GET` per message, 4 KiB cookie) | 25,757 | 20,125 | partly: 5.6 KB less per message, but not "the headers" (see group 4: the header bytes were never copied; what went is the header map, the URI and the parsed query of each request) |
| 4 (fewer copies) | bytes per `server-emit` message, WebSocket, 16 KiB | 115,431 (7.0 payloads) | 82,507 (5.0 payloads) | partly: two payloads less; the target of about two needs the `tokio-tungstenite` upgrade (the fallback was taken) and counts the application's own copy of the payload into its arguments, which the benchmark makes |
| 4 | per broadcast recipient, WebSocket, 16 KiB: bytes / allocations | 16,764 / 3.0 | 16,716 / 1.0 | allocations yes (two fewer: the size vector and the group vector); the payload copy per recipient stays until the upgrade |

All rates went up and the latencies down, most in the polling scenarios (`emit` over polling doubled), where the request
copies and the per-message registrations went away. The figures are from one machine; the comparison is what counts.

### Final checks (after the last code change, on `ed1e480` + this branch)

- `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy -p corvis
  --no-default-features --features socket-io -- -D warnings`; `cargo build -p corvis --features socket-io`: clean.
- `cargo check -p corvis --tests --features <f>` for websockets, compression, tls, auth-local, auth-api-key,
  auth-session, auth-oauth2 and socket-io: no warning (a test-only counter had to be gated on `socket-io`).
- `cargo test --workspace --no-fail-fast`: 2,641 passed, 0 failed, 101 ignored (190 test binaries). An earlier run of
  it failed one test, the first version of the lookup test, which opened 1,000 TCP connections and ran the machine out
  of ephemeral ports (`WSAENOBUFS`) late in the workspace run; it now makes its 1,000 CONNECTs over 50 connections.
- `cargo test -p corvis --features socket-io --no-fail-fast`: 1,698 passed, 0 failed, 57 ignored (114 binaries).
- The CI fuzz steps with `PROPTEST_CASES=20000`: `--features socket-io --lib fuzz` 32 passed, `--lib fuzz` 16 passed.
- Repeated: `socket_io_rooms_security` 30 of 30 (the `socket_of` waits are gone), `socket_io_connect_order` 5 of 5
  (twice), `engine_io` and `engine_io_security` 5 of 5.
- `node scripts/lint-ratchet.mjs`: no count above the baseline (see group 11); `node scripts/check-naming.mjs`,
  `node scripts/check-crate-files.mjs`, `cargo deny check`, `npm run docs:check` (179 + 179 pages),
  `openspec validate` of this change and of `e2-socket-io-redis-adapter`: pass.
- Not met: the upstream suite 20 of 20 (task 7.4, above); `socket_io_diff` was not run in the end, for the same reason.

### What this change does not claim (decided after the report)

- **The no-copy-per-recipient scenarios are deferred to the `tokio-tungstenite` upgrade.** With the fallback of Decision
  4 accepted, the spec delta of `ws/socket-io` keeps "One recipient" (the frame is the encoder's buffer) and states for
  broadcasts only what the code does: one shared encoding queued on every recipient (`One encoding per broadcast`,
  pinned by `an_emit_queues_the_encoder_s_buffer_and_a_broadcast_one_shared_buffer` and
  `a_broadcast_holds_one_copy_of_its_encoding_however_many_sockets_it_reaches`). The scenarios "Broadcast" (every
  recipient's frame points into the shared buffer) and "Binary attachment" (the binary frame is the attachment's
  buffer) were removed from this change's delta: with 0.24 the library takes an owned `String` and `Vec<u8>`, so each
  WebSocket recipient's frame is a copy. They belong to the upgrade issue, which the maintainer files with the patch.
- **The `POST` scenario is stated as what is bounded.** "Handling the `POST` allocates less than 16 KiB beyond its body"
  cannot hold: the HTTP server alone peaks at about three times the headers for such a request (group 4). The delta of
  `ws/engine-io` now says what `a_post_of_an_existing_session_copies_nothing_of_its_request` checks: the engine's
  allocation for the `POST` stays under 16 KiB beyond what the HTTP server allocates for the same request.
- **#184 is not fixed by this change and stays open on its own.** The scenario "Upstream suite is stable" was removed from
  the delta: the suite loses a frame whenever two arrive in one read (its listener is registered after an `await`), and
  the Node reference fails the same way on this machine. What this change guarantees, and its tests pin, is the order and
  the separate writes of "CONNECT leaves in a write of its own". Task 7.4 and the suite part of 15.3 stay unticked.

### 12.4: the waits left in the socket-io and engine-io suites, and why

Rewritten to conditions (`eventually`, `testing::poll_pending`, `testing::post_active`, `testing::unread_bytes`, an
acknowledged `echo` round trip, or the paused clock where the code reads tokio time): every wait for something that has
an observable end. Removed: every wall-clock upper bound (`elapsed() < ...`). What is left, each with a comment beside it:

| Where | What | Why it stays |
|---|---|---|
| `socket_io.rs:149`, `socket_io_rooms.rs:464`, `engine_io_security.rs:310, 570`, `engine_io_ws.rs:189, 253, 313` | `elapsed() >= configured` | "Not before" bounds against the configured timeout, over real sockets |
| `socket_io_security.rs:110`, `engine_io_security.rs:327` | a wait shorter or longer than the configured timeout, then a check that nothing closed | "Not before" bounds over real sockets (the server's timers run on the real clock there) |
| `socket_io_accounting.rs:159` (`Handlers::settle`), `socket_io_memory.rs:219, 273`, `socket_io_rooms_review_memory.rs:137` | a quiet period before reading how much a flood got in | The end of admissions is an absence over real sockets (a paused reader resumes on any release): no state says every flooding reader stopped for good. A shorter wait makes the bound weaker, never wrong |
| `socket_io_gateways.rs:1291`, `socket_io_log.rs:74, 110`, `socket_io_security.rs:197`, `engine_io_security.rs:127, 212, 429` | a quiet period before checking that something did not happen (a discarded event's handler, a second log line, a 33rd handler, reads past a pause) | Absence of an effect on another task, with nothing that marks its end |
| `socket_io_rooms_memory.rs:90`, `socket_io_transit.rs:72` (`settled`) | a quiet heap before or while measuring | The heap of a server on real sockets has no "done" condition; it guards the measurement, it orders nothing |
| `socket_io_accounting.rs:132`, `socket_io_security.rs:146`, `engine_io_security.rs:645`, `engine_support/io_app.rs:77` | a handler, middleware or hook that takes time | The load of the test (slow application code), not a synchronisation; `io_app` is compared with the Node application, which does the same |
| `engine_io_ws.rs:182` | a producer's pace of sending | The application's behavior while the peer does not read |
| `engine_support/mod.rs:32` | the re-check cadence inside `eventually` | Decides nothing: only the condition does |
| `socket_io_accounting.rs:483, 516` | `elapsed()` read for messages and a "not before" bound | No upper bound left |

Each rewritten suite passed 5 runs in a row (`socket_io_accounting`, `socket_io_lingering`, `engine_io_security`,
`engine_io`); the swept ones passed in one run before the final checks.

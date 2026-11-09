# Design

## Context

Design source: the Socket.IO spike #31 (`openspec/changes/archive/2026-11-08-e2-spike-engine-io-v4-socket-io-v5-in-rust/`):
Decision 1 (own implementation, `socketioxide` as fallback behind the same API), Decision 2 (module and feature),
Decision 4 (verification), Decision 5 (security divergences), Review notes 1, 2, 3 and 9. Its delta
`specs/websockets/engine-io` is the starting point of this change's delta, adjusted below. Code verified at `6f5ff4d`
(`origin/dev`).

### Request entry and upgrades

- `handle` (`crates/corvis/src/server.rs:551-614`) is the entry of every request. Under `websockets` it first diverts
  a WebSocket upgrade whose path is a gateway path (`match_upgrade`, `websockets.rs:671-678`, called at
  `server.rs:558-563`), then gives any other upgrade to the raw `upgrade` listeners of `HttpServer`, or destroys it
  when gateways exist (`:565-577`), then reads the body with `from_hyper` (`:472-530`) and dispatches to the router.
- `from_hyper` reads non-multipart bodies with `http_body_util::Limited` (`:499`) under the `request_timeout`
  deadline, rejects a declared `Content-Length` above the limit without reading it, and leaves the body to `drain`
  (`:535-549`), which discards at most `cap` bytes until the deadline so the client still reads the error response.
- The connection is marked upgraded when the response is `101` (`server.rs:2204-2206`). From then on `TimedIo`
  (`:1245-1291`) applies only the write deadline of `send_timeout` (`poll_write_deadline`, `:1577`) and the read
  credit (`ReadCredit`, `:1296-1330`, `READ_CREDIT_MAX` 64 KiB; `read_paused`, `:1839-1879`): reads pause while a
  write is blocked and the credit is spent. Any WebSocket accepted through hyper's upgrade gets this for free.
- `serve` (`:2273-2345`) stops accepting on shutdown, drops the listener (`:2336`) and waits at most 10 s for
  connections to drain (`:2342`). Nothing closes long-lived requests at that point. `CorvisApplication::close`
  (`application.rs:709-724`) runs the lifecycle hooks.
- `HttpServer` (`crates/corvis/src/http_adapter.rs:103-131`) is the `getHttpServer()` of `HttpAdapterHost`, holding
  the `upgrade` listeners (`on_upgrade`, `:110`). `AppShared.http_server` (`router/mod.rs:574`) gives `handle` access
  to it. This is where Node's `engine.attach(httpServer)` has its equivalent.

### WebSocket runtime pieces to reuse

- `OutQueue` (`crates/corvis/src/ws.rs:77-178`): queue bounded in bytes (`push`, `:101-116`: a frame is always
  accepted when nothing is pending, otherwise `PushError::Full` above the limit), `close` with optional discard
  (`:120-137`), `next` for the single writer (`:140-156`), `buffered` (`:163-166`), `shutdown` (`:169-177`). Its items
  are tungstenite `Message`s. `DEFAULT_MAX_BUFFERED_AMOUNT` 8 MiB (`:46`), `DEFAULT_CLOSE_TIMEOUT` 30 s (`:49`),
  `DEFAULT_MAX_UNREAD_BYTES` 16 MiB (`:51`).
- `write_loop` (`:193-221`) drains an `OutQueue` into a sink and returns `WriteEnd::{Closed, Failed(kind),
  Refused}` (`:181-189`); `guarded` (`:224-237`) contains panics of listeners; `UnreadGauge`/`UnreadTicket`
  (`:241-271`) count received bytes not yet taken and pause the read loop above the limit (`run`, `:984-986`);
  `protocol_error` (`:960-967`) maps tungstenite errors to `1009`/`1007`/`1002`.
- Gateways: `validate_handshake` (`websockets.rs:695-722`) does `ws`' checks (method, `Sec-WebSocket-Key`,
  `Sec-WebSocket-Version`) plus the `allowed_origins` check (`:715-720`), but takes the whole `GatewayOptions`;
  `reject_handshake` (`:682-691`) builds the `text/html` rejection with `Connection: close`. `upgrade`
  (`:725-769`) builds the handshake request with `Request::for_upgrade` (`:739`), runs `verify_client` under
  `guarded` (`:742-747`) and configures tungstenite with `max_message_size`/`max_frame_size` (`:750-755`).
  `run_connection` (`:975-1092`) holds the gateway loop: writer task, close deadline, writer arm that ends on
  `WriteEnd::Failed` (`:1027-1044`), protocol errors (`:1055-1063`), `max_inflight` (`:1006`, `:1017`). The client's
  address is resolved once (`ClientInner.ip`, `:241-242`, set at `:988`) and read by `#[ip]` in gateway handlers.
- `Request::for_upgrade` (`crates/corvis/src/http/request.rs:65-78`) builds a body-less request with the
  application's query parser and `trust_proxy`, so `ip()` (`:174-176`) honors `set_trust_proxy`
  (`application.rs:420-421`).
- `CorsOptions` (`crates/corvis/src/router/cors.rs:93-118`) mirrors the `cors` package (the one Node's `engine.io`
  uses for its `cors` option); `is_preflight` (`:286-288`), `preflight` (`:291-298`), and the startup warning for
  `reflect_origin()` with `credentials(true)` (`:189-194`).

### Node engine.io 6.6.x behavior relied on

From the spike's Context plus a reading of `packages/engine.io/lib/{server,socket,transports/polling}.ts` on
`socketio/socket.io` `main` (2026-11-09):

- `verify`: unknown transport → 0; with `sid`: unknown → 1, transport mismatch without upgrade → 3, protocol mismatch →
  3, method not checked; handshake: non-`GET` → 2, `websocket` without upgrade → 3; invalid `Origin` characters → 3.
  `abortRequest` sends 403 only for code 4, 400 otherwise, JSON `{code, message}`. `abortUpgrade` writes a `400`
  status line and destroys the socket.
- Defaults: `pingInterval` 25000, `pingTimeout` 20000, `upgradeTimeout` 10000, `maxHttpBufferSize` 1e6, transports
  polling + websocket, `allowUpgrades` true, `httpCompression` threshold 1024, `cors` off, `cookie` off, `allowEIO3`
  false, path `/engine.io` with trailing slash (socket.io sets `/socket.io/`). No session cap.
- Polling: an overlapping `GET` → `onError("overlap from client")` and 400; an overlapping `POST` → 400; an
  `application/octet-stream` body in v4 → 400; above `maxHttpBufferSize` → `413` without `onError` (the session
  survives); a valid `POST` → `200`, `text/html`, `ok`; responses go through `headers()` which sets
  `cache-control: no-store`. No packet count limit.
- Socket: v4 server pings after `pingInterval`, waits `pingTimeout` for the pong (not interval + timeout), and a pong
  refreshes the interval timer; a client ping → `transport error`. Upgrade: `2probe` → `3probe`, a noop every 100 ms
  on polling while upgrading, `5` switches transports and flushes the same `writeBuffer`; any other packet closes the
  candidate only; a second upgrade attempt, or one after the upgrade, closes the new WebSocket. No bound on
  `writeBuffer`. `close()` on the server closes every socket with `close(true)`, reason `forced close`.

Correction to the spike's delta: `server shutting down` is a Socket.IO disconnect reason (#33); at the Engine.IO level
a server close is `forced close`.

### Harness

#131 provides `parity/socket-io/` (runner, normalizer, comparer, `expected-differences.json`, capabilities, the
`socket-io` reference mode) and `crates/corvis/tests/socket_io_diff.rs` without a Corvis target. Its
`fetch-suites.mjs` downloads the engine.io suite at a pinned commit.

## Goals / Non-Goals

**Goals:**

- Engine.IO v4 observably equal to Node `engine.io` 6.6.11 for the official clients, verified by the harness and the
  upstream suite.
- A bounded default for every resource a client can make the server hold, with a test each.
- Reuse the WebSocket hardening and the request settings (trust proxy, query parser) rather than copy them.
- An internal API that #33 can build `IoServer` on, and that the `socketioxide` fallback could replace.

**Non-Goals:**

- Socket.IO, rooms, gateways, Redis (#33-#36); Engine.IO v3; compression; cookies (proposal, Out of scope).
- A supported public Engine.IO API (Open question 1).

## Decisions

### 1. Feature and module layout

`socket-io = ["websockets"]` in `crates/corvis/Cargo.toml` (`[features]`, `:56-62`), off by default (spike Review
note 1). `crates/corvis/src/lib.rs` gets `#[cfg(feature = "socket-io")] pub mod socket_io;` next to `websockets`
(`:61-65`). Layout:

```
socket_io/mod.rs              re-exports EngineOptions; #[doc(hidden)] pub mod engine
socket_io/engine/mod.rs       EngineServer, EngineSocket, CloseReason, EngineMessage
socket_io/engine/options.rs   EngineOptions, defaults, validation
socket_io/engine/packet.rs    packet and payload codec (text, b+base64, \x1e)
socket_io/engine/sid.rs       session id generation and shape check
socket_io/engine/session.rs   session state machine: outbox, unread gauge, heartbeat, upgrade, close
socket_io/engine/polling.rs   GET/POST/OPTIONS handling, CORS, errors
socket_io/engine/websocket.rs WebSocket transport (WebSocket-first and upgrade candidate)
socket_io/engine/fuzz.rs      proptest properties (named so `cargo test --lib fuzz` selects them)
```

Alternative considered: a separate crate. Rejected by the spike (Decision 2): it needs `pub(crate)` internals of
`ws.rs`, `server.rs` and `websockets.rs`.

### 2. Attaching and the API surface

- `EngineServer::new(EngineOptions) -> Result<EngineServer, EngineOptionsError>` validates options (spec "Options are
  validated"); `server.attach(&HttpServer)` registers it in a new `engines` list of `HttpServer`'s inner state
  (`http_adapter.rs:103-131`), like Node's `engine.attach(httpServer)`. Attaching works before or after `init` and
  before `listen`, since `handle` reads the list per request. Two servers whose paths overlap are rejected at attach.
- `handle` (`server.rs:551`) checks the attached engines first, under `#[cfg(feature = "socket-io")]`, before the
  `websockets` block: a path equal to the engine path or starting with it (Node's prefix rule) goes to
  `EngineServer::handle(req, remote, &shared.http_settings.body_parser, options)` for every method and for upgrades.
  The returned response flows through the same connection code, so a `101` marks the connection upgraded and the
  write deadline and read credit apply.
- Public and documented: `corvis::socket_io::EngineOptions` (builder methods, defaults and the divergences in its
  rustdoc); #33's Socket.IO server options embed it, as Node's `socket.io` options include the `engine.io` ones.
- `#[doc(hidden)] pub mod engine` (unstable, stated in its module doc): `EngineServer` (`attach`, `on_connection`,
  `close`, `session_count`), `EngineSocket` (`id`, `handshake() -> &Request`, `ip()`, `transport()`, `send`,
  `messages()` stream, `close()`, `closed() -> CloseReason`). Integration tests, the conformance server and the
  harness's Corvis target use it; #33 builds `IoServer` on it and decides whether it stays hidden.

Alternatives considered: everything `pub(crate)` with in-crate tests only (rejected: the conformance and diff tests
are integration tests and must start a real server through public items); a supported public `engine` module like
Node's standalone package (rejected for now: it would pin the implementation the spike wants to keep replaceable;
Open question 1).

### 3. Reuse instead of duplication

- `OutQueue` becomes generic over its item, `OutQueue<M: QueueItem = Message>` with `QueueItem::size()`; `close()`
  and the close-frame helpers stay on `OutQueue<Message>`. A session's outbox is an `OutQueue<Packet>` with the
  same byte bound and `PushError::Full` rule. `write_loop` gets a variant taking an encoder (`Packet -> Message`)
  so the WebSocket transport drains the session outbox with the same `WriteEnd` results. Existing callers compile
  unchanged through the default parameter; `tests/ws_security.rs` must pass unmodified.
- `validate_handshake` is split: `validate_ws_handshake(headers, method, allowed_origins: Option<&[String]>)` holds
  the checks of `:700-720`; the gateway wrapper passes `options.allowed_origins.as_deref()`. Engine.IO calls it after
  its own `verify` (Node checks the Engine.IO query first, then `ws` validates the upgrade).
- The handshake request (both transports) is `Request::for_upgrade(parts, remote, &shared.http_settings.body_parser)`
  kept by the session, with `ip()` resolved once, like `ClientInner.ip`; #35 maps `#[ip]` onto it.
- `POST` bodies are read like `from_hyper` (`Limited` + `request_timeout` deadline, `Content-Length` above the cap
  answered `413` before reading, the unread body left to `drain` with the cap), not through the router's body parser.
- The WebSocket transport loop follows `run_connection`'s structure (writer task, writer arm ending on `Failed`,
  close deadline, `protocol_error` codes) but is its own function: the gateway loop's dispatch, `max_inflight` and
  `id` echo do not apply, and changing `run_connection` would risk the gateway behavior the port apps depend on.
- Panics from application callbacks (`on_connection`, consumers of `messages()`) are contained with `guarded` and the
  `catch_unwind` pattern of `dispatch` (`websockets.rs:911-921`).

### 4. Session ids and the session store

15 bytes from `rand::rngs::OsRng` (`rand` 0.8 is already a dependency, `Cargo.toml:43`) encoded with `base64`'s
URL-safe engine without padding: 20 characters, Node's length and alphabet, 120 random bits (Node's `base64id` has 96
random bits plus a counter). The store is a `RwLock<HashMap<SessionId, Arc<Session>>>` with the default SipHash
hasher. A `sid` that is not 20 base64url characters is answered as unknown without touching the map, which bounds the
work per request and keeps attacker-sized strings out of the hasher. Logs name sessions by a per-process counter,
never by sid (the sid is the session's bearer credential).

Alternative considered: 16 bytes (22 characters). Rejected: Node's 20-character shape keeps the harness normalizer and
any client-side assumption identical, and 120 bits is far beyond guessing range.

### 5. Long-polling transport

- One pending `GET` and one in-progress `POST` per session (Node's rule); overlap → `400` and `transport error`.
- A pending `GET` waits on the session's notify; it is answered when the outbox has packets, by a ping, by a noop
  during an upgrade, or by the close packet. Its response encodes packets until adding the next would pass
  `max_http_buffer_size` bytes (at least one packet), so one response never holds the whole 8 MiB outbox and never
  trips the server's per-connection pending-response cap (`max_pending_response_bytes`, `server.rs:71`, 8 MiB).
- `POST`: `413` above the cap (session kept, as Node); `application/octet-stream` → `400`; the body is decoded lossily
  as UTF-8 like Node's string concatenation; the payload is split on `\x1e` without allocating every packet first,
  counting packets, and rejected as a whole with `parse error` past `max_payload_packets` before any packet is
  delivered. A malformed packet closes the session with `parse error` (Node: an `error` packet → `onClose("parse
  error")`); the HTTP answer follows Node, as recorded by the harness.
- Default packets per `POST`: 10,000. A minimal packet is 2 bytes, so a 1 MB body could hold 500,000 packets; each
  decoded packet costs far more than its 2 bytes, an amplification of the body's memory and of the work handed to the
  application. 10,000 is above any burst a client buffers in practice (one packet per emit, plus one per binary
  attachment).
- CORS: `EngineOptions::cors(CorsOptions)`; requests under the engine path never go through the router, so the
  application's `enable_cors` does not apply, as in Node where `engine.io` answers before Express. Preflight and
  response headers come from `CorsOptions::is_preflight`/`preflight`/the header helper; the reflect-with-credentials
  warning is logged at attach. `allowed_origins` reuses the gateway semantics (`websockets.rs:387-390`) on both
  transports.

### 6. WebSocket transport and upgrade

- tungstenite is configured with `max_message_size` and `max_frame_size` equal to `max_http_buffer_size`, as Node's
  `ws` gets `maxPayload` from it.
- The session outbox is the single source of queued packets; only the active transport drains it. On `5` the polling
  transport stops draining (a pending `GET` gets a noop), the WebSocket becomes active and drains from where polling
  stopped, so packets are neither lost, duplicated nor reordered. Packets already moved into a written `GET` response
  belong to that response.
- Upgrade state per session: `None | Probing(candidate) | Upgraded`; a second WebSocket for a session in `Probing` or
  `Upgraded` is closed (Node). `upgrade_timeout` starts when the candidate WebSocket is accepted.
- While `Probing`, a 100 ms interval sends a noop on polling if a `GET` is pending (Node's `check`), so the client's
  poll returns and it can send `5`.

### 7. Heartbeat and timers

Each session runs one task owning its timers (ping interval, ping timeout, upgrade timeout, close timeout), driven by
`tokio::time`, so unit tests run the state machine under `tokio::time::pause()` (dev-dependency `tokio` with
`test-util`, `Cargo.toml:66`) and assert exact virtual times. Integration and harness tests only use short intervals
(300 ms / 200 ms, the suite's values) with "not before the configured value" and "within several seconds" bounds.

### 8. Bounds and their defaults

| Bound | Default | Node 6.6.11 | When exceeded |
|---|---|---|---|
| `max_http_buffer_size` (POST body, WebSocket message, `maxPayload`) | 1,000,000 B | same | `413` (session kept) / close `1009`, `transport error` |
| `max_payload_packets` (packets per POST) | 10,000 | none | session closed, `parse error` |
| bytes per `GET` response | `max_http_buffer_size` (at least one packet) | whole buffer | rest sent on the next poll |
| pending requests per session | 1 `GET` + 1 `POST` | same | `400`, `transport error` |
| `ping_interval` / `ping_timeout` | 25 s / 20 s | same | `ping timeout` |
| `upgrade_timeout`, upgrades in progress | 10 s, 1 per session | same | candidate closed, session kept |
| `max_buffered_amount` (outbox) | 8 MiB (`ws.rs:46`) | none | `transport error` |
| `max_unread_bytes` (received, not taken) | 16 MiB (`ws.rs:51`) | none | WebSocket reads pause; `POST` answered later |
| `max_sessions` | 10,000 per server | none | handshake `503` |
| `close_timeout` (WebSocket closing handshake) | 30 s (`ws.rs:49`) | `ws` 30 s | TCP closed |
| session id | 120 random bits, shape-checked | 96 bits + counter | unknown session |
| `cors`, `allowed_origins` | none | none | no CORS headers / `403` code 4 |

`max_sessions` departs from the spike's "optional cap, default off" (Decision 5): the requirement for this change is a
bounded default for every limit. A session costs little while idle, but each one can hold up to 8 MiB of outbox and
16 MiB of backlog, and handshakes are cheap to send; 10,000 per server is well above the clients of most single
instances and keeps the worst case finite. Over the cap the handshake gets `503` and the JSON body
`{"code":3,"message":"Too many sessions"}` (Open question 2); the official client treats it as a connection error and
retries with its backoff.

### 9. Close and shutdown

Close reasons at this level: `transport close`, `transport error`, `ping timeout`, `parse error`, `forced close`.
`EngineServer::close()` closes every session (`forced close`), answers pending polls with a close packet and rejects
new handshakes with `503`. `serve` calls it, through `shared.http_server`, right after it stops accepting
(`server.rs:2336`), before the 10-second drain; `CorvisApplication::close` calls it too (idempotent), for applications
served with `listen_in_background`.

### 10. Verification

- Unit tests and proptest properties for the codec (round trips, malformed input never panics, output proportional to
  input, packet count bound), sid shape, option validation, and the session state machine under paused time.
- `crates/corvis/tests/engine_io.rs`: raw HTTP (hyper client or `TcpStream`) and `tokio-tungstenite` client against an
  in-process application, one test per spec scenario that does not need Node.
- `crates/corvis/tests/engine_io_security.rs`, in the style of `tests/ws_security.rs`: each bound of Decision 8
  (packet flood, large backlog, slow consumer, unread backlog, session cap, oversized sid, oversized WebSocket message,
  never-read peer, upgrade timeout and second upgrade, overlap), and memory bounds measured as queue sizes, not RSS.
- Harness (#131): `reference/engine-server.mjs` (Node `engine.io` 6.6.11 attached at `/socket.io/`, echo of
  messages, a `flood:<bytes>` command, a loopback-only control route `GET /harness/close-reason/<sid>` returning the
  server-side close reason; options from arguments) and the same app on the Corvis side, started in-process by
  `socket_io_diff.rs` with capability `engine-io`. Wire scenarios under `parity/socket-io/scenarios/engine-io/`, one
  per spec scenario that a client can observe. `expected-differences.json` gets one entry per observable bound of
  Decision 8, with its scenario, plus compression (a scenario that sends `Accept-Encoding: gzip` with a payload above
  1 KiB).
- Conformance: `crates/corvis/tests/engine_io_conformance.rs`, gated like `socket_io_diff.rs`, starts a Corvis
  engine server as the suite's README requires (its ping values, CORS and echo) on `127.0.0.1:0` and runs mocha on
  `parity/socket-io/suites/engine.io-protocol` with `SUITE_URL`. It is a test target, matching the spike's Review
  note 9 (a test-only server, not an example).
- CI (`.github/workflows/ci.yml`): `socket-io` added to the per-feature clippy loop; the Linux test job runs
  `cargo test -p corvis --features socket-io`; the fuzz job runs `cargo test -p corvis --features socket-io --lib
  fuzz`. The Node-gated tests stay out of CI until #37's job.

## Risks / Trade-offs

- Polling/upgrade races are subtle → one task per session owns the state; the single-outbox rule of Decision 6; the
  upstream suite and the upgrade wire scenarios from the first task; the `socketioxide` fallback stays possible
  behind `EngineOptions` and the hidden `engine` module.
- `max_sessions` and `max_payload_packets` can reject legitimate traffic in large deployments → both configurable,
  documented, the cap logs a warning; Open question 2.
- Making `OutQueue` generic touches the gateway and `corvis::ws` hot path → no behavior change, existing security
  tests unmodified, corvis-parity check.
- Pending `GET`s hold HTTP connections (one per polling client); `max_connections` is off by default → bounded by
  `max_sessions` and released at the latest by each ping.
- An Engine.IO path that shadows a `ws`-style gateway path or a route → engine wins, as in Node; documented.
- Timing-dependent tests flake → paused time in unit tests, generous bounds elsewhere, no retries.
- WebSocket over HTTP/2 (RFC 8441) is not supported by the server; over TLS with ALPN `h2` only polling works, as with
  gateways today. Documented.

## Open questions

1. Should the `engine` module become a supported public API (like Node's standalone `engine.io`), or stay hidden once
   #33 exposes `IoServer`? Recommendation: decide in #33; hidden until then.
2. `max_sessions` default 10,000 and the `503` body `{"code":3,"message":"Too many sessions"}`: confirm the number and
   the body (the client only looks at the status). Recommendation: keep both.
3. Should a session be bound to the address that opened it (polls or upgrades from another `ip()` refused)? Node does
   not, and clients behind changing NAT or proxies would break. Recommendation: no, documented as a property of the
   sid being a bearer credential.
4. The issue and `ROADMAP.md` name the change `e2-engine-io-handshake-polling-upgrade-and-heartbeat`; this draft uses
   `e2-engine-io-handshake-polling-upgrade-and-heartbeat` as assigned. Rename one of them when the change is accepted.

## Review notes (2026-11-09, maintainer)

1. The `engine` module stays `#[doc(hidden)]`; #33 decides whether it becomes public.
2. `max_sessions` 10,000 and the `503` body `{"code":3,"message":"Too many sessions"}` are confirmed. The cap is a
   bound by default even though the spike left it off: every limit of this layer has a bounded default.
3. A session is not bound to the address that opened it (clients behind NAT changes and proxies would break). The
   sid is a bearer credential: 120 random bits, never logged, shape-checked before lookup.
4. Open question 4 is settled: the change is named `e2-engine-io-handshake-polling-upgrade-and-heartbeat`, as in the
   issue and ROADMAP.md.
5. Capability names under `ws/` (not `websockets/`) follow the main spec tree; accepted.
6. The shutdown close reason at the Engine.IO level is `forced close`, as Node's engine.io; `server shutting down` is
   for #33.
7. Every timing test runs on Tokio's paused clock or asserts only lower bounds with generous margins: CI runners are
   slow and shared (a Windows SSE test failed on a timing race on 2026-11-09).
8. Security review (task 9.7) is mandatory; run `security-reviewer` rounds until closed, as for the WebSocket work.

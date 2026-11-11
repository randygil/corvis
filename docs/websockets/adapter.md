---
status: partial
---

# Adapters

Corvis gateways use, by default, a built-in adapter equivalent to the one based on Node.js's `ws` package: RFC 6455 WebSockets on the same HTTP server as the application and JSON `{ event, data }` frames. With the `socket-io` feature an application can serve its gateways over Socket.IO instead, for the official `socket.io-client` ([Gateways over Socket.IO](#gateways-over-socket-io)). **There is no API yet to register custom adapters.**

When the `{ event, data }` protocol doesn't work for you (for example, because you talk to a device or a client that already has its own format), you have a lower-level alternative: **`corvis::ws`**, a plain WebSocket server modeled on the `ws` package in `noServer` mode.

## Plain WebSockets with `corvis::ws`

The idea is the same as in Node.js: you listen for the HTTP server's `upgrade` event and decide what to do with each request. Access to the HTTP server is obtained by injecting `HttpAdapterHost`, which is available in any module.

Tertulia exposes a plain WebSocket at `/ws/firehose` that relays every chat event to moderators (`src/firehose.rs`):

```rust
use std::collections::HashSet;
use std::sync::Mutex;

use corvis::JwtService;
use corvis::prelude::*;
use corvis::ws::{WebSocket, WebSocketServer, WebSocketServerOptions};

pub const FIREHOSE_PATH: &str = "/ws/firehose";

#[derive(Default)]
pub struct FirehoseState {
    clients: Mutex<HashSet<WebSocket>>,
}

#[injectable]
pub struct FirehoseService {
    adapter_host: Arc<HttpAdapterHost>,
    jwt: Arc<JwtService>,
    #[default]
    state: Arc<FirehoseState>,
}

impl FirehoseService {
    /// Sends `event` to all open sockets; returns how many it reached.
    pub fn publish(&self, event: Value) -> usize {
        let clients: Vec<WebSocket> = self.state.clients.lock().unwrap().iter().cloned().collect();
        clients.iter().filter(|ws| ws.is_open() && ws.send(event.clone()).is_ok()).count()
    }
}

#[async_trait]
impl OnApplicationBootstrap for FirehoseService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default().max_payload(1024));
        let (state, jwt) = (self.state.clone(), self.jwt.clone());
        server.on_upgrade(move |req| {
            if req.pathname() != FIREHOSE_PATH {
                req.destroy();
                return;
            }
            // Unlike a gateway, here the handshake can be rejected with a 401.
            let user = req.query_param("token").and_then(|t| jwt.verify::<ChatUser>(t).ok());
            if !user.as_ref().is_some_and(ChatUser::is_moderator) {
                let _ = req.respond(Response::text("Moderator token required").with_status(401));
                return;
            }
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| {
                state.clients.lock().unwrap().insert(ws.clone());
                let (s, me) = (state.clone(), ws.clone());
                ws.on_close(move |_code, _reason| {
                    s.clients.lock().unwrap().remove(&me);
                });
                let peer = ws.clone();
                ws.on_message(move |m| {
                    if m.to_text() == "ping" {
                        let _ = peer.send("pong");
                    }
                });
            });
        });
        Ok(())
    }
}
```

Listeners are registered in `on_application_bootstrap` because at that point the application's HTTP server already exists.

## The `upgrade` event

`server.on_upgrade(f)` registers a listener: a function that receives an `UpgradeRequest`. Listeners are synchronous and are called in registration order.

| `UpgradeRequest` method | Description |
|---|---|
| `req.pathname()`, `req.url()`, `req.query_string()`, `req.query_param("x")` | The requested URL; `query_param` reads the query parsed with the application's `set_query_parser` (`None` when it is disabled), `query_string` is the raw string |
| `req.method()`, `req.headers()`, `req.header("origin")`, `req.remote_addr()` | The rest of the request |
| `req.request()` | The complete `Request` (`ip()`, `ips()` and `hostname()` follow `set_trust_proxy`, like HTTP requests) |
| `req.destroy()` | Closes the socket without responding |
| `req.respond(response)` | Responds with a normal HTTP response (adds `Connection: close`) |
| `req.switch_protocols(headers)` | Responds `101` and hands over the raw stream, for protocols that are not WebSocket |
| `req.is_handled()` | Whether some listener has already decided |

The first listener that accepts, responds to, or destroys the request wins. If none decides, the socket is destroyed.

Rules for coexisting with gateways:

- The routes of `#[websocket_gateway]`s are handled first by their gateway.
- As long as there is at least one `upgrade` listener, **every** HTTP/1.1 request with `Connection: upgrade` that is not for a gateway goes to the listeners, not to the router.
- Without listeners, if the application has gateways, an upgrade to a route without a gateway destroys the socket; if there are no gateways, it is handled as a normal request.

## `WebSocketServer` and `WebSocket`

`wss.handle_upgrade(&req, |ws| ...)` validates the handshake (with the same `400` and `405` as the `ws` package) and hands over the socket. Register the socket's listeners inside that callback: frames start being read when the callback finishes, so none is lost.

| `WebSocket` | Description |
|---|---|
| `ws.send(data)` | Queues a frame. Accepts `String`, `&str`, `Value`, `Vec<u8>`, and `Bytes` (text or binary) |
| `ws.send_and_wait(data).await` | Sends and waits for it to be written |
| `ws.on_message(f)` | Received messages; `f` receives a `WsMessage` (`m.to_text()`, `m.as_bytes()`, `m.is_binary()`) |
| `ws.on_close(f)` | Close; `f` receives the code and the reason: `1005` if the peer closed without a code, `1006` if the connection dropped |
| `ws.on_error`, `ws.on_ping`, `ws.on_pong` | Errors and control |
| `ws.messages()` | The messages as a `Stream`, instead of callbacks |
| `ws.close(Some(1000), "bye")`, `ws.terminate()`, `ws.ping(data)` | Close, cut, or send a ping |
| `ws.closed().await` | Waits for the close and returns `(code, reason)` |
| `ws.ready_state()`, `ws.is_open()` | `ReadyState::{Connecting, Open, Closing, Closed}` |
| `ws.protocol()`, `ws.upgrade_request()`, `ws.remote_addr()`, `ws.buffered_amount()` | Connection data (`upgrade_request()` resolves the IP with `set_trust_proxy`; `remote_addr()` is the socket address) |

`WebSocket` can be cloned and stored in a `HashSet`, as `FirehoseState` does. `wss.clients()` returns the open sockets and `wss.close()` closes them.

### Options

| `WebSocketServerOptions` | Default | Effect |
|---|---|---|
| `max_payload(bytes)` | 100 MiB | Maximum size of a message. Lower it on public routes (the example uses 1024) |
| `max_buffered_amount(bytes)` | 8 MiB | Pending bytes per connection; when exceeded `send` fails with `WS_ERR_BUFFER_FULL` and it closes with `1008`. `ws.buffered_amount()` is the equivalent of `bufferedAmount` |
| `close_timeout(duration)` | 30 s | Wait for the close response (also when the peer does not read) |
| `max_unread_bytes(bytes)` | 16 MiB | Unread data in `messages()` before the socket stops being read |
| `client_tracking(bool)` | `true` | Keep sockets in `wss.clients()` |
| `handle_protocols(f)` | First requested subprotocol | Choose the subprotocol |

A `panic` inside a listener or the `handle_upgrade` callback is logged and closes the socket with `1011`; the socket leaves `wss.clients()` and `on_close` is always emitted.

### Peers that stop reading

The server's [`send_timeout`](/http/performance#clients-that-stop-reading-send-timeout) also covers upgraded connections: when the socket of a
`WebSocket` accepts nothing for that long while frames are queued for it, the connection is destroyed without a close
frame. The socket becomes `CLOSED` at once, `on_close` and `closed()` report `1006` with an empty reason, and pending
`send_and_wait` calls fail. Bytes the peer accepts of any frame restart the deadline, received frames (including pings)
do not, and a connection with nothing to send is never closed by it, so use a heartbeat for dead idle peers.
`ServerOptions::send_timeout(None)` turns it off. The `ws` package has no such limit. See
[gateways](/websockets/gateways#peers-that-stop-reading).

## Socket.IO (preview)

The `socket-io` Cargo feature (off by default, it enables `websockets`) adds a Socket.IO v5 server and the Engine.IO v4
server under it, written for Corvis and checked against Node's `socket.io` 4.8.4 and `engine.io` 6.6.x with a
differential harness and the official protocol test suite. It is a preview with a low-level API (`IoServer`,
`IoNamespace`, `IoSocket`), rooms and broadcast, and gateways over Socket.IO; a Redis adapter comes in a later change. The
Engine.IO API under it is hidden from the documentation and may change; `EngineOptions` (embedded in `IoOptions`) is the
part you can rely on.

```toml
corvis = { version = "0.1", features = ["socket-io"] }
```

An Engine.IO server is attached to the application's HTTP server, like Node's `engine.attach(httpServer)`. Every request
whose path equals the engine path (`/socket.io/` by default) or starts with it is served by the engine, for every method
and for WebSocket upgrades, before routes, gateways and `upgrade` listeners: the engine path takes precedence over
anything registered on the same path, as in Node. Other paths behave as without the feature. The application's
`enable_cors` does not apply to the engine path; use `EngineOptions::cors`, which has the options of the `cors` package.

The server speaks HTTP long-polling and WebSocket, upgrades from polling to WebSocket without losing or reordering
packets, pings every 25 s and closes a session whose pong takes more than 20 s. A shutdown (`listen` stopping on
SIGINT, or `CorvisApplication::close`) closes every session with the reason `forced close` and answers pending polls
with a close packet, so the server drains quickly; new handshakes get `503` meanwhile. The session id is a bearer
credential: 120 random bits, checked for its shape before any lookup, never written to logs, and not bound to the client
address. WebSocket over HTTP/2 is not supported, so over TLS with ALPN `h2` only polling works.

### Bounds

Every resource a client can make the server hold has a bounded default, and a zero value is refused when the server is
created. Where Node has no bound, Corvis differs on purpose; each difference is listed with its reason in the harness.

| Bound | Default | Node `engine.io` | When exceeded |
|---|---|---|---|
| `max_http_buffer_size` (`POST` body, WebSocket message) | 1,000,000 bytes | same | `413` (session kept) or close code `1009` and `transport error` |
| `max_payload_packets` (packets in one `POST`) | 10,000 | none | `400`, session closed with `parse error` |
| Bytes in one polling response | `max_http_buffer_size` (at least one packet) | the whole buffer | the rest goes out with the next poll |
| Pending requests per session | one `GET` and one `POST` | same | `400` and `transport error` |
| `ping_interval` / `ping_timeout` | 25 s / 20 s | same | `ping timeout` |
| `upgrade_timeout`, upgrades in progress | 10 s, one per session | same | the candidate is closed, the session stays |
| `max_buffered_amount` (queued for the client) | 8 MiB | none | the send fails, the queue is discarded, `transport error` |
| `max_unread_bytes` (received, not yet read; each message counts its length plus 64 bytes, empty ones too) | 16 MiB | none | WebSocket reads pause; the `POST` answer waits |
| `max_total_unread_bytes` (the same, across all sessions; at least `max_unread_bytes`) | 256 MiB | none | while exceeded, sessions holding more than a fair share (`max(16 KiB, total / max_sessions)`) pause and their `POST`s wait; the others keep reading. Worst case: `max_total_unread_bytes + max_sessions * fair share + max_sessions * max_http_buffer_size` (one message per session, which dominates with the defaults) |
| `max_sessions` | 10,000 | none | handshake answered `503` with `{"code":3,"message":"Too many sessions"}` |
| `max_sessions_per_ip` (per `ip()`, IPv6 per `/64`) | 256 | none | the same `503` |
| `first_request_timeout` (polling session with no request after its handshake) | 10 s, at most `ping_interval + ping_timeout` | none | session closed with `ping timeout`, place released |
| `allow_request` (runs after the caps, within `allow_request_timeout`, 5 s) | off | off | `403` with `{"code":4,"message":"Forbidden"}` before any session exists; also on timeout or panic |
| `close_timeout` | 30 s | 30 s | the connection is closed |
| `cors`, `allowed_origins` | off | off (`allowed_origins` does not exist) | no CORS headers, or `403` with code 4 |

Requests under the engine path never reach guards or throttlers, so use `EngineOptions::allow_request` to plug a rate
limiter or an authentication check into the handshake: it receives the handshake request (`ip()` follows
`set_trust_proxy`) and returns `Admission::Allow` or `Admission::Reject`. It runs for polling handshakes and
WebSocket-first connections, not for requests of an existing session. `max_sessions_per_ip` counts clients behind one
NAT or one CDN that is not a trusted proxy as a single address: raise it with `EngineOptions::max_sessions_per_ip(n)`
for such deployments, or configure `set_trust_proxy` so the real client address is used. Clients behind one shared
address (a mobile carrier's or a company's NAT) share one address quota, so they can delay each other (their sessions
count together, and on the Socket.IO server so does everything they hold of the budget); `trust_proxy` only helps when
the proxy in front is one you control. A session id handed out by a handshake the client never uses is released after
`first_request_timeout`.

#### Memory of the WebSocket transport outside the budget

The WebSocket library holds two buffers per session that no unread budget counts: its read buffer, which grows to hold the
largest frame the session ever received and is not shrunk afterwards, and, while a fragmented message is incomplete, the
buffer of that message (it doubles as it grows). The library (tungstenite 0.24, and 0.30 when measured) has no setting that
shrinks them, and the frame size cannot be lowered without refusing messages Node accepts, so this is measured, stated and
bounded by the options below instead. With `M` = `max_http_buffer_size`, one session holds at most
`1.25 x 2^ceil(log2(M + 4,110))` bytes after a large message, for as long as it lives, and at most
`2 x 2^ceil(log2(M))` more than that while it has an incomplete message. At the default `M` of 1,000,000 the measurement is
1.12 MiB and 3.14 MiB per session (1,169,504 and 3,294,439 bytes).

| Default options | Per session | One address (`max_sessions_per_ip` 256) | The server (`max_sessions` 10,000) |
|---|---|---|---|
| After one message of nearly `M`, until the session ends | 1.12 MiB | 286 MiB | 10.9 GiB |
| While a fragmented message is incomplete | 3.14 MiB | 804 MiB | 30.7 GiB |

These figures are in addition to `max_total_unread_bytes` (256 MiB), which does not count them. Lower them with
`max_http_buffer_size` (they fall in proportion, and 100,000 bytes gives 0.13 MiB and 0.32 MiB per session),
`max_sessions_per_ip` and `max_sessions`. Charging these bytes to the session would need a wrapper around the socket and is
left as a possible later change.

The per-address cap does not stop an attacker who has many addresses: an IPv6 `/48` holds 65,536 `/64`s, and about 40
addresses are enough to fill the default `max_sessions` of 10,000. Rate-limit handshakes at the edge (a reverse proxy
or load balancer) or inside `allow_request`, and size `max_sessions` for the deployment. Behind a proxy the cap counts
the address `ip()` resolves, written in any form (`203.0.113.7`, `203.0.113.7:40000`, `[2001:db8::1]:40000`, with or
without a zone; a value that is no address counts as the socket's peer). With `TrustProxy::All` the client chooses
that address, so the per-address cap protects nothing there: trust only the proxies you run.

Polling responses are not compressed (Node compresses above 1 KiB when the client accepts gzip). Engine.IO v3, cookies and
WebTransport are not supported. The server's [`send_timeout`](/http/performance#clients-that-stop-reading-send-timeout)
applies to the WebSocket transport like to any other upgraded connection.

### The Socket.IO server

An `IoServer` is created with `IoOptions`, given namespaces, middleware and handlers, and attached to the HTTP server.
The namespace `/` exists from the start; every other namespace is registered by the application with `io.of("/admin")`,
because a client cannot create one (dynamic namespaces are not supported). A name gets a leading `/` and may not be
longer than 256 bytes or contain `,` or a control character.

```rust
use corvis::socket_io::{ConnectError, IoOptions, IoServer};

let io = IoServer::new(IoOptions::new())?;

// Middleware runs before the socket joins the namespace; `auth` is what the client sent.
io.of("/admin")?.use_middleware(|socket| async move {
    match socket.handshake().auth().get("token").and_then(|t| t.as_str()) {
        Some("secret") => Ok(()),
        _ => Err(ConnectError::new("not authorized").data(serde_json::json!({ "content": "retry later" }))),
    }
});

io.main().on_connection(|socket| {
    // Handlers are registered per socket, in the connection handler, so no event is missed.
    socket.on("sum", |_socket, args, ack| async move {
        let total: i64 = args.iter().filter_map(|a| a.as_i64()).sum();
        if let Some(ack) = ack {
            let _ = ack.send((total, "ok")); // the client's callback receives (3, "ok")
        }
    });
    let _ = socket.emit("hello", ("world", 1)); // the arguments are a tuple of `Serialize` values
    socket.on_disconnect(|socket, reason| println!("{} left: {reason}", socket.id()));
});

io.attach(&app.get_http_server())?;
```

A server emit that waits for the client's answer is `socket.emit_with_ack("ask", ("question",)).await`, with
`socket.timeout(Duration::from_millis(200))` to choose the deadline; it fails with `operation has timed out`, or at once
when the socket disconnects. Binary values (`corvis::socket_io::Binary`, or `IoValue::Binary`) may sit anywhere in the
arguments and travel as attachments; `args.get::<T>(i)` deserializes an argument into any `serde` type, a field of type
`Binary` takes a buffer. `disconnect(false)` leaves one namespace and `disconnect(true)` leaves all of them and closes the
connection. Each socket reports `client namespace disconnect`, `server namespace disconnect`, `transport close`,
`transport error`, `ping timeout`, `parse error`, `forced close`, `forced server close`, `server shutting down`
(`io.close()` and the application's shutdown report it before the sessions close) or `admission timeout` (a packet waited
longer than `admission_timeout` for room in the unread budget; Node has no such reason), first to the
`on_disconnecting` handlers, while the socket is still in its namespace, and then to the `on_disconnect` handlers.
Reserved event names (`connect`, `connect_error`, `disconnect`, `disconnecting`, `newListener`, `removeListener`) cannot
be emitted by the server, and a packet from a client that uses one is undecodable.

When a socket connects: once its connect middleware passed, the socket is added to the namespace and to the room of
its own id, then CONNECT is queued, and its connection handlers run once CONNECT was written (handed to the WebSocket, or
taken by a poll), so what they emit leaves in a later write, as with the reference server; over polling that is the
next poll. `io.of(name).socket(id)` finds a socket from the moment its client can know its id, with no retry: a socket
returned there, by `sockets()` or counted by `socket_count()`, may still be joining (or already leaving), and its `emit`
then fails with `NotConnected`; broadcasts and bulk operations skip a socket until its CONNECT is queued. If the adapter
fails to add the socket to the room of its own id (it panics or returns an error), the client gets CONNECT_ERROR
`Internal server error`, nothing of the socket stays, and the connection stays open. A connection that closes before
CONNECT was written runs no connection handler.

The request that opened the Engine.IO session is kept once per session and shared by every socket of the connection
(`socket.handshake().request()`): at most one request per session, bounded by the HTTP server's header limit, and not
charged to the unread budget. `DisconnectReason`, `EmitError`, `AckError`, `RoomError`, `AdapterError`,
`BroadcastAckErrorKind` and the Engine.IO `CloseReason` are `#[non_exhaustive]`: a `match` on one of them needs a
wildcard arm, so a release can add a variant.

A packet that cannot be decoded, a second CONNECT for a namespace the connection joined (or is joining), a packet for a
namespace it has not joined (or was refused) and a CONNECT_ERROR from a client close the connection. A packet for a
namespace the connection did join and whose socket has since left (the server called `disconnect(false)` while the
client's packet was already on its way) is dropped and the connection and its other namespaces go on (the Node server closes the whole connection there; this is a deliberate difference): the
packet is read, measured and charged like any other, its attachments are consumed, and then it is discarded; a new CONNECT
for that namespace joins it again.

Handlers and the client's connection: when the client drops the connection (the transport closes or fails, or it stops
answering pings, or it fails to make progress: a protocol violation, a packet that waited longer than `admission_timeout`) the handlers that are still running are cancelled at once, so a handler must not assume it runs to
completion after its client is gone (do the work that has to survive in a task the handler spawns, or release what it
holds in a `Drop`). A client that reconnects in a loop therefore cannot leave a crowd of handlers behind. When the server
closes the connection, or shuts down, the handlers get `close_timeout` to finish; `IoOptions::finish_handlers_after_client_close(true)`
gives them that grace after a client drop too. Handlers that run on after their connection ended count against its
client address, and an address that has `max_sessions_per_ip x max_inflight` of them (8,192 with the defaults) is refused
new sessions with `503` until some end.

#### Socket.IO bounds

Everything a client can make the server hold has a bounded default and a validated option (`IoServer::new` refuses a zero
value and names the option). Where Node has no bound, Corvis differs on purpose.

| Bound | Default | Node `socket.io` 4.8.4 | When exceeded |
|---|---|---|---|
| Packet size (a text packet, and each attachment) | the engine's `max_http_buffer_size`, 1,000,000 bytes | same | `413` or close code `1009` (Engine.IO) |
| `max_attachments` per binary packet, both directions | 10 | 10 when decoding, none when encoding | a received packet that declares more closes the connection before any attachment is read; an emit fails with `TooManyAttachments` |
| Bytes held while a binary packet is reassembled | at most `(max_attachments + 1) x max_http_buffer_size`, counted in `max_unread_bytes`; options that cannot fit are refused | not counted | reads pause |
| `attachment_timeout` (time for the attachments after the header) | 30 s | none | the connection is closed, whatever the heartbeat says |
| `max_decoded_bytes` (the memory a packet decodes to: a 40-byte slot per JSON value, an object adds a box and a hash table, a string its bytes, 24 bytes per allocation) | 32 MiB | none | the connection is closed, before anything is built; reserved in the server-wide budget before decoding and charged until the handler ends |
| Server-wide budget `max_total_unread_bytes` | 256 MiB; a session may always hold its fair share, `max(16 KiB, budget / max_sessions)` | none | the packet waits, and the connection takes no other message |
| `max_address_share`: everything the sessions of one client address hold of that budget, in every state (unread messages above the fair share, packets waiting for room, packets being reassembled, decoded packets and attachments until their handlers end, the `auth` of their sockets) | 1/16 of the budget, never less than one maximum packet (`max_decoded_bytes` plus the wire bytes of the largest packet): with the defaults the floor binds, about 41.5 MiB, so **seven addresses at their share exhaust the 256 MiB budget** (sixteen where the packet limits are small enough for the 1/16 to bind); the sessions an address may have open, `max_sessions_per_ip`, keep their fair shares on top | none | the sessions of that address above their fair share are not read, and their packets wait |
| `admission_timeout` (how long a packet may wait for room) | 30 s | none | the connection is closed with the disconnect reason `admission timeout` |
| `max_auth_bytes` (the `auth` of a CONNECT, kept as long as the socket lives; the stored `serde_json::Value` is charged at its real size, which can be twice the size this limit measures) | 16 KiB | none | the connection is closed |
| JSON nesting | 127 levels | none | the connection is closed |
| `max_event_name_len`, `max_args` (from a client) | 256 bytes, 128 | none | the connection is closed |
| Acknowledgement id from a client | at most 16 digits and 2^53 - 1 | any digit run, read as a float | the connection is closed |
| Namespaces per connection | one socket per registered namespace, attempts in middleware included | the same once joined; pending attempts are unbounded | the connection is closed (`forced server close`) |
| `middleware_timeout` | 10 s | none | CONNECT_ERROR `Internal server error` |
| `connect_timeout` | 45 s | 45 s | the connection is closed |
| `max_inflight` handlers per connection | 32, started in arrival order | none | the connection is not read until one finishes |
| Handlers that outlive their connection, per client address (only after a server close, or when `finish_handlers_after_client_close` is set) | `max_sessions_per_ip x max_inflight`, 8,192 | none (Node runs them to completion) | new sessions from that address get `503` until some end |
| `max_pending_acks` per socket | 1,000 | none | the call fails with `TooManyPending` and sends nothing |
| `ack_timeout` (default deadline of `emit_with_ack`) | 30 s | none | `operation has timed out` |
| Namespace name | 256 bytes, no `,`, no control characters | none | `of` returns an error |
| Socket id | 120 random bits, never the Engine.IO session id | 96 bits plus a counter | n/a |

How much a packet decodes to depends on its shape, not only on its size. Text of ordinary JSON (objects with a handful of
string and number fields) decodes to 11 to 20 times its size, so a 1 MB packet costs 11 to 20 MB and is accepted under the
default. The worst shapes expand more: 1 MB of `{}` costs 45 MB, and 1 MB of objects nested through empty keys costs 58 MB
(the worst case, 58 times). With 1 MB of text the default 32 MiB therefore accepts anything up to 33 times; a packet above
`max_decoded_bytes` closes the connection, and nothing of it is allocated (the text is measured first, then the cost is
reserved and only then is the packet decoded). Everything a client can make the server hold is charged to its session, to its
client address and to the server-wide budget in one chain, whatever state it is in: so one address holds at most its share
(`max_address_share`) plus, for each session it has open, the fair share and the messages in hand (the transport stops
reading a session above its fair share while its address is above the share or the budget is exhausted, so at most one
message of at most `max_http_buffer_size` has been read ahead, and one more is being received), and the server holds at most
`max_total_unread_bytes` plus the same terms for every open session; within `max_unread_bytes` plus one packet for each
session. When the client drops the connection its running handlers are cancelled and release their charges at once; when the
server closes it (or shuts down) they keep their packets charged, to the server and to the client address, until they end (at
most `close_timeout`), and count against the per-address cap above, so reconnecting does not multiply what an address holds. A message is charged for the allocation
behind it, not only its length (a message the WebSocket library assembled from fragments is shrunk when its buffer holds more
than an eighth of slack). Another address keeps the rest of the budget, until enough addresses at their share exhaust it
(see the table; an application behind a reverse proxy must set `trust_proxy`, or every client is the proxy's one address;
clients behind a carrier or corporate NAT share one address and so one share, and can delay each other). The WebSocket
library's own buffers are not in this accounting: the figures per session, address and server, and the options that lower
them, are in "Memory of the WebSocket transport outside the budget" above.

While the budget is exhausted by others a packet bigger than the session's fair share waits for room, and the session is
not read meanwhile. That wait is not the session's doing, so its heartbeat is held (it is not closed with `ping timeout`
for memory it did not hold) for at most `admission_timeout` in total, and a pong already awaited resumes with what was left of its wait (never a whole
new `ping_timeout`), so a client that never answers is closed within `ping_interval + ping_timeout + admission_timeout` of its
last pong however the pressure comes and goes; a packet that cannot be admitted in that time closes the
connection with the reason `admission timeout`. A session that is over its own limit, or whose address is over its share,
keeps the ordinary heartbeat. A waiting packet is admitted as soon as a handler of another connection returns. Bytes of
messages that wait behind a packet that cannot be admitted stay charged until it is admitted or the connection closes.

A packet whose JSON holds an object with a key `serde_json` reads as an instruction (`$serde_json::private::RawValue`) is
delivered, but only as an `IoValue`: `args.get::<serde_json::Value>(i)` and `to_json()` refuse it, because `serde_json`
would parse its value again with a fresh recursion budget when another crate of the build enables `raw_value`. Handlers
that wait for the client's answers (`emit_with_ack`) while the connection has `max_inflight` handlers running cannot receive
the answer: the ACK sits behind the same read gate as every other packet and is taken only once a slot frees, so the call
fails with `operation has timed out` at `ack_timeout`. Do not await acknowledgements from the same client inside many
concurrent handlers (keep the handlers that wait for answers well below `max_inflight`, or send the question from a task
the handler spawns and return).

Differences the client can observe, each listed in the harness with the scenario that exercises it: a second CONNECT while
the middleware of that namespace still runs closes the connection (Node starts another pending socket for each packet),
an acknowledgement id above 2^53 - 1 closes the connection (Node answers with a rounded id), and JSON nested deeper than
127 levels closes the connection (Node delivers it), and so does a string with a lone surrogate escape such as `"\ud800"`
(a Rust string cannot hold one; Node keeps it; a valid pair is fine). Also stricter, with no scenario because the official
client never does them: a packet type that is not a digit, an event or acknowledgement without a payload, an acknowledgement without an
id, an id on a CONNECT, a placeholder index that is not an integer.

A handler that panics is contained and logged once a minute per namespace, with the number left out in the next line; the
line names the namespace and never the panic message or the client's data (so does the warning of a connect middleware that
timed out). That line is the framework's. Containing a panic does not silence the standard library's default panic hook,
which runs before the unwind is caught: it writes the panic message to standard error every time, at the rate the client
sends, and the message holds whatever the handler formatted into it, client data included. So do not `unwrap()` or `expect()`
anything derived from client data (`args.get::<T>(i)?` is a `Result`; so are the conversions of values and the lookups of
keys): a client chooses its data, and a handler that panics on bad input is a handler the client can make fail at will, and
write to your standard error. An application that wants silence installs its own hook with `std::panic::set_hook` (the
framework installs none, since a hook is process-wide). Arguments a handler moves out of `Args`
(`into_vec`) or into a task it spawns stop being charged when the handler returns: they are the application's memory from then on.

Not supported: a Redis adapter (a later change); dynamic namespaces, connection
state recovery, `volatile` and `compress` flags, catch-all listeners, custom parsers, Engine.IO v3 clients and serving the
client bundle.

### Rooms and broadcast

A socket joins rooms with `socket.join(...)` and leaves them with `socket.leave(room)`; `socket.rooms()` lists them. Rooms
belong to a namespace (`r` in `/` and `r` in `/admin` are two rooms), and every socket is in the room named after its own id
from the moment it connects, before its connection handlers run, so emitting to an id reaches that socket. A room is deleted
when its last socket leaves it. A disconnecting socket is still in its rooms while the `on_disconnecting` handlers run (a
handler can read `socket.rooms()` and say goodbye to the people in them) and in none at `on_disconnect`; joining a socket
that has already disconnected does nothing. A client cannot make the server put it in another socket's room by naming that
socket's id: `join` refuses a room named like the id of another socket of the namespace (`RoomError::SocketIdRoom`, and
`sockets_join` skips such a socket), a socket cannot leave the room named like its own id, and the id assigned when a socket
connects is never the name of an existing room. Node allows all three; they are listed differences.

Broadcasts start from a server, a namespace or a socket and chain `to`, `within` (Node's `in`, a keyword in Rust), `except`,
`local` and `timeout`. Each call returns a new operator, so a base operator can be reused. They select the sockets Node's
in-memory adapter selects: `to` takes one or more rooms and reaches the union, each socket once; `except` takes rooms,
socket ids included, and removes their members; without `to` a broadcast reaches every socket of the namespace; a room nobody
is in selects nobody, and so does a `to` or `within` given an empty list (Node reaches everyone, which would let an empty
list that came from a client mean "everyone"; `except` with an empty list changes nothing, and no `to` at all is everyone).
A broadcast started from a socket (`socket.broadcast()`, `socket.to(...)`, `socket.within(...)`, `socket.except(...)`)
excludes that socket.

```rust
use corvis::socket_io::{IoOptions, IoServer};

let io = IoServer::new(IoOptions::new())?;
io.main().on_connection(|socket| {
    // A room name that comes from the client is checked: a name over 256 bytes fails, it is never silently dropped.
    socket.on("join", |socket, args, _ack| async move {
        if let Ok(room) = args.get::<String>(0) {
            let _ = socket.join(room);
        }
    });
    socket.on("say", |socket, args, _ack| async move {
        let (Ok(room), Ok(text)) = (args.get::<String>(0), args.get::<String>(1)) else { return };
        // Everyone in the room but this socket.
        let _ = socket.to(room).emit("said", (text,)).await;
    });
});

// From anywhere that holds the server:
io.emit("tick", (1,)).await?;                                       // every socket of `/`
io.to(["lobby", "news"]).except("muted").emit("notice", ("hi",)).await?;
io.of("/admin")?.to("ops").emit("alert", ("disk",)).await?;
```

The terminal operations (`emit`, `emit_with_ack`, `fetch_sockets`, `sockets_join`, `sockets_leave`,
`disconnect_sockets`) are `async` and return a `Result`, because the adapter behind them may be a cluster one: the same code
works when the adapter is replaced. With the default in-memory adapter they complete without waiting on any client. A
room name over the limit never panics and is never dropped (dropping it from `to` could leave the list empty, which
means everyone): the operator keeps the error and every terminal operation returns `EmitError::InvalidRoom` without sending
or changing anything. An operator names at most `max_rooms_per_socket` rooms in all (`to`, `within` and `except` together):
past that every terminal operation returns `EmitError::InvalidRoom(RoomError::TooManyTargets)`. Chaining is linear: each call
costs the rooms it adds, however long the chain. A call of the adapter made by an operator, and the adapter's `init`, is
bounded by `adapter_timeout` (5 s by default): an adapter that never answers fails the call with `AdapterError::Timeout`.
Reserved event names are refused, as for a direct emit.

#### What a broadcast costs

A broadcast encodes its packet once and queues the same bytes on every recipient (a reference to one shared buffer, binary
attachments included), so while it is delivered the process holds the encoding `E` once, plus a few dozen bytes per
recipient (a reference and a queue entry, and the snapshot of recipients: about 24 bytes each), not `N x E`. It never
waits for a recipient: each recipient's queue is bounded by `max_buffered_amount` (8 MiB by default), and a recipient
that is over it is closed with `transport error` while the emitter and the other recipients go on (the broadcast returns
`Ok`; it does not tell which recipients were closed). There is no server-wide outbound budget yet (issue #176), so a
broadcast to `S` clients that do not read can leave `S x max_buffered_amount` queued, plus one frame in flight per session
as for any emit; size `max_buffered_amount` and `max_sessions` with that product in mind. A WebSocket session copies the
shared text into its own frame when its writer starts, so `S` WebSocket recipients that do not read hold `S` frames at once,
not one: a 1 MiB text to 100 of them held 104.85 MB (measured), where long-polling recipients hold only the shared
reference. One address can pin up to 256 x (8 MiB + one frame) of outbound memory with the defaults, because no server-wide
outbound budget bounds the sessions together (issues #175 and #176).

Room membership is not locked while packets are delivered: a broadcast selects its recipients under a read lock, releases it
and then delivers, so a handler can join, leave and broadcast during a broadcast and a large fan-out does not block joins.
The selection is the moment of the broadcast: a socket that left a room before it started does not receive it, one that
disconnected is never sent a packet, and a socket that joins or leaves while a broadcast is being delivered may or may not
get that one. Packets emitted by one task to one socket, by broadcast or directly, arrive in the order they were emitted:
two broadcasts of one caller, the second awaited after the first returned, reach every recipient they have in common in that
order. Broadcasts of concurrent tasks have no order between them (the handlers of one connection run concurrently, so two
relays of one client are two tasks). A very large fan-out yields to the runtime every 512 recipients.

#### Acknowledgements

`io.timeout(Duration::from_secs(5)).to("room").emit_with_ack("question", ()).await` sends one packet with one id to the
recipients and resolves with the answer of each, in the order they arrive, once all of them answered. It fails with
`BroadcastAckError`, whose message is `operation has timed out` and whose `responses()` hold the answers that arrived, at
the deadline (the `timeout`, else `ack_timeout`, 30 s: Node without `timeout()` fails at once, which is a listed difference),
and with `ResponsesTooLarge` when the answers would pass `max_broadcast_ack_bytes` (the ones that arrived are kept). A
recipient that disconnects never answers, and the call ends at the deadline with what arrived; one that already waits for
`max_pending_acks` answers is sent the event but is not awaited. At the deadline, or when the call is dropped, no recipient
keeps a pending acknowledgement for it. Each recipient's answer counts once and only
recipients of the broadcast answer. Node keeps only the first value of each answer; Corvis returns all of them
(`Args`). While it waits, a broadcast with acknowledgements holds at most `max_broadcast_ack_bytes` (16 MiB) of answers
plus a pending entry (a reference and an id) per recipient. The answers of all pending calls together are bounded too, by a
server-wide answer budget of a quarter of the engine's `max_total_unread_bytes` (never less than `max_broadcast_ack_bytes`),
which belongs to the calls and not to the clients that answered: a connection's unread budget (its session, its address and
the server) is charged only while its answer is read and handled and is released when the call takes the answer, so a
handler that starts many calls cannot make the budget of other clients pay for what the calls hold. One client address may
hold at most a sixteenth of that budget (4 MiB at the defaults), so a single address cannot keep it full; an answer that
would pass the address's share, or the budget, does not wait: the call ends with `ResponsesTooLarge` and the answers
collected so far (the other calls are not affected by a share that is full). The answer budget is outside
`max_total_unread_bytes`: the heap the unread data and the held answers can add up to is `max_total_unread_bytes` plus
`max(max_total_unread_bytes / 4, max_broadcast_ack_bytes)`. A single answer larger than the address share is always refused.

#### Bulk operations

`sockets_join(rooms)`, `sockets_leave(rooms)` and `disconnect_sockets(close)` act on the sockets an operator selects:
`io.to("r1").sockets_join("r2").await?` makes every socket in `r1` join `r2`, and `io.to("r").disconnect_sockets(false)`
makes them leave the namespace (they hear `io server disconnect`) without closing their connections. A socket at its room
cap is skipped and a warning is logged; the others join. `fetch_sockets()` returns `RemoteSocket`s with `id()`,
`handshake()`, `rooms()` and `data()`, and `emit`, `join`, `leave` and `disconnect` that work whether or not the socket is
connected to this process: for a socket of this process they act on that socket directly, for one held by another process
they go through the adapter addressed to that socket's id and to nothing else; `local()` gives the `IoSocket` when it is local.
The result is a snapshot, and its cost is proportional to the selection, not to the server: the handshake of a local socket
is shared, not copied, so a result allocates a few hundred bytes a socket whatever the handshake holds. `Debug` of
`RemoteSocket`, `SocketInfo` and `HandshakeInfo` never prints `auth` values or a query string, and prints header values only for `host`, `user-agent`, `origin`, `accept`,
`accept-encoding`, `accept-language`, `connection`, `upgrade` and `sec-websocket-version`: every other header (cookies,
authorization, `x-api-key` and any custom one) shows its name and `<hidden>`.

An adapter must keep two things in mind: `room_exists` defaults to `false` and a distributed adapter has to implement it,
or the rule that no socket gets an id that is already a room silently stops working; and the synchronous methods
(`add_all`, `del`, `del_all`, `room_exists`) cannot be bounded by `adapter_timeout`, so they must not block or wait for the
network.

#### Rooms bounds

| Bound | Default | Node `socket.io` 4.8.4 | When exceeded |
|---|---|---|---|
| `max_rooms_per_socket` (the room of its own id included) | 1,000 | none | `RoomError::TooManyRooms`; the call joins none of its rooms |
| `max_room_name_length` (in `join`, `to`, `within`, `except`) | 256 bytes, at least 20 | any value | `RoomError::NameTooLong`; in an operator, `EmitError::InvalidRoom` and nothing is sent |
| Rooms in a namespace | at most sockets x `max_rooms_per_socket`; empty rooms are deleted, and a socket that leaves or disconnects leaves no membership behind | unbounded | derived |
| Memory of the memberships | about 475 bytes each, outside every budget: 121.6 MB per namespace for one address at the defaults (256 sessions x 1,000 rooms), 4.75 GB per namespace server-wide (10,000 sessions x 1,000 rooms) | unbounded | derived; lower `max_rooms_per_socket`, `max_room_name_length`, `max_sessions_per_ip` or `max_sessions` |
| Names in one `join` / `sockets_join` | at most `max_rooms_per_socket`, refused before any lock is taken | not capped | `RoomError::TooManyRooms` |
| Rooms named in one `to` / `within` / `except` chain | `max_rooms_per_socket` in all | not capped | `EmitError::InvalidRoom(RoomError::TooManyTargets)`; nothing is sent |
| `adapter_timeout` (a call of the adapter made by an operator, and `init`) | 5 s | none | `AdapterError::Timeout` |
| Answers held by all pending broadcasts with acknowledgements together | a quarter of `max_total_unread_bytes` (at least `max_broadcast_ack_bytes`), a budget of its own that no connection pays for; one client address holds at most a sixteenth of it | none | the call that finds no room (in the budget or in the address's share) ends with `ResponsesTooLarge` at once |
| Encodings per broadcast | one, shared by every recipient | one | n/a |
| Send buffer per recipient | `max_buffered_amount`, 8 MiB | none | that session is closed with `transport error`; the emitter and the others are not affected |
| Emitter waiting on recipients | never | never | n/a |
| Broadcast acknowledgement deadline | `timeout()`, else `ack_timeout`, 30 s | `timeout()`; without it the call fails at once | `operation has timed out` with the answers so far |
| `max_broadcast_ack_bytes` (collected answers of one broadcast) | 16 MiB | none | further answers are dropped; `ResponsesTooLarge` |

Differences a client can see, each listed in the harness with the scenario that exercises it: the 1,001st room of a socket
and a room name of 257 bytes are refused (Node joins them), joining another socket's id is refused and leaving the own id
room does nothing (Node allows both), a `to` given an empty list reaches nobody (Node reaches everyone), and a broadcast
acknowledgement without `timeout()` waits for `ack_timeout` (Node fails before any client answers). Other rows equal Node or
are visible only to server code.

#### The adapter

Rooms and broadcasts go through an adapter, one per namespace: `IoOptions::adapter(factory)` replaces the in-memory default
(Node's `io.adapter(...)`); closing the server closes each namespace's adapter once. Its trait has synchronous membership of the local sockets and `async`, fallible operations for
everything that spans a cluster (broadcast, broadcast with acknowledgements, sockets of rooms, fetch, add and remove sockets,
disconnect, server count), the packet reaches it before it is encoded, and broadcast options carry the rooms, the except
rooms and the flags, as `@socket.io/redis-adapter`'s messages do. It is hidden and unstable until the Redis adapter
(issue #36) implements it for the first time.

### Gateways over Socket.IO

An application chooses how its gateways are served once, before `init()`. The default is the `ws`-style protocol; with the
`socket-io` feature, `IoAdapter` serves **every** gateway of the application over Socket.IO:

```rust
use corvis::prelude::*;
use corvis::socket_io::IoAdapter;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_websocket_adapter(IoAdapter::new());
app.listen("127.0.0.1:3000").await?;
```

Calling it after `init()` logs a warning and has no effect. It changes only the gateways of that application:
`corvis::ws` servers, `on_upgrade` listeners and other applications in the process are not affected, and a plain
WebSocket upgrade to a gateway's namespace is not served (and the other way round: under the default adapter there is no
Socket.IO path). `IoAdapter::options(IoOptions)` sets the options of one Engine.IO path (every limit of the sections above,
`allow_request`, `allowed_origins`, CORS, the cluster adapter), once per path; a second call for a path, or options that do
not validate, make `init()` fail naming the path.

#### What `#[websocket_gateway]` means under Socket.IO

| Form | `ws` adapter (unchanged) | Socket.IO adapter |
|---|---|---|
| `#[websocket_gateway]` | path `/` | namespace `/` on the Engine.IO path `/socket.io` |
| `#[websocket_gateway("/chat")]` | path `/chat` | namespace `/chat` |
| `namespace = "/chat"` | path `/chat` (synonym) | namespace `/chat` |
| `path = "/realtime"` | path `/realtime` | Engine.IO path `/realtime`, namespace `/` unless given |
| `cors = true`, `cors = "https://a"`, `cors = ["https://a", "https://b"]` | no effect (a warning at `init()`) | CORS of the Engine.IO path |

`path` is the server path of whichever platform is used, as in the original framework: a gateway written as
`path = "/chat"` becomes namespace `/` on the Engine.IO path `/chat` when you switch the adapter (the startup log names
the namespace and the path of each gateway). The namespace is checked when the code compiles (at most 256 bytes, no `,`, no
control characters), and an option the macro does not know is a compile error naming `path`, `namespace` and `cors`.
`cors = true` allows any origin without credentials, a string a single origin, an array a list; anything richer is
`EngineOptions::cors` in `IoAdapter::options`. The other Engine.IO and Socket.IO options (`ping_interval`, buffer sizes) are
not macro options: they belong to the server of a path, not to a gateway.

Gateways that share an Engine.IO path are namespaces of one Socket.IO server, so one client connection reaches all of
them. Several gateways may declare the same namespace: their handlers are served together and their hooks run in
registration order, but the same event in two of them fails `init()` (the original runs both handlers; one
acknowledgement cannot be answered twice). `init()` also fails, naming the gateways or the path, when gateways of one path
declare different `cors`, when a gateway declares `cors` and the options for its path already set one, when options are given
twice for a path, or when two Engine.IO paths overlap. A client can reach only the namespaces gateways declare (`/` too is
refused when no gateway declares it): there are no dynamic namespaces.

#### Handles

Under Socket.IO the handles are `IoSocket` (`#[connected_socket]`), `IoGatewayServer` (`#[websocket_server]`) and, for
`after_init`, `IoNamespace`. The lifecycle traits take the handle as a type argument whose default is the `ws` one, so
existing `impl OnGatewayConnection for Chat` blocks compile unchanged:

```rust
use corvis::prelude::*;
use corvis::socket_io::{IoGatewayServer, IoNamespace, IoSocket};
use corvis::{OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, websocket_gateway};

#[websocket_gateway("/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: IoGatewayServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("shout")]
    async fn shout(&self, #[message_body("room")] room: String, #[connected_socket] socket: IoSocket) {
        let _ = socket.id();
        let _ = self.server.to(room).emit("news", ("hello",)).await;
    }
}

#[async_trait]
impl OnGatewayInit<IoNamespace> for ChatGateway {
    async fn after_init(&self, namespace: IoNamespace) {
        // Connect middleware, connection handlers, ...
        let _ = namespace;
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for ChatGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        let _ = socket.handshake().auth();
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for ChatGateway {
    async fn handle_disconnect(&self, _socket: IoSocket) {}
}
```

`IoGatewayServer` offers the broadcast operators of the gateway's namespace (`emit`, `to`, `within`, `except`, `local`,
`timeout`, `fetch_sockets`, `sockets_join`, `sockets_leave`, `disconnect_sockets`) and `namespace()`. It is empty until the
application starts its gateways: before that every operation fails with `EmitError::NotStarted`, sends nothing and does not
panic. A `#[connected_socket]` or `#[websocket_server]` of any other type is a compile error. **A gateway whose handles do
not match the adapter fails `init()`** with a message that names the gateway and the item, before any of its code runs:
a hook that silently never ran could be an authentication check. A gateway may implement each hook for both handles and
then works under either adapter. `#[ip]` is the client address resolved from the handshake with the `trust proxy` setting,
and guards read the socket with `ctx.switch_to_ws().get_client::<IoSocket>()`; `ctx.request()` is the handshake request
under both adapters, which is what a guard shared between them should use.

#### Messages, results and errors

A `#[subscribe_message("e")]` handler runs when a socket of the namespace emits `e`, through the same pipeline as under the
`ws` adapter: global, gateway and handler guards, interceptors, pipes, the handler, and the handler's and gateway's exception
filters. The body (`#[message_body]`, `get_data()`, the input of pipes) is built from the event's arguments as the original
does: none gives `null`, one gives that argument, several give the array of them, and the acknowledgement is not an argument.
An event no handler subscribes to is dropped without a reply and without running guards; a handler registered with
`IoSocket::on` (in `handle_connection`) takes precedence over the gateway's.

| What the handler returns | The client sent a callback | No callback |
|---|---|---|
| nothing, or a value that serializes to `null` | nothing is sent; the callback is not called | nothing |
| `WsResponse::new(event, data)` | emitted as `event`; the callback is not called | emitted as `event` |
| an object with a non-empty string `event` | emitted as that event with its `data` | the same |
| any other value (`false`, `0`, `""` included) | the callback, with the value | dropped |
| an error, a denial, a panic | the event `exception`; the callback is not called | the event `exception` |

A `WsResponse` for a reserved event name (`disconnect`, `connect_error`, ...) sends nothing and is logged with the
gateway and the handler; the socket stays connected. The `exception` payload is the one of the gateway's filters or the
built-in mapping: `{ "status": "error", "message": ..., "cause": { "pattern": <event>, "data": <body> } }` for a
`WsException` with a string (`data` is left out when the event had no argument) and for the denial `Forbidden resource`; the
object itself for `WsException::object`; and `Internal server error` for any other error and for a panic, without their text.
The `cause` is the original framework's default (its `includeCause` option); it carries only what the client sent. An HTTP
exception (`BadRequestException`, a failing validation pipe) keeps its own message and fields, as under the `ws` adapter,
where the original answers `Internal server error`.

Binary values in the arguments are `null` in the body: the pipeline has no binary type. Register `socket.on("upload", ...)`
in `handle_connection` to receive the real `Args` (and a manual acknowledgement).

#### Hooks and the connection gate

`after_init` runs once per gateway with its namespace, before the server is attached and so before any client can connect.
`handle_connection` runs for each socket after the namespace's connect middleware accepted it, in the registration order
of the namespace's gateways; `handle_disconnect` runs exactly once per socket after it left its rooms, whichever way it
left (the client, the server, a timeout, a kick, a shutdown). The original calls `handleConnection` without waiting for it,
so a message sent right after connecting can be handled before an asynchronous hook finishes. **Corvis holds the events of a
socket until every `handle_connection` hook of its namespace returned**: an authentication check in the hook decides before
any handler runs, and an event that arrives meanwhile is neither lost nor handled early. Waiting events count as handlers in
flight (`max_inflight`, 32 per connection): at the limit the server stops reading from that connection, and they are
discarded, without running guards or handlers, when a hook disconnects the socket. A hook that panics, or exceeds
`IoAdapter::hook_timeout` (30 s by default), disconnects the socket and closes its connection; a `handle_disconnect` that
exceeds it is abandoned; an `after_init` that panics or exceeds it fails `init()`. A zero `hook_timeout` fails `init()`.

Events held behind the hook start in the order they arrived, by construction: each event takes a place in its socket's order
when the connection reads it, and starts only when every event before it has started, so an event that arrives just as the
gate opens cannot overtake the ones that waited.

**Disconnect hooks.** A socket that leaves waits for its `handle_disconnect` hooks in one queue for the whole application
(every namespace and Engine.IO path):

- At most 32 hooks run at once, at most 8 of them for one client address, and a free worker serves the addresses with hooks
  waiting in turn: a socket's hook starts after at most one hook of each other address that had hooks waiting before it,
  however many each of them queued.
- Each address has a 64th of the application's caps: while its pending hooks (waiting or running) reach that count, or pin
  that many bytes, new sessions from it are answered `503`, and a CONNECT from it to a namespace with disconnect hooks is
  answered `CONNECT_ERROR` (`Too many pending disconnects`) before any socket exists, until they drain. A pending hook comes
  only from a socket of such a namespace, and such a socket only from a CONNECT that passed that check, so on one Engine.IO
  path an address has at most `share - 1 + max_sessions_per_ip x K` hooks pending (`K` the namespaces with disconnect
  hooks on that path: a session holds at most one socket per namespace), pinning at most
  `byte share + max_sessions_per_ip x K x (what one socket pins)` bytes. The sockets an address already had connected when it reached its share may still
  leave, and their hooks are queued (that is the second term); within this bound **no hook is skipped**. Clients behind one
  address (a NAT, or a reverse proxy without `trust_proxy`) share the 8 workers and the share: while it is full they get
  `503` on new sessions and `CONNECT_ERROR` on those namespaces.
- At most `IoAdapter::max_pending_disconnect_hooks` (10,000) hooks are pending in the application, pinning at most
  `IoAdapter::max_pending_disconnect_hook_bytes` (64 MiB). A socket that leaves while either cap is full is **the one case in
  which a hook is dropped**; it is logged (rate limited). Since every address is held to its share, filling a cap takes many
  addresses at once: at least `cap / (share + max_sessions_per_ip x namespaces with disconnect hooks)`.
- What a pending socket pins: the socket holds its connection weakly, so the Engine.IO session and its buffers are freed
  with the connection, and the session keeps a copy of its handshake (not a slice of the connection's read buffer). Each
  pending socket is charged, at most, its own structures (about 3 KiB), its id, its handshake (`auth`, headers, query) and
  its `data`; what the application attached with `insert` is not counted. When the socket is queued the unread-budget charge
  of its `auth` is released: the queue's budget charges it from then on. The queue's memory is at most the pending hooks
  times that, so at most `max_pending_disconnect_hook_bytes` (measured with a counting allocator: 212 sockets charged
  1,047,524 bytes held 833,724).

`init()` also refuses an Engine.IO path of `/` or an empty one (Engine.IO would take every HTTP request of the application),
an Engine.IO path under which an HTTP route lives (`path = "/status"` with a route `GET /status/ping`: Engine.IO answers every
request under its path before routing), naming the gateway and the route, and a `#[subscribe_message]` for a reserved
Socket.IO event name (`connect`, `connect_error`, `disconnect`, `disconnecting`, `newListener`, `removeListener`: no client can
emit them, so the handler could never run), naming the gateway. A route whose leading segments are parameters or a wildcard
(`/:id/ping`, `/*rest`) is not refused: only the requests whose values spell the Engine.IO path go to Engine.IO, which the
router cannot know of. A duplicated `#[websocket_gateway]` option is a compile error.

**What a message holds.** Once the body is built from the event's arguments, the arguments are dropped, and a gateway handler
holds the body as one shared JSON value and a copy for each `#[message_body]` parameter (a `Value` parameter takes its copy
as it is, without rebuilding it); after the handler, an error's `cause` echoes one more copy, when the parameters are gone.
The charge is `(1 + max(P, 1)) x json_cost(body)` for `P` body parameters, where `json_cost` is the body's real size: one
`Value` slot (72 bytes with `preserve_order`) per array element or object entry, the bytes of every string and key, and the
capacity of every container, each allocation with 24 bytes of overhead; the copies are built with exact capacity, so none
is larger than the body. It goes to the connection's unread budget (session, address and server-wide) by a try-charge that
never waits, **before** any guard runs, with the packet's own charge counted as room (it is released right after, with the
arguments). When there is no room the event is answered with `exception` (`Too many messages in flight`), no guard or handler
sees it, and the callback stays unanswered. Measured with a counting allocator for an array of 2^17 + 1 zeros, the heap of the
event stays within 1.04 times its charge with one body parameter and 1.00 times with two (the tests allow 1.25). The copies
take more memory than the packet did, so when the server-wide budget is under pressure a message the Socket.IO layer would
still read can be refused here: that is the memory its copies really need, and letting it through would put the heap over
the budget.

#### What authenticates a Socket.IO connection

Nothing does by default. The strategies of the authentication module run on HTTP requests and do not run at the WebSocket
handshake, `AuthGuard` denies outside HTTP, and guards run per message. Check the connection yourself, from the outermost in:
`EngineOptions::allow_request` and `allowed_origins` (before a session exists), connect middleware registered in `after_init`
(the socket is not connected until it passes; a rejection is `CONNECT_ERROR`), `handle_connection` (call
`socket.disconnect(true)`), and `IoAdapter::guard_connections(true)`.

`guard_connections` is **off by default**, as in the original: a socket that never sends a message is never checked, yet it
is connected, can be joined to rooms by server code and receives broadcasts. Turn it on and the global guards (while global
enhancers apply to gateways) and the guards declared on the gateways of a namespace also run when a socket connects, before
it is connected. A connection reaches every gateway of its namespace, so the global guards run once **for each gateway**,
each time with that gateway as the class, and all of them must pass: the strictest gateway decides, whatever the order the
gateways were registered in (with a `#[public]` gateway and a protected one on the same namespace, an anonymous client is
refused). The route posture report lists a `connect` entry for each gateway with exactly the guards that run for it. The context is a WebSocket context with the pending socket as client, the handshake as request, `null` data,
the gateway as class and a handler named `connect` that carries the gateway's metadata (so `#[public]` on the gateway is
honored by guards that honor it; handler-level guards run per message only). A denial answers `CONNECT_ERROR` with
`Forbidden resource`, a guard that fails answers its message (and the rest of its payload as `data`), and internal errors and
panics answer `Internal server error`; no hook runs for the socket and it receives no broadcast. Exception filters do not run
for the connection. The default is revisited with the 1.0 defaults.

#### Limits

This layer adds one queue of its own, the disconnect hooks'; the other limits are those of the Engine.IO, Socket.IO and rooms
sections, set through `IoAdapter::options`, plus these:

| Limit | Default | When exceeded |
|---|---|---|
| Handlers in flight per connection, **including events waiting for `handle_connection`** | 32 (`max_inflight`) | reads pause |
| Lifecycle hooks | 30 s (`IoAdapter::hook_timeout`) | the socket is disconnected; `init()` fails |
| `handle_disconnect` hooks running | 32 in the application, 8 per client address | the others wait their turn |
| Pending `handle_disconnect` hooks, and the bytes their sockets pin | 10,000 and 64 MiB in the application (`IoAdapter::max_pending_disconnect_hooks`, `max_pending_disconnect_hook_bytes`); a 64th of each per client address | at the address's share: new sessions from it `503`, a CONNECT from it to a namespace with disconnect hooks `CONNECT_ERROR`; at the application's cap: the hook is dropped and logged (filling it takes many addresses at once, like the session caps) |
| Namespaces a client can reach | the ones gateways declare | `CONNECT_ERROR` `Invalid namespace` |
| Handlers per event per namespace | 1 | `init()` fails |
| Acknowledgements per event | at most 1, one argument | n/a |
| CORS | off; `cors = true` is any origin without credentials | no CORS headers |
| Connection guards | off | n/a |
| Errors sent to clients | internal errors and panics as `Internal server error`, never their text | n/a |
| Adapter and gateway mismatch | `init()` fails | n/a |

`init()` logs a warning for two configurations that are legal but risky: CORS with credentials on an Engine.IO path without
`allowed_origins` (CORS does not apply to the WebSocket transport, so a cookie-authenticated namespace is open to
cross-site WebSocket hijacking unless the allowlist or a connection check is set), and `cors` declared on a gateway under the
`ws` adapter (it has no effect).

#### Differences from the original gateway adapter

Checked with the official client against a reference application written with the original framework's Socket.IO platform
adapter (the differential harness in `parity/socket-io/`). What a client can observe differs in two cases: the payload of an
HTTP exception (the original answers `Internal server error`; Corvis forwards its message and fields) and a binary value in a
message body (`null` in Corvis). Not observable by one client: events are held until `handle_connection` returns; one handler
per event per namespace; startup errors for conflicting options, `cors` and adapter mismatches; connection guards (opt in).

## Security

Global guards and gateway guards do not protect `upgrade` listeners: validate access inside the listener, as the example does with the moderator token. The same goes for static files and the Swagger/Scalar routes; see [Guards](/websockets/guards).

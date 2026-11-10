---
status: partial
---

# Adapters

Corvis gateways use a single built-in adapter, equivalent to the one based on Node.js's `ws` package: RFC 6455 WebSockets on the same HTTP server as the application and JSON `{ event, data }` frames. **There is no API yet to register custom adapters** nor a socket.io-compatible adapter (socket.io namespaces, *long polling*, the socket.io protocol's automatic reconnection).

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

The `socket-io` Cargo feature (off by default, it enables `websockets`) adds an Engine.IO v4 server, the transport layer
of Socket.IO, written for Corvis and checked against Node's `engine.io` 6.6.x with a differential harness and the
official protocol test suite. It is a preview: there is no Socket.IO server yet and it cannot be used from gateways, so
the Engine.IO API is hidden from the documentation and may change. `corvis::socket_io::EngineOptions` is the part you
can rely on; Socket.IO servers will take it as their engine options.

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
| `max_unread_bytes` (received, not yet read) | 16 MiB | none | WebSocket reads pause; the `POST` answer waits |
| `max_sessions` | 10,000 | none | handshake answered `503` with `{"code":3,"message":"Too many sessions"}` |
| `close_timeout` | 30 s | 30 s | the connection is closed |
| `cors`, `allowed_origins` | off | off (`allowed_origins` does not exist) | no CORS headers, or `403` with code 4 |

Polling responses are not compressed (Node compresses above 1 KiB when the client accepts gzip). Engine.IO v3, cookies and
WebTransport are not supported. The server's [`send_timeout`](/http/performance#clients-that-stop-reading-send-timeout)
applies to the WebSocket transport like to any other upgraded connection.

## Security

Global guards and gateway guards do not protect `upgrade` listeners: validate access inside the listener, as the example does with the moderator token. The same goes for static files and the Swagger/Scalar routes; see [Guards](/websockets/guards).

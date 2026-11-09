---
status: documented
---

# Gateways

A **gateway** is a provider that handles WebSocket connections on a path. Just as a controller groups HTTP routes, a gateway groups *message handlers*: each one subscribes to an event name and receives the data the client sends with that name.

The pieces you already know from the HTTP side work here too: dependency injection, [guards](/websockets/guards), [interceptors](/websockets/interceptors), and [pipes](/websockets/pipes). What changes is the transport.

WebSocket support comes in the `websockets` feature of the `corvis` crate, enabled by default.

## Protocol

Gateways speak a simple JSON protocol over text frames. Each incoming frame has this shape:

```json
{ "event": "message", "data": { "room": "general", "text": "hello" } }
```

and each server response has the same one:

```json
{ "event": "sent", "data": { "id": 7, "room": "general", "text": "hello" } }
```

It is the format of the adapter based on Node.js's `ws` package, so any standard WebSocket client (the browser's `WebSocket`, `websocat`, `tokio-tungstenite`) works without additional libraries. Frames that are not JSON and events that have no handler are ignored.

## Your first gateway

The `examples/tertulia` example is a chat with rooms. Its gateway is declared like this (abridged version of `src/chat/chat_gateway.rs`):

```rust
use corvis::prelude::*;
use corvis::{JwtService, WsClient, WsException, WsResponse, WsServer, websocket_gateway};
use corvis_orm::OrmResultExt; // `.http()`: converts an `OrmError` into an `HttpException`

#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]
#[use_interceptors(WsLoggingInterceptor)]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
    jwt: Arc<JwtService>,
    rooms: Arc<RoomsService>,
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("message")]
    #[use_guards(RoomMemberGuard)]
    async fn message(
        &self,
        #[message_body] dto: SendMessageDto,
        #[connected_socket] client: WsClient,
    ) -> HttpResult<WsResponse> {
        let user = client.get::<ChatUser>().ok_or_else(|| WsException::new("Unauthorized"))?;
        let saved = self.rooms.save_message(&dto.room, &user.username, &dto.text).await.http()?;
        // Everyone else in the room receives `message`; the sender receives the `sent` ack.
        self.server.to(&dto.room).except(client.id()).emit("message", &saved);
        Ok(WsResponse::new("sent", saved))
    }

    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "pong"
    }
}
```

And it is registered like any other provider, in the module it belongs to (`src/chat/mod.rs`):

```rust
#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

A gateway that isn't in some module's `providers` is not mounted. Since it is a regular provider, other components can inject it: in Tertulia, `RoomsController` receives `chat: Arc<ChatGateway>` to query who is connected and to send announcements from a REST endpoint.

### `#[websocket_gateway]`

| Form | Effect |
|---|---|
| `#[websocket_gateway(path = "/chat")]` | Handles upgrades to `/chat` |
| `#[websocket_gateway("/chat")]` | The same, with the path as a literal |
| `#[websocket_gateway(namespace = "/chat")]` | Synonym for `path` |
| `#[websocket_gateway]` | Path `/` |

The path is normalized: the leading `/` is added if missing and the trailing one is removed. The global prefix (`app.set_global_prefix`) is **not** applied to gateways: in Tertulia the REST API lives at `/api/...` and the chat at `/chat`.

Below `#[websocket_gateway]` you can put `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]`, and `#[set_metadata(key, value)]`; they apply to all the gateway's handlers. As with controllers, they must go **below** the main attribute because Rust processes attributes in order (see [Exception filters](/websockets/exception-filters) for `#[use_filters]`).

### `#[handlers]` and `#[subscribe_message]`

Handlers are declared in an `impl` block marked with `#[handlers]`, the same attribute that `#[on_event]` and `#[cron]` use. Each method with `#[subscribe_message("event")]` handles the frames whose `event` matches. A single block can mix message handlers with others: Tertulia's gateway also has an `#[on_event("room.created")]` that notifies all clients when a room is created through REST.

Methods can be `async` or not. On top of each `#[subscribe_message]` you can put `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, and `#[set_metadata]`; they add to those of the gateway. A method without `#[subscribe_message]` is a plain helper: putting `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` or `#[set_metadata]` on it is a compile error.

Each event can have only one handler per gateway. If two `#[subscribe_message]` methods use the same event, the application fails to start with `ChatGateway has more than one handler for the "message" event (on_message and on_message_v2); a gateway can have only one handler per event`. Event names are case-sensitive (`"message"` and `"Message"` are different events), and different gateways can handle the same event name.

### Parameters

| Attribute | Receives |
|---|---|
| `#[message_body] dto: T` | The frame's `data` field, deserialized into `T` |
| `#[message_body("room")] room: String` | Only that property of `data` |
| `#[message_body("id", ParseIntPipe)] id: i64` | A property passed through the parameter's own pipes |
| `#[payload]` | Synonym for `#[message_body]` |
| `#[connected_socket] client: WsClient` | The client that sent the frame |

A parameter without an attribute is treated as `#[message_body]`. If the handler needs neither the data nor the client, it can take nothing, like `ping` in the example.

### Return values

| The handler returns | The client receives |
|---|---|
| `()` | Nothing |
| `WsResponse::new("event", data)` | `{ "event": "event", "data": data }` |
| Any `Serialize` value | `{ "event": <the same event>, "data": value }` |
| `Option` of the above | Nothing with `None` |
| `HttpResult<T>` | The above with `Ok`; an `exception` frame with `Err` |

`ping` returns `"pong"`, so the client receives `{ "event": "ping", "data": "pong" }`.

## Errors: `WsException`

A handler, guard, or pipe that fails produces a frame with the `exception` event. `WsException` builds the error, which is an `HttpException` and therefore fits in `HttpResult`:

```rust
// { "event": "exception", "data": { "status": "error", "message": "Unauthorized" } }
return Err(WsException::new("Unauthorized"));

// The object is sent as is as `data`.
return Err(WsException::object(json!({
    "status": "error",
    "code": "ROOM_FULL",
    "message": format!("#{} is full", room.name),
})));
```

| Error | `data` of the `exception` frame |
|---|---|
| `WsException::new(msg)` | `{ "status": "error", "message": msg }` |
| `WsException::object(obj)` | `obj` |
| Another `HttpException` (`BadRequestException`, validation errors...) | `{ "status": "error", "message", ...rest of its body }`, for example with `statusCode` and `errors` |
| Unknown error or `panic` in the handler | `{ "status": "error", "message": "Internal server error" }` (and it is logged) |

`WsException::get_error(&e)` returns the original value of an error created with `WsException`. A `panic` inside a handler does not close the connection.

## Acknowledgements (acks)

If the client's frame includes an `id`, the response (or the `exception` frame) carries the same `id`:

```json
→ { "event": "join", "data": { "room": "rust" }, "id": 42 }
← { "event": "joined", "data": { ... }, "id": 42 }
```

This way the client can correlate each request with its response, as Tertulia's web client does (`public/app.js`). It is a Corvis-specific extension; clients that don't send an `id` notice no difference.

## The server: `WsServer`

The field marked with `#[websocket_server]` receives the gateway's server. Each gateway has its own, and each application creates its own servers, so two applications in the same process (for example, in tests) don't share clients.

| Method | Description |
|---|---|
| `server.emit(event, data)` | Sends to all connected clients; returns how many it reached |
| `server.to(room).emit(event, data)` | Sends to the clients in a room |
| `server.to(room).except(id).emit(...)` | The same, excluding one client (usually the one that originated the message) |
| `server.clients()` | The connected clients |
| `server.client(id)` | A client by its id |
| `server.count()` | How many clients there are |
| `server.set_options(GatewayOptions)` / `server.options()` | Limits and handshake verification (see below) |

`WsServer::for_path("/chat")` returns the server of the last gateway initialized on that path. It exists for one-off cases; the recommended approach is to inject the gateway and use its field.

## The clients: `WsClient`

| Method | Description |
|---|---|
| `client.id()` | Unique connection identifier |
| `client.emit(event, data)` | Sends an `{ event, data }` frame to this client only |
| `client.send_text(text)` | Sends a raw text frame |
| `client.join(room)`, `client.leave(room)` | Joins or leaves a room |
| `client.rooms()`, `client.in_room(room)` | The client's rooms |
| `client.handshake()` | The upgrade's HTTP `Request`: headers, query string, IP (`ip()`, `ips()` and `hostname()` follow `set_trust_proxy`, like HTTP requests; `remote_addr()` keeps the socket address) |
| `client.insert(value)`, `client.get::<T>()` | Data associated with the connection (for example, the authenticated user) |
| `client.disconnect()` | Closes the connection |
| `client.disconnect_with(4401, "Unauthorized")` | Closes with a code and a reason |
| `client.is_closing()` | `true` after `disconnect` |
| `client.buffered_amount()` | Bytes pending to be sent |
| `client.server()` | The `WsServer` it belongs to |

Rooms are neither created nor destroyed: they exist while some client is in them. `disconnect_with` accepts the codes 1000-1003, 1007-1014, and 3000-4999; any other is sent as `1008`. As soon as you call `disconnect`, the client leaves `server.clients()` and frames that arrive afterwards are no longer dispatched.

## Lifecycle

Implement these traits on the gateway to react to connection events. They are detected automatically, without registering them anywhere:

| Trait | Method | When |
|---|---|---|
| `OnGatewayInit` | `after_init(&self, server: WsServer)` | When the gateway is mounted, during `app.init()` |
| `OnGatewayConnection` | `handle_connection(&self, client: WsClient)` | When each connection is accepted |
| `OnGatewayDisconnect` | `handle_disconnect(&self, client: WsClient)` | When each connection closes |

Tertulia authenticates in `handle_connection`. A browser can't send headers when opening a WebSocket, so the token arrives in the query string:

```rust
#[async_trait]
impl OnGatewayConnection for ChatGateway {
    async fn handle_connection(&self, client: WsClient) {
        let handshake = client.handshake();
        let token = handshake
            .query_param("token")
            .map(str::to_string)
            .or_else(|| extract_bearer_token(handshake));
        let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
            client.emit("exception", json!({ "status": "error", "message": "Unauthorized" }));
            client.disconnect_with(CLOSE_UNAUTHORIZED, "Unauthorized");
            return;
        };
        client.insert(user.clone());
        client.emit("welcome", json!({ "id": client.id(), "user": user }));
    }
}

#[async_trait]
impl OnGatewayDisconnect for ChatGateway {
    async fn handle_disconnect(&self, client: WsClient) {
        let Some(user) = client.get::<ChatUser>() else { return };
        // The client is no longer in `server.clients()`, but it keeps its rooms.
        for room in client.rooms() {
            self.notify_left(&room, &user);
        }
    }
}
```

To reject the connection **before** completing the handshake (with an HTTP `401` or `403`), use `GatewayOptions::verify_client` or `allowed_origins`, or an `upgrade` listener with [`corvis::ws`](/websockets/adapter).

## Options and limits: `GatewayOptions`

Options are set on the gateway's server, usually in `after_init`, and apply to the handshakes and connections that start afterwards:

```rust
use std::time::Duration;
use corvis::{GatewayOptions, OnGatewayInit, WsServer};

#[async_trait]
impl OnGatewayInit for ChatGateway {
    async fn after_init(&self, server: WsServer) {
        server.set_options(
            GatewayOptions::default()
                .allowed_origins(["https://chat.example.com"])
                .max_payload(64 * 1024)
                .close_timeout(Duration::from_secs(10))
                .verify_client(|req| req.query_param("token").is_some()),
        );
    }
}
```

| Option | Default | Effect |
|---|---|---|
| `max_payload(bytes)` | 1 MiB | A larger message closes the connection with `1009` |
| `max_buffered_amount(bytes)` | 8 MiB | Bytes pending to be sent per client; when exceeded they are discarded and the connection closes with `1008` |
| `max_inflight(n)` | 32 | Simultaneous handlers per connection; at the cap, reading from the socket stops |
| `close_timeout(duration)` | 30 s | Wait for the close response before cutting the TCP connection |
| `allowed_origins([...])` | No list | An `Origin` that isn't in the list receives `403`; requests without an `Origin` are accepted |
| `verify_client(f)` | None | If `f` returns `false`, the handshake receives `401` |

The route posture report lists each gateway handshake as not checked, with whether `verify_client` is set: the check cannot enforce what runs before the first message. See [WebSocket guards](/websockets/guards#route-posture-for-gateways).

`allowed_origins` is important if you authenticate the gateway with cookies: without that list, a page from another domain could open a connection on the user's behalf.

`client.buffered_amount()` returns the pending bytes. Pings are answered automatically (a single pong stays pending). A `panic` in `handle_connection` closes the connection with `1011`, and one in `handle_disconnect` is logged; in both cases the client leaves `server.clients()`.

The handshake is validated the same way as in the `ws` package: `405` if the method is not `GET` and `400` if `Sec-WebSocket-Key` or `Sec-WebSocket-Version` are missing or invalid.

## Peers that stop reading

A client whose socket accepts nothing for [`send_timeout`](/http/performance#clients-that-stop-reading-send-timeout) (60 s by default, a server
option) while the gateway has frames for it loses its connection. The connection is destroyed without a close frame,
because nothing can be delivered to that peer: `handle_disconnect` runs, the client leaves `server.clients()` and its
`max_connections` slot is freed. One warning naming the client is logged.

- Progress is measured at the socket: every byte the client accepts, of any frame (data, pings, pongs, close),
  restarts the deadline. Frames received from the client never do, so a client that pings but does not read is still
  dropped.
- It only runs while the server has something to write. An idle connection is never closed by it; to detect dead peers
  on idle connections, send your own heartbeat.
- The same applies when a closing handshake is waiting for a peer that does not read: the connection ends after
  `send_timeout`, not after `close_timeout`.
- While a write is blocked the server reads from that client only as many bytes as the socket has accepted, plus a
  64 KiB allowance, so a flood of pings cannot make it queue unbounded pongs, and a client that writes a moderately large
  message before it reads is still served. Every byte read counts against that credit, even when no write is blocked,
  so a client that has sent much more than it has received may be cut by `send_timeout` if it then stops reading.
- `ServerOptions::send_timeout(None)` turns it off; the queue is then bounded only by `max_buffered_amount`.

The `ws` package has no such limit: it keeps a peer that never reads. Corvis diverges on purpose (the memory and the
connection slot of that peer would be held forever); nginx applies its `send_timeout` to WebSocket connections the same
way.

## Concurrency and ordering

Handlers of the same connection **start** in the order their frames arrived and then run in parallel, up to `max_inflight`. A synchronous `join` followed by a `message` always sees the room already joined; but if a handler does an `await` before `client.join`, the next frame may start being processed before it finishes.

## Global enhancers

Global guards, interceptors and pipes (`app.use_global_guards`, `use_global_interceptors`, `use_global_pipes` and the `Provider::app_guard`, `app_interceptor`, and `app_pipe` providers) apply to every message handler, as in the original framework. They run before the gateway's and the handler's own enhancers: global, then gateway, then handler (providers registered with `Provider::app_*` come ahead of the `use_global_*` ones). Global exception filters do **not** apply to gateways; declare filters with `#[use_filters]` (see [Exception filters](/websockets/exception-filters)).

Things to keep in mind:

- A global guard that only knows HTTP (it reads a bearer token from `ctx.switch_to_http()`, for example) refuses or breaks every message. Tell the transports apart with `ctx.get_type()` (`ContextType::Ws`), or mark the gateway with metadata the guard checks, such as `#[set_metadata(IS_PUBLIC_KEY, true)]` below `#[websocket_gateway]`.
- A global interceptor sees WebSocket contexts too, so one that reads the HTTP request or sets headers has to check `ctx.get_type()` first.
- A global pipe such as `ZodValidationPipe::auto()` runs on every `#[message_body]`. Repeating it on the gateway, as Tertulia does, makes it run twice, which is harmless for validation pipes.
- A global `ThrottlerGuard` counts messages per handshake IP and handler. Behind a reverse proxy, configure `app.set_trust_proxy(...)` so that the handshake IP is the client's and not the proxy's (see [Rate limiting](/security/rate-limiting#behind-a-proxy)).

::: warning Difference with a Node gateway behind Express
In Node, a WebSocket upgrade never goes through Express, so `trust proxy` does not apply to it: a gateway guard sees the proxy's socket address (or nothing, with the stock throttler). Corvis applies the `set_trust_proxy` setting to the handshake, as it does for HTTP requests, so with a trust setting `client.handshake().ip()` is the forwarded client address. As with HTTP, a setting that trusts too many hops lets a client choose its IP with `X-Forwarded-For`.
:::

Applications that adopt gateways gradually can keep global enhancers HTTP-only with one call before `init()` (or `listen`):

```rust
app.apply_global_enhancers_to_gateways(false);
```

With `false`, a handler runs only its gateway's and its own enhancers. Calling it after the application was initialized logs a warning and changes nothing.

## Testing a gateway

`examples/tertulia/tests/chat.rs` starts the application on a real port with `app.listen_in_background("127.0.0.1:5170")` and connects with `tokio-tungstenite`. The same test uses `corvis::testing::request(&app)` for the HTTP routes. See [Testing](/fundamentals/testing) for the rest of the utilities.

## See also

- [Pipes](/websockets/pipes), [Guards](/websockets/guards), and [Interceptors](/websockets/interceptors) in gateways.
- [Adapters](/websockets/adapter): plain WebSockets with `corvis::ws`, without the `{ event, data }` protocol.
- [Server-Sent Events](/http/server-sent-events), if you only need to send data from the server to the client.

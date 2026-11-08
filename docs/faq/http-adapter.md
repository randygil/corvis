---
status: documented
---

# HTTP adapter

Sometimes a provider needs the underlying HTTP server, for example to accept WebSocket
connections "by hand" or to know which address the application is listening on. That is what
`HttpAdapterHost` is for, a provider that the container always registers and that you can inject into
any class.

## Injecting `HttpAdapterHost`

```rust
use corvis::prelude::*;

#[injectable]
pub struct FirehoseService {
    adapter_host: Arc<HttpAdapterHost>,
}
```

You do not need to import any module: it lives in the container's internal global module, like
`Reflector`.

| Method | Returns |
|---|---|
| `adapter_host.http_adapter()` | The application's `HttpAdapter` |
| `adapter_host.listening()` | `true` once the server is listening |
| `http_adapter().get_http_server()` | The `HttpServer` |
| `http_adapter().get_type()` | `"corvis"` (there are no other adapters) |
| `server.address()` | The listening address (`Option<SocketAddr>`) |
| `server.on_upgrade(listener)` | Registers a listener for the `upgrade` event |
| `server.upgrade_listener_count()`, `server.remove_all_upgrade_listeners()` | Listener management |

From the bootstrap code you have the same on the application: `app.get_http_adapter()`,
`app.get_http_server()` and `app.get_url()` (the URL once it is listening).

The server exists from the moment the application is created, but it does not listen until `listen`. Register the
listeners in `on_application_bootstrap`, which runs before the port is opened.

## `upgrade` listeners

`on_upgrade` receives each HTTP/1.1 request with `Connection: upgrade` as an `UpgradeRequest`. The
listener can accept it as a WebSocket with `corvis::ws::WebSocketServer`, respond to it or destroy it.
`examples/tertulia` uses it for a plain WebSocket for moderators only:

```rust
#[async_trait]
impl OnApplicationBootstrap for FirehoseService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default().max_payload(1024));
        let (state, jwt) = (self.state.clone(), self.jwt.clone());
        server.on_upgrade(move |req| {
            if req.pathname() != "/ws/firehose" {
                req.destroy();
                return;
            }
            let user = req.query_param("token").and_then(|t| jwt.verify::<ChatUser>(t).ok());
            if !user.as_ref().is_some_and(ChatUser::is_moderator) {
                let _ = req.respond(Response::text("Moderator token required").with_status(401));
                return;
            }
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| {
                state.clients.lock().unwrap().insert(ws.clone());
                // ...
            });
        });
        Ok(())
    }
}
```

Rules:

- As long as there is at least one listener, **every** upgrade request goes to the listeners and not to the
  router. The routes of `#[websocket_gateway]` are handled first by their gateway, so both
  coexist.
- Listeners run in registration order. The first one that accepts, responds to or destroys the
  request wins; if nobody decides, the socket is destroyed.
- `UpgradeRequest` offers `pathname()`, `url()`, `query_param()`, `headers()`, `header()`,
  `method()`, `remote_addr()` and `request()`. For protocols that are not WebSocket,
  `switch_protocols(headers)` responds `101` and hands over the raw stream.

Global guards are not applied to these listeners: validate the request inside the listener, as in
the example. More details in [WebSocket adapters](/websockets/adapter).

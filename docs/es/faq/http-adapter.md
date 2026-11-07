---
status: documentado
---

# Adaptador HTTP

A veces un provider necesita el servidor HTTP subyacente, por ejemplo para aceptar conexiones
WebSocket "a mano" o para saber en qué dirección escucha la aplicación. Para eso existe
`HttpAdapterHost`, un provider que el contenedor registra siempre y que puedes inyectar en
cualquier clase.

## Inyectar `HttpAdapterHost`

```rust
use corvis::prelude::*;

#[injectable]
pub struct FirehoseService {
    adapter_host: Arc<HttpAdapterHost>,
}
```

No hace falta importar ningún módulo: vive en el módulo interno global del contenedor, igual que
`Reflector`.

| Método | Devuelve |
|---|---|
| `adapter_host.http_adapter()` | El `HttpAdapter` de la aplicación |
| `adapter_host.listening()` | `true` cuando el servidor ya escucha |
| `http_adapter().get_http_server()` | El `HttpServer` |
| `http_adapter().get_type()` | `"corvis"` (no hay otros adaptadores) |
| `server.address()` | La dirección de escucha (`Option<SocketAddr>`) |
| `server.on_upgrade(listener)` | Registra un listener del evento `upgrade` |
| `server.upgrade_listener_count()`, `server.remove_all_upgrade_listeners()` | Gestión de listeners |

Desde el código de arranque tienes lo mismo en la aplicación: `app.get_http_adapter()`,
`app.get_http_server()` y `app.get_url()` (la URL una vez que escucha).

El servidor existe desde que se crea la aplicación, pero no escucha hasta `listen`. Registra los
listeners en `on_application_bootstrap`, que se ejecuta antes de abrir el puerto.

## Listeners `upgrade`

`on_upgrade` recibe cada petición HTTP/1.1 con `Connection: upgrade` como un `UpgradeRequest`. El
listener puede aceptarla como WebSocket con `corvis::ws::WebSocketServer`, responderla o destruirla.
`examples/tertulia` lo usa para un WebSocket plano solo para moderadores:

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

Reglas:

- Mientras haya al menos un listener, **toda** petición de upgrade va a los listeners y no al
  router. Las rutas de los `#[websocket_gateway]` se atienden primero con su gateway, así que ambos
  conviven.
- Los listeners se ejecutan en orden de registro. Gana el primero que acepta, responde o destruye la
  petición; si nadie decide, el socket se destruye.
- `UpgradeRequest` ofrece `pathname()`, `url()`, `query_param()`, `headers()`, `header()`,
  `method()`, `remote_addr()` y `request()`. Para protocolos que no son WebSocket,
  `switch_protocols(headers)` responde `101` y entrega el stream crudo.

Los guards globales no se aplican a estos listeners: valida la petición dentro del listener, como en
el ejemplo. Más detalles en [Adaptadores de WebSockets](/es/websockets/adapter).

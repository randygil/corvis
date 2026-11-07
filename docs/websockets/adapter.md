---
status: parcial
---

# Adaptadores

Los gateways de Corvis usan un único adaptador integrado, equivalente al basado en el paquete `ws` de Node.js: WebSockets RFC 6455 sobre el mismo servidor HTTP de la aplicación y frames JSON `{ event, data }`. **No hay todavía una API para registrar adaptadores propios** ni un adaptador compatible con socket.io (namespaces de socket.io, *long polling*, reconexión automática del protocolo de socket.io).

Cuando el protocolo `{ event, data }` no te sirve (por ejemplo, porque hablas con un dispositivo o un cliente que ya tiene su propio formato), tienes una alternativa de más bajo nivel: **`corvis::ws`**, un servidor WebSocket plano modelado sobre el paquete `ws` en modo `noServer`.

## WebSockets planos con `corvis::ws`

La idea es la misma que en Node.js: escuchas el evento `upgrade` del servidor HTTP y decides qué hacer con cada petición. El acceso al servidor HTTP se obtiene inyectando `HttpAdapterHost`, que está disponible en cualquier módulo.

Tertulia expone en `/ws/firehose` un WebSocket plano que retransmite cada evento del chat a los moderadores (`src/firehose.rs`):

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
    /// Envía `event` a todos los sockets abiertos; devuelve a cuántos llegó.
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
            // A diferencia de un gateway, aquí se puede rechazar el handshake con un 401.
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

Los listeners se registran en `on_application_bootstrap` porque en ese momento el servidor HTTP de la aplicación ya existe.

## El evento `upgrade`

`server.on_upgrade(f)` registra un listener: una función que recibe un `UpgradeRequest`. Los listeners son síncronos y se llaman en orden de registro.

| Método de `UpgradeRequest` | Descripción |
|---|---|
| `req.pathname()`, `req.url()`, `req.query_string()`, `req.query_param("x")` | La URL pedida |
| `req.method()`, `req.headers()`, `req.header("origin")`, `req.remote_addr()` | El resto de la petición |
| `req.request()` | La `Request` completa |
| `req.destroy()` | Cierra el socket sin responder |
| `req.respond(response)` | Responde con una respuesta HTTP normal (agrega `Connection: close`) |
| `req.switch_protocols(headers)` | Responde `101` y entrega el stream crudo, para protocolos que no son WebSocket |
| `req.is_handled()` | Si algún listener ya decidió |

El primer listener que acepta, responde o destruye la petición gana. Si ninguno decide, el socket se destruye.

Reglas de convivencia con los gateways:

- Las rutas de los `#[websocket_gateway]` se atienden primero con su gateway.
- Mientras haya al menos un listener `upgrade`, **toda** petición HTTP/1.1 con `Connection: upgrade` que no sea de un gateway va a los listeners, no al router.
- Sin listeners, si la aplicación tiene gateways, un upgrade a una ruta sin gateway destruye el socket; si no hay gateways, se atiende como una petición normal.

## `WebSocketServer` y `WebSocket`

`wss.handle_upgrade(&req, |ws| ...)` valida el handshake (con los mismos `400` y `405` del paquete `ws`) y entrega el socket. Registra los listeners del socket dentro de ese callback: los frames empiezan a leerse cuando el callback termina, así que no se pierde ninguno.

| `WebSocket` | Descripción |
|---|---|
| `ws.send(data)` | Encola un frame. Acepta `String`, `&str`, `Value`, `Vec<u8>` y `Bytes` (texto o binario) |
| `ws.send_and_wait(data).await` | Envía y espera a que se escriba |
| `ws.on_message(f)` | Mensajes recibidos; `f` recibe un `WsMessage` (`m.to_text()`, `m.as_bytes()`, `m.is_binary()`) |
| `ws.on_close(f)` | Cierre; `f` recibe el código y el motivo: `1005` si el par cerró sin código, `1006` si la conexión se cortó |
| `ws.on_error`, `ws.on_ping`, `ws.on_pong` | Errores y control |
| `ws.messages()` | Los mensajes como `Stream`, en lugar de callbacks |
| `ws.close(Some(1000), "bye")`, `ws.terminate()`, `ws.ping(data)` | Cerrar, cortar o enviar ping |
| `ws.closed().await` | Espera el cierre y devuelve `(código, motivo)` |
| `ws.ready_state()`, `ws.is_open()` | `ReadyState::{Connecting, Open, Closing, Closed}` |
| `ws.protocol()`, `ws.upgrade_request()`, `ws.remote_addr()`, `ws.buffered_amount()` | Datos de la conexión |

`WebSocket` se puede clonar y guardar en un `HashSet`, como hace `FirehoseState`. `wss.clients()` devuelve los sockets abiertos y `wss.close()` los cierra.

### Opciones

| `WebSocketServerOptions` | Por defecto | Efecto |
|---|---|---|
| `max_payload(bytes)` | 100 MiB | Tamaño máximo de un mensaje. Bájalo en rutas públicas (el ejemplo usa 1024) |
| `max_buffered_amount(bytes)` | 8 MiB | Bytes pendientes por conexión; al superarlo `send` falla y se cierra con `1008` |
| `close_timeout(duración)` | 30 s | Espera de la respuesta al cierre |
| `max_unread_bytes(bytes)` | 16 MiB | Datos sin leer en `messages()` antes de dejar de leer el socket |
| `client_tracking(bool)` | `true` | Mantener los sockets en `wss.clients()` |
| `handle_protocols(f)` | Primer subprotocolo pedido | Elegir el subprotocolo |

Un `panic` dentro de un listener o del callback de `handle_upgrade` se registra y cierra el socket con `1011`.

## Seguridad

Los guards globales y los de los gateways no protegen los listeners `upgrade`: valida el acceso dentro del listener, como hace el ejemplo con el token de moderador.

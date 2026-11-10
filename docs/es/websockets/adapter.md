---
status: partial
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
| `req.pathname()`, `req.url()`, `req.query_string()`, `req.query_param("x")` | La URL pedida; `query_param` lee la query parseada con el `set_query_parser` de la aplicación (`None` si está desactivado), `query_string` es el texto crudo |
| `req.method()`, `req.headers()`, `req.header("origin")`, `req.remote_addr()` | El resto de la petición |
| `req.request()` | La `Request` completa (`ip()`, `ips()` y `hostname()` siguen `set_trust_proxy`, como las peticiones HTTP) |
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
| `ws.protocol()`, `ws.upgrade_request()`, `ws.remote_addr()`, `ws.buffered_amount()` | Datos de la conexión (`upgrade_request()` resuelve la IP con `set_trust_proxy`; `remote_addr()` es la dirección del socket) |

`WebSocket` se puede clonar y guardar en un `HashSet`, como hace `FirehoseState`. `wss.clients()` devuelve los sockets abiertos y `wss.close()` los cierra.

### Opciones

| `WebSocketServerOptions` | Por defecto | Efecto |
|---|---|---|
| `max_payload(bytes)` | 100 MiB | Tamaño máximo de un mensaje. Bájalo en rutas públicas (el ejemplo usa 1024) |
| `max_buffered_amount(bytes)` | 8 MiB | Bytes pendientes por conexión; al superarlo `send` falla con `WS_ERR_BUFFER_FULL` y se cierra con `1008`. `ws.buffered_amount()` equivale a `bufferedAmount` |
| `close_timeout(duración)` | 30 s | Espera de la respuesta al cierre (también cuando el par no lee) |
| `max_unread_bytes(bytes)` | 16 MiB | Datos sin leer en `messages()` antes de dejar de leer el socket |
| `client_tracking(bool)` | `true` | Mantener los sockets en `wss.clients()` |
| `handle_protocols(f)` | Primer subprotocolo pedido | Elegir el subprotocolo |

Un `panic` dentro de un listener o del callback de `handle_upgrade` se registra y cierra el socket con `1011`; el socket sale de `wss.clients()` y `on_close` se emite siempre.

### Pares que dejan de leer

El [`send_timeout`](/es/http/performance#clientes-que-dejan-de-leer-send-timeout) del servidor también cubre las conexiones actualizadas: cuando el
socket de un `WebSocket` no acepta nada durante ese tiempo mientras hay tramas en cola para él, la conexión se destruye
sin trama de cierre. El socket pasa a `CLOSED` de inmediato, `on_close` y `closed()` informan `1006` con motivo vacío y
las llamadas pendientes a `send_and_wait` fallan. Los bytes que el par acepta de cualquier trama reinician el plazo, las
tramas recibidas (incluidos los pings) no, y una conexión sin nada que enviar nunca se cierra por esto, así que usa un
heartbeat para pares inactivos muertos. `ServerOptions::send_timeout(None)` lo desactiva. El paquete `ws` no tiene este
límite. Ver [gateways](/es/websockets/gateways#pares-que-dejan-de-leer).

## Socket.IO (vista previa)

La feature de Cargo `socket-io` (desactivada por defecto, activa `websockets`) añade un servidor Engine.IO v4, la capa de
transporte de Socket.IO, escrito para Corvis y comprobado contra `engine.io` 6.6.x de Node con un arnés diferencial y la
suite oficial de pruebas del protocolo. Es una vista previa: todavía no hay servidor Socket.IO y no se puede usar desde
los gateways, así que la API de Engine.IO está oculta en la documentación y puede cambiar.
`corvis::socket_io::EngineOptions` es la parte en la que puedes confiar; los servidores Socket.IO la tomarán como sus
opciones de motor.

```toml
corvis = { version = "0.1", features = ["socket-io"] }
```

Un servidor Engine.IO se adjunta al servidor HTTP de la aplicación, como `engine.attach(httpServer)` de Node. Toda
petición cuya ruta sea igual a la ruta del motor (`/socket.io/` por defecto) o empiece con ella la atiende el motor, con
cualquier método y para los upgrades de WebSocket, antes que las rutas, los gateways y los listeners `upgrade`: la ruta
del motor tiene precedencia sobre cualquier cosa registrada en la misma ruta, como en Node. Las demás rutas se comportan
como sin la feature. El `enable_cors` de la aplicación no se aplica a la ruta del motor; usa `EngineOptions::cors`, que
tiene las opciones del paquete `cors`.

El servidor habla HTTP long-polling y WebSocket, pasa de polling a WebSocket sin perder ni reordenar paquetes, envía ping
cada 25 s y cierra una sesión cuyo pong tarda más de 20 s. Un apagado (`listen` al detenerse por SIGINT, o
`CorvisApplication::close`) cierra todas las sesiones con el motivo `forced close` y responde los polls pendientes con un
paquete de cierre, de modo que el servidor drena rápido; mientras tanto los nuevos handshakes reciben `503`. El id de
sesión es una credencial al portador: 120 bits aleatorios, con la forma comprobada antes de cualquier búsqueda, nunca se
escribe en los logs y no se liga a la dirección del cliente. WebSocket sobre HTTP/2 no está soportado, así que con TLS y
ALPN `h2` solo funciona polling.

### Límites

Cada recurso que un cliente puede hacer que el servidor retenga tiene un valor por defecto acotado, y un valor cero se
rechaza al crear el servidor. Donde Node no tiene límite, Corvis difiere a propósito; cada diferencia está listada con su
motivo en el arnés.

| Límite | Por defecto | `engine.io` de Node | Al excederlo |
|---|---|---|---|
| `max_http_buffer_size` (cuerpo del `POST`, mensaje WebSocket) | 1.000.000 bytes | igual | `413` (la sesión se mantiene) o cierre con código `1009` y `transport error` |
| `max_payload_packets` (paquetes en un `POST`) | 10.000 | ninguno | `400`, sesión cerrada con `parse error` |
| Bytes en una respuesta de polling | `max_http_buffer_size` (al menos un paquete) | todo el búfer | el resto sale con el siguiente poll |
| Peticiones pendientes por sesión | un `GET` y un `POST` | igual | `400` y `transport error` |
| `ping_interval` / `ping_timeout` | 25 s / 20 s | igual | `ping timeout` |
| `upgrade_timeout`, upgrades en curso | 10 s, uno por sesión | igual | se cierra el candidato, la sesión sigue |
| `max_buffered_amount` (en cola para el cliente) | 8 MiB | ninguno | el envío falla, se descarta la cola, `transport error` |
| `max_unread_bytes` (recibido, aún sin leer; cada mensaje cuenta su longitud más 64 bytes, también los vacíos) | 16 MiB | ninguno | se pausa la lectura del WebSocket; la respuesta del `POST` espera |
| `max_total_unread_bytes` (lo mismo, entre todas las sesiones; al menos `max_unread_bytes`) | 256 MiB | ninguno | mientras se excede, las sesiones con más de una cuota justa (`max(16 KiB, total / max_sessions)`) se pausan y sus `POST` esperan; las demás siguen leyendo. Peor caso: `max_total_unread_bytes + max_sessions * cuota justa + max_sessions * max_http_buffer_size` (un mensaje por sesión, que domina con los valores por defecto) |
| `max_sessions` | 10.000 | ninguno | handshake con `503` y `{"code":3,"message":"Too many sessions"}` |
| `max_sessions_per_ip` (por `ip()`, IPv6 por `/64`) | 256 | ninguno | el mismo `503` |
| `first_request_timeout` (sesión de polling sin ninguna petición tras su handshake) | 10 s, como máximo `ping_interval + ping_timeout` | ninguno | sesión cerrada con `ping timeout`, su lugar se libera |
| `allow_request` (se ejecuta tras los límites, dentro de `allow_request_timeout`, 5 s) | desactivado | desactivado | `403` con `{"code":4,"message":"Forbidden"}` antes de que exista una sesión; también por tiempo agotado o pánico |
| `close_timeout` | 30 s | 30 s | se cierra la conexión |
| `cors`, `allowed_origins` | desactivado | desactivado (`allowed_origins` no existe) | sin cabeceras CORS, o `403` con código 4 |

Las peticiones bajo la ruta del motor nunca llegan a guards ni a throttlers, así que usa `EngineOptions::allow_request`
para conectar un limitador de tasa o una comprobación de autenticación al handshake: recibe la petición del handshake
(`ip()` sigue a `set_trust_proxy`) y devuelve `Admission::Allow` o `Admission::Reject`. Se ejecuta para los handshakes de
polling y las conexiones WebSocket directas, no para las peticiones de una sesión existente. `max_sessions_per_ip` cuenta
como una sola dirección a los clientes detrás de un mismo NAT o de un CDN que no es un proxy de confianza: súbelo con
`EngineOptions::max_sessions_per_ip(n)` en esos despliegues, o configura `set_trust_proxy` para que se use la dirección
real del cliente. Un id de sesión entregado por un handshake que el cliente nunca usa se libera tras
`first_request_timeout`.

El límite por dirección no detiene a un atacante con muchas direcciones: un `/48` de IPv6 contiene 65.536 `/64` y unas
40 direcciones bastan para llenar el `max_sessions` por defecto de 10.000. Limita la tasa de handshakes en el borde (un
proxy inverso o balanceador) o dentro de `allow_request`, y dimensiona `max_sessions` para el despliegue. Detrás de un
proxy el límite cuenta la dirección que resuelve `ip()`, escrita en cualquier forma (`203.0.113.7`, `203.0.113.7:40000`,
`[2001:db8::1]:40000`, con o sin zona; un valor que no es una dirección cuenta como el par del socket). Con
`TrustProxy::All` es el cliente quien elige esa dirección, así que el límite por dirección no protege nada: confía solo
en los proxies que ejecutas.

Las respuestas de polling no se comprimen (Node comprime por encima de 1 KiB cuando el cliente acepta gzip). Engine.IO
v3, las cookies y WebTransport no están soportados. El
[`send_timeout`](/es/http/performance#clientes-que-dejan-de-leer-send-timeout) del servidor se aplica al transporte
WebSocket como a cualquier otra conexión actualizada.

## Seguridad

Los guards globales y los de los gateways no protegen los listeners `upgrade`: valida el acceso dentro del listener, como hace el ejemplo con el token de moderador. Lo mismo vale para los archivos estáticos y las rutas de Swagger/Scalar; ver [Guards](/es/websockets/guards).

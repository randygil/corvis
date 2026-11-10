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

La feature de Cargo `socket-io` (desactivada por defecto, activa `websockets`) añade un servidor Socket.IO v5 y, debajo,
el servidor Engine.IO v4, escritos para Corvis y comprobados contra `socket.io` 4.8.4 y `engine.io` 6.6.x de Node con un
arnés diferencial y la suite oficial de pruebas del protocolo. Es una vista previa con una API de bajo nivel (`IoServer`,
`IoNamespace`, `IoSocket`): las salas y el broadcast, los gateways sobre Socket.IO y un adaptador de Redis llegan en
cambios posteriores. La API de Engine.IO que hay debajo está oculta en la documentación y puede cambiar; `EngineOptions`
(incluido en `IoOptions`) es la parte en la que puedes confiar.

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

### El servidor Socket.IO

Un `IoServer` se crea con `IoOptions`, se le dan espacios de nombres, middleware y manejadores, y se adjunta al servidor
HTTP. El espacio `/` existe desde el principio; cualquier otro lo registra la aplicación con `io.of("/admin")`, porque un
cliente no puede crearlo (los espacios dinámicos no están soportados). Un nombre recibe una `/` inicial y no puede tener
más de 256 bytes ni contener `,` ni caracteres de control.

```rust
use corvis::socket_io::{ConnectError, IoOptions, IoServer};

let io = IoServer::new(IoOptions::new())?;

// El middleware corre antes de que el socket entre al espacio; `auth` es lo que envió el cliente.
io.of("/admin")?.use_middleware(|socket| async move {
    match socket.handshake().auth().get("token").and_then(|t| t.as_str()) {
        Some("secret") => Ok(()),
        _ => Err(ConnectError::new("not authorized").data(serde_json::json!({ "content": "retry later" }))),
    }
});

io.main().on_connection(|socket| {
    // Los manejadores se registran por socket, en el manejador de conexión, así no se pierde ningún evento.
    socket.on("sum", |_socket, args, ack| async move {
        let total: i64 = args.iter().filter_map(|a| a.as_i64()).sum();
        if let Some(ack) = ack {
            let _ = ack.send((total, "ok")); // el callback del cliente recibe (3, "ok")
        }
    });
    let _ = socket.emit("hello", ("world", 1)); // los argumentos son una tupla de valores `Serialize`
    socket.on_disconnect(|socket, reason| println!("{} se fue: {reason}", socket.id()));
});

io.attach(&app.get_http_server())?;
```

Un emit del servidor que espera la respuesta del cliente es `socket.emit_with_ack("ask", ("question",)).await`, con
`socket.timeout(Duration::from_millis(200))` para elegir el plazo; falla con `operation has timed out`, o de inmediato si
el socket se desconecta. Los valores binarios (`corvis::socket_io::Binary` o `IoValue::Binary`) pueden estar en cualquier
parte de los argumentos y viajan como adjuntos; `args.get::<T>(i)` deserializa un argumento a cualquier tipo de `serde`, y
un campo de tipo `Binary` recibe un buffer. `disconnect(false)` sale de un espacio y `disconnect(true)` sale de todos y
cierra la conexión. Cada socket informa `client namespace disconnect`, `server namespace disconnect`, `transport close`,
`transport error`, `ping timeout`, `parse error`, `forced close`, `forced server close` o `server shutting down`
(`io.close()` y el apagado de la aplicación informan el último antes de cerrar las sesiones), primero a los manejadores
`on_disconnecting`, mientras el socket sigue en su espacio, y luego a los `on_disconnect`. Los nombres de evento reservados
(`connect`, `connect_error`, `disconnect`, `disconnecting`, `newListener`, `removeListener`) no los puede emitir el
servidor, y un paquete de un cliente que use uno no se puede decodificar.

Un paquete que no se puede decodificar, un segundo CONNECT a un espacio al que la conexión ya entró (o está entrando), un
paquete para un espacio al que no entró y un CONNECT_ERROR de un cliente cierran la conexión, como en Node.

#### Límites de Socket.IO

Todo lo que un cliente puede hacer que el servidor retenga tiene un valor por defecto acotado y una opción validada
(`IoServer::new` rechaza un valor cero y nombra la opción). Donde Node no tiene límite, Corvis difiere a propósito.

| Límite | Por defecto | `socket.io` 4.8.4 de Node | Al superarlo |
|---|---|---|---|
| Tamaño de paquete (un paquete de texto y cada adjunto) | el `max_http_buffer_size` del motor, 1.000.000 bytes | igual | `413` o código de cierre `1009` (Engine.IO) |
| `max_attachments` por paquete binario, en ambos sentidos | 10 | 10 al decodificar, ninguno al codificar | un paquete recibido que declara más cierra la conexión antes de leer ningún adjunto; un emit falla con `TooManyAttachments` |
| Bytes retenidos mientras se reensambla un paquete binario | como mucho `(max_attachments + 1) x max_http_buffer_size`, contados en `max_unread_bytes`; las opciones que no caben se rechazan | no se cuentan | se pausan las lecturas |
| `attachment_timeout` (tiempo para los adjuntos tras la cabecera) | 30 s | ninguno | se cierra la conexión, diga lo que diga el latido |
| `max_decoded_bytes` (la memoria en que se decodifica un paquete: una casilla de 40 bytes por valor JSON, un objeto añade una caja y una tabla hash, una cadena sus bytes, 24 bytes por asignación) | 32 MiB | ninguno | se cierra la conexión, antes de construir nada; se reserva en el presupuesto del servidor antes de decodificar y se carga hasta que termina el manejador |
| Presupuesto del servidor `max_total_unread_bytes`, y la parte que puede retener una dirección de cliente | 256 MiB, y una cuarta parte (al menos un paquete); una sesión siempre puede retener su parte justa, `max(16 KiB, presupuesto / max_sessions)` | ninguno | el paquete espera y la conexión no toma otro mensaje; su latido se sigue atendiendo |
| `max_auth_bytes` (el `auth` de un CONNECT, que se conserva mientras vive el socket) | 16 KiB | ninguno | se cierra la conexión |
| Anidamiento de JSON | 127 niveles | ninguno | se cierra la conexión |
| `max_event_name_len`, `max_args` (de un cliente) | 256 bytes, 128 | ninguno | se cierra la conexión |
| Id de confirmación de un cliente | como mucho 16 dígitos y 2^53 - 1 | cualquier racha de dígitos, leída como float | se cierra la conexión |
| Espacios por conexión | un socket por espacio registrado, contando los intentos en middleware | igual una vez unido; los intentos pendientes no tienen límite | se cierra la conexión (`forced server close`) |
| `middleware_timeout` | 10 s | ninguno | CONNECT_ERROR `Internal server error` |
| `connect_timeout` | 45 s | 45 s | se cierra la conexión |
| `max_inflight` manejadores por conexión | 32, iniciados en orden de llegada | ninguno | no se lee la conexión hasta que uno termina |
| `max_pending_acks` por socket | 1.000 | ninguno | la llamada falla con `TooManyPending` y no envía nada |
| `ack_timeout` (plazo por defecto de `emit_with_ack`) | 30 s | ninguno | `operation has timed out` |
| Nombre de espacio | 256 bytes, sin `,` ni caracteres de control | ninguno | `of` devuelve un error |
| Id de socket | 120 bits aleatorios, nunca el id de sesión de Engine.IO | 96 bits más un contador | n/a |

Lo que ocupa un paquete decodificado depende de su forma, no solo de su tamaño. El texto de un JSON corriente (objetos con
unos pocos campos de texto y número) se decodifica en 11 a 20 veces su tamaño, así que un paquete de 1 MB cuesta de 11 a
20 MB y se acepta con el valor por defecto. Las peores formas se expanden más: 1 MB de `{}` cuesta 45 MB, y 1 MB de objetos
anidados con claves vacías cuesta 58 MB (el peor caso, 58 veces). Con 1 MB de texto, los 32 MiB por defecto aceptan hasta 33
veces; un paquete por encima de `max_decoded_bytes` cierra la conexión y no se reserva nada de él (primero se mide el texto,
luego se reserva el coste y solo entonces se decodifica). Lo que el servidor retiene por paquetes, buffers de reensamblado y
mensajes sin leer queda dentro de `max_total_unread_bytes` más, por cada sesión abierta, su parte justa y el único mensaje
que puede tener en la mano, y dentro de `max_unread_bytes` más un paquete por sesión; los manejadores de una sesión cerrada
mantienen sus paquetes cargados, al servidor y a la dirección del cliente, hasta que terminan (como mucho `close_timeout`),
así que reconectar no multiplica lo que retiene una dirección. Otra dirección conserva siempre el resto del presupuesto.

Un paquete cuyo JSON tenga un objeto con una clave que `serde_json` lee como instrucción
(`$serde_json::private::RawValue`) se entrega, pero solo como `IoValue`: `args.get::<serde_json::Value>(i)` y `to_json()` lo
rechazan, porque `serde_json` volvería a parsear su valor con un presupuesto de recursión nuevo cuando otro crate de la
compilación activa `raw_value`. Los manejadores que esperan respuestas del cliente (`emit_with_ack`) mientras la conexión
está en `max_inflight` reciben la respuesta solo cuando se libera una plaza, o fallan con `operation has timed out`:
mantén el número de manejadores que esperan respuestas por debajo del límite.

Diferencias que el cliente puede observar, cada una listada en el arnés con el escenario que la ejercita: un segundo
CONNECT mientras el middleware de ese espacio sigue corriendo cierra la conexión (Node inicia otro socket pendiente por
cada paquete), un id de confirmación superior a 2^53 - 1 cierra la conexión (Node responde con un id redondeado) y un JSON
anidado a más de 127 niveles cierra la conexión (Node lo entrega), y también una cadena con un escape de sustituto
suelto como `"\ud800"` (una cadena de Rust no puede contenerlo; Node lo conserva; un par válido sí se acepta). También más
estricto, sin escenario porque el cliente oficial nunca lo hace: un tipo de paquete que no es un dígito, un evento o confirmación sin payload, una confirmación sin
id, un id en un CONNECT, un índice de marcador que no es un entero.

Un manejador que entra en pánico se contiene y se registra una vez por minuto y por espacio, con el número de los omitidos
en la línea siguiente; la línea nombra el espacio y nunca el mensaje del pánico ni los datos del cliente (igual el aviso de un
middleware de conexión que agotó su plazo). No hagas `unwrap()` de la conversión de un argumento (`args.get::<T>(i)?` es un
`Result`): el cliente elige sus datos, y un manejador que entra en pánico con datos malos es un manejador que el cliente puede
hacer fallar a voluntad. Los argumentos que un manejador saca de `Args` (`into_vec`) o mueve a una tarea que lanza dejan de
cargarse cuando el manejador termina: desde entonces son memoria de la aplicación.

No soportado: salas y broadcast, gateways sobre Socket.IO, un adaptador de Redis (cambios posteriores); espacios dinámicos,
recuperación del estado de la conexión, las banderas `volatile` y `compress`, oyentes globales, parsers personalizados,
clientes de Engine.IO v3 y servir el bundle del cliente.

## Seguridad

Los guards globales y los de los gateways no protegen los listeners `upgrade`: valida el acceso dentro del listener, como hace el ejemplo con el token de moderador. Lo mismo vale para los archivos estáticos y las rutas de Swagger/Scalar; ver [Guards](/es/websockets/guards).

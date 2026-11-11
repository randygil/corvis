---
status: partial
---

# Adaptadores

Los gateways de Corvis usan, por defecto, un adaptador integrado equivalente al basado en el paquete `ws` de Node.js: WebSockets RFC 6455 sobre el mismo servidor HTTP de la aplicación y frames JSON `{ event, data }`. Con la feature `socket-io` una aplicación puede servir sus gateways sobre Socket.IO, para el `socket.io-client` oficial ([Gateways sobre Socket.IO](#gateways-sobre-socket-io)). **No hay todavía una API para registrar adaptadores propios.**

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
`IoNamespace`, `IoSocket`), las salas y el broadcast, y los gateways sobre Socket.IO; un adaptador de Redis llega en un
cambio posterior. La API de Engine.IO que hay debajo está oculta en la documentación y puede cambiar; `EngineOptions`
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
real del cliente. Los clientes detrás de una misma dirección compartida (el NAT de un operador móvil o de una empresa)
comparten una sola cuota de dirección y pueden retrasarse unos a otros (sus sesiones cuentan juntas, y en el servidor
Socket.IO también todo lo que retienen del presupuesto); `trust_proxy` solo ayuda cuando el proxy que está delante es uno
que controlas. Un id de sesión entregado por un handshake que el cliente nunca usa se libera tras `first_request_timeout`.

#### Memoria del transporte WebSocket fuera del presupuesto

La biblioteca de WebSocket retiene dos buffers por sesión que ningún presupuesto de lectura cuenta: su buffer de lectura, que
crece hasta alojar el mayor fotograma que la sesión recibió y después no se reduce, y, mientras un mensaje fragmentado está
incompleto, el buffer de ese mensaje (que se duplica al crecer). La biblioteca (tungstenite 0.24, y 0.30 al medirla) no tiene
ningún ajuste que los reduzca, y el tamaño del fotograma no se puede bajar sin rechazar mensajes que Node acepta, así que se
mide, se declara y se acota con las opciones de abajo. Con `M` = `max_http_buffer_size`, una sesión retiene como mucho
`1,25 x 2^ceil(log2(M + 4.110))` bytes tras un mensaje grande, mientras viva, y como mucho `2 x 2^ceil(log2(M))` más que eso
mientras tiene un mensaje incompleto. Con el `M` por defecto de 1.000.000 la medición es 1,12 MiB y 3,14 MiB por sesión
(1.169.504 y 3.294.439 bytes).

| Opciones por defecto | Por sesión | Una dirección (`max_sessions_per_ip` 256) | El servidor (`max_sessions` 10.000) |
|---|---|---|---|
| Tras un mensaje de casi `M`, hasta que la sesión termina | 1,12 MiB | 286 MiB | 10,9 GiB |
| Mientras un mensaje fragmentado está incompleto | 3,14 MiB | 804 MiB | 30,7 GiB |

Estas cifras se suman a `max_total_unread_bytes` (256 MiB), que no las cuenta. Redúcelas con `max_http_buffer_size` (bajan en
proporción, y 100.000 bytes dan 0,13 MiB y 0,32 MiB por sesión), `max_sessions_per_ip` y `max_sessions`. Cargar estos bytes a
la sesión exigiría envolver el socket y queda como posible cambio posterior.

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
`transport error`, `ping timeout`, `parse error`, `forced close`, `forced server close`, `server shutting down`
(`io.close()` y el apagado de la aplicación lo informan antes de cerrar las sesiones) o `admission timeout` (un paquete
esperó más de `admission_timeout` a que hubiera sitio en el presupuesto; Node no tiene ese motivo), primero a los manejadores
`on_disconnecting`, mientras el socket sigue en su espacio, y luego a los `on_disconnect`. Los nombres de evento reservados
(`connect`, `connect_error`, `disconnect`, `disconnecting`, `newListener`, `removeListener`) no los puede emitir el
servidor, y un paquete de un cliente que use uno no se puede decodificar.

Un paquete que no se puede decodificar, un segundo CONNECT a un espacio al que la conexión ya entró (o está entrando), un
paquete para un espacio al que no entró (o que se le rechazó) y un CONNECT_ERROR de un cliente cierran la conexión. Un
paquete para un espacio al que la conexión sí entró y cuyo socket se fue desde entonces (el servidor llamó a
`disconnect(false)` cuando el paquete del cliente ya iba de camino) se descarta (el servidor de Node cierra ahí toda la conexión; es una diferencia deliberada), y la conexión y sus demás
espacios siguen: el paquete se lee, se mide y se carga como cualquier otro, sus adjuntos se consumen y luego se descarta; un
nuevo CONNECT a ese espacio vuelve a unirlo.

Manejadores y la conexión del cliente: cuando el cliente cierra la conexión (el transporte se cierra o falla, o deja de
responder a los pings, o no avanza: una violación del protocolo, un paquete que esperó más de `admission_timeout`) los manejadores que siguen en ejecución se cancelan de inmediato, así que un manejador no debe
suponer que llega a terminar cuando su cliente ya no está (el trabajo que debe sobrevivir va en una tarea que el manejador
lanza, o lo que retiene se libera en un `Drop`). Un cliente que reconecta en bucle no puede dejar así una multitud de
manejadores. Cuando el servidor cierra la conexión, o se apaga, los manejadores tienen `close_timeout` para terminar;
`IoOptions::finish_handlers_after_client_close(true)` les da esa gracia también tras un cierre del cliente. Los manejadores
que siguen tras el fin de su conexión cuentan contra su dirección de cliente, y una dirección que tiene
`max_sessions_per_ip x max_inflight` de ellos (8.192 con los valores por defecto) recibe `503` en las sesiones nuevas hasta
que algunos terminen.

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
| Presupuesto del servidor `max_total_unread_bytes` | 256 MiB; una sesión siempre puede retener su parte justa, `max(16 KiB, presupuesto / max_sessions)` | ninguno | el paquete espera y la conexión no toma otro mensaje |
| `max_address_share`: todo lo que retienen las sesiones de una dirección de cliente de ese presupuesto, en cualquier estado (mensajes sin leer por encima de la parte justa, paquetes que esperan sitio, paquetes que se reensamblan, paquetes decodificados y adjuntos hasta que terminan sus manejadores, el `auth` de sus sockets) | 1/16 del presupuesto, nunca menos de un paquete máximo (`max_decoded_bytes` más los bytes en el cable del mayor paquete): con los valores por defecto manda el mínimo, unos 41,5 MiB, así que **siete direcciones en su límite agotan el presupuesto de 256 MiB** (dieciséis donde los límites de paquete son lo bastante pequeños para que mande 1/16); las sesiones que una dirección puede tener abiertas, `max_sessions_per_ip`, conservan sus partes justas además | ninguno | las sesiones de esa dirección por encima de su parte justa no se leen y sus paquetes esperan |
| `admission_timeout` (cuánto puede esperar un paquete a que haya sitio) | 30 s | ninguno | se cierra la conexión con el motivo de desconexión `admission timeout` |
| `max_auth_bytes` (el `auth` de un CONNECT, que se conserva mientras vive el socket; el `serde_json::Value` guardado se carga por su tamaño real, que puede ser el doble del que mide este límite) | 16 KiB | ninguno | se cierra la conexión |
| Anidamiento de JSON | 127 niveles | ninguno | se cierra la conexión |
| `max_event_name_len`, `max_args` (de un cliente) | 256 bytes, 128 | ninguno | se cierra la conexión |
| Id de confirmación de un cliente | como mucho 16 dígitos y 2^53 - 1 | cualquier racha de dígitos, leída como float | se cierra la conexión |
| Espacios por conexión | un socket por espacio registrado, contando los intentos en middleware | igual una vez unido; los intentos pendientes no tienen límite | se cierra la conexión (`forced server close`) |
| `middleware_timeout` | 10 s | ninguno | CONNECT_ERROR `Internal server error` |
| `connect_timeout` | 45 s | 45 s | se cierra la conexión |
| `max_inflight` manejadores por conexión | 32, iniciados en orden de llegada | ninguno | no se lee la conexión hasta que uno termina |
| Manejadores que sobreviven a su conexión, por dirección de cliente (solo tras un cierre del servidor, o con `finish_handlers_after_client_close`) | `max_sessions_per_ip x max_inflight`, 8.192 | ninguno (Node los ejecuta hasta el final) | las sesiones nuevas de esa dirección reciben `503` hasta que algunos terminen |
| `max_pending_acks` por socket | 1.000 | ninguno | la llamada falla con `TooManyPending` y no envía nada |
| `ack_timeout` (plazo por defecto de `emit_with_ack`) | 30 s | ninguno | `operation has timed out` |
| Nombre de espacio | 256 bytes, sin `,` ni caracteres de control | ninguno | `of` devuelve un error |
| Id de socket | 120 bits aleatorios, nunca el id de sesión de Engine.IO | 96 bits más un contador | n/a |

Lo que ocupa un paquete decodificado depende de su forma, no solo de su tamaño. El texto de un JSON corriente (objetos con
unos pocos campos de texto y número) se decodifica en 11 a 20 veces su tamaño, así que un paquete de 1 MB cuesta de 11 a
20 MB y se acepta con el valor por defecto. Las peores formas se expanden más: 1 MB de `{}` cuesta 45 MB, y 1 MB de objetos
anidados con claves vacías cuesta 58 MB (el peor caso, 58 veces). Con 1 MB de texto, los 32 MiB por defecto aceptan hasta 33
veces; un paquete por encima de `max_decoded_bytes` cierra la conexión y no se reserva nada de él (primero se mide el texto,
luego se reserva el coste y solo entonces se decodifica). Todo lo que un cliente puede hacer que el servidor retenga se carga a
su sesión, a su dirección de cliente y al presupuesto del servidor en una sola cadena, sea cual sea su estado: así una
dirección retiene como mucho su parte (`max_address_share`) más, por cada sesión que tenga abierta, la parte justa y los
mensajes en la mano (el transporte deja de leer una sesión por encima de su parte justa mientras su dirección supera su
parte o el presupuesto está agotado, así que como mucho se ha leído por adelantado un mensaje de a lo sumo
`max_http_buffer_size`, y otro se está recibiendo), y el servidor retiene como mucho `max_total_unread_bytes` más los mismos
términos por cada sesión abierta; y dentro de `max_unread_bytes` más un paquete por sesión. Los manejadores de una sesión
cerrada: cuando el cliente cierra la conexión, sus manejadores en ejecución se cancelan y liberan sus cargas de inmediato;
cuando la cierra el servidor (o se apaga) mantienen sus paquetes cargados, al servidor y a la dirección del cliente, hasta
que terminan (como mucho `close_timeout`), y cuentan para el tope por dirección de arriba, así que reconectar no multiplica
lo que retiene una dirección. Un mensaje se carga por la asignación que
hay detrás, no solo por su longitud (el mensaje que la biblioteca de WebSocket ensambló a partir de fragmentos se reduce
cuando su buffer tiene más de un octavo de holgura). Otra dirección conserva el resto del presupuesto, hasta que bastan
direcciones en su límite para agotarlo (véase la tabla; una aplicación detrás de un proxy inverso debe configurar
`trust_proxy`, o todos los clientes son la única dirección del proxy; los clientes detrás del NAT de un operador o de una
empresa comparten una dirección y por tanto una parte, y pueden retrasarse unos a otros). Los buffers de la propia biblioteca
de WebSocket no entran en esta contabilidad: las cifras por sesión, dirección y servidor, y las opciones que las reducen,
están en «Memoria del transporte WebSocket fuera del presupuesto», arriba.

Mientras otros agotan el presupuesto, un paquete mayor que la parte justa de la sesión espera sitio, y la sesión no se lee
entretanto. Esa espera no es obra de la sesión, así que su latido se retiene (no se cierra con `ping timeout` por una memoria
que no retuvo) como mucho `admission_timeout` en total, y un pong que ya se esperaba se reanuda con lo que quedaba de su espera (nunca un
`ping_timeout` nuevo entero), así que un cliente que nunca responde se cierra dentro de `ping_interval + ping_timeout +
admission_timeout` de su último pong, aparezca y desaparezca la presión como sea; un paquete que no se puede admitir en ese tiempo cierra la conexión con el
motivo `admission timeout`. Una sesión que supera sus propios límites, o cuya dirección supera su parte, conserva el latido
ordinario. Un paquete que espera se admite en cuanto termina un manejador de otra conexión. Los bytes de los mensajes que
esperan detrás de un paquete que no se puede admitir siguen cargados hasta que se admite o se cierra la conexión.

Un paquete cuyo JSON tenga un objeto con una clave que `serde_json` lee como instrucción
(`$serde_json::private::RawValue`) se entrega, pero solo como `IoValue`: `args.get::<serde_json::Value>(i)` y `to_json()` lo
rechazan, porque `serde_json` volvería a parsear su valor con un presupuesto de recursión nuevo cuando otro crate de la
compilación activa `raw_value`. Los manejadores que esperan respuestas del cliente (`emit_with_ack`) mientras la conexión
tiene `max_inflight` manejadores en ejecución no pueden recibir la respuesta: la confirmación espera tras la misma puerta de
lectura que cualquier otro paquete y solo se toma cuando se libera una plaza, así que la llamada falla con `operation has
timed out` al llegar a `ack_timeout`. No esperes confirmaciones del mismo cliente dentro de muchos manejadores concurrentes
(mantén los manejadores que esperan respuestas muy por debajo de `max_inflight`, o envía la pregunta desde una tarea que el
manejador lanza y retorna).

Diferencias que el cliente puede observar, cada una listada en el arnés con el escenario que la ejercita: un segundo
CONNECT mientras el middleware de ese espacio sigue corriendo cierra la conexión (Node inicia otro socket pendiente por
cada paquete), un id de confirmación superior a 2^53 - 1 cierra la conexión (Node responde con un id redondeado) y un JSON
anidado a más de 127 niveles cierra la conexión (Node lo entrega), y también una cadena con un escape de sustituto
suelto como `"\ud800"` (una cadena de Rust no puede contenerlo; Node lo conserva; un par válido sí se acepta). También más
estricto, sin escenario porque el cliente oficial nunca lo hace: un tipo de paquete que no es un dígito, un evento o confirmación sin payload, una confirmación sin
id, un id en un CONNECT, un índice de marcador que no es un entero.

Un manejador que entra en pánico se contiene y se registra una vez por minuto y por espacio, con el número de los omitidos
en la línea siguiente; la línea nombra el espacio y nunca el mensaje del pánico ni los datos del cliente (igual el aviso de un
middleware de conexión que agotó su plazo). Esa línea es la del framework. Contener un pánico no silencia el gancho de pánico
por defecto de la biblioteca estándar, que corre antes de capturar el desenrollado: escribe el mensaje del pánico en la salida
de error cada vez, al ritmo al que el cliente envía, y el mensaje contiene lo que el manejador haya formateado en él, datos
del cliente incluidos. Así que no hagas `unwrap()` ni `expect()` de nada derivado de datos del cliente (`args.get::<T>(i)?` es
un `Result`; también lo son las conversiones de valores y las búsquedas de claves): el cliente elige sus datos, y un manejador
que entra en pánico con datos malos es un manejador que el cliente puede hacer fallar a voluntad, y escribir en tu salida de
error. Una aplicación que quiera silencio instala su propio gancho con `std::panic::set_hook` (el framework no instala
ninguno, porque un gancho es global al proceso). Los argumentos que un manejador saca de `Args` (`into_vec`) o mueve a una tarea que lanza dejan de
cargarse cuando el manejador termina: desde entonces son memoria de la aplicación.

No soportado: un adaptador de Redis (un cambio posterior); espacios dinámicos,
recuperación del estado de la conexión, las banderas `volatile` y `compress`, oyentes globales, parsers personalizados,
clientes de Engine.IO v3 y servir el bundle del cliente.

### Salas y broadcast

Un socket entra en salas con `socket.join(...)` y sale con `socket.leave(sala)`; `socket.rooms()` las lista. Las salas
pertenecen a un espacio de nombres (`r` en `/` y `r` en `/admin` son dos salas), y todo socket está en la sala que lleva el
nombre de su propio id desde que se conecta, antes de que corran sus manejadores de conexión, así que emitir a un id llega a
ese socket. Una sala se elimina cuando sale su último socket. Un socket que se desconecta sigue en sus salas mientras corren
los manejadores `on_disconnecting` (un manejador puede leer `socket.rooms()` y despedirse de quienes están en ellas) y en
ninguna en `on_disconnect`; unir a una sala un socket que ya se desconectó no hace nada. Un cliente no puede lograr que el
servidor lo ponga en la sala de otro socket nombrando su id: `join` rechaza una sala que se llama como el id de otro socket
del espacio de nombres (`RoomError::SocketIdRoom`, y `sockets_join` omite a ese socket), un socket no puede salir de la sala
que lleva su propio id, y el id que se asigna al conectarse un socket nunca es el nombre de una sala existente. Node permite
las tres cosas; son diferencias declaradas.

Los broadcasts parten de un servidor, un espacio de nombres o un socket y encadenan `to`, `within` (el `in` de Node, una
palabra clave en Rust), `except`, `local` y `timeout`. Cada llamada devuelve un operador nuevo, así que un operador base se
puede reutilizar. Seleccionan los sockets que selecciona el adaptador en memoria de Node: `to` recibe una o más salas y llega
a la unión, cada socket una vez; `except` recibe salas, ids de socket incluidos, y quita a sus miembros; sin `to`, un
broadcast llega a todos los sockets del espacio de nombres; una sala en la que no hay nadie no selecciona a nadie, y tampoco
un `to` o `within` con una lista vacía (Node llega a todos, lo que dejaría que una lista vacía que vino de un cliente
signifique "todos"; `except` con una lista vacía no cambia nada, y sin `to` son todos). Un broadcast que parte de un socket
(`socket.broadcast()`, `socket.to(...)`, `socket.within(...)`, `socket.except(...)`) excluye a ese socket.

```rust
use corvis::socket_io::{IoOptions, IoServer};

let io = IoServer::new(IoOptions::new())?;
io.main().on_connection(|socket| {
    // Un nombre de sala que viene del cliente se comprueba: uno de más de 256 bytes falla, nunca se descarta en silencio.
    socket.on("join", |socket, args, _ack| async move {
        if let Ok(room) = args.get::<String>(0) {
            let _ = socket.join(room);
        }
    });
    socket.on("say", |socket, args, _ack| async move {
        let (Ok(room), Ok(text)) = (args.get::<String>(0), args.get::<String>(1)) else { return };
        // Todos los de la sala menos este socket.
        let _ = socket.to(room).emit("said", (text,)).await;
    });
});

// Desde cualquier sitio que tenga el servidor:
io.emit("tick", (1,)).await?;                                       // todos los sockets de `/`
io.to(["lobby", "news"]).except("muted").emit("notice", ("hi",)).await?;
io.of("/admin")?.to("ops").emit("alert", ("disk",)).await?;
```

Las operaciones terminales (`emit`, `emit_with_ack`, `fetch_sockets`, `sockets_join`, `sockets_leave`,
`disconnect_sockets`) son `async` y devuelven un `Result`, porque el adaptador que hay detrás puede ser uno de clúster: el
mismo código funciona cuando se cambia el adaptador. Con el adaptador en memoria por defecto terminan sin esperar a ningún
cliente. Un nombre de sala por encima del límite nunca provoca un pánico y nunca se descarta (quitarlo de `to` podría dejar
la lista vacía, que significa todos): el operador guarda el error y toda operación terminal devuelve `EmitError::InvalidRoom`
sin enviar ni cambiar nada. Un operador nombra como mucho `max_rooms_per_socket` salas en total (`to`, `within` y `except`
juntos): pasado eso toda operación terminal devuelve `EmitError::InvalidRoom(RoomError::TooManyTargets)`. Encadenar es
lineal: cada llamada cuesta las salas que añade, por larga que sea la cadena. Una llamada al adaptador hecha por un operador,
y el `init` del adaptador, están acotados por `adapter_timeout` (5 s por defecto): un adaptador que nunca responde hace
fallar la llamada con `AdapterError::Timeout`. Los nombres de evento reservados se rechazan, igual que en un emit directo.

#### Qué cuesta un broadcast

Un broadcast codifica su paquete una vez y encola los mismos bytes en cada destinatario (una referencia a un único búfer
compartido, adjuntos binarios incluidos), así que mientras se entrega el proceso retiene la codificación `E` una vez, más unas
pocas decenas de bytes por destinatario (una referencia y una entrada de cola, y la instantánea de destinatarios: unos 24
bytes cada uno), no `N x E`. Nunca espera a un destinatario: la cola de cada destinatario está acotada por
`max_buffered_amount` (8 MiB por defecto), y un destinatario que la rebasa se cierra con `transport error` mientras el emisor
y los demás destinatarios siguen (el broadcast devuelve `Ok`; no dice qué destinatarios se cerraron). Aún no hay un
presupuesto de salida para todo el servidor (issue #176), así que un broadcast a `S` clientes que no leen puede dejar
`S x max_buffered_amount` en cola, más un marco en vuelo por sesión como en cualquier emit; dimensiona
`max_buffered_amount` y `max_sessions` con ese producto en mente. Una sesión WebSocket copia el texto compartido a su propio
marco cuando arranca su escritor, así que `S` destinatarios WebSocket que no leen retienen `S` marcos a la vez, no uno: un
texto de 1 MiB a 100 de ellos retuvo 104,85 MB (medido), mientras que los destinatarios de long polling retienen solo la
referencia compartida. Una sola dirección puede fijar hasta 256 x (8 MiB + un marco) de memoria de salida con los valores por
defecto, porque ningún presupuesto de salida de todo el servidor acota las sesiones en conjunto (issues #175 y #176).

La pertenencia a salas no se bloquea mientras se entregan los paquetes: un broadcast selecciona a sus destinatarios con un
bloqueo de lectura, lo suelta y después entrega, así que un manejador puede entrar, salir y hacer broadcast durante un
broadcast y un abanico grande no bloquea las entradas. La selección es el momento del broadcast: un socket que salió de una
sala antes de que empezara no lo recibe, uno que se desconectó nunca recibe un paquete, y un socket que entra o sale mientras
se entrega un broadcast puede recibir ese o no. Los paquetes que una tarea emite a un socket, por broadcast o directamente,
llegan en el orden en que se emitieron: dos broadcasts de un mismo llamador, el segundo esperado después de que el primero
volvió, llegan a cada destinatario que tienen en común en ese orden. Los broadcasts de tareas concurrentes no tienen orden
entre sí (los manejadores de una conexión corren de forma concurrente, así que dos retransmisiones de un mismo cliente son dos
tareas). Un abanico muy grande cede el turno al runtime cada 512 destinatarios.

#### Confirmaciones

`io.timeout(Duration::from_secs(5)).to("room").emit_with_ack("question", ()).await` envía un paquete con un id a los
destinatarios y se resuelve con la respuesta de cada uno, en el orden en que llegan, cuando todos han respondido. Falla con
`BroadcastAckError`, cuyo mensaje es `operation has timed out` y cuyo `responses()` contiene las respuestas que llegaron, al
llegar el plazo (el `timeout`, o si no `ack_timeout`, 30 s: Node sin `timeout()` falla de inmediato, una diferencia
declarada), y con `ResponsesTooLarge` cuando las respuestas pasarían de `max_broadcast_ack_bytes` (se conservan las que
llegaron). Un destinatario que se desconecta nunca responde, y la llamada termina en el plazo con lo que llegó; uno que ya
espera `max_pending_acks` respuestas recibe el evento pero no se le espera. En el plazo, o cuando se abandona la llamada,
ningún destinatario conserva una confirmación pendiente de ella. La respuesta de cada destinatario cuenta una vez y solo
responden los destinatarios del broadcast. Node conserva solo el primer valor de cada respuesta; Corvis devuelve todos
(`Args`). Mientras espera, un broadcast con confirmaciones retiene como mucho `max_broadcast_ack_bytes` (16 MiB) de
respuestas más una entrada pendiente (una referencia y un id) por destinatario. Las respuestas de todas las llamadas
pendientes juntas también tienen un límite: un presupuesto de respuestas de todo el servidor, un cuarto de
`max_total_unread_bytes` del motor (nunca menos que `max_broadcast_ack_bytes`), que pertenece a las llamadas y no a los
clientes que respondieron: el presupuesto de lectura de una conexión (su sesión, su dirección y el servidor) se carga solo
mientras su respuesta se lee y se procesa, y se libera cuando la llamada toma la respuesta, así que un manejador que inicia
muchas llamadas no puede hacer que el presupuesto de otros clientes pague lo que retienen las llamadas. Una dirección de
cliente puede retener como mucho una dieciseisava parte de ese presupuesto (4 MiB con los valores por defecto), así que una
sola dirección no puede mantenerlo lleno; una respuesta que pasaría de la parte de la dirección, o del presupuesto, no espera:
la llamada termina con `ResponsesTooLarge` y las respuestas reunidas hasta entonces (las demás llamadas no se ven afectadas
por una parte llena). El presupuesto de respuestas está fuera de `max_total_unread_bytes`: la memoria que pueden sumar los
datos sin leer y las respuestas retenidas es `max_total_unread_bytes` más `max(max_total_unread_bytes / 4,
max_broadcast_ack_bytes)`. Una sola respuesta mayor que la parte de la dirección se rechaza siempre.

#### Operaciones en bloque

`sockets_join(salas)`, `sockets_leave(salas)` y `disconnect_sockets(close)` actúan sobre los sockets que selecciona un
operador: `io.to("r1").sockets_join("r2").await?` hace que todos los sockets de `r1` entren en `r2`, y
`io.to("r").disconnect_sockets(false)` hace que salgan del espacio de nombres (reciben `io server disconnect`) sin cerrar sus
conexiones. Un socket en su tope de salas se omite y se registra un aviso; los demás entran. `fetch_sockets()` devuelve
`RemoteSocket` con `id()`, `handshake()`, `rooms()` y `data()`, y `emit`, `join`, `leave` y `disconnect` que funcionan esté o
no el socket conectado a este proceso: para un socket de este proceso actúan directamente sobre ese socket, para uno que
tiene otro proceso pasan por el adaptador dirigidas al id de ese socket y a nada más; `local()` da el `IoSocket` cuando es
local. El resultado es una instantánea y su coste es proporcional a la selección, no al servidor: el handshake de un socket
local se comparte, no se copia, así que un resultado reserva unos cientos de bytes por socket sea cual sea el tamaño del
handshake. `Debug` de `RemoteSocket`, `SocketInfo` y `HandshakeInfo` nunca imprime valores de `auth` ni una cadena de consulta, y
solo imprime el valor de las cabeceras `host`, `user-agent`, `origin`, `accept`, `accept-encoding`, `accept-language`,
`connection`, `upgrade` y `sec-websocket-version`: cualquier otra (cookies, autorización, `x-api-key` o una propia) muestra su
nombre y `<hidden>`.

Un adaptador debe tener presentes dos cosas: `room_exists` devuelve `false` por defecto y un adaptador distribuido tiene que
implementarlo, o la regla de que ningún socket recibe un id que ya es una sala deja de funcionar sin avisar; y los métodos
síncronos (`add_all`, `del`, `del_all`, `room_exists`) no se pueden acotar con `adapter_timeout`, así que no deben bloquearse
ni esperar a la red.

#### Límites de las salas

| Límite | Valor por defecto | Node `socket.io` 4.8.4 | Al excederlo |
|---|---|---|---|
| `max_rooms_per_socket` (la sala de su propio id incluida) | 1.000 | ninguno | `RoomError::TooManyRooms`; la llamada no une ninguna de sus salas |
| `max_room_name_length` (en `join`, `to`, `within`, `except`) | 256 bytes, al menos 20 | cualquier valor | `RoomError::NameTooLong`; en un operador, `EmitError::InvalidRoom` y no se envía nada |
| Salas de un espacio de nombres | como mucho sockets x `max_rooms_per_socket`; las salas vacías se eliminan, y un socket que sale o se desconecta no deja pertenencia atrás | sin límite | derivado |
| Memoria de las pertenencias | unos 475 bytes cada una, fuera de todo presupuesto: 121,6 MB por espacio de nombres para una dirección con los valores por defecto (256 sesiones x 1.000 salas), 4,75 GB por espacio de nombres en todo el servidor (10.000 sesiones x 1.000 salas) | sin límite | derivado; baja `max_rooms_per_socket`, `max_room_name_length`, `max_sessions_per_ip` o `max_sessions` |
| Nombres en un `join` / `sockets_join` | como mucho `max_rooms_per_socket`, rechazado antes de tomar ningún bloqueo | sin tope | `RoomError::TooManyRooms` |
| Salas nombradas en una cadena `to` / `within` / `except` | `max_rooms_per_socket` en total | sin tope | `EmitError::InvalidRoom(RoomError::TooManyTargets)`; no se envía nada |
| `adapter_timeout` (una llamada al adaptador hecha por un operador, y `init`) | 5 s | ninguno | `AdapterError::Timeout` |
| Respuestas retenidas por todos los broadcasts con confirmaciones pendientes juntos | un cuarto de `max_total_unread_bytes` (al menos `max_broadcast_ack_bytes`), un presupuesto propio que ninguna conexión paga; una dirección de cliente retiene como mucho una dieciseisava parte | ninguno | la llamada que no encuentra espacio (en el presupuesto o en la parte de la dirección) termina de inmediato con `ResponsesTooLarge` |
| Codificaciones por broadcast | una, compartida por todos los destinatarios | una | n/a |
| Búfer de envío por destinatario | `max_buffered_amount`, 8 MiB | ninguno | esa sesión se cierra con `transport error`; el emisor y los demás no se ven afectados |
| El emisor esperando a los destinatarios | nunca | nunca | n/a |
| Plazo de la confirmación de un broadcast | `timeout()`, o si no `ack_timeout`, 30 s | `timeout()`; sin él la llamada falla de inmediato | `operation has timed out` con las respuestas hasta entonces |
| `max_broadcast_ack_bytes` (respuestas reunidas por un broadcast) | 16 MiB | ninguno | se descartan las respuestas siguientes; `ResponsesTooLarge` |

Diferencias que un cliente puede ver, cada una declarada en el arnés con el escenario que la ejercita: la sala 1.001 de un
socket y un nombre de sala de 257 bytes se rechazan (Node los acepta), unirse al id de otro socket se rechaza y salir de la
sala del propio id no hace nada (Node permite ambas cosas), un `to` con una lista vacía no llega a nadie (Node llega a
todos), y una confirmación de broadcast sin `timeout()` espera `ack_timeout` (Node falla antes de que ningún cliente
responda). Las demás filas son iguales a Node o solo las ve el código del servidor.

#### El adaptador

Las salas y los broadcasts pasan por un adaptador, uno por espacio de nombres: `IoOptions::adapter(fábrica)` reemplaza el
adaptador en memoria por defecto (el `io.adapter(...)` de Node); al cerrar el servidor se cierra una vez el adaptador de cada
espacio de nombres. Su trait tiene pertenencia síncrona de los sockets locales
y operaciones `async` y falibles para todo lo que abarca un clúster (broadcast, broadcast con confirmaciones, sockets de
salas, fetch, añadir y quitar sockets, desconectar, número de servidores), el paquete le llega antes de codificarse, y las
opciones de broadcast llevan las salas, las salas excluidas y las banderas, como los mensajes de `@socket.io/redis-adapter`.
Está oculto e inestable hasta que el adaptador de Redis (issue #36) lo implemente por primera vez.

### Gateways sobre Socket.IO

Una aplicación elige una sola vez, antes de `init()`, cómo se sirven sus gateways. Por defecto es el protocolo al estilo
`ws`; con la feature `socket-io`, `IoAdapter` sirve **todos** los gateways de la aplicación sobre Socket.IO:

```rust
use corvis::prelude::*;
use corvis::socket_io::IoAdapter;

let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_websocket_adapter(IoAdapter::new());
app.listen("127.0.0.1:3000").await?;
```

Llamarla después de `init()` registra una advertencia y no tiene efecto. Solo cambia los gateways de esa aplicación: los
servidores `corvis::ws`, los listeners `on_upgrade` y otras aplicaciones del proceso no se ven afectados, y un upgrade
WebSocket plano al namespace de un gateway no se atiende (y al revés: con el adaptador por defecto no existe ruta de
Socket.IO). `IoAdapter::options(IoOptions)` fija las opciones de una ruta de Engine.IO (todos los límites de las secciones
anteriores, `allow_request`, `allowed_origins`, CORS, el adaptador de clúster), una vez por ruta; una segunda llamada para una
ruta, u opciones que no validan, hacen fallar `init()` con un error que nombra la ruta.

#### Qué significa `#[websocket_gateway]` con Socket.IO

| Forma | Adaptador `ws` (sin cambios) | Adaptador Socket.IO |
|---|---|---|
| `#[websocket_gateway]` | ruta `/` | namespace `/` en la ruta de Engine.IO `/socket.io` |
| `#[websocket_gateway("/chat")]` | ruta `/chat` | namespace `/chat` |
| `namespace = "/chat"` | ruta `/chat` (sinónimo) | namespace `/chat` |
| `path = "/realtime"` | ruta `/realtime` | ruta de Engine.IO `/realtime`, namespace `/` salvo que se indique |
| `cors = true`, `cors = "https://a"`, `cors = ["https://a", "https://b"]` | sin efecto (advertencia en `init()`) | CORS de la ruta de Engine.IO |

`path` es la ruta del servidor de la plataforma que se use, como en el framework original: un gateway escrito como
`path = "/chat"` pasa a ser el namespace `/` en la ruta de Engine.IO `/chat` al cambiar de adaptador (el log de arranque
nombra el namespace y la ruta de cada gateway). El namespace se comprueba al compilar (como máximo 256 bytes, sin `,`, sin
caracteres de control), y una opción que la macro no conoce es un error de compilación que nombra `path`, `namespace` y
`cors`. `cors = true` permite cualquier origen sin credenciales, una cadena un solo origen, un arreglo una lista; algo más
rico es `EngineOptions::cors` en `IoAdapter::options`. Las demás opciones de Engine.IO y Socket.IO (`ping_interval`, tamaños
de búfer) no son opciones de la macro: pertenecen al servidor de una ruta, no a un gateway.

Los gateways que comparten ruta de Engine.IO son namespaces de un mismo servidor Socket.IO, así que una conexión de cliente
llega a todos. Varios gateways pueden declarar el mismo namespace: sus handlers se sirven juntos y sus hooks corren en orden
de registro, pero el mismo evento en dos de ellos hace fallar `init()` (el original ejecuta ambos handlers; una confirmación
no puede responderse dos veces). `init()` también falla, nombrando los gateways o la ruta, cuando los gateways de una ruta
declaran `cors` distintos, cuando un gateway declara `cors` y las opciones de su ruta ya lo fijan, cuando se dan opciones dos
veces para una ruta, o cuando dos rutas de Engine.IO se solapan. Un cliente solo llega a los namespaces que declaran los
gateways (`/` también se rechaza si ningún gateway lo declara): no hay namespaces dinámicos.

#### Handles

Con Socket.IO los handles son `IoSocket` (`#[connected_socket]`), `IoGatewayServer` (`#[websocket_server]`) y, para
`after_init`, `IoNamespace`. Los traits del ciclo de vida reciben el handle como argumento de tipo cuyo valor por defecto es
el de `ws`, así que los bloques `impl OnGatewayConnection for Chat` existentes compilan sin cambios:

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
        // Middleware de conexión, manejadores de conexión, ...
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

`IoGatewayServer` ofrece los operadores de broadcast del namespace del gateway (`emit`, `to`, `within`, `except`, `local`,
`timeout`, `fetch_sockets`, `sockets_join`, `sockets_leave`, `disconnect_sockets`) y `namespace()`. Está vacío hasta que la
aplicación arranca sus gateways: antes, toda operación falla con `EmitError::NotStarted`, no envía nada y no entra en pánico.
Un `#[connected_socket]` o un `#[websocket_server]` de cualquier otro tipo es un error de compilación. **Un gateway cuyos
handles no coinciden con el adaptador hace fallar `init()`** con un mensaje que nombra el gateway y el elemento, antes de que
corra nada de su código: un hook que nunca se ejecuta en silencio podría ser una verificación de autenticación. Un gateway
puede implementar cada hook para ambos handles y entonces funciona con cualquiera de los dos adaptadores. `#[ip]` es la
dirección del cliente resuelta desde el handshake con la configuración `trust proxy`, y los guards leen el socket con
`ctx.switch_to_ws().get_client::<IoSocket>()`; `ctx.request()` es la petición del handshake con ambos adaptadores, que es lo
que debe usar un guard compartido entre ellos.

#### Mensajes, resultados y errores

Un handler `#[subscribe_message("e")]` corre cuando un socket del namespace emite `e`, por el mismo pipeline que con el
adaptador `ws`: guards globales, del gateway y del handler, interceptores, pipes, el handler y los filtros de excepción del
handler y del gateway. El cuerpo (`#[message_body]`, `get_data()`, la entrada de los pipes) se construye con los argumentos del
evento como hace el original: ninguno da `null`, uno da ese argumento, varios dan el arreglo con ellos, y la confirmación no
es un argumento. Un evento al que ningún handler está suscrito se descarta sin respuesta y sin ejecutar guards; un handler
registrado con `IoSocket::on` (en `handle_connection`) tiene prioridad sobre el del gateway.

| Qué devuelve el handler | El cliente envió un callback | Sin callback |
|---|---|---|
| nada, o un valor que se serializa a `null` | no se envía nada; el callback no se llama | nada |
| `WsResponse::new(evento, datos)` | se emite como `evento`; el callback no se llama | se emite como `evento` |
| un objeto con un `event` de texto no vacío | se emite como ese evento con su `data` | igual |
| cualquier otro valor (`false`, `0`, `""` incluidos) | el callback, con el valor | se descarta |
| un error, una denegación, un pánico | el evento `exception`; el callback no se llama | el evento `exception` |

Un `WsResponse` para un nombre de evento reservado (`disconnect`, `connect_error`, ...) no envía nada y se registra con el
gateway y el handler; el socket sigue conectado. La carga de `exception` es la de los filtros del gateway o la del mapeo
integrado: `{ "status": "error", "message": ..., "cause": { "pattern": <evento>, "data": <cuerpo> } }` para un `WsException`
con texto (`data` se omite cuando el evento no tenía argumentos) y para la denegación `Forbidden resource`; el propio objeto
para `WsException::object`; e `Internal server error` para cualquier otro error y para un pánico, sin su texto. La `cause`
es el valor por defecto del framework original (su opción `includeCause`); solo lleva lo que envió el cliente. Una excepción
HTTP (`BadRequestException`, un pipe de validación que falla) conserva su mensaje y sus campos, como con el adaptador `ws`,
donde el original responde `Internal server error`.

Los valores binarios de los argumentos son `null` en el cuerpo: el pipeline no tiene tipo binario. Registra
`socket.on("upload", ...)` en `handle_connection` para recibir los `Args` reales (y una confirmación manual).

#### Hooks y la compuerta de conexión

`after_init` corre una vez por gateway con su namespace, antes de adjuntar el servidor y por tanto antes de que pueda
conectarse ningún cliente. `handle_connection` corre para cada socket después de que el middleware de conexión del namespace
lo aceptó, en el orden de registro de los gateways del namespace; `handle_disconnect` corre exactamente una vez por socket
después de que salió de sus salas, sea cual sea la forma en que se fue (el cliente, el servidor, un tiempo de espera, una
expulsión, un apagado). El original llama a `handleConnection` sin esperarlo, así que un mensaje enviado justo al conectar
puede atenderse antes de que termine un hook asíncrono. **Corvis retiene los eventos de un socket hasta que todos los hooks
`handle_connection` de su namespace han terminado**: una verificación de autenticación en el hook decide antes de que corra
ningún handler, y un evento que llega mientras tanto ni se pierde ni se atiende antes de tiempo. Los eventos en espera cuentan
como handlers en curso (`max_inflight`, 32 por conexión): en el límite el servidor deja de leer esa conexión, y se descartan,
sin ejecutar guards ni handlers, cuando un hook desconecta el socket. Un hook que entra en pánico, o supera
`IoAdapter::hook_timeout` (30 s por defecto), desconecta el socket y cierra su conexión; un `handle_disconnect` que lo supera
se abandona; un `after_init` que entra en pánico o lo supera hace fallar `init()`. Un `hook_timeout` igual a cero hace fallar `init()`.

Los eventos retenidos tras el hook empiezan en el orden en que llegaron (la compuerta es primero en entrar, primero en salir).
Los hooks `handle_disconnect` de un namespace corren en como máximo 16 tareas, sea cual sea el número de sockets que se van, y
como máximo 10.000 sockets esperan a un trabajador; un hook que encuentra la cola llena se omite y se registra (con límite de
frecuencia). Entre el paquete CONNECT y el final de los manejadores de conexión el ping del latido se retiene (como máximo un
segundo), así que lo que emite un manejador sigue a CONNECT como en el original.

`init()` también rechaza una ruta de Engine.IO `/` o vacía (Engine.IO se quedaría con todas las peticiones HTTP de la
aplicación) y un `#[subscribe_message]` para un nombre de evento reservado de Socket.IO (`connect`, `connect_error`,
`disconnect`, `disconnecting`, `newListener`, `removeListener`: ningún cliente puede emitirlos, así que el handler nunca
correría), ambos nombrando el gateway. Una opción de `#[websocket_gateway]` duplicada es un error de compilación.

**Qué retiene un mensaje.** Además de los argumentos del paquete (que carga la capa Socket.IO), un handler de gateway retiene el
cuerpo como un valor JSON compartido, una copia por cada parámetro `#[message_body]` y, si falla, la copia que repite su
`cause`: `2 + P` copias de los argumentos para `P` parámetros de cuerpo. Se cargan al presupuesto de lectura de la conexión
(sesión, dirección y global, como los argumentos) con una carga inmediata que nunca espera, **antes** de que corra ningún
guard. Si no hay sitio, el evento se responde con `exception` (`Too many messages in flight`), ningún guard ni handler lo ve y
el callback queda sin respuesta. Así, el heap de los handlers en curso se mantiene dentro del presupuesto que limita a la
dirección (su cuota `max_address_share`, o el global `max_total_unread_bytes`) por una constante pequeña, medido con un
asignador contador (el doble de la cuota de la dirección con 30 eventos de 400 KiB desde una dirección), y no dentro de ese
presupuesto por el número de copias.

#### Qué autentica una conexión Socket.IO

Nada, por defecto. Las estrategias del módulo de autenticación corren sobre peticiones HTTP y no corren en el handshake
WebSocket, `AuthGuard` deniega fuera de HTTP, y los guards corren por mensaje. Comprueba tú la conexión, de fuera hacia
dentro: `EngineOptions::allow_request` y `allowed_origins` (antes de que exista una sesión), middleware de conexión registrado
en `after_init` (el socket no está conectado hasta que lo pasa; un rechazo es `CONNECT_ERROR`), `handle_connection` (llama a
`socket.disconnect(true)`) e `IoAdapter::guard_connections(true)`.

`guard_connections` está **desactivado por defecto**, como en el original: un socket que nunca envía un mensaje nunca se
comprueba, y aun así está conectado, el código del servidor puede unirlo a salas y recibe broadcasts. Actívalo y los guards
globales (mientras los enhancers globales se apliquen a los gateways) y los declarados en los gateways de un namespace también
corren cuando un socket se conecta, antes de que esté conectado. Una conexión llega a todos los gateways de su namespace, así
que los guards globales corren una vez **por cada gateway**, cada vez con ese gateway como clase, y todos deben pasar: manda el
gateway más estricto, sea cual sea el orden en que se registraron (con un gateway `#[public]` y uno protegido en el mismo
namespace, un cliente anónimo es rechazado). El informe de route posture lista una entrada `connect` por gateway con
exactamente los guards que corren para él. El contexto es un contexto WebSocket con el socket pendiente
como cliente, el handshake como petición, datos `null`, el gateway como clase y un handler llamado `connect` que lleva los
metadatos del gateway (así `#[public]` en el gateway lo respetan los guards que lo respetan; los guards a nivel de handler
corren solo por mensaje). Una denegación responde `CONNECT_ERROR` con `Forbidden resource`, un guard que falla responde con su
mensaje (y el resto de su carga como `data`), y los errores internos y los pánicos responden `Internal server error`; no corre
ningún hook para el socket y no recibe broadcasts. Los filtros de excepción no corren para la conexión. El valor por defecto
se revisará con los valores por defecto de la 1.0.

#### Límites

Esta capa no añade tablas, colas ni búferes propios; los límites son los de las secciones de Engine.IO, Socket.IO y salas,
fijados con `IoAdapter::options`, más estos:

| Límite | Por defecto | Al superarse |
|---|---|---|
| Handlers en curso por conexión, **incluidos los eventos que esperan a `handle_connection`** | 32 (`max_inflight`) | se pausa la lectura |
| Hooks del ciclo de vida | 30 s (`IoAdapter::hook_timeout`) | se desconecta el socket; `init()` falla |
| Namespaces a los que llega un cliente | los que declaran los gateways | `CONNECT_ERROR` `Invalid namespace` |
| Handlers por evento y namespace | 1 | `init()` falla |
| Confirmaciones por evento | como máximo 1, con un argumento | n/a |
| CORS | desactivado; `cors = true` es cualquier origen sin credenciales | sin cabeceras CORS |
| Guards de conexión | desactivados | n/a |
| Errores enviados a los clientes | errores internos y pánicos como `Internal server error`, nunca su texto | n/a |
| Desajuste entre adaptador y gateway | `init()` falla | n/a |

`init()` registra una advertencia para dos configuraciones legales pero arriesgadas: CORS con credenciales en una ruta de
Engine.IO sin `allowed_origins` (CORS no se aplica al transporte WebSocket, así que un namespace autenticado con cookies queda
expuesto al secuestro de WebSocket entre sitios salvo que se fije la lista de orígenes o una comprobación de conexión), y
`cors` declarado en un gateway con el adaptador `ws` (no tiene efecto).

#### Diferencias con el adaptador de gateways original

Comprobado con el cliente oficial contra una aplicación de referencia escrita con el adaptador de Socket.IO del framework
original (el arnés diferencial de `parity/socket-io/`). Lo que un cliente puede observar difiere en dos casos: la carga de una
excepción HTTP (el original responde `Internal server error`; Corvis reenvía su mensaje y sus campos) y un valor binario en el
cuerpo de un mensaje (`null` en Corvis). No observable por un solo cliente: los eventos se retienen hasta que
`handle_connection` termina; un handler por evento y namespace; errores de arranque por opciones, `cors` y desajustes de
adaptador en conflicto; guards de conexión (opcionales).

## Seguridad

Los guards globales y los de los gateways no protegen los listeners `upgrade`: valida el acceso dentro del listener, como hace el ejemplo con el token de moderador. Lo mismo vale para los archivos estáticos y las rutas de Swagger/Scalar; ver [Guards](/es/websockets/guards).

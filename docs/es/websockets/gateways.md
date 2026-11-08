---
status: documented
---

# Gateways

Un **gateway** es un provider que atiende conexiones WebSocket en una ruta. Igual que un controlador agrupa rutas HTTP, un gateway agrupa *handlers de mensajes*: cada uno se suscribe a un nombre de evento y recibe los datos que el cliente envía con ese nombre.

Las piezas que ya conoces del lado HTTP funcionan también aquí: inyección de dependencias, [guards](/es/websockets/guards), [interceptors](/es/websockets/interceptors) y [pipes](/es/websockets/pipes). Lo que cambia es el transporte.

La compatibilidad con WebSockets viene en la feature `websockets` del crate `corvis`, activa por defecto.

## Protocolo

Los gateways hablan un protocolo JSON sencillo sobre frames de texto. Cada frame que llega tiene esta forma:

```json
{ "event": "message", "data": { "room": "general", "text": "hola" } }
```

y cada respuesta del servidor, la misma:

```json
{ "event": "sent", "data": { "id": 7, "room": "general", "text": "hola" } }
```

Es el formato del adaptador basado en el paquete `ws` de Node.js, así que cualquier cliente WebSocket estándar (el `WebSocket` del navegador, `websocat`, `tokio-tungstenite`) sirve sin librerías adicionales. Los frames que no son JSON y los eventos que no tienen handler se ignoran.

## Tu primer gateway

El ejemplo `examples/tertulia` es un chat con salas. Su gateway se declara así (versión resumida de `src/chat/chat_gateway.rs`):

```rust
use corvis::prelude::*;
use corvis::{JwtService, WsClient, WsException, WsResponse, WsServer, websocket_gateway};
use corvis_orm::OrmResultExt; // `.http()`: convierte un `OrmError` en `HttpException`

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
        // Todos los demás de la sala reciben `message`; quien envió recibe el acuse `sent`.
        self.server.to(&dto.room).except(client.id()).emit("message", &saved);
        Ok(WsResponse::new("sent", saved))
    }

    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "pong"
    }
}
```

Y se registra como cualquier otro provider, en el módulo que le corresponde (`src/chat/mod.rs`):

```rust
#[module(
    controllers = [RoomsController, StatsController],
    providers = [ChatGateway, RoomsService, FirehoseService],
    exports = [ChatGateway, RoomsService],
)]
pub struct ChatModule;
```

Un gateway que no está en `providers` de algún módulo no se monta. Como es un provider normal, otros componentes pueden inyectarlo: en tertulia, `RoomsController` recibe `chat: Arc<ChatGateway>` para consultar quién está conectado y para enviar anuncios desde un endpoint REST.

### `#[websocket_gateway]`

| Forma | Efecto |
|---|---|
| `#[websocket_gateway(path = "/chat")]` | Atiende los upgrades a `/chat` |
| `#[websocket_gateway("/chat")]` | Lo mismo, con la ruta como literal |
| `#[websocket_gateway(namespace = "/chat")]` | Sinónimo de `path` |
| `#[websocket_gateway]` | Ruta `/` |

La ruta se normaliza: se añade la `/` inicial si falta y se quita la final. El prefijo global (`app.set_global_prefix`) **no** se aplica a los gateways: en tertulia la API REST vive en `/api/...` y el chat en `/chat`.

Debajo de `#[websocket_gateway]` puedes poner `#[use_guards(...)]`, `#[use_interceptors(...)]`, `#[use_pipes(...)]`, `#[use_filters(...)]` y `#[set_metadata(clave, valor)]`; se aplican a todos los handlers del gateway. Como en los controladores, deben ir **debajo** del atributo principal porque Rust procesa los atributos en orden (ver [Filtros de excepción](/es/websockets/exception-filters) para `#[use_filters]`).

### `#[handlers]` y `#[subscribe_message]`

Los handlers se declaran en un bloque `impl` marcado con `#[handlers]`, el mismo atributo que usan `#[on_event]` y `#[cron]`. Cada método con `#[subscribe_message("evento")]` atiende los frames cuyo `event` coincide. Un mismo bloque puede mezclar handlers de mensajes con otros: el gateway de tertulia tiene también un `#[on_event("room.created")]` que avisa a todos los clientes cuando se crea una sala por REST.

Los métodos pueden ser `async` o no. Encima de cada `#[subscribe_message]` puedes poner `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` y `#[set_metadata]`; se suman a los del gateway. Un método sin `#[subscribe_message]` es un helper común: ponerle `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]` o `#[set_metadata]` es un error de compilación.

Cada evento puede tener un solo handler por gateway. Si dos métodos `#[subscribe_message]` usan el mismo evento, la aplicación no arranca y muestra `ChatGateway has more than one handler for the "message" event (on_message and on_message_v2); a gateway can have only one handler per event`. Los nombres de evento distinguen mayúsculas y minúsculas (`"message"` y `"Message"` son eventos distintos), y gateways distintos pueden atender el mismo nombre de evento.

### Parámetros

| Atributo | Recibe |
|---|---|
| `#[message_body] dto: T` | El campo `data` del frame, deserializado en `T` |
| `#[message_body("room")] room: String` | Solo esa propiedad de `data` |
| `#[message_body("id", ParseIntPipe)] id: i64` | Una propiedad pasada por pipes propios del parámetro |
| `#[payload]` | Sinónimo de `#[message_body]` |
| `#[connected_socket] client: WsClient` | El cliente que envió el frame |

Un parámetro sin atributo se trata como `#[message_body]`. Si el handler no necesita los datos ni el cliente, puede no recibir nada, como `ping` en el ejemplo.

### Valores de retorno

| El handler devuelve | El cliente recibe |
|---|---|
| `()` | Nada |
| `WsResponse::new("evento", datos)` | `{ "event": "evento", "data": datos }` |
| Cualquier valor `Serialize` | `{ "event": <el mismo evento>, "data": valor }` |
| `Option` de lo anterior | Nada con `None` |
| `HttpResult<T>` | Lo anterior con `Ok`; un frame `exception` con `Err` |

`ping` devuelve `"pong"`, así que el cliente recibe `{ "event": "ping", "data": "pong" }`.

## Errores: `WsException`

Un handler, guard o pipe que falla produce un frame con el evento `exception`. `WsException` construye el error, que es un `HttpException` y por lo tanto encaja en `HttpResult`:

```rust
// { "event": "exception", "data": { "status": "error", "message": "Unauthorized" } }
return Err(WsException::new("Unauthorized"));

// El objeto se envía tal cual como `data`.
return Err(WsException::object(json!({
    "status": "error",
    "code": "ROOM_FULL",
    "message": format!("#{} is full", room.name),
})));
```

| Error | `data` del frame `exception` |
|---|---|
| `WsException::new(msg)` | `{ "status": "error", "message": msg }` |
| `WsException::object(obj)` | `obj` |
| Otro `HttpException` (`BadRequestException`, errores de validación...) | `{ "status": "error", "message", ...resto de su cuerpo }`, por ejemplo con `statusCode` y `errors` |
| Error desconocido o `panic` en el handler | `{ "status": "error", "message": "Internal server error" }` (y se registra en el log) |

`WsException::get_error(&e)` devuelve el valor original de un error creado con `WsException`. Un `panic` dentro de un handler no cierra la conexión.

## Acuses de recibo (acks)

Si el frame del cliente incluye un `id`, la respuesta (o el frame `exception`) lleva el mismo `id`:

```json
→ { "event": "join", "data": { "room": "rust" }, "id": 42 }
← { "event": "joined", "data": { ... }, "id": 42 }
```

Así el cliente puede correlacionar cada petición con su respuesta, como hace el cliente web de tertulia (`public/app.js`). Es una extensión propia de Corvis; los clientes que no envían `id` no notan ninguna diferencia.

## El servidor: `WsServer`

El campo marcado con `#[websocket_server]` recibe el servidor del gateway. Cada gateway tiene el suyo, y cada aplicación crea sus propios servidores, así que dos aplicaciones en el mismo proceso (por ejemplo, en tests) no comparten clientes.

| Método | Descripción |
|---|---|
| `server.emit(evento, datos)` | Envía a todos los clientes conectados; devuelve a cuántos llegó |
| `server.to(sala).emit(evento, datos)` | Envía a los clientes de una sala |
| `server.to(sala).except(id).emit(...)` | Igual, excluyendo a un cliente (normalmente quien originó el mensaje) |
| `server.clients()` | Los clientes conectados |
| `server.client(id)` | Un cliente por su id |
| `server.count()` | Cuántos clientes hay |
| `server.set_options(GatewayOptions)` / `server.options()` | Límites y verificación del handshake (ver abajo) |

`WsServer::for_path("/chat")` devuelve el servidor del último gateway inicializado en esa ruta. Existe para casos puntuales; lo recomendable es inyectar el gateway y usar su campo.

## Los clientes: `WsClient`

| Método | Descripción |
|---|---|
| `client.id()` | Identificador único de la conexión |
| `client.emit(evento, datos)` | Envía un frame `{ event, data }` solo a este cliente |
| `client.send_text(texto)` | Envía un frame de texto crudo |
| `client.join(sala)`, `client.leave(sala)` | Entra o sale de una sala |
| `client.rooms()`, `client.in_room(sala)` | Salas del cliente |
| `client.handshake()` | La `Request` HTTP del upgrade: cabeceras, query string, IP |
| `client.insert(valor)`, `client.get::<T>()` | Datos asociados a la conexión (por ejemplo, el usuario autenticado) |
| `client.disconnect()` | Cierra la conexión |
| `client.disconnect_with(4401, "Unauthorized")` | Cierra con un código y un motivo |
| `client.is_closing()` | `true` después de `disconnect` |
| `client.buffered_amount()` | Bytes pendientes de envío |
| `client.server()` | El `WsServer` al que pertenece |

Las salas no se crean ni se destruyen: existen mientras algún cliente esté en ellas. `disconnect_with` acepta los códigos 1000-1003, 1007-1014 y 3000-4999; cualquier otro se envía como `1008`. En cuanto llamas a `disconnect`, el cliente sale de `server.clients()` y los frames que lleguen después ya no se despachan.

## Ciclo de vida

Implementa estos traits en el gateway para reaccionar a los eventos de la conexión. Se detectan solos, sin registrarlos en ningún lado:

| Trait | Método | Cuándo |
|---|---|---|
| `OnGatewayInit` | `after_init(&self, server: WsServer)` | Al montar el gateway, durante `app.init()` |
| `OnGatewayConnection` | `handle_connection(&self, client: WsClient)` | Al aceptar cada conexión |
| `OnGatewayDisconnect` | `handle_disconnect(&self, client: WsClient)` | Al cerrarse cada conexión |

Tertulia autentica en `handle_connection`. Un navegador no puede enviar cabeceras al abrir un WebSocket, así que el token llega en la query string:

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
        // El cliente ya no está en `server.clients()`, pero conserva sus salas.
        for room in client.rooms() {
            self.notify_left(&room, &user);
        }
    }
}
```

Para rechazar la conexión **antes** de completar el handshake (con un `401` o un `403` HTTP), usa `GatewayOptions::verify_client` o `allowed_origins`, o un listener de `upgrade` con [`corvis::ws`](/es/websockets/adapter).

## Opciones y límites: `GatewayOptions`

Las opciones se fijan sobre el servidor del gateway, normalmente en `after_init`, y se aplican a los handshakes y conexiones que empiezan después:

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

| Opción | Por defecto | Efecto |
|---|---|---|
| `max_payload(bytes)` | 1 MiB | Un mensaje más grande cierra la conexión con `1009` |
| `max_buffered_amount(bytes)` | 8 MiB | Bytes pendientes de envío por cliente; al superarlo se descartan y se cierra con `1008` |
| `max_inflight(n)` | 32 | Handlers simultáneos por conexión; en el tope se deja de leer el socket |
| `close_timeout(duración)` | 30 s | Espera de la respuesta al cierre antes de cortar la conexión TCP |
| `allowed_origins([...])` | Sin lista | Un `Origin` que no está en la lista recibe `403`; las peticiones sin `Origin` se aceptan |
| `verify_client(f)` | Ninguno | Si `f` devuelve `false`, el handshake recibe `401` |

El reporte de route posture lista el handshake de cada gateway como no revisado, con si `verify_client` está configurado: la revisión no puede exigir nada sobre lo que corre antes del primer mensaje. Ver [guards en WebSockets](/es/websockets/guards#route-posture-en-gateways).

`allowed_origins` es importante si autenticas el gateway con cookies: sin esa lista, una página de otro dominio podría abrir una conexión en nombre del usuario.

`client.buffered_amount()` devuelve los bytes pendientes. Los pings se responden automáticamente (queda un solo pong pendiente). Un `panic` en `handle_connection` cierra la conexión con `1011`, y uno en `handle_disconnect` se registra; en ambos casos el cliente sale de `server.clients()`.

El handshake se valida igual que en el paquete `ws`: `405` si el método no es `GET` y `400` si faltan o son inválidas `Sec-WebSocket-Key` o `Sec-WebSocket-Version`.

## Concurrencia y orden

Los handlers de una misma conexión **empiezan** en el orden en que llegaron sus frames y después corren en paralelo, hasta `max_inflight`. Un `join` síncrono seguido de un `message` siempre ve la sala ya unida; pero si un handler hace `await` antes de `client.join`, el siguiente frame puede empezar a procesarse antes de que termine.

## Enhancers globales

Los guards, interceptors y pipes globales (`app.use_global_guards`, `use_global_interceptors`, `use_global_pipes` y los providers `Provider::app_guard`, `app_interceptor` y `app_pipe`) se aplican a todos los handlers de mensajes, como en el framework original. Corren antes que los enhancers del gateway y del handler: primero los globales, después los del gateway y al final los del handler (los registrados con `Provider::app_*` van delante de los de `use_global_*`). Los filtros de excepción globales **no** se aplican a los gateways; declara filtros con `#[use_filters]` (ver [Filtros de excepción](/es/websockets/exception-filters)).

Cosas a tener en cuenta:

- Un guard global que solo conoce HTTP (por ejemplo, lee un token bearer de `ctx.switch_to_http()`) rechaza o rompe todos los mensajes. Distingue los transportes con `ctx.get_type()` (`ContextType::Ws`), o marca el gateway con metadatos que el guard revise, como `#[set_metadata(IS_PUBLIC_KEY, true)]` debajo de `#[websocket_gateway]`.
- Un interceptor global también ve contextos WebSocket, así que uno que lee la petición HTTP o escribe cabeceras debe comprobar antes `ctx.get_type()`.
- Un pipe global como `ZodValidationPipe::auto()` corre en cada `#[message_body]`. Repetirlo en el gateway, como hace tertulia, hace que corra dos veces, lo cual es inofensivo para pipes de validación.
- Un `ThrottlerGuard` global cuenta los mensajes por IP del handshake y por handler.

Las aplicaciones que adoptan los gateways de forma gradual pueden mantener los enhancers globales solo en HTTP con una llamada antes de `init()` (o `listen`):

```rust
app.apply_global_enhancers_to_gateways(false);
```

Con `false`, un handler ejecuta solo los enhancers de su gateway y los suyos. Llamarla después de inicializar la aplicación registra una advertencia y no cambia nada.

## Probar un gateway

`examples/tertulia/tests/chat.rs` levanta la aplicación en un puerto real con `app.listen_in_background("127.0.0.1:5170")` y se conecta con `tokio-tungstenite`. El mismo test usa `corvis::testing::request(&app)` para las rutas HTTP. Consulta [Testing](/es/fundamentals/testing) para el resto de utilidades.

## Ver también

- [Pipes](/es/websockets/pipes), [Guards](/es/websockets/guards) e [Interceptors](/es/websockets/interceptors) en gateways.
- [Adaptadores](/es/websockets/adapter): WebSockets planos con `corvis::ws`, sin el protocolo `{ event, data }`.
- [Server-Sent Events](/es/http/server-sent-events), si solo necesitas enviar datos del servidor al cliente.

# Tertulia

Chat en tiempo real con salas, presencia e indicadores de escritura, hecho con Corvis. Sirve para
probar de punta a punta el soporte de WebSockets del framework:

- **Gateway `/chat`** (`#[websocket_gateway]`): salas, difusión a la sala sin el emisor, acks con
  `WsResponse` + `id`, errores con `WsException`, hooks `OnGatewayInit/Connection/Disconnect`.
- **Autenticación JWT al conectar** (`?token=`), con cierre `4401` si el token falta o no es válido.
- **Guards, pipes e interceptors en los mensajes**: un `AuthGuard` que sirve para HTTP y para WS, un
  `RolesGuard` con `#[set_metadata]`, un `RoomMemberGuard` que lee el payload, validación zod
  (`ZodValidationPipe::auto()` + `zod_dto!`) y un interceptor que registra cada mensaje.
- **REST ↔ WebSocket**: `RoomsService` se inyecta en el gateway y en el controller; el controller
  inyecta el gateway para leer quién está conectado y para empujar anuncios. Crear una sala por REST
  emite `room.created`, que el gateway reenvía con `#[on_event]`.
- **Persistencia** del historial y de las salas en SQLite con `corvis-orm`.
- **SSE** (`/api/stats/stream`) y un **WebSocket plano** con `corvis::ws` (`/ws/firehose`), para
  comparar.
- **Cliente web** en `public/`, servido con `corvis::serve_static`.

## Cómo ejecutarlo

```bash
cargo run -p tertulia
```

Abre <http://127.0.0.1:5150> en dos navegadores (o una ventana normal y otra privada), entra con dos
apodos distintos y únete a la misma sala. Entra como `admin` para ver las acciones de moderación:
borrar el historial (por WebSocket) y publicar anuncios (por REST, que llegan por WebSocket).

Variables de entorno (también se leen de un `.env`):

| Variable | Valor por defecto | Uso |
|---|---|---|
| `PORT` | `5150` | Puerto (siempre en `127.0.0.1`) |
| `DATABASE_URL` | `file:./tertulia.db` | SQLite; `sqlite::memory:` para no guardar nada |
| `JWT_SECRET` / `JWT_EXPIRES_IN` | `tertulia-dev-secret` / `12h` | Firma de los tokens |
| `MODERATORS` | `admin` | Apodos con rol `moderator`, separados por comas |
| `STATS_INTERVAL_MS` | `2000` | Cada cuánto emite el stream SSE |

El login no pide contraseña: cualquier apodo válido (2 a 20 caracteres `A-Z a-z 0-9 _ . -`) recibe un
token. Es una demo.

## Protocolo del gateway `/chat`

Conéctate a `ws://127.0.0.1:5150/chat?token=<accessToken>` (o con `Authorization: Bearer`). Todos los
frames son JSON `{ "event": "...", "data": ..., "id": ... }`; el `id` es opcional y, si lo envías,
vuelve en la respuesta o en el error.

| El cliente envía | Respuesta | Los demás de la sala reciben |
|---|---|---|
| `join { room }` | `joined { room, members, history }` | `user_joined { room, user, members }` |
| `leave { room }` | `left { room }` | `user_left { room, user, members }` |
| `message { room, text }` | `sent { id, room, username, text, createdAt }` | `message { ... }` |
| `typing { room, typing }` | (nada) | `typing { room, user, typing }` |
| `clear { room }` (moderador) | (nada) | toda la sala: `cleared { room, by, deleted }` |
| `rooms` | `rooms [ ... ]` | |
| `ping` | `ping "pong"` | |

El servidor también envía `welcome { id, user, rooms }` al conectar, `room_created` a todos cuando se
crea una sala y `announcement { room, from, text }` a la sala cuando un moderador publica un anuncio.

Errores (`event: "exception"`):

```json
{ "status": "error", "message": "Unauthorized" }
{ "status": "error", "message": "Forbidden resource" }
{ "status": "error", "code": "NOT_IN_ROOM", "message": "Join #random first", "room": "random" }
{ "status": "error", "code": "ROOM_FULL", "message": "#tiny is full", "room": "tiny", "max": 1 }
{ "status": "error", "message": "Validation failed", "statusCode": 400, "errors": [ ...issues de zod ] }
```

Prueba rápida desde la terminal con [websocat](https://github.com/vi/websocat):

```bash
TOKEN=$(curl -s -X POST http://127.0.0.1:5150/api/auth/login \
  -H 'content-type: application/json' -d '{"username":"ana"}' | jq -r .accessToken)
websocat "ws://127.0.0.1:5150/chat?token=$TOKEN"
{"event":"join","data":{"room":"general"},"id":1}
{"event":"message","data":{"room":"general","text":"hola"},"id":2}
```

## REST

Todas bajo `/api`:

| Método y ruta | Auth | Qué hace |
|---|---|---|
| `POST /auth/login { username }` | | `{ accessToken, user }` |
| `GET /auth/me` | Bearer | El usuario del token |
| `GET /rooms` | | Salas con sus miembros conectados ahora (estado del gateway) |
| `POST /rooms { name, topic?, maxMembers? }` | Bearer | Crea la sala y avisa por WebSocket (`409` si ya existe) |
| `GET /rooms/:room/messages?limit=50` | | Historial guardado |
| `GET /rooms/:room/members` | | Miembros conectados, leídos del gateway |
| `POST /rooms/:room/announcements { text }` | Bearer, moderador | Envía `announcement` a la sala por WebSocket; responde `{ delivered }` |
| `GET /stats` | | Conexiones, usuarios en línea, mensajes y salas |
| `GET /stats/stream` | | Lo mismo como Server-Sent Events (`event: stats`) |

## WebSocket plano: `/ws/firehose`

`ws://127.0.0.1:5150/ws/firehose?token=<token de moderador>` transmite cada evento del chat (mensajes,
anuncios, borrados) como JSON crudo, sin el sobre `{ event, data }`. Está hecho con un listener
`upgrade` de `HttpAdapterHost` y `corvis::ws::WebSocketServer`, así que, a diferencia del gateway,
puede rechazar el handshake: sin token de moderador responde `401` y no abre el WebSocket. Si envías el
texto `ping`, responde `pong`.

## Estructura

```text
src/
  lib.rs                 create_app(): prefijo /api, pipes, filtros y archivos estáticos
  main.rs                arranque en 127.0.0.1:$PORT
  app_module.rs          Config, EventEmitter, JWT, corvis-orm (SQLite)
  models.rs              Room, ChatMessage (corvis-orm)
  auth/                  login, ChatUser, AuthGuard y RolesGuard (HTTP y WS)
  chat/chat_gateway.rs   el gateway /chat
  chat/dto.rs            payloads validados con zod
  chat/room_member_guard.rs, chat/ws_logging_interceptor.rs
  rooms/                 RoomsService (compartido) y RoomsController (REST)
  stats/                 JSON + SSE
  firehose.rs            WebSocket plano con corvis::ws
public/                  cliente web (HTML, CSS y JS sin dependencias)
tests/chat.rs            tests de punta a punta
```

## Tests

```bash
cargo test -p tertulia
```

Cada test levanta un servidor real en `127.0.0.1` (puertos 5170 a 5199) con SQLite en memoria, conecta
clientes WebSocket reales con `tokio-tungstenite` y usa el cliente HTTP en memoria de
`corvis::testing` contra la misma aplicación. Cubren: rechazo sin token, unirse a salas, difusión que
llega a los demás y no al emisor, acks con `id`, errores de validación, guards de sala y de rol,
indicadores de escritura, limpieza de presencia al salir o desconectarse, anuncios y salas creadas por
REST que llegan por WebSocket, capacidad de las salas, historial, el WebSocket plano, el stream SSE y
el cliente web.

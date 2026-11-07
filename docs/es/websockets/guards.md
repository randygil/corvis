---
status: documented
---

# Guards

Un guard de gateway es el mismo `CanActivate` que usas en HTTP. Recibe un `ExecutionContext` y decide si el mensaje llega al handler. Por eso un único guard puede proteger rutas REST y mensajes WebSocket a la vez.

## Declararlos

```rust
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]                 // todos los handlers
pub struct ChatGateway { /* ... */ }

#[handlers]
impl ChatGateway {
    #[subscribe_message("clear")]
    #[set_metadata(ROLES_KEY, [MODERATOR])]
    #[use_guards(RolesGuard, RoomMemberGuard)]   // solo este handler, después de AuthGuard
    async fn clear(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
        // ...
        Ok(())
    }
}
```

Los guards del gateway corren primero y después los del handler, en el orden en que aparecen. Si uno devuelve `Ok(false)`, el cliente recibe `{ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }`. Si devuelve `Err(...)`, recibe ese error (ver [Gateways](/es/websockets/gateways#errores-wsexception)).

Los guards globales (`app.use_global_guards`, `Provider::app_guard`) no se aplican a los gateways.

## El contexto en un gateway

`ctx.get_type()` devuelve `ContextType::Ws` y `ctx.switch_to_ws()` da acceso a los datos del mensaje:

| Método | Devuelve |
|---|---|
| `ctx.switch_to_ws().get_client::<WsClient>()` | El cliente que envió el frame |
| `ctx.switch_to_ws().get_data()` | El `data` del frame, **sin validar** |
| `ctx.request()` | La `Request` del handshake: cabeceras, query string e IP |
| `ctx.get_handler()`, `ctx.get_class()` | El handler y el gateway, con su metadata (`#[set_metadata]`) |

El guard de pertenencia a una sala de tertulia (`src/chat/room_member_guard.rs`):

```rust
#[injectable]
pub struct RoomMemberGuard;

#[async_trait]
impl CanActivate for RoomMemberGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let Some(client) = ws.get_client::<WsClient>() else { return Ok(false) };
        // Sin sala el payload es inválido: lo reportará el pipe de validación.
        let Some(room) = ws.get_data().and_then(|d| d.get("room")).and_then(Value::as_str) else {
            return Ok(true);
        };
        if client.in_room(room) {
            return Ok(true);
        }
        Err(WsException::object(json!({
            "status": "error",
            "code": "NOT_IN_ROOM",
            "message": format!("Join #{room} first"),
            "room": room,
        })))
    }
}
```

## Un guard para los dos transportes

El `AuthGuard` de tertulia verifica un JWT en HTTP y, en WebSocket, comprueba que la conexión ya trae un usuario (lo adjuntó el gateway en `handle_connection` con `client.insert`):

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        if ctx.get_type() == ContextType::Ws {
            return match ChatUser::of(ctx) {
                Some(_) => Ok(true),
                None => Err(WsException::new("Unauthorized")),
            };
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request)
            .ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

`ChatUser::of(ctx)` lee el usuario del cliente WebSocket (`client.get::<ChatUser>()`) o de la request HTTP (`ctx.request().get::<ChatUser>()`), según el tipo de contexto. El mismo `RolesGuard`, que lee los roles con `Reflector::get_all_and_override`, protege tanto `POST /api/rooms/:room/announcements` como el mensaje `clear`.

## Autenticar al conectar

Un guard se evalúa en cada mensaje. Para rechazar la conexión completa:

- verifica el token en `handle_connection` y llama a `client.disconnect_with(4401, "Unauthorized")`, como tertulia;
- o rechaza el handshake con `GatewayOptions::verify_client` (`401`) o `allowed_origins` (`403`).

Ver [Gateways](/es/websockets/gateways#opciones-y-limites-gatewayoptions).

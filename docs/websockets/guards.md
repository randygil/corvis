---
status: documented
---

# Guards

A gateway guard is the same `CanActivate` you use in HTTP. It receives an `ExecutionContext` and decides whether the message reaches the handler. That is why a single guard can protect REST routes and WebSocket messages at once.

## Declaring them

```rust
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]                 // all handlers
pub struct ChatGateway { /* ... */ }

#[handlers]
impl ChatGateway {
    #[subscribe_message("clear")]
    #[set_metadata(ROLES_KEY, [MODERATOR])]
    #[use_guards(RolesGuard, RoomMemberGuard)]   // this handler only, after AuthGuard
    async fn clear(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
        // ...
        Ok(())
    }
}
```

The gateway's guards run first and then the handler's, in the order they appear. If one returns `Ok(false)`, the client receives `{ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }`. If it returns `Err(...)`, it receives that error (see [Gateways](/websockets/gateways#errors-wsexception)).

Global guards (`app.use_global_guards`, `Provider::app_guard`) also run on every gateway message, ahead of the gateway's and the handler's guards. A guard written for HTTP can tell the transports apart with `ctx.get_type()`, or skip a gateway marked with `#[set_metadata(...)]`; `app.apply_global_enhancers_to_gateways(false)` keeps global guards HTTP-only (see [Gateways](/websockets/gateways#global-enhancers)). They do not cover the `upgrade` listeners of [`corvis::ws`](/websockets/adapter), static files (`serve_static`, `ServeStaticModule`) or the Swagger/Scalar routes. A static file with the same path as a protected endpoint is served before the endpoint. Protect those surfaces separately (middleware, `verify_client`, another static root).

## The context in a gateway

`ctx.get_type()` returns `ContextType::Ws` and `ctx.switch_to_ws()` gives access to the message data:

| Method | Returns |
|---|---|
| `ctx.switch_to_ws().get_client::<WsClient>()` | The client that sent the frame |
| `ctx.switch_to_ws().get_data()` | The frame's `data`, **unvalidated** |
| `ctx.request()` | The handshake `Request`: headers, query string, and IP |
| `ctx.get_handler()`, `ctx.get_class()` | The handler and the gateway, with their metadata (`#[set_metadata]`) |

Tertulia's room membership guard (`src/chat/room_member_guard.rs`):

```rust
#[injectable]
pub struct RoomMemberGuard;

#[async_trait]
impl CanActivate for RoomMemberGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let Some(client) = ws.get_client::<WsClient>() else { return Ok(false) };
        // Without a room the payload is invalid: the validation pipe will report it.
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

## One guard for both transports

Tertulia's `AuthGuard` verifies a JWT on HTTP and, on WebSocket, checks that the connection already carries a user (the gateway attached it in `handle_connection` with `client.insert`):

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

`ChatUser::of(ctx)` reads the user from the WebSocket client (`client.get::<ChatUser>()`) or from the HTTP request (`ctx.request().get::<ChatUser>()`), depending on the context type. The same `RolesGuard`, which reads the roles with `Reflector::get_all_and_override`, protects both `POST /api/rooms/:room/announcements` and the `clear` message.

## Route posture for gateways

With [`require_route_posture()`](/guards#require-a-guard-on-every-route) the application checks every `#[subscribe_message]` handler, not only HTTP routes. A handler is accepted when a guard that controls access runs for it, when it is public, or when a message exemption matches it. Otherwise `init()` fails, before any gateway `after_init` runs:

```text
Route posture check failed: 1 message handler has no guard and is not marked #[public].
  WS  /chat  event "rooms"  ChatGateway::rooms
Add #[use_guards(...)] to the route, the message handler, its controller or the gateway (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

The check counts the guards that run for each message: global guards first, then the gateway's, then the handler's. A global guard counts only while global enhancers apply to gateways; after `app.apply_global_enhancers_to_gateways(false)` it does not, and a gateway that relied on it is reported. A global `ThrottlerGuard` never counts.

To open a gateway or a handler on purpose, mark it `#[public]`. It takes no arguments and only stores the `PUBLIC_METADATA` entry; a handler's value wins over the gateway's:

```rust
#[websocket_gateway(path = "/status")]
#[public]
pub struct StatusGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl StatusGateway {
    #[subscribe_message("ping")]
    #[public]
    fn ping(&self) {}
}
```

Mark public gateways `#[public]` instead of letting a global guard return `Ok(true)` for `ContextType::Ws`. A global guard that lets WebSocket contexts through counts as protection, so the check would accept every handler while the report shows no real authentication; `#[public]` keeps the intent visible.

A handler protected outside the framework can be exempted with a reason, by gateway path and event name (exact, or `*` for every event of the gateway):

```rust
use corvis::RoutePostureOptions;

app.set_route_posture(
    RoutePostureOptions::enforce()
        .exempt_message("/chat", "ping", "health probe, no data"),
);
```

A blank reason makes `init()` fail, and an exemption that matches no handler logs a warning. Route exemptions never match handlers and message exemptions never match routes.

The handshake is not a message: it runs no guard, so the check cannot enforce it. `app.route_posture()` lists each gateway handshake under "Not checked" with whether `verify_client` is set.

## Authenticating on connect

A guard is evaluated on every message. To reject the whole connection:

- verify the token in `handle_connection` and call `client.disconnect_with(4401, "Unauthorized")`, like Tertulia;
- or reject the handshake with `GatewayOptions::verify_client` (`401`) or `allowed_origins` (`403`).

See [Gateways](/websockets/gateways#options-and-limits-gatewayoptions).

---
status: documented
---

# Execution context

Guards, interceptors, exception filters, and parameter decorators (`FromContext`) receive an `ExecutionContext`. It is the description of "what is running right now": the request, the chosen handler, its controller, and the transport (HTTP or WebSocket). Thanks to it you can write generic pieces that work with any route.

## `ArgumentsHost` and `ExecutionContext`

In Corvis both names designate the same type:

```rust
pub type ArgumentsHost = ExecutionContext;
```

Exception filters receive it as `host: &ArgumentsHost`; guards and interceptors, as `ctx: &ExecutionContext`. They have the same methods.

| Method | Returns |
|---|---|
| `get_type()` | `ContextType::Http` or `ContextType::Ws` |
| `switch_to_http()` | HTTP view: request, response, and status |
| `switch_to_ws()` | WebSocket view: client and message data |
| `request()` | Shortcut for `switch_to_http().get_request()` |
| `get_handler()` | The handler about to run (`&Target`) |
| `get_class()` | The controller or gateway (`&Target`) |
| `module_ref()` | `Option<&ModuleRef>` of the module that owns the controller |

`ContextType` is at the crate root: `use corvis::ContextType;`. It also has an `Rpc` variant and the `as_str()` method (`"http"`, `"ws"`, `"rpc"`), but today no Corvis transport produces `Rpc` contexts (see [Microservices](/microservices/basics)).

### HTTP view

```rust
let http = ctx.switch_to_http();
let request: &Request = http.get_request();
let status: u16 = http.status_code();   // the route's default status
let res: Option<Res> = http.get_response();
```

- `get_request()` gives the `Request`: method, path, headers, query, body, IP, cookies, and the extensions (`insert` / `get`) for passing data between pipeline pieces.
- `status_code()` is the status the response will have if nobody changes it: the one from `#[http_code]`, `201` on `POST`, or `200`.
- `get_response()` returns the `Res` object only if the handler declares a `#[res]` parameter; otherwise it is `None` (the response is built from the return value).

### WebSocket view

In a gateway, each message's context is of type `Ws`:

```rust
let ws = ctx.switch_to_ws();
let client: Option<Arc<WsClient>> = ws.get_client::<WsClient>();
let data: Option<&Value> = ws.get_data();   // the message payload, unvalidated
```

In that case `ctx.request()` is the connection's *handshake* request (headers, query, IP). The `RoomMemberGuard` guard in `examples/tertulia` reads the raw payload because guards run before pipes:

```rust
#[injectable]
pub struct RoomMemberGuard;

#[async_trait]
impl CanActivate for RoomMemberGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let Some(client) = ws.get_client::<WsClient>() else { return Ok(false) };
        let Some(room) = ws.get_data().and_then(|d| d.get("room")).and_then(Value::as_str) else { return Ok(true) };
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

### Pieces that serve both transports

Checking `get_type()` lets the same guard work on HTTP and on WebSockets. This is what `AuthGuard` does in `examples/tertulia/src/auth/auth_guard.rs`:

```rust
async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
    if ctx.get_type() == ContextType::Ws {
        return match ChatUser::of(ctx) {
            Some(_) => Ok(true),
            None => Err(WsException::new("Unauthorized")),
        };
    }
    let request = ctx.switch_to_http().get_request();
    // ... verify the bearer token
    Ok(true)
}
```

## Handler and class: `Target`

`get_handler()` and `get_class()` return a `Target`:

```rust
pub struct Target {
    pub name: String,
    pub metadata: HashMap<String, Value>,
}
```

- `name` is the method name (`"find_one"`) or the type name (`"CatsController"`). It is useful for logs: `format!("{}.{}", ctx.get_class().name, ctx.get_handler().name)`.
- `metadata` contains what was declared with `#[set_metadata]`, `#[apply_decorators]`, and the `#[api_*]` attributes, as JSON. `target.get_metadata("roles")` returns the raw value.

## `Reflector`

`Reflector` reads that metadata and deserializes it into the type you ask for. It is a global provider: inject it with a `reflector: Arc<Reflector>` field.

| Method | Result |
|---|---|
| `get::<T>(key, target)` | The value from a single target, or `None` |
| `get_all_and_override::<T>(key, &[targets])` | The first defined value in the list |
| `get_all_and_merge::<T>(key, &[targets])` | A `Vec<T>` that concatenates the arrays from all targets (single values are added as one element) |
| `get_all::<T>(key, &[targets])` | A `Vec<Option<T>>`, one per target |

```rust
// The handler takes precedence over the controller:
let roles: Option<Vec<String>> =
    self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);

// The controller's roles and the handler's, together:
let roles: Vec<String> =
    self.reflector.get_all_and_merge(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
```

If the stored value can't be deserialized into `T`, it is treated as absent.

## Other uses of the context

- In a filter, `host.switch_to_http().get_request().url()` lets you include the path in the error body (see [Exception filters](/exception-filters)).
- In an interceptor, `ctx.get_handler().name` identifies the operation for metrics (see `WsLoggingInterceptor` in `examples/tertulia/src/chat/ws_logging_interceptor.rs`).
- From `FromContext`, the context is the source of any computed parameter (see [Custom decorators](/custom-decorators)).
- `ctx.module_ref()` gives access to the controller's module container to resolve providers on demand (see [Module reference](/fundamentals/module-ref)).

::: info Differences from the reference model
There are no `getArgs()` or `getArgByIndex()`: the handler's arguments are computed after guards and interceptors and are not in the context. To read request data use `request()`.
:::

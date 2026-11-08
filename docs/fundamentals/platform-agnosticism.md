---
status: partial
---

# Platform agnosticism

The underlying idea is that the application's logic (modules, providers, guards, interceptors) doesn't depend on how the data arrives. In Corvis this holds to a large extent across **transports** (HTTP, WebSockets, server-less context), but not across **HTTP servers**: there is only one.

## A single HTTP server

Corvis ships its own server on top of [hyper](https://hyper.rs) and Tokio. Its behavior reproduces that of an Express 5 application: the body parser, the router, the default headers, `ETag`, CORS, and the `res` object follow that semantics (see [HTTP adapters](/http/performance)).

- There are no interchangeable adapters: you can't switch to another HTTP server without changing the framework.
- `HttpAdapterHost` exists and is injected into any provider, but it gives access to Corvis's server, not to an abstraction over several. `http_adapter().get_type()` returns `"corvis"`.
- What you can do at a low level is register listeners for the `upgrade` event with `get_http_server().on_upgrade(...)` (see [WebSockets: adapters](/websockets/adapter)).

## What is transport-independent

### Providers and modules

Providers know nothing about HTTP. The same module graph works in:

- an HTTP application (`CorvisFactory::create`),
- a server-less context for scripts, CLIs, and workers (`CorvisFactory::create_application_context`, see [Standalone applications](/standalone-applications)),
- a testing module that serves requests in memory, without sockets (see [Testing](/fundamentals/testing)).

### Enhancers shared between HTTP and WebSockets

Guards, interceptors, and pipes use the same traits in controllers and in gateways. The `ExecutionContext` indicates the transport with `get_type()`, so the same piece can serve both. In `examples/tertulia`, `AuthGuard` protects REST routes and chat messages:

```rust
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
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

```rust
// In a controller:
#[post]
#[use_guards(AuthGuard)]
async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> { ... }

// In a gateway:
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]
#[use_interceptors(WsLoggingInterceptor)]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway { ... }
```

`RolesGuard`, in the same example, reads metadata with `Reflector` and works the same on a route with `#[set_metadata(ROLES_KEY, [MODERATOR])]` as on a `#[subscribe_message]` with the same attribute.

DTOs with a schema (`zod_dto!`) and `ZodValidationPipe` are also reused: the chat gateway validates its messages with the same schemas a controller would use.

## Limitations

- **A single HTTP server**, as explained above.
- **No microservices**: `ContextType` has an `Rpc` variant, but no transport produces it. See [Microservices](/microservices/basics).
- **Global enhancers**: global guards, interceptors and pipes (`use_global_*`, `Provider::app_*`) also apply to gateways; `app.apply_global_enhancers_to_gateways(false)` keeps them HTTP-only.
- **Exception filters**: global filters don't apply to gateways; use `#[use_filters]` there (see [WebSockets: exception filters](/websockets/exception-filters)).

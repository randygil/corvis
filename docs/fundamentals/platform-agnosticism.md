---
status: parcial
---

# Independencia de la plataforma

La idea de fondo es que la lógica de la aplicación (módulos, providers, guards, interceptors) no dependa de cómo llegan los datos. En Corvis eso se cumple en buena medida entre **transportes** (HTTP, WebSockets, contexto sin servidor), pero no entre **servidores HTTP**: hay uno solo.

## Un solo servidor HTTP

Corvis trae su propio servidor sobre [hyper](https://hyper.rs) y Tokio. Su comportamiento reproduce el de una aplicación sobre Express 5: el body parser, el router, las cabeceras por defecto, `ETag`, CORS y el objeto `res` siguen esa semántica (ver [Adaptadores HTTP](/http/performance)).

- No hay adaptadores intercambiables: no puedes cambiar a otro servidor HTTP sin cambiar el framework.
- `HttpAdapterHost` existe y se inyecta en cualquier provider, pero da acceso al servidor de Corvis, no a una abstracción sobre varios. `http_adapter().get_type()` devuelve `"corvis"`.
- Lo que sí puedes hacer a bajo nivel es registrar listeners del evento `upgrade` con `get_http_server().on_upgrade(...)` (ver [WebSockets: adaptadores](/websockets/adapter)).

## Lo que sí es independiente del transporte

### Providers y módulos

Los providers no saben nada de HTTP. El mismo grafo de módulos funciona en:

- una aplicación HTTP (`CorvisFactory::create`),
- un contexto sin servidor para scripts, CLIs y workers (`CorvisFactory::create_application_context`, ver [Aplicaciones standalone](/standalone-applications)),
- un módulo de pruebas que atiende peticiones en memoria, sin sockets (ver [Testing](/fundamentals/testing)).

### Enhancers compartidos entre HTTP y WebSockets

Guards, interceptors y pipes usan los mismos traits en controladores y en gateways. El `ExecutionContext` indica el transporte con `get_type()`, así que una misma pieza puede servir a los dos. En `examples/tertulia`, `AuthGuard` protege rutas REST y mensajes del chat:

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
// En un controlador:
#[post]
#[use_guards(AuthGuard)]
async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> { ... }

// En un gateway:
#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]
#[use_interceptors(WsLoggingInterceptor)]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway { ... }
```

`RolesGuard`, en el mismo ejemplo, lee los metadatos con `Reflector` y funciona igual sobre una ruta con `#[set_metadata(ROLES_KEY, [MODERATOR])]` que sobre un `#[subscribe_message]` con el mismo atributo.

Los DTOs con schema (`zod_dto!`) y `ZodValidationPipe` también se reutilizan: el gateway del chat valida sus mensajes con los mismos schemas que usaría un controlador.

## Limitaciones

- **Un solo servidor HTTP**, como se explicó arriba.
- **Sin microservicios**: `ContextType` tiene una variante `Rpc`, pero ningún transporte la produce. Ver [Microservicios](/microservices/basics).
- **Enhancers globales**: `use_global_*` y `Provider::app_*` solo se aplican a las rutas HTTP, no a los gateways. Declara los enhancers del gateway en el propio gateway.
- **Filtros de excepción**: no se aplican a los gateways (ver [WebSockets: filtros de excepción](/websockets/exception-filters)).

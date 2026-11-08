---
status: documented
---

# Guards

Un guard decide si una petición puede llegar al handler. Es el lugar natural para la autenticación y la autorización: a diferencia de un middleware, un guard se ejecuta **después** de elegir la ruta, así que sabe qué handler y qué controlador se van a ejecutar y puede leer sus metadatos.

## El trait `CanActivate`

```rust
#[async_trait]
pub trait CanActivate: Send + Sync + 'static {
    async fn can_activate(&self, context: &ExecutionContext) -> Result<bool, HttpException>;
}
```

| Resultado | Efecto |
|---|---|
| `Ok(true)` | La petición sigue: siguiente guard, interceptors, pipes y handler |
| `Ok(false)` | Se corta con `403` `{"message":"Forbidden resource","error":"Forbidden","statusCode":403}` |
| `Err(excepción)` | Se corta con esa excepción, por ejemplo `UnauthorizedException` |

El `ExecutionContext` da acceso a la petición (`context.switch_to_http().get_request()`), al handler y a la clase (`get_handler()`, `get_class()`) y al tipo de transporte (`get_type()`). Ver [Contexto de ejecución](/es/fundamentals/execution-context).

## Un guard de autenticación

Este guard de `examples/tertulia` verifica un JWT y deja al usuario en las extensiones de la petición para que lo usen los handlers:

```rust
use corvis::prelude::*;
use corvis::{ContextType, JwtService, WsException, extract_bearer_token};

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
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

- Un guard es un `#[injectable]`, así que recibe dependencias como cualquier provider (`jwt: Arc<JwtService>`).
- `request.insert(valor)` guarda un dato en la petición; el handler lo recupera con `req.get::<T>()` o, más cómodo, con un parámetro `FromContext` (ver [Decoradores personalizados](/es/custom-decorators)).
- El mismo guard sirve para HTTP y para WebSockets mirando `ctx.get_type()`.

## Aplicar guards

```rust
#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

| Alcance | Forma |
|---|---|
| Una ruta | `#[use_guards(AuthGuard)]` sobre el método |
| Un controlador | `#[use_guards(AuthGuard)]` debajo de `#[controller]` |
| Varias a la vez | `#[use_guards(AuthGuard, RolesGuard)]` (se ejecutan en ese orden) |
| Global, instancia | `app.use_global_guards(MiGuard)` |
| Global, con DI | `Provider::app_guard::<AuthGuard>()` en `providers` de un módulo |

En `#[use_guards(...)]` puedes pasar un tipo o una expresión. Un tipo se resuelve con inyección de dependencias **en el módulo del controlador**: si está en `providers` se usa esa instancia; si no, Corvis lo construye y lo cachea para ese módulo. En ambos casos sus dependencias deben ser visibles desde ese módulo. Una expresión (`#[use_guards(RolesGuard::new("admin"))]`) construye una instancia sin inyección.

`app.use_global_guards` también recibe una instancia. Para un guard global que necesita providers, regístralo con `Provider::app_guard`:

```rust
#[module(
    imports = [/* ... */ AuthModule, UsersModule, CatsModule],
    controllers = [AppController],
    providers = [
        AppService,
        // Guards globales con inyección de dependencias.
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;
```

(`examples/cats-app/src/app_module.rs`)

### Orden

Para cada ruta los guards se ejecutan así:

1. Globales registrados como provider (`app_guard`), en el orden en que aparecen.
2. Globales de `use_global_guards`.
3. Los del controlador.
4. Los de la ruta.

El primero que devuelve `false` o un error corta la cadena. Los guards se ejecutan después de todos los middleware y antes de los interceptors.

::: warning Lo que no cubren los guards globales
`use_global_guards` y `Provider::app_guard` protegen las rutas de los controladores. También corren en los mensajes de los gateways de WebSocket (ver [guards en WebSockets](/es/websockets/guards)), pero no en los listeners `upgrade` de `corvis::ws`, a los archivos estáticos ni a las rutas de Swagger o Scalar. Un archivo estático con la misma ruta que un endpoint protegido se sirve antes que el endpoint. Protege esas superficies por separado (middleware, `verify_client`, otra raíz de estáticos).
:::

## Autorización por roles con metadatos

Un guard puede leer metadatos que el handler o el controlador declaran con `#[set_metadata(clave, valor)]`. La lectura se hace con `Reflector`, que se inyecta como cualquier provider:

```rust
pub const ROLES_KEY: &str = "roles";

#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let roles: Option<Vec<String>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(roles) = roles else { return Ok(true) };
        Ok(ChatUser::of(ctx).is_some_and(|u| roles.contains(&u.role)))
    }
}
```

```rust
#[post(":room/announcements")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(AuthGuard, RolesGuard)]
async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Value> {
    ...
}
```

- `get_all_and_override(clave, &[handler, clase])` devuelve el primer valor definido: el del handler tiene prioridad sobre el de la clase.
- El valor se deserializa en el tipo que pidas (`Vec<String>`, un `enum` con `serde`, `bool`, ...). Si no existe o no encaja, devuelve `None`.
- Los demás métodos de `Reflector` (`get`, `get_all`, `get_all_and_merge`) se describen en [Contexto de ejecución](/es/fundamentals/execution-context).

## Rutas públicas con un guard global

Una configuración típica instala un `AuthGuard` como guard global (`examples/cats-app` lo hace). Las rutas públicas se marcan con `#[public]`, que guarda la entrada de metadatos `PUBLIC_METADATA` (`"isPublic"`) con el valor `true`. El guard la lee con `Reflector`:

```rust
use corvis::PUBLIC_METADATA;

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool =
            self.reflector.get_all_and_override(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
        if is_public {
            return Ok(true);
        }
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or(UnauthorizedException)?;
        let payload: JwtPayload = self.jwt_service.verify_or_unauthorized(&token)?;
        request.set_user(&payload);
        Ok(true)
    }
}
```

```rust
#[get(":id")]
#[public]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> { ... }
```

`request.set_user(..)` guarda el payload como JSON; se lee con `request.user_as::<JwtPayload>()`.

`#[public]` va sobre un método de ruta dentro de `#[routes]` o debajo de `#[controller]` (todo el controlador es público). No recibe argumentos y no cambia cómo se atienden las peticiones: solo guarda la entrada de metadatos. El valor de la ruta gana sobre el del controlador, así que `#[set_metadata(PUBLIC_METADATA, false)]` en una ruta la vuelve privada dentro de un controlador público. Las aplicaciones que ya marcan rutas con `#[set_metadata(IS_PUBLIC_KEY, true)]` y `IS_PUBLIC_KEY = "isPublic"` siguen funcionando: es la misma clave.

## Exigir un guard en cada ruta

Olvidar `#[use_guards(...)]` pasa en silencio: la ruta funciona y queda abierta a todos. Puedes pedirle a la aplicación que verifique al arrancar que cada ruta de controlador tiene un guard, está marcada `#[public]` o está exenta:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.require_route_posture();
app.listen(3000).await?;
```

En este modo `init()` (y por tanto `listen()`) falla antes de enlazar el servidor y antes de ejecutar cualquier hook de módulo, con un error que lista todas las rutas sin guard:

```text
Route posture check failed: 2 routes have no guard and are not marked #[public].
  GET   /api/rooms                 RoomsController::list
  POST  /api/rooms/:room/messages  RoomsController::post
Add #[use_guards(...)] to the route or its controller (or register a global guard), mark it #[public] if it is meant to be open, or add an exemption with a reason. Guards that do not control access (ThrottlerGuard, ZodGuard) do not count.
```

(El mensaje del error está en inglés.) Los guards cuentan en cada nivel en que el router los ejecuta: `use_global_guards`, `Provider::app_guard`, el controlador y la ruta. Cada alias de ruta y cada versión de URI se revisa por separado.

Para adoptarlo paso a paso, usa el modo de advertencia (warn), que registra la misma lista una vez y arranca con normalidad. Las rutas protegidas fuera del framework (un proxy inverso, una red privada) se pueden eximir, siempre con una razón:

```rust
use corvis::{RequestMethod, RoutePostureOptions};

app.set_route_posture(
    RoutePostureOptions::warn()
        .exempt(RequestMethod::Get, "/metrics", "scraped from the private network only"),
);
```

- El patrón de la exención se compara con la ruta completa (prefijo global y versión incluidos) con las mismas reglas que las [rutas de middleware](/es/middleware): `/api/*/internal/*`. `RequestMethod::All` coincide con cualquier método; una ruta declarada con `#[all]` solo la cubre una exención `All`.
- Una razón en blanco hace que `init()` falle. Una exención que no coincide con ninguna ruta registra una advertencia.
- Un guard que solo limita o valida peticiones no decide quién puede llamar a una ruta. `ThrottlerGuard` y `ZodGuard` no cuentan. Tu propio guard cuenta por defecto; sobrescribe `controls_access()` para que devuelva `false` cuando deja pasar todo salvo que esté configurado (un guard de roles sin roles, por ejemplo). Solo cambia la declaración, no cómo se ejecuta el guard.
- `app.route_posture()` devuelve, después de `init()` y en cualquier modo (también apagado), la postura de cada ruta: método, ruta, controlador y handler, guards, y si está protegida, pública, exenta (con su razón) o sin protección. Implementa `Display` (una tabla) y `serde::Serialize`.
- Llama a `require_route_posture()` o `set_route_posture()` antes de `init()` o `listen()`; después no tienen efecto.
- Las rutas `#[sse]` se revisan como cualquier otra y se marcan como `SSE` en el error y en el reporte.
- Cada handler `#[subscribe_message]` de un gateway de WebSocket también se revisa y aparece en el mismo error como `WS  /chat  event "rooms"  ChatGateway::rooms`. Ver [guards en WebSockets](/es/websockets/guards#route-posture-en-gateways).
- La revisión no puede exigir nada sobre el handshake de WebSocket, los montajes de `ServeStaticModule` ni los listeners `upgrade` directos. `app.route_posture()` los lista en "Not checked" para que quien revisa vea que existen; nunca hacen fallar `init()`.

Contar un guard es una declaración, no una prueba de que autentique: la verificación no distingue un guard de autenticación real de uno que siempre devuelve `true`.

## Guards en los tests

`Test::create_testing_module().override_guard::<RolesGuard>(AllowAll)` reemplaza un guard por un doble de prueba. Ver [Testing](/es/fundamentals/testing).

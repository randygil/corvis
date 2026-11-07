---
status: documentado
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
`use_global_guards` y `Provider::app_guard` protegen las rutas de los controladores. No se aplican a los gateways de WebSocket (decláralos en el gateway), a los listeners `upgrade` de `corvis::ws`, a los archivos estáticos ni a las rutas de Swagger o Scalar. Protege esas superficies por separado.
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

`examples/cats-app` instala `AuthGuard` como guard global y marca las rutas públicas con un metadato:

```rust
pub const IS_PUBLIC_KEY: &str = "isPublic";

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool =
            self.reflector.get_all_and_override(IS_PUBLIC_KEY, &[ctx.get_handler(), ctx.get_class()]).unwrap_or(false);
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
#[set_metadata(IS_PUBLIC_KEY, true)]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> { ... }
```

`request.set_user(..)` guarda el payload como JSON; se lee con `request.user_as::<JwtPayload>()`.

## Guards en los tests

`Test::create_testing_module().override_guard::<RolesGuard>(AllowAll)` reemplaza un guard por un doble de prueba. Ver [Testing](/es/fundamentals/testing).

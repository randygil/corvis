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
`use_global_guards` y `Provider::app_guard` protegen las rutas de los controladores. También corren en los mensajes de los gateways de WebSocket (ver [guards en WebSockets](/es/websockets/guards)), pero no en los listeners `upgrade` de `corvis::ws`, a los archivos estáticos ni a las rutas de Swagger o Scalar. Un archivo estático con la misma ruta que un endpoint protegido se sirve antes que el endpoint. Protege esas superficies por separado (middleware, `verify_client`, otra raíz de estáticos). Para que la aplicación se niegue a arrancar cuando una ruta no tiene guard, y para ver qué superficies no ve la verificación, lee [Postura de rutas](/es/security/route-posture).
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

Una configuración típica instala un `AuthGuard` como guard global (`examples/cats-app` lo hace). El `AuthGuard` de `corvis::auth` ejecuta [estrategias](/es/security/authentication) (JWT, usuario y contraseña, API keys, sesiones, OAuth2) y construye la forma global con `.skip_public()`; el guard de abajo es el que escribes tú mismo. Las rutas públicas se marcan con `#[public]`, que guarda la entrada de metadatos `PUBLIC_METADATA` (`"isPublic"`) con el valor `true`. El guard la lee con `Reflector`:

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

`#[public]` va sobre un método de ruta dentro de `#[routes]` o debajo de `#[controller]` (todo el controlador es público). No recibe argumentos y no cambia cómo se atienden las peticiones: solo guarda la entrada de metadatos. El valor de la ruta gana sobre el del controlador, así que `#[set_metadata(PUBLIC_METADATA, false)]` en una ruta la vuelve privada dentro de un controlador público. Las aplicaciones que ya marcan rutas con `#[set_metadata(IS_PUBLIC_KEY, true)]` y `IS_PUBLIC_KEY = "isPublic"` siguen funcionando: es la misma clave. Con la [verificación de postura de rutas](/es/security/route-posture#marcar-rutas-como-publicas) activada, `#[public]` es también la forma de decirle al framework que una ruta está abierta a propósito.

## Exigir un guard en cada ruta

Olvidar `#[use_guards(...)]` pasa en silencio: la ruta funciona y queda abierta a todos. Puedes pedirle a la aplicación que verifique al arrancar que cada ruta de controlador y cada handler de mensajes de gateway tiene un guard que controla el acceso, está marcado `#[public]` o está exento, y que se niegue a arrancar si no:

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.require_route_posture();
app.listen("127.0.0.1:3000").await?;
```

La página [Postura de rutas](/es/security/route-posture) explica toda la función: la regla, el modo de advertencia y las exenciones con una razón, `#[public]`, el error, `app.route_posture()` y `corvis posture`. Los guards cuentan en cada nivel en que el router los ejecuta: `use_global_guards`, `Provider::app_guard`, el controlador y la ruta. Un guard que solo limita o valida peticiones (`ThrottlerGuard`, `ZodGuard`) no cuenta, y tu propio guard cuenta por defecto. Las secciones siguientes cubren los dos límites de esa verificación que afectan a los guards.

Contar un guard es una declaración, no una prueba de que autentique: la verificación no distingue un guard de autenticación real de uno que siempre devuelve `true`.

### Guards globales que dejan pasar todo

El riesgo es un guard global que deja pasar todo cuando no tiene nada que comprobar. Un guard de roles registrado con `Provider::app_guard::<RolesGuard>()` que devuelve `Ok(true)` cuando la ruta no tiene metadatos `roles` es el caso típico; también lo son los guards de tenant, CSRF, modo mantenimiento y feature flags. Por defecto cuenta como control de acceso, así que todas las rutas parecen protegidas y la verificación pasa aunque nadie se autentique. Sobrescribe `controls_access()` para que devuelva `false` en todo guard que no deniega por defecto:

```rust
#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> { ... }

    fn controls_access(&self) -> bool {
        false
    }
}
```

Para detectar el caso, en modo warn y enforce la verificación registra una línea con los guards que contó, su alcance y cuántas rutas cubre cada uno, por ejemplo `Route posture: access guards counted: RolesGuard (global, 42 routes), AuthGuard (controller CatsController, 5 routes)`. Los handlers de mensajes de gateways se cuentan en la misma línea, con los alcances `gateway ChatGateway` y `message handler ChatGateway::rooms`. Un guard global que cubre todas las rutas y no es tu guard de autenticación es el que hay que revisar. Los mismos datos están en `app.route_posture().unwrap().access_guards()`.

### Modo informe

`corvis posture` ejecuta la aplicación con `CORVIS_ROUTE_POSTURE_REPORT` definida para que imprima este informe y se detenga (ver [`corvis posture`](/es/cli/usages#corvis-posture)). No definas nunca esa variable en producción ni en un trabajo de CI que ejecute `cargo test`: cualquier binario que la lea se detiene en `init()` con el código de salida 40 o 41.

### Lo que la verificación no ve

La verificación cubre las rutas de controladores y los handlers de mensajes de gateways. Estas superficies HTTP quedan fuera, así que protégelas por separado:

- la interfaz de Swagger y Scalar y su documento JSON (se montan como middleware);
- `ServeStaticModule` y `serve_static` (archivos estáticos);
- cualquier middleware de `use_fn` o `configure` que responda sin llamar a `next`;
- los listeners `upgrade` directos y los handshakes de WebSocket (los handlers de mensajes de los gateways sí se cubren, ver [guards en WebSockets](/es/websockets/guards#route-posture-en-gateways)).

## Poner cabeceras de respuesta

Un guard no tiene una respuesta donde escribir, pero puede registrar cabeceras con `ctx.switch_to_http().set_response_header(nombre, valor)`. Devuelve `false` y no registra nada cuando el nombre o el valor no son una cabecera válida (los valores admiten tabulador, ASCII visible y U+0080..U+00FF; CR y LF se rechazan), para las cabeceras de framing que gestiona el framework (`Content-Length`, `Transfer-Encoding`, `Connection`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Upgrade`), cuando ya hay 128 nombres distintos registrados para la petición y en un contexto de WebSocket. Un `Content-Type` puesto así solo se aplica a una respuesta sin cuerpo: JSON, texto, bytes y streams conservan su propio tipo.

```rust
ctx.switch_to_http().set_response_header("X-Trace-Id", "abc123");
```

Las cabeceras registradas se añaden a la respuesta final, en éxito, en errores (incluido el `403` que devuelve el propio guard) y en la respuesta de un filtro de excepciones, pero nunca reemplazan una cabecera que la respuesta ya tiene: ganan `#[header]`, las cabeceras de un `Response` devuelto, las de un filtro y las que el handler pone en `Res`. Llamarla otra vez con el mismo nombre reemplaza el valor registrado. `ThrottlerGuard` lo usa para enviar sus [cabeceras de límite de peticiones](/es/security/rate-limiting#cabeceras-de-respuesta).

## Terminar la petición y añadir cabeceras

Un guard puede hacer más que permitir o denegar. Dos métodos de `ctx.switch_to_http()` cubren lo que un middleware de Express hace con `res`:

- `respond_with(response)` termina la petición con una respuesta propia, normalmente una redirección.
- `append_response_header(nombre, valor)` añade un valor de cabecera que sobrevive al handler, a una excepción y a `#[res]`: la forma de poner una cookie desde un guard.

```rust
#[async_trait]
impl CanActivate for LoginGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        if http.get_request().cookie("sid").is_none() {
            http.append_response_header("Set-Cookie", "next=/cats; Path=/; HttpOnly");
            http.respond_with(Response::redirect("/login", 302u16));
        }
        Ok(true)
    }
}
```

`respond_with` solo funciona mientras corre un guard: desde un interceptor, un handler o un contexto construido a mano no guarda nada y devuelve `false`. Guarda la respuesta y el router la envía en cuanto el guard retorna: los guards siguientes, los interceptores, los pipes y el handler no se ejecutan, ni tampoco los filtros de excepciones, `#[header]` o el `#[redirect]` de la ruta. Funciona con guards globales, de controlador y de ruta. La respuesta guardada gana sea cual sea el resultado del guard: si el guard respondió `Ok(false)` o un error después de guardarla, el resultado se descarta y se registra un error `ERR_HTTP_HEADERS_SENT`, como hace Express cuando algo lanza después de terminar la respuesta. Una segunda llamada reemplaza la primera. Las cabeceras registradas con `set_response_header` y `append_response_header`, y las cabeceras CORS de la aplicación, se añaden a la respuesta guardada como a cualquier otra.

`append_response_header` registra valores que se acumulan en el orden de las llamadas, a diferencia de `set_response_header`, que conserva un valor por nombre. Usa la misma validación: un nombre o valor que Node rechazaría (CR y LF incluidos) y las cabeceras de framing devuelven `false` y no registran nada, y también devuelve `false` cuando ya hay 128 valores registrados para la petición y en un contexto de WebSocket. En la respuesta:

- todos los valores de `Set-Cookie` se añaden, antes de las cookies propias de la respuesta (el orden que produce Express cuando el `res.cookie` de un guard corre antes que el del handler), en éxito, en errores, en la respuesta de un filtro, en rutas con `#[res]` y en una respuesta guardada;
- cualquier otro nombre se añade solo cuando la respuesta no tiene una cabecera con ese nombre, así que ganan `#[header]` y las cabeceras propias del handler; un nombre que se puso y se añadió recibe primero el valor puesto.

Los dos métodos devuelven `false` y no hacen nada en un contexto de WebSocket. Un guard que no usa ninguno se comporta exactamente como antes. [`AuthGuard`](/es/recipes/passport) los usa para redirigir a un proveedor de identidad y poner cookies.

## Guards en los tests

`Test::create_testing_module().override_guard::<RolesGuard>(AllowAll)` reemplaza un guard por un doble de prueba. Ver [Testing](/es/fundamentals/testing).

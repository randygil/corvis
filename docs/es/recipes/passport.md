---
status: partial
---

# Passport (autenticación)

Corvis tiene el **modelo de estrategias** de Passport y de su integración con el framework original: estrategias registradas por nombre, un `AuthGuard` que las ejecuta, el usuario en la request y el `401` que responde el original cuando todas las estrategias fallan. Su comportamiento se comprueba contra la integración original de passport (12.0.1) sobre `passport` 0.7.0 con respuestas grabadas.

Lo que hay ahora es el núcleo: el trait `Strategy` que implementas para tus propias estrategias, `AuthGuard`, `PassportModule` y el parámetro `User<T>`. Las estrategias en sí llegan una a una: JWT, local (usuario y contraseña), API keys, sesiones y OAuth2 tienen su propio issue (#42, #41, #45, #44 y #43). Mientras tanto, [`JwtModule` y un guard propio](#alternativa-jwtmodule-guards) cubren el caso común de los tokens.

## El modelo

| Original | Corvis |
|---|---|
| `class JwtStrategy extends PassportStrategy(Strategy, 'jwt')` | un tipo que implementa `Strategy` (`name()` es `"jwt"`), registrado con `Provider::strategy::<JwtStrategy>()` |
| `validate(payload)` devolviendo el usuario | `authenticate(ctx, options)` devolviendo un `StrategyOutcome::Success` |
| `this.success(user, info)`, `fail`, `redirect`, `pass`, `error` | `StrategyOutcome::{Success, Fail, Redirect, Pass, Error}` |
| `@UseGuards(AuthGuard('jwt'))` | `#[use_guards(AuthGuard::new("jwt"))]` |
| `AuthGuard(['jwt', 'apikey'])` | `AuthGuard::chain(["jwt", "apikey"])` |
| `AuthGuard()` con `PassportModule.register({ defaultStrategy })` | `AuthGuard::default()` con `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` |
| `handleRequest` devolviendo `user \|\| null` | `.optional()` |
| `handleRequest` lanzando otra excepción | `.on_failure(\|failures\| UnauthorizedException::new("Token required"))` |
| `if (isPublic) return true` antes de `super.canActivate` | `.skip_public()` (lee `#[public]`) |
| `req.user` | `request.user()` (JSON) o el valor tipado: un parámetro `User<T>` |
| `req.authInfo` | `request.auth_info()` |
| un decorador de parámetro `@User()` | el parámetro `User<T>` |

## Escribir una estrategia

Una estrategia convierte las credenciales de una request en un usuario. Es un provider normal: recibe sus dependencias por inyección y puede vivir en cualquier módulo.

```rust
#[injectable]
pub struct HeaderStrategy {
    users: Arc<UsersService>,
}

#[async_trait]
impl Strategy for HeaderStrategy {
    fn name(&self) -> &str {
        "header"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        let Some(user) = ctx.request().header("x-user").and_then(|id| self.users.find(id)) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match AuthenticatedUser::new(user) {
            Ok(user) => StrategyOutcome::Success { user, info: None },
            Err(error) => StrategyOutcome::Error(error),
        }
    }
}

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("header"))],
    providers = [Provider::strategy::<HeaderStrategy>()],
    controllers = [CatsController],
)]
pub struct AppModule;
```

`Provider::strategy::<S>()` registra la estrategia con el nombre `S::name()`. La aplicación recoge todas las estrategias al inicializarse; dos estrategias con el mismo nombre detienen el arranque con un error que nombra los dos tipos (el original se queda con la última en silencio).

Una estrategia termina un intento de cinco maneras, las llamadas que puede hacer una estrategia de Passport:

| Resultado | Efecto |
|---|---|
| `Success { user, info }` | el usuario se guarda para el handler, `info` pasa a ser `request.auth_info()` |
| `Fail { challenge, status }` | no autenticado; corre la siguiente estrategia de una cadena. Si todas fallan, la respuesta es el `401` de abajo |
| `Redirect { url, status }` | la petición termina con una redirección (302 salvo que se dé `status`): la URL se envía tal cual, con `Content-Length: 0` y sin cuerpo; un `status` fuera de `3xx` es un error interno (`500`) |
| `Pass` | la ruta corre sin usuario |
| `Error(exception)` | la cadena se detiene y la excepción es la respuesta, sin cambios |

`AuthenticatedUser::new(user)` serializa el usuario a JSON una vez y conserva el valor tipado. Un usuario que no se puede serializar es un error interno (un `500`): la petición no continúa con un usuario tipado y sin usuario JSON.

**Las URL de redirección se envían tal cual.** Construye la URL con datos de confianza (la dirección configurada del proveedor de identidad, una ruta fija). Si el destino de la redirección viene de la petición (un parámetro `return_to` o `next`), valídalo contra una lista de permitidos, o acepta solo una ruta relativa que empiece por una sola `/`, antes de ponerlo en `Redirect`: de lo contrario el login se convierte en una redirección abierta.

Las cabeceras que una estrategia o un guard añadió con `append_response_header` se envían en cada respuesta de esa petición, también en un `401` o `403` posterior de la misma cadena o de otro guard. Añade una cookie solo cuando el intento tuvo éxito.

Una estrategia también puede poner cookies o terminar la petición con `ctx.switch_to_http().append_response_header(..)` y `respond_with(..)`, ver [Guards](/es/guards#terminar-la-peticion-y-anadir-cabeceras).

## `AuthGuard`

```rust
#[controller("cats")]
#[use_guards(AuthGuard::new("header"))]
pub struct CatsController;

#[routes]
impl CatsController {
    #[get("me")]
    fn me(&self, user: User<CatUser>) -> Value {
        json!({ "id": user.id })
    }
}
```

`AuthGuard` es un guard de instancia: `#[use_guards(AuthGuard::new("jwt"))]` en un controlador o una ruta, o `app.use_global_guards(AuthGuard::new("jwt"))`. Está en `corvis::auth`, no en el prelude (muchas aplicaciones tienen un guard propio con ese nombre).

| Constructor o método | Qué hace |
|---|---|
| `AuthGuard::new("jwt")` | una estrategia |
| `AuthGuard::chain(["jwt", "apikey"])` | estrategias probadas en orden: gana el primer éxito, un fallo pasa a la siguiente, un error detiene la cadena, un pass o una redirección la terminan |
| `AuthGuard::default()` | las estrategias de `PassportModule::register(..)` |
| `.optional()` | la ruta corre sin usuario cuando todas las estrategias fallan; los errores se siguen propagando. Un guard opcional no cuenta como control de acceso para el [route posture](/es/guards#exigir-un-guard-en-cada-ruta) |
| `.on_failure(\|failures\| ..)` | la excepción a responder cuando todas las estrategias fallan, construida con el nombre, el challenge y el status de cada fallo |
| `.skip_public()` | deja pasar las rutas `#[public]` sin ejecutar una estrategia: el valor de `PUBLIC_METADATA` del handler, si no el del controlador. Sin él, las rutas públicas se autentican como cualquier otra, igual que en el original |
| `.with_options(json!({ .. }))` | opciones que reciben las estrategias (`AuthenticateOptions::get`) |

Cuando todas las estrategias fallan, la respuesta es `401 {"message":"Unauthorized","statusCode":401}` a través de los filtros de excepciones, sin cabecera `WWW-Authenticate`, sea cual sea el status o el challenge que las estrategias adjuntaron a su fallo (una estrategia que falla con status `400` sigue dando un `401`). El challenge se conserva como `request.auth_info()`: el challenge del fallo con una estrategia, o la lista de challenges, `null` donde una estrategia no dio ninguno, con una cadena.

`User<T>` lee el usuario tipado que guardó el guard. En una ruta donde no hay ninguno (sin guard, o con un guard opcional que no encontró credenciales) responde el mismo `401`. `Request::user()` y `user_as::<T>()` leen la forma JSON.

### Diferencias con el original

- **Errores al arrancar.** El original responde `500 Internal server error` en cada petición cuando un guard nombra una estrategia desconocida, y solo registra un error cuando `AuthGuard()` no tiene estrategia por defecto. Corvis detiene `init()` antes de `configure` del middleware y de los hooks de inicio, con un único error que lista todas las rutas (método y ruta) y el nombre que falta. Cubre también un `AuthGuard::chain` vacío, un valor por defecto no registrado y dos llamadas a `PassportModule::register` con valores por defecto distintos. Un guard propio que llama al `Authenticator` con un nombre desconocido sigue recibiendo el `500` del original en tiempo de petición. Un guard que envuelve un `AuthGuard` puede devolverlo desde `CanActivate::as_any` para que también se compruebe.
- **Sin `AuthGuard` en gateways de WebSocket.** Un `AuthGuard` global también llega a los mensajes de los gateways por defecto, donde el original tampoco funciona. Deniega el mensaje con `Forbidden resource` sin ejecutar una estrategia, y la aplicación registra un aviso por gateway al arrancar. Autentica los sockets en `handle_connection`, o llama a `apply_global_enhancers_to_gateways(false)`, que quita todos los enhancers globales de los gateways, no solo el `AuthGuard`.
- **El usuario tipado vive en su propio espacio.** `User<T>` lee solo lo que guardó un `AuthGuard`, nunca otra extensión de la petición de tipo `T`. Cuando una estrategia hace pass, o un guard opcional no encuentra credenciales, se eliminan tanto el usuario JSON como el tipado (`req.user` es `undefined` o `null` en el original).
- **Sin login de sesión.** `AuthGuard` nunca inicia sesión por sí solo; las sesiones llegan con su propio issue.
- **Sin `property`, `failureRedirect` ni mensajes flash.** El usuario siempre se guarda para `User<T>` y `request.user()`.

## Pruebas diferenciales

Cada caso de arriba está grabado del original: `crates/corvis/tests/fixtures/auth/strategies.mjs` ejecuta la integración original de passport (12.0.1) sobre `passport` 0.7.0 con estrategias propias de `passport-strategy` 1.0.0 llevadas a cada resultado, y `tests/auth_strategies.rs` reproduce las respuestas grabadas contra las mismas rutas en Corvis, sin necesitar Node al ejecutar los tests. Cada tipo de estrategia que llegue después añade sus propios casos.

## Alternativa: `JwtModule` + guards

Hasta que llegue la estrategia JWT, las piezas para autenticar con tokens están en el framework y cubren el caso más común, el de una API con JWT:

- `JwtModule::register(JwtModuleOptions::new(secret))` o `JwtModule::register_async(global, |ctx| ...)` para configurar la firma;
- `JwtService::sign` y `JwtService::verify` (o `verify_or_unauthorized`, que ya devuelve un `401`);
- `extract_bearer_token(&request)` para leer la cabecera `Authorization: Bearer ...`;
- un [guard](/es/guards) que verifica el token y deja el usuario en la request con `request.insert(user)`;
- un parámetro `FromContext` (el equivalente a un decorador `@User()`) que lo recupera en el handler.

El guard de `examples/tertulia` (`src/auth/auth_guard.rs`) hace exactamente eso:

```rust
#[injectable]
pub struct AuthGuard {
    jwt: Arc<JwtService>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let request = ctx.switch_to_http().get_request();
        let token = extract_bearer_token(request).ok_or_else(|| UnauthorizedException::new("Missing bearer token"))?;
        let user: ChatUser = self.jwt.verify_or_unauthorized(&token)?;
        request.insert(user);
        Ok(true)
    }
}
```

El flujo completo (login, guard, usuario actual, roles) se explica en [Autenticación](/es/security/authentication) y [Autorización](/es/security/authorization). Para un inicio de sesión con usuario y contraseña, el "paso local" de Passport es un endpoint propio que verifica la contraseña (ver [Cifrado y hashing](/es/security/encryption-and-hashing)) y devuelve el token.

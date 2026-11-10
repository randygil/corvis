---
status: partial
---

# Passport (autenticación)

Corvis tiene el **modelo de estrategias** de Passport y de su integración con el framework original: estrategias registradas por nombre, un `AuthGuard` que las ejecuta, el usuario en la request y el `401` que responde el original cuando todas las estrategias fallan. Su comportamiento se comprueba contra la integración original de passport (12.0.1) sobre `passport` 0.7.0 con respuestas grabadas.

Lo que hay ahora es el núcleo: el trait `Strategy` que implementas para tus propias estrategias, `AuthGuard`, `PassportModule` y el parámetro `User<T>`, la [estrategia JWT](#la-estrategia-jwt), la [estrategia local](#estrategia-local-usuario-y-contrasena) (usuario y contraseña, con la feature `auth-local`) y las [API keys](#api-keys) (con la feature `auth-api-key`), además de la guía para [escribir una estrategia](#escribir-una-estrategia) propia. Las [sesiones](#sesiones) (la feature `auth-session`) inician sesión de un usuario y lo restauran en las peticiones siguientes. El [login OAuth2](#oauth2-google-github-y-otros) (Google, GitHub y cualquier proveedor, con la feature `auth-oauth2`) hace el flujo de código de autorización. Un [guard propio sobre `JwtModule`](#alternativa-jwtmodule-guards) sigue disponible.

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
pub struct DeviceStrategy {
    devices: Arc<DevicesService>,
}

#[async_trait]
impl Strategy for DeviceStrategy {
    fn name(&self) -> &str {
        "device"
    }

    async fn authenticate(&self, ctx: &ExecutionContext, _options: &AuthenticateOptions) -> StrategyOutcome {
        // Lee la credencial, con límite, de los lugares que esta estrategia acepta y de ningún otro.
        let Some(id) = ctx.request().header("x-device-id").filter(|id| !id.is_empty() && id.len() <= 64) else {
            return StrategyOutcome::Fail { challenge: None, status: None };
        };
        match self.devices.find(id).await {
            Ok(Some(device)) => {
                let info = (id == "dev-info").then(|| json!({ "scope": "telemetry" }));
                match AuthenticatedUser::new(device) {
                    Ok(user) => StrategyOutcome::Success { user, info },
                    Err(e) => StrategyOutcome::Error(e),
                }
            }
            Ok(None) => StrategyOutcome::Fail { challenge: None, status: None },
            // Un error termina la petición: nunca es una forma de entrar.
            Err(DevicesError::Blocked) => StrategyOutcome::Error(ForbiddenException::new("Device blocked")),
            Err(e) => StrategyOutcome::Error(HttpException::internal(e)),
        }
    }
}

#[module(
    imports = [PassportModule::register(PassportModuleOptions::new().default_strategy("device"))],
    providers = [Provider::class::<DevicesService>(), Provider::strategy::<DeviceStrategy>()],
    controllers = [DevicesController],
)]
pub struct AppModule;
```

Este es el código de `crates/corvis/tests/auth_custom.rs`, que un test reproduce contra una estrategia `passport-custom` del original. No hace falta ninguna feature de cargo.

`Provider::strategy::<S>()` registra la estrategia con el nombre `S::name()`. La aplicación recoge todas las estrategias al inicializarse; dos estrategias con el mismo nombre detienen el arranque con un error que nombra los dos tipos (el original se queda con la última en silencio), y lo mismo ocurre con un nombre vacío, de más de 128 bytes o con un carácter de control.

Una estrategia termina un intento de cinco maneras, las llamadas que puede hacer una estrategia de Passport:

| Resultado | Efecto |
|---|---|
| `Success { user, info }` | el usuario se guarda para el handler, `info` pasa a ser `request.auth_info()` |
| `Fail { challenge, status }` | no autenticado; corre la siguiente estrategia de una cadena. Si todas fallan, la respuesta es el `401` de abajo |
| `Redirect { url, status }` | la petición termina con una redirección (302 salvo que se dé `status`): la URL se envía tal cual, con `Content-Length: 0` y sin cuerpo; un `status` fuera de `3xx` es un error interno (`500`) |
| `Pass` | la ruta corre sin usuario |
| `Error(exception)` | la cadena se detiene y la excepción es la respuesta, sin cambios |

`AuthenticatedUser::new(user)` serializa el usuario a JSON una vez y conserva el valor tipado. Un usuario que no se puede serializar es un error interno (un `500`): la petición no continúa con un usuario tipado y sin usuario JSON.

**Las URL de redirección se envían tal cual.** Construye la URL con datos de confianza (la dirección configurada del proveedor de identidad, una ruta fija). Si el destino de la redirección viene de la petición (un parámetro `return_to` o `next`), valídalo contra una lista de permitidos, o acepta solo una ruta relativa que empiece por una sola `/`, que no vaya seguida de `/` ni de `\` y que no contenga barras invertidas ni caracteres de control (los navegadores tratan `\` como `/`, así que `/\evil.com` se convierte en `//evil.com`), antes de ponerlo en `Redirect`: de lo contrario el login se convierte en una redirección abierta.

Las cabeceras que una estrategia o un guard añadió con `append_response_header` se envían en cada respuesta de esa petición, también en un `401` o `403` posterior de la misma cadena o de otro guard. Añade una cookie solo cuando el intento tuvo éxito.

Una estrategia también puede poner cookies o terminar la petición con `ctx.switch_to_http().append_response_header(..)` y `respond_with(..)`, ver [Guards](/es/guards#terminar-la-peticion-y-anadir-cabeceras).

### Si vienes de `passport-custom` y `passport-strategy`

| Original | Corvis |
|---|---|
| `done(null, user, info)` en un callback de `passport-custom` | `StrategyOutcome::Success { user, info }` |
| `done(null, false)` | `StrategyOutcome::Fail { challenge: None, status: None }`: el `401` |
| `done(err)` con una `ForbiddenException` | `StrategyOutcome::Error(ForbiddenException::new(..))`: la respuesta de esa excepción |
| `done(err)` con cualquier otro error, o un callback que lanza una excepción | `StrategyOutcome::Error(HttpException::internal(err))`: el `500`, con el mismo cuerpo para toda petición |
| `this.redirect(url, status)` de una subclase de `passport-strategy` | `StrategyOutcome::Redirect { url, status }` (`passport-custom` no puede llegar a él) |
| `this.pass()` | `StrategyOutcome::Pass` (`passport-custom` no puede llegar a él) |
| una clase `PassportStrategy(CustomStrategy, 'device')` con servicios inyectados | un tipo que implementa `Strategy`, con los servicios como campos |

Las respuestas se comprueban contra `passport-custom` 1.2.1 detrás del guard original (`crates/corvis/tests/fixtures/auth/custom.mjs`, 24 casos: éxito con info, sin usuario, `ForbiddenException`, un `Error` simple, solo y en segundo lugar de una cadena, y un usuario que es `false`, `0`, `""`, `null`, `undefined` o `NaN`).

::: danger El usuario debe ser un valor real
Una estrategia que tiene éxito con un usuario que es JSON `null`, `false`, el número `0` (también `-0` y `0.0`) o la cadena vacía **no autentica**. `Authenticator::authenticate` convierte ese `Success` en un fallo de la estrategia (la cadena sigue, el `auth_info` es `{"message":"The strategy returned an empty user"}` y la respuesta es el `401` de siempre) y registra una advertencia, como máximo una por minuto y por estrategia, que dice que el validador debió devolver `None`. Vale para toda estrategia: JWT, local, API key y las tuyas. Los originales fallan cerrado igual (`if (!user) return self.fail()` en las estrategias, y `handleRequest` lanza un `401` para un usuario falsy).

Lo normal es caer en esto con un validador que devuelve lo que calculó. `type User = bool; Ok(Some(expected.matches(key)))` y `type User = Option<Service>; Ok(Some(repo.find(id)))` parecen correctos, pero dicen "encontrado" para toda clave, y el usuario es `false` o `null` cuando la clave es incorrecta. Devuelve `Ok(None)` para "no hay usuario": `Ok(expected.matches(key).then_some(service))`, `Ok(repo.find(id))`.

Si los usuarios de una aplicación pueden ser legítimamente `0` o `""` (un id numérico como usuario completo), envuélvelos en un struct: `struct Account { id: u64 }` se serializa a un objeto y nunca está vacío. Un objeto vacío `{}`, un arreglo vacío y las cadenas `"0"` y `"false"` son usuarios reales, como en JavaScript.
:::

### Qué garantiza el framework alrededor de una estrategia

- **Un error nunca deja pasar una petición.** `Error(..)` termina la cadena y es la respuesta, también detrás de `.optional()`: un guard opcional deja pasar una petición solo cuando todas las estrategias *fallaron*.
- **Un pánico es un error.** Un pánico dentro de `authenticate` se captura y se responde como un `500 {"statusCode":500,"message":"Internal server error"}`, igual para toda petición, también detrás de un guard opcional y sin ejecutar las estrategias siguientes de la cadena. Corvis nunca lee el contenido del pánico ni lo registra. No queda oculto para el resto: el hook de pánico por defecto de la biblioteca estándar imprime el mensaje del pánico en stderr, así que una credencial en un mensaje de pánico acaba en los logs de tu proceso. Una estrategia no debe poner credenciales en mensajes de pánico (`unwrap`, `expect` y `panic!` sobre un valor que la contenga) ni en errores. Corvis registra una advertencia por minuto y por estrategia (con la cantidad de las omitidas), y el filtro de excepciones por defecto no añade una línea por petición.
- **Un usuario vacío es un fallo.** Ver el aviso de arriba.
- **Los nombres se comprueban al arrancar.** `AuthGuard::new("devise")` para una estrategia llamada `device` detiene `init()`, también dentro de un guard que lista sus guards internos en `CanActivate::inner_guards`.
- **Respuestas durante la fase de guards.** Una estrategia termina una petición por sí misma solo con `Redirect` (solo estados 3xx) y, para cookies, con `append_response_header`. `respond_with` guarda una respuesta que prevalece sobre todo lo demás que haga la cadena: no lo llames desde una estrategia para decir "no".
- **No hay timeout alrededor de una estrategia.** Una estrategia que espera a una base de datos o a un servicio debe acotar la espera ella misma (`tokio::time::timeout`), o una dependencia lenta retiene todas las peticiones. La [estrategia de API key](#api-keys) lo hace con su validador.

### Probar una ruta que usa una estrategia

`override_strategy` reemplaza una estrategia registrada en un módulo de testing. Pon al stub el nombre de la estrategia que reemplaza, para que las rutas y las comprobaciones de arranque sigan igual:

```rust
let module = Test::create_testing_module()
    .import::<DevicesModule>()
    .override_strategy::<DeviceStrategy>(StubDevice) // StubDevice::name() es "device"
    .compile()
    .await?;
let mut app = module.create_corvis_application();
app.init().await?;
```

### Lista de seguridad para una estrategia

- **Falla, nunca dejes pasar.** Devuelve `Fail` (o `Error`) cuando las credenciales faltan, están mal formadas o son incorrectas. `Pass` deja pasar la petición sin usuario: úsalo solo en una estrategia pensada para omitir la autenticación.
- **Compara los secretos en tiempo constante.** `==` se detiene en el primer byte distinto. Con la feature `auth-api-key`, `corvis::auth::api_key::constant_time_eq(a, b)` compara digests SHA-256 con una primitiva de tiempo constante; guarda los secretos con hash.
- **Rechaza un secreto vacío o sin configurar antes de comparar.** `constant_time_eq(b"", b"")` es `true`: un secreto esperado que falte en la configuración (una variable de entorno sin definir leída como `""`) aceptaría una petición con credencial vacía. Comprueba que el secreto esperado no esté vacío (y sea suficientemente largo) al arrancar o antes de comparar, y falla si lo está.
- **Devuelve `None`, no un usuario vacío.** `Ok(Some(false))`, `Ok(Some(None))`, `0` y `""` se tratan como un fallo con una advertencia (ver arriba); di "no hay usuario" con `None` o `Fail`.
- **Mantén las credenciales fuera de los mensajes de pánico.** El hook de pánico por defecto imprime el mensaje en stderr, que Corvis no puede interceptar.
- **Mantén los datos de la petición fuera de errores y logs.** El mensaje de un `Error`, un challenge de `Fail` y una línea de log acaban en algún sitio. Solo cadenas fijas, y nunca una credencial ni parte de ella.
- **Acota lo que lees.** Comprueba la longitud de una credencial antes de aplicarle un hash, compararla o enviarla a una base de datos, y lee solo las cabeceras, parámetros o cookies que documentas. Lee un solo lugar, o define cuál prevalece y falla cuando dos discrepan.
- **Registra las credenciales leídas de la URL.** Una estrategia lista los parámetros de query que lee en `Strategy::credential_query_params`, y `LoggingInterceptor` oculta sus valores en toda petición (ver [API keys](#api-keys)); un guard propio marca uno con `request.redact_query_param("name")` antes de leerlo. Prefiere una cabecera: una URL se escribe en los logs de proxies y de acceso, y nada fuera de Corvis la oculta.
- **Nunca construyas el destino de una redirección con datos de la petición** (ver la advertencia de arriba).
- **Trata una dependencia lenta o que falla como un error**, con la misma respuesta para toda credencial: el cliente no debe saber por el estado ni por el cuerpo cuál clave o usuario era.

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

- **Errores al arrancar.** El original responde `500 Internal server error` en cada petición cuando un guard nombra una estrategia desconocida, y solo registra un error cuando `AuthGuard()` no tiene estrategia por defecto. Corvis detiene `init()` antes de `configure` del middleware y de los hooks de inicio, con un único error que lista todas las rutas (método y ruta) y el nombre que falta. Cubre también un `AuthGuard::chain` vacío, un valor por defecto no registrado y dos llamadas a `PassportModule::register` con valores por defecto distintos. Un guard propio que llama al `Authenticator` con un nombre desconocido sigue recibiendo el `500` del original en tiempo de petición. Un guard que envuelve un `AuthGuard` (un guard compuesto) lo lista en `CanActivate::inner_guards`, que la comprobación recorre a cualquier profundidad, así que un error de escritura en el nombre envuelto también detiene `init()`: `fn inner_guards(&self) -> Vec<&dyn CanActivate> { vec![&self.jwt] }`. No devuelvas el guard interno desde `as_any`: ese método es para el guard mismo, y haría saltar el aviso de gateways de abajo en un wrapper que deja pasar los destinos públicos.
- **Sin `AuthGuard` en gateways de WebSocket.** Un `AuthGuard` global también llega a los mensajes de los gateways por defecto, donde el original tampoco funciona. Deniega el mensaje con `Forbidden resource` sin ejecutar una estrategia, y la aplicación registra un aviso por gateway al arrancar. Autentica los sockets en `handle_connection`, o llama a `apply_global_enhancers_to_gateways(false)`, que quita todos los enhancers globales de los gateways, no solo el `AuthGuard`.
- **El usuario tipado vive en su propio espacio.** `User<T>` refleja solo lo que guardó un `AuthGuard`, nunca otra extensión de la petición de tipo `T`, y `Request::set_user` lo elimina, así que si un guard posterior fija su propio usuario `User<T>` responde `401`. Cuando una estrategia hace pass, o un guard opcional no encuentra credenciales, se eliminan tanto el usuario JSON como el tipado (`req.user` es `undefined` o `null` en el original).
- **El login de sesión es opcional.** `AuthGuard` nunca escribe en una sesión salvo que se construya con `.log_in()`; ver [Sesiones](#sesiones).
- **Sin `property`, `failureRedirect` ni mensajes flash.** El usuario siempre se guarda para `User<T>` y `request.user()`.

## Sesiones

La feature `auth-session` añade la parte de sesión de Passport sobre las [sesiones de servidor](/es/http/session): el
usuario se guarda en la sesión al iniciar sesión y se restaura en las peticiones siguientes.

| Original | Corvis |
|---|---|
| `passport.serializeUser` y `passport.deserializeUser` | `SessionSerializer::serialize_user` y `deserialize_user` |
| `app.use(passport.session())` | `SessionModule` (su middleware) y `SessionStrategy<S>`, la estrategia llamada `"session"` |
| `req.logIn(user)` | `AuthGuard::new("local").log_in()`, o `Authenticator::log_in(&req, &authenticated_user).await?` |
| `req.logout()` | `Authenticator::log_out(&req).await?` |
| un guard sobre `req.isAuthenticated()` | `AuthGuard::new("session")`: `401 {"message":"Unauthorized","statusCode":401}` si es anónimo. La estrategia de sesión del original siempre pasa |
| `keepSessionInfo` | `SessionOptions::keep_session_info_on_login(true)` (una opción del módulo, no de cada llamada) |

Un ejemplo completo, con la [estrategia local](#estrategia-local-usuario-y-contrasena) para el login:

```rust
use corvis::auth::session::{SessionModule, SessionSerializer, SessionStrategy};
use corvis::auth::{AuthGuard, Authenticator, User};

#[injectable]
pub struct UserSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    // lo que va en `passport.user`: un objeto, nunca un `0` o `""` suelto
    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    // `None` cuando el usuario ya no existe: la petición es anónima y se descarta el usuario obsoleto
    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64) else { return Ok(None) };
        Ok(self.users.find(id).await)
    }
}

#[module(
    imports = [SessionModule::for_root(session_options())],
    providers = [
        UsersService,
        Provider::class::<PasswordHasher>(),
        Provider::class::<LocalAuth>(),
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
        Provider::strategy::<SessionStrategy<UserSerializer>>(),
        UserSerializer,
    ],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth")]
pub struct AuthController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Account {
        user.0
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(204)]
    async fn logout(&self, #[req] req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}
```

`session_options()` construye las `SessionOptions`, ver [Sesión](/es/http/session#configuracion). `Authenticator::log_in`
recibe un `AuthenticatedUser` (`AuthenticatedUser::new(user)?`). Iniciar sesión genera un identificador de sesión nuevo y
destruye el anterior, lo que evita la fijación de sesión. El middleware restaura el usuario en todas las rutas, también
las públicas, antes de que se ejecuten los guards. `AuthGuard::new("local").log_in()` es un error de arranque cuando no
hay ningún `SessionStrategy` registrado, y cuando se combina con la estrategia `"session"`. El resto de las reglas (qué
hace un guard opcional con el usuario de la sesión, `AuthGuard::chain(["session", "jwt"])`, el `500` de un guardado
fallido) están en [Sesión](/es/http/session#guards-y-el-usuario-de-la-sesion).

## La estrategia JWT

`corvis::auth::jwt` (con el feature `jwt`, activo por defecto) es el port de `passport-jwt` 4.0.1 sobre `jsonwebtoken` 9. Escribes la clase del original, `PassportStrategy(Strategy)` con sus opciones y `validate`, como un `JwtValidator`, y registras `JwtStrategy<TuValidator>`:

```rust
use corvis::auth::jwt::{ExtractJwt, JwtAlgorithm, JwtKey, JwtStrategy, JwtStrategyOptions, JwtValidator};

#[injectable]
pub struct JwtAuth {
    config: Arc<ConfigService>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Claims; // lo que reciben los handlers como User<Claims>

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.config.get::<String>("JWT_SECRET").unwrap_or_default()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Claims>, HttpException> {
        Ok(serde_json::from_value(payload).ok()) // None responde 401
    }
}

#[module(
    providers = [JwtAuth, Provider::strategy::<JwtStrategy<JwtAuth>>()],
    controllers = [CatsController],
)]
pub struct AuthModule;

// en un controlador, una ruta o de forma global
#[use_guards(AuthGuard::new("jwt"))]
```

`options()` se ejecuta una vez, al arrancar la aplicación, y cualquier problema en ella detiene el arranque con un error que nombra la estrategia (`Error while instantiating JwtStrategy "jwt" (..): invalid JWT strategy options: ..`). `validate` solo se ejecuta para un token que se encontró y se verificó; el payload es un objeto JSON. Devuelve el usuario (`Some`), ningún usuario (`None`, un `401`) o una excepción (`Err`, que se propaga sin cambios, como hace `done(err)` en el original). La estrategia se llama `"jwt"`; define `const NAME: &'static str = "jwt-refresh";` en un segundo validador para registrar otra estrategia a su lado (cada una con su propia clave). `fn info(&self, user)` puede devolver la información de autenticación de un éxito.

`passReqToCallback` no existe: la request es siempre el primer argumento de `validate`.

### Opciones

| `passport-jwt` | Corvis | Notas |
|---|---|---|
| `jwtFromRequest` | `.jwt_from_request(ExtractJwt::..)` | obligatoria; no hay un lugar por defecto |
| `secretOrKey` | `.secret_or_key(JwtKey::..)` | `JwtKey::secret`, `rsa_pem`, `ec_pem`, `ed_pem` |
| `secretOrKeyProvider` | `.secret_or_key_provider(provider)` | un `JwtKeyProvider` asíncrono; da una clave o un provider, no ambos |
| `issuer` | `.issuer("https://auth.example")` o una lista | el `iss` del token debe ser igual a uno de ellos |
| `audience` | `.audience("web")` o una lista | el `aud` del token (cadena o lista) debe contener uno de ellos |
| `algorithms` | `.algorithms([..])` | ver [Algoritmos y claves](#algoritmos-y-claves) |
| `ignoreExpiration` | `.ignore_expiration(true)` | registra un aviso al arrancar |
| `jsonWebTokenOptions.clockTolerance` | `.clock_tolerance(Duration)` | 0 por defecto, como máximo 300 s |
| `passReqToCallback` | | siempre activa |
| | `.max_token_length(bytes)` | 8192 por defecto, se comprueba antes de decodificar |
| | `.allow_missing_exp()` | acepta tokens sin `exp`; registra un aviso al arrancar |

`maxAge`, `subject`, `jwtid`, `nonce`, `complete`, `clockTimestamp` y las audiencias con expresiones regulares no están soportadas.

### De dónde se lee el token

| `ExtractJwt` en `passport-jwt` | Corvis |
|---|---|
| `fromAuthHeaderAsBearerToken()` | `ExtractJwt::from_auth_header_as_bearer_token()` |
| `fromAuthHeaderWithScheme(s)` | `ExtractJwt::from_auth_header_with_scheme(s)` |
| `fromHeader(name)` | `ExtractJwt::from_header(name)` |
| `fromUrlQueryParameter(name)` | `ExtractJwt::from_url_query_parameter(name)` |
| `fromBodyField(name)` | `ExtractJwt::from_body_field(name)` |
| `fromExtractors([..])` | `ExtractJwt::from_extractors([..])`: gana el primer token no vacío, sin recurrir al siguiente |
| `req => req.cookies?.[name]` | `ExtractJwt::from_cookie(name)` |
| cualquier función | `ExtractJwt::custom(\|req\| ..)` |

La cabecera `Authorization` se interpreta exactamente como lo hace `passport-jwt`: el primer valor de `Authorization` se compara con `(\S+)\s+(\S+)` (así que un tabulador puede separar el esquema y el token, y las palabras después del token se ignoran) y el esquema se compara sin distinguir mayúsculas. `extract_bearer_token` conserva su propio comportamiento, más estricto. Un valor vacío, o que no es una cadena (un número en un cuerpo JSON), no es un token. Los tokens se leen solo de los lugares que configuras: un token válido en el query o en una cookie no hace nada si ningún extractor lo lee.

::: warning El orden importa en `from_extractors`
Solo se prueba el primer token no vacío: si falla, no se consultan los extractores siguientes. Con un extractor de cookie antes del de cabecera, un atacante que pueda plantar una cookie (desde un subdominio hermano, por ejemplo) anula la cabecera `Authorization` del usuario. Pon primero el lugar más confiable y lee solo los lugares que necesites.
:::

::: warning Un token en una cookie es una credencial ambiental
El navegador envía la cookie por sí solo, así que un token en cookie necesita las protecciones de cualquier sesión por cookie: mitiga CSRF (un token o una cabecera propia en las peticiones que cambian estado, o una comprobación de `Origin`) y usa `SameSite=Lax` o `Strict`, `Secure` y `HttpOnly` al emitirla.
:::

::: warning Tokens en URLs
Un token en el query string acaba en los logs de acceso, el historial del navegador y las cabeceras `Referer`. La estrategia registra un aviso al arrancar cuando se configura `from_url_query_parameter`; prefiere la cabecera `Authorization` o una cookie.
:::

### Algoritmos y claves

Un token se acepta solo cuando el `alg` de su cabecera está en la lista permitida **y** pertenece a la familia de la clave con la que se verifica, así que una clave pública nunca puede usarse como secreto HMAC, y `none` (con cualquier combinación de mayúsculas) nunca puede aceptarse: no hay forma de escribirlo.

| Clave | Permitidos por defecto |
|---|---|
| `JwtKey::secret(..)` | `HS256`, `HS384`, `HS512` |
| `JwtKey::rsa_pem(..)` | `RS256`, `RS384`, `RS512`, `PS256`, `PS384`, `PS512` |
| `JwtKey::ec_pem(..)` | `ES256`, `ES384` |
| `JwtKey::ed_pem(..)` | ninguno: escribe `.algorithms([JwtAlgorithm::EdDSA])` |

Con una clave fija, `.algorithms([..])` no puede estar vacía y debe pertenecer a la familia de la clave (un algoritmo HMAC con una clave RSA, o una lista que mezcla familias, detiene el arranque). Con un provider de claves, la lista (o todas las de por defecto) se reduce a la familia de la clave que recibe cada request; un resultado vacío es un `401`. `jsonwebtoken` 9 no tiene `ES512`, así que un token `ES512` siempre falla.

Un secreto HMAC debe tener al menos 32 bytes (RFC 7518, sección 3.2): uno más corto, o vacío, detiene el arranque, y un provider de claves que devuelve uno hace fallar esa request (`JwtService` sigue aceptando secretos más cortos). Genera uno con `openssl rand -hex 32`.

Un provider de claves recibe la request y el token en bruto, que **todavía no está verificado**: usa su `iss` o su `kid` solo para elegir una clave, nunca para confiar en un valor. Se ejecuta en cada request, así que guarda en caché lo que obtengas; un error es un `401`, no un `500`. El texto del error se registra en nivel debug, así que un provider nunca debe poner el token en bruto en su error. Antes de que el provider se ejecute, la estrategia rechaza un token más largo que `max_token_length`, uno con cabecera `crit` (RFC 7515, sección 4.1.11; Corvis no soporta extensiones críticas) y uno que no sean exactamente tres segmentos base64url no vacíos.

```rust
struct ByIssuer {
    keys: HashMap<String, JwtKey>,
}

#[async_trait]
impl JwtKeyProvider for ByIssuer {
    async fn key(&self, _req: &Request, raw_token: &str) -> anyhow::Result<JwtKey> {
        let iss = unverified_issuer(raw_token).ok_or_else(|| anyhow::anyhow!("no issuer"))?;
        self.keys.get(&iss).cloned().ok_or_else(|| anyhow::anyhow!("unknown issuer"))
    }
}
```

### Claims y tiempo

`exp` y `nbf` siguen a `jsonwebtoken` 9: con `now` en segundos enteros y la tolerancia `t`, un token falla cuando `now >= exp + t` (`jwt expired`) o `nbf > now + t` (`jwt not active`), y cuando `exp`, `nbf` o `iat` están presentes y no son un número. `ignore_expiration` omite la comprobación y la exigencia de `exp`. La tolerancia es de 0 s por defecto y tiene un tope de 300 s: un valor mayor detiene el arranque.

### Diferencias con `passport-jwt`

Cada una es a propósito, y el test diferencial la marca:

- **`exp` es obligatorio.** El original acepta un token sin `exp`, que entonces nunca caduca. Añade `.allow_missing_exp()` para aceptarlos (registra un aviso al arrancar); los tokens de `JwtService::sign` llevan `exp` siempre que `expires_in` esté definido.
- **Un `iat` no numérico falla.** `jsonwebtoken` comprueba `iat` solo con `maxAge`.
- **El payload debe ser un objeto JSON.** El original pasa un payload de tipo cadena a `validate`.
- **Los nombres de cabecera no distinguen mayúsculas** en `from_header`; el original solo encuentra nombres escritos en minúsculas.
- **Los tokens están limitados a 8192 bytes** por defecto (`.max_token_length(..)`).
- **La configuración se comprueba al arrancar**: un secreto HMAC de menos de 32 bytes, un issuer o una audience vacíos, una tolerancia superior a 300 s y las demás combinaciones de arriba no arrancan.
- **`ES512` no está disponible** (`jsonwebtoken` 9).
- **Un token con cabecera `crit`, o que no sean tres segmentos base64url, falla** antes de que se ejecute el provider de claves.

`typ` no se valida, y la estrategia no tiene opción para ello. No firmes otros tipos de token (refresh, verificación de correo, restablecimiento de contraseña) con la misma clave y audience que los tokens de acceso: uno se aceptaría como el otro. Usa una clave o un `aud` distintos para cada tipo. Una comprobación de `typ` podría llegar después como opción.

Un intento fallido conserva su motivo como `request.auth_info()` (`{"message": "jwt expired"}`, `invalid signature`, `invalid algorithm`, `jwt malformed`, `No auth token`, `token too long`, ..), nunca en la respuesta, que es el `401` habitual. Los motivos nunca repiten nada del token.

### Pruebas contra el original

`crates/corvis/tests/fixtures/auth/jwt.mjs` ejecuta `passport-jwt` 4.0.1 (sobre `jsonwebtoken` 9.0.x) detrás del guard original con una ruta por estrategia y graba 65 casos (`jwt.json`): extractores, interpretación de `Authorization`, algoritmos (`none`, confusión entre familias, listas explícitas), claims de tiempo, audience e issuer, providers de claves, RS256, PS256 y ES256. `crates/corvis/tests/auth_jwt.rs` los reproduce contra las mismas rutas, firmando los tokens de nuevo, sin necesitar Node al ejecutar los tests.

## Estrategia local (usuario y contraseña)

La estrategia local es el port de `passport-local` 1.0.0. Necesita la feature de cargo `auth-local`, que también trae [`PasswordHasher`](/es/security/encryption-and-hashing). Tú escribes el validador, la clase `PassportStrategy(Strategy)` del original, y registras `LocalStrategy<TuValidador>`:

```rust
use corvis::auth::local::{LocalStrategy, LocalValidator, PasswordError, PasswordHasher};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

#[injectable]
pub struct LocalAuth {
    users: Arc<UsersService>,
    hasher: Arc<PasswordHasher>,
}

#[async_trait]
impl LocalValidator for LocalAuth {
    type User = PublicUser; // Serialize + Clone; deja fuera el hash de la contraseña

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<PublicUser>, HttpException> {
        let user = self.users.find_by_username(username).await;
        // una verificación real exista o no el usuario
        let ok = self
            .hasher
            .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
            .await
            .map_err(PasswordError::into_http_exception)?; // Busy es un 503, igual para todos los usuarios
        Ok(user.filter(|_| ok).map(PublicUser::from))
    }
}

#[module(
    providers = [
        Provider::class::<PasswordHasher>(),
        Provider::class::<LocalAuth>(),
        Provider::strategy::<LocalStrategy<LocalAuth>>(),
    ],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth")]
pub struct AuthController;

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<PublicUser>) -> Value {
        json!({ "username": user.username })
    }
}
```

`validate` devuelve el usuario (un éxito), `Ok(None)` (se rechazan las credenciales: `401`) o una excepción, que se responde tal cual (un `ForbiddenException::new("Account locked")` da el `403`). La estrategia se llama `"local"`; define `const NAME` en el validador para otro nombre.

| `passport-local` | Corvis (`LocalValidator::options`) |
|---|---|
| `usernameField` (`username`) | `LocalStrategyOptions::default().username_field("email")` |
| `passwordField` (`password`) | `.password_field("pass")` |
| `passReqToCallback` | `validate` siempre recibe la `Request` |
| ninguna | `.max_username_bytes(1024)`, `.max_password_bytes(1024)` |
| leer el query string | `.allow_query_credentials()`, desactivado por defecto |

Las credenciales se leen como lo hace `passport-local`: cada campo se busca en el body ya parseado (JSON o `application/x-www-form-urlencoded`). A diferencia del original, el query string se lee solo con `.allow_query_credentials()`; entonces un campo sin valor en el body se busca también allí. Un nombre de campo como `user[email]` es una ruta (`body.user.email`); un segmento de un array es un índice; una cadena que aparece antes del final de la ruta es el valor; un objeto o un array al final no da valor. Un valor ausente, vacío o que no es una cadena cuenta como ausente, y cuando falta cualquiera de las dos credenciales la estrategia falla sin llamar a `validate`. `AuthGuard` responde el mismo `401` para cualquier fallo; el fallo en sí (`{"message":"Missing credentials"}`, estado `400`) está disponible en `.on_failure(..)`. Un nombre de campo vacío detiene la aplicación al arrancar.

No envíes credenciales en el query string salvo que no haya otra opción: las URL acaban en los logs de acceso, el historial del navegador y las cabeceras `Referer`. Por eso leerlas es opcional, y se registra una advertencia al arrancar cuando está activado.

### Diferencias con passport-local

- **El query string no se lee por defecto.** El original toma el usuario y la contraseña del query cuando el body no los tiene; Corvis solo lo hace con `.allow_query_credentials()`, porque una contraseña en una URL queda escrita en los logs. El fixture grabado reproduce estos casos de las dos formas.
- **Las credenciales que no son cadenas cuentan como ausentes.** El original pasa un número o un booleano a `validate()`; Corvis los trata como ausentes, de modo que una búsqueda de usuario nunca recibe un tipo que no espera.
- **Las credenciales tienen un límite.** Un usuario o una contraseña de más de 1024 bytes (configurable) falla sin llamar a `validate`. El original no tiene más límite que el tamaño del body. Sube los límites con `.max_password_bytes(4096)` si tus usuarios lo necesitan; el hasher tiene su propio `max_password_bytes`.
- **Los fallos son uniformes.** Credenciales ausentes, contraseñas incorrectas y usuarios desconocidos dan el mismo `401`, y el validador de arriba gasta una verificación de hash en todos los casos. Un hash guardado que el hasher rechaza (una cuenta sin contraseña, un formato no compatible) también es una contraseña incorrecta, no un `500`. Cuando el hasher está saturado (`PasswordError::Busy`) la respuesta es `503 Service Unavailable` para todos los usuarios; ver [un hasher ocupado](/es/security/encryption-and-hashing#un-hasher-ocupado).
- **El límite de intentos de login es aparte.** Combina la ruta de login con el [throttler](/es/security/rate-limiting) (el ejemplo de gatos permite cinco intentos por minuto y cliente); el hashing es caro a propósito, y una avalancha hace cola en los huecos `max_concurrent` del hasher.


## API keys

La estrategia de API key es el port de `passport-headerapikey` 1.2.2 para clientes máquina (webhooks, servicios internos, integraciones de socios). Necesita la feature de cargo `auth-api-key` (una dependencia extra, `subtle`). Tú escribes el validador, el callback `verify` del original, y registras `ApiKeyStrategy<TuValidador>`:

```toml
corvis = { version = "0.1", features = ["auth-api-key"] }
```

```rust
use corvis::auth::api_key::{ApiKey, ApiKeySet, ApiKeyStrategy, ApiKeyValidator, ExpectedApiKey};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

/// Las claves de los servicios que llaman a esta aplicación, desde la configuración.
pub struct ServiceKeys(ApiKeySet<Service>);

#[injectable]
pub struct ServiceKeyAuth {
    keys: Arc<ServiceKeys>,
}

#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = Service; // Serialize + Clone; deja la clave fuera

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
        Ok(self.keys.0.lookup(key).cloned()) // None responde 401
    }
}

#[module(
    providers = [
        // una clave de menos de 32 bytes es un error aquí, y la aplicación no arranca
        Provider::use_factory(Token::of::<ServiceKeys>(), |ctx| async move {
            let config = ctx.get_or_construct::<ConfigService>().await?;
            let mut keys = ApiKeySet::new();
            keys.insert(ExpectedApiKey::new(config.get::<String>("BILLING_API_KEY").unwrap_or_default())?, Service::billing());
            Ok(ServiceKeys(keys))
        }),
        Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>(),
    ],
    controllers = [DataController],
)]
pub struct MachineModule;

#[get("data")]
#[use_guards(AuthGuard::new("api-key"))]
fn data(&self, service: User<Service>) -> Value { /* .. */ }
```

La estrategia se llama `"api-key"` (el original la llama `headerapikey`: define `const NAME: &'static str = "headerapikey";` en el validador para portar una configuración sin cambios, u otro nombre para tener varias estrategias de API key). `validate` solo se ejecuta para una clave que se encontró en los lugares configurados y está dentro de los límites. Devuelve el usuario (`Some`), ningún usuario (`None`, un `401`) o una excepción (se pasa tal cual, ver [fallos](#fallos-del-validador)). `fn info(&self, user)` puede devolver la info de auth de un éxito: nunca pongas la clave en ella.

Una ruta para personas y máquinas prueba primero la estrategia de usuario: `AuthGuard::chain(["jwt", "api-key"])`.

### De dónde se lee la clave

`ApiKeyStrategyOptions::default()` lee la cabecera `X-Api-Key` y nada más. Añade los lugares que aceptas con `fn options(&self)`:

```rust
fn options(&self) -> ApiKeyStrategyOptions {
    ApiKeyStrategyOptions::new()
        .source(ApiKeySource::header("X-Api-Key"))
        .source(ApiKeySource::header("Authorization").prefix("Api-Key "))
}
```

| Fuente | Lee | Equivale a |
|---|---|---|
| `ApiKeySource::header(name)` | todos los valores de la cabecera, leídos como latin1 y unidos con `", "` como hace Node; el nombre no distingue mayúsculas | `passport-headerapikey` (`header`) |
| `.prefix("Api-Key ")` en una fuente de cabecera | el valor debe empezar con el prefijo (comparado literalmente, distinguiendo mayúsculas), que se quita; un valor que no empieza con él está ausente, sea cual sea su largo o contenido, así que una cabecera `Authorization` ajena nunca hace fallar una clave encontrada en otro sitio | `passport-headerapikey` (`prefix`), pero literal |
| `ApiKeySource::bearer()` | el primer valor de `Authorization`, analizado como el extractor bearer de `passport-jwt`: `(\S+)\s+(\S+)`, con el esquema comparado sin distinguir mayúsculas | una estrategia `passport-custom` con `ExtractJwt.fromAuthHeaderAsBearerToken()` |
| `ApiKeySource::query(name)` | el parámetro del query, cuando es una sola cadena | una estrategia `passport-custom` que lee `req.query[name]` |

No hay valor por defecto para las fuentes bearer y query, y el body y las cookies nunca son una fuente (una cookie es una credencial ambiental, y con ella viene CSRF; escribe una [estrategia propia](#escribir-una-estrategia) si necesitas una). Otros lugares no se leen aunque la clave esté allí: una clave en el query se ignora salvo que se configure una fuente query.

Se lee cada fuente configurada. Una fuente cuyo valor está **presente pero es inutilizable** (demasiado largo, con un carácter de control, un parámetro de query repetido, o con corchetes cuando la aplicación usa el parser de query extendido) hace fallar el intento, y **dos fuentes que llevan claves distintas lo hacen fallar también**, sin llamar al validador. Eso cierra el caso en que un proxy o un firewall autentica o limita por un lugar y la aplicación confía en otro. Una fuente que no encuentra nada (sin cabecera, un valor vacío, otro esquema, un valor sin el prefijo) se omite, y la misma clave en dos lugares es una sola clave. Como máximo 8 fuentes.

::: warning Claves en URLs
`ApiKeySource::query("api_key")` pone la clave en la URL, y **lo desaconsejamos**: usa una cabecera. Está desactivado por defecto y la estrategia registra una advertencia al arrancar por cada parámetro de query.

Qué se oculta: al arrancar la aplicación, toda estrategia que lee un parámetro de query registra su nombre, y `LoggingInterceptor` y `Request::redacted_url()` reemplazan el valor de ese parámetro por `REDACTED` en **toda** petición, corra o no esa estrategia para ella (`GET /data?page=2&api_key=REDACTED 200`), incluidos los nombres con percent-encoding y, con el parser de query extendido, `api_key[x]=`. Un query con más de 1000 pares tiene todos los valores ocultos. La forma `Debug` de un `Request` imprime la misma URL redactada. Un nombre de parámetro que contiene `%` se rechaza al arrancar, porque no se puede ocultar de forma fiable.

Qué no: todo lo que está fuera de Corvis (los logs de acceso de tu reverse proxy, balanceador y CDN, el historial del navegador, la cabecera `Referer` que la página envía a otros sitios) y todo lo de dentro que imprima `request.url()`, `request.uri()` o el query en lugar de `redacted_url()` (un middleware, tu propio logging, un reporte de errores). Solo se conocen los nombres que las estrategias declaran: una estrategia que lee el query en su propio código, como `ExtractJwt::custom` llamando a `query_param`, o una estrategia propia que no sobrescribe `credential_query_params()`, no registra nada, así que su parámetro se imprime tal cual. Llama ahí a `request.redact_query_param(nombre)` o sobrescribe el método.
:::

### Límites

| Límite | Por defecto | Notas |
|---|---|---|
| `max_key_bytes` | 512 | De 1 a 4096, de lo contrario el arranque falla. Una clave más larga falla antes de aplicarle un hash, compararla o dársela al validador. El prefijo de una fuente de cabecera no cuenta |
| Caracteres | ningún byte `0x00` a `0x1F` ni `0x7F` | una clave con alguno falla; un tabulador en una cabecera cuenta |
| Fuentes | como máximo 8, ninguna por defecto en `ApiKeyStrategyOptions::new()` | una lista vacía detiene el arranque |
| `validator_timeout` | 5 s | De 1 ms a 300 s. Un validador que tarda más es un `503` |
| Nombres de fuente y prefijos | no vacíos; nombres de cabecera válidos; `Cookie` rechazada; prefijos de 1 a 64 caracteres ASCII imprimibles | cada problema detiene el arranque con un error que nombra la estrategia y la opción |

Los valores repetidos de una cabecera no se copian más allá del límite (una petición con miles de cabeceras `X-Api-Key` se detiene en la primera que lo cruza), y la fuente bearer lee como máximo `max_key_bytes + 128` bytes de espacio inicial, esquema y separador antes de la clave.

### Guardar las claves

Una API key es un secreto bearer de larga vida, así que guarda solo un digest de ella y compara digests:

1. **Genera** al menos 32 bytes aleatorios por clave: `openssl rand -hex 32`, con un id público delante si tienes muchas claves (`ck_live_3f9a.<secret>`). `ExpectedApiKey::new` rechaza un secreto de menos de 32 bytes; una clave que generas tú mismo es responsabilidad tuya, así que haz el generador lo bastante largo.
2. **Guarda** `ApiKey::sha256_hex()` de la clave, no la clave. Un digest de un secreto aleatorio de 32 bytes no se puede convertir de vuelta en la clave, y no necesita un hash lento (argon2 y bcrypt son para contraseñas). Muestra la clave a su dueño una sola vez.
3. **Compara** con los helpers, nunca con `==`:

| Helper | Uso |
|---|---|
| `ExpectedApiKey::new(secret)` | una clave desde la configuración; en memoria solo se conserva su digest |
| `ExpectedApiKey::from_sha256_hex(hex)` | una clave desde un digest guardado |
| `ExpectedApiKey::matches(&key)` | una comparación |
| `ApiKeySet::lookup(&key)` | un puñado de claves con sus dueños; compara cada entrada sin salida anticipada, así que el tiempo depende de cuántas claves hay, no de cuál coincidió |
| `ApiKey::ct_eq(bytes)` y `constant_time_eq(a, b)` | cualquier otro secreto: ambos lados se pasan por SHA-256 y los digests se comparan con `subtle`, de modo que ni la posición del primer byte distinto ni la longitud del secreto guardado se notan en el tiempo |

Para **muchas claves** en una base de datos, busca la clave por un id público y luego compara el digest. Un id desconocido cuesta una búsqueda y una comparación ficticia, así que las claves desconocidas y las incorrectas tardan lo mismo:

```rust
async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
    let Some(id) = key.id('.') else { return Ok(None) }; // `<id>.<secret>`, id de 1 a 64 de A-Za-z0-9_-, público
    let record = self.keys.find(id).await.map_err(|_| ServiceUnavailableException::empty())?;
    let stored = record.as_ref().map(|r| &r.expected); // ExpectedApiKey::from_sha256_hex(&row.digest)
    Ok(ExpectedApiKey::matches_or_dummy(stored, key).then(|| record.map(|r| r.service)).flatten())
}
```

Sin un id, guarda el digest en una columna indexada y busca el digest de la clave presentada: `self.keys.find_by_digest(&key.sha256_hex())`. Esa búsqueda no es de tiempo constante, pero lo que puede filtrar es un prefijo del digest de un secreto aleatorio, que no ayuda a encontrar la clave.

`ApiKey`, `ExpectedApiKey` y `ApiKeySet` muestran `<redacted>` con `{:?}`, así que un `Debug` derivado en un struct que los contenga no puede escribir una clave en un log; no tienen `Display`, `Serialize`, `Clone` ni `PartialEq` (un `==` entre claves es la comparación que filtra). Llama a `key.expose()` solo para pasar la clave a algo que necesite el texto, y nunca la registres, ni la pongas en un mensaje de error, una respuesta o la info de auth.

### Fallos

Las claves ausentes, mal formadas, desconocidas, revocadas y vencidas responden todas el mismo `401 {"message":"Unauthorized","statusCode":401}` a través de `AuthGuard`, sin cabecera `WWW-Authenticate`. Devuelve `Ok(None)` desde el validador para toda clave que no sea válida, sea cual sea el motivo. El motivo se conserva como `request.auth_info()` y nunca se envía: `Missing API Key`, `Conflicting API keys`, `API key too long`, `Invalid API key characters` o `Ambiguous API key` (un parámetro de query repetido o con corchetes). Una clave que el validador rechazó no tiene motivo. Ninguno repite datos de la petición. Un validador que responde `Ok(Some(user))` con un usuario vacío (`false`, `null`, `0`, `""`) también es un fallo, con el motivo `The strategy returned an empty user`: responde `Ok(None)` (ver [el usuario debe ser un valor real](#escribir-una-estrategia)).

#### Fallos del validador

- Una excepción con estado **4xx** se pasa tal cual, como `done(err)` en el original (`ForbiddenException::new("Key suspended")` da el `403`). Úsala solo para una respuesta que quieras que el cliente vea; le dice algo sobre la clave.
- Una excepción con estado **500 o superior** (o inferior a 400) se reemplaza por la excepción simple de ese estado: `{"statusCode":503,"message":"Service Unavailable"}`, `{"statusCode":500,"message":"Internal server error"}`. Un error de base de datos que cita la clave en su texto, o un mensaje que difiere según la clave, nunca llega al cliente. La aplicación registra una advertencia por minuto con el estado, nunca el mensaje.
- Un validador que se ejecuta más que `validator_timeout` es un `503` con ese mismo cuerpo, y un **pánico** en el validador es un `500`: ambos para toda clave, también detrás de `.optional()`. Ninguno es una forma de entrar.

Un fallo de infraestructura (la base de datos está caída) debe ser un error, no `Ok(None)`: un `401` le dice al cliente máquina que su clave es incorrecta, un `503` le dice que reintente.

### Diferencias con passport-headerapikey

Cada una es a propósito y el test diferencial la marca:

- **El nombre es `"api-key"`**, no `headerapikey`, y la clave puede venir de más lugares que una cabecera.
- **Por defecto lee una cabecera y nada más.** El query es una fuente solo cuando se configura, con una advertencia.
- **Un prefijo es una cadena literal.** El original construye con él una expresión regular sin escapar: con el prefijo `Key+`, `Key+<key>` llega a su `verify` como `+<key>`.
- **Claves distintas en dos fuentes, o un valor inutilizable en una, fallan** (el original tiene una sola fuente).
- **Las claves de más de `max_key_bytes` (512) o con caracteres de control fallan** antes del validador (el original no tiene más límite que el tamaño de la cabecera).
- **Una clave vacía, o un prefijo sin nada detrás, nunca llega al validador.**
- **Un fallo del validador con estado 500 o superior tiene un cuerpo fijo, y un validador lento es un `503`.**

`passport-headerapikey` responde a una cabecera ausente con un challenge `BadRequestError('Missing API Key')`; a través del guard original eso es el mismo `401`, que es lo que está grabado.

El `404` de una ruta que no existe es `Cannot GET /x?api_key=...` con la URL tal como se envió, como en el original; ninguna estrategia se ejecuta para él y va solo al remitente.

### Pruebas contra el original

`crates/corvis/tests/fixtures/auth/api_key.mjs` ejecuta `passport-headerapikey` 1.2.2, y estrategias `passport-custom` 1.2.1 para las variantes de query y bearer, detrás del guard original y graba 55 casos (`api_key.json`): nombres de cabecera, prefijos, valores repetidos, claves incorrectas, ausentes y vacías, el validador devolviendo ningún usuario, una excepción y un error simple, cadenas. `crates/corvis/tests/auth_api_key.rs` los reproduce contra las mismas rutas sin necesitar Node al ejecutar los tests; las cinco divergencias marcadas (una clave sobredimensionada dos veces, un carácter de control, claves en conflicto, el prefijo literal) se comprueban a partir de la especificación.

## OAuth2 (Google, GitHub y otros)

La estrategia OAuth2 es el port de `passport-oauth2` 1.8.0 con los perfiles de Google y GitHub de `passport-google-oauth20` 2.0.0 y `passport-github2` 0.1.12, y la petición de refresh de `passport-oauth2-refresh` 2.2.0. Hace solo el flujo de código de autorización y necesita la feature de cargo `auth-oauth2` (trae `auth-session`; el cliente del proveedor usa `tokio-rustls` y las raíces web PKI incluidas, ningún otro crate):

```toml
corvis = { version = "0.1", features = ["auth-oauth2"] }
```

Tú escribes el validador (el callback `verify` del original, que recibe todo lo que entregó el proveedor como un `OAuth2Grant`) y registras `OAuth2Strategy<TuValidador>`. Dos rutas llevan el mismo guard: la ruta de login, que redirige al proveedor, y la ruta de callback, cuyo handler se ejecuta con el usuario que devolvió tu validador.

```rust
use corvis::auth::oauth2::{OAuth2Grant, OAuth2Options, OAuth2Strategy, OAuth2Validator, return_to};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;

#[injectable]
pub struct GoogleAuth {
    users: Arc<UsersService>,
    config: Arc<ConfigService>,
}

#[async_trait]
impl OAuth2Validator for GoogleAuth {
    const NAME: &'static str = "google";
    type User = SessionUser;

    fn options(&self) -> OAuth2Options {
        let get = |name: &str| self.config.get::<String>(name).unwrap_or_default();
        OAuth2Options::google(get("GOOGLE_CLIENT_ID"), get("GOOGLE_CLIENT_SECRET"), "https://app.example/auth/google/callback")
            .scope(["openid", "email", "profile"])
            .state_cookie_secrets([get("OAUTH_STATE_SECRET")])
            .return_to_param("return_to")
            .failure_redirect("/login?error=oauth")
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<SessionUser>, HttpException> {
        let Some(profile) = grant.profile else { return Ok(None) };
        // Identifica al usuario por (provider, id). Vincula una cuenta existente por e-mail solo mediante verified_email().
        let user = self.users.find_or_create(&profile.provider, &profile.id, profile.verified_email()).await?;
        Ok(Some(user))
    }
}

#[module(
    providers = [GoogleAuth, Provider::strategy::<OAuth2Strategy<GoogleAuth>>()],
    controllers = [AuthController],
)]
pub struct AuthModule;

#[controller("auth/google")]
pub struct AuthController;

#[routes]
impl AuthController {
    #[get("login")]
    #[public]
    #[use_guards(AuthGuard::new("google"))] // redirige a Google
    fn login(&self) {}

    #[get("callback")]
    #[public]
    #[use_guards(AuthGuard::new("google"))] // el handler se ejecuta con el usuario que devolvió `validate`
    fn callback(&self, req: Request, user: User<SessionUser>) -> Value {
        json!({ "user": user.0, "return_to": return_to(&req) })
    }
}
```

`validate` devuelve `Ok(Some(user))` para un éxito, `Ok(None)` para un 401 y `Err(exception)` para responder esa excepción, como hace toda estrategia. Unas opciones inválidas detienen `init()` con un error que nombra la estrategia y la opción. Los presets son `OAuth2Options::google(..)` y `OAuth2Options::github(..)`; `OAuth2Options::new(OAuth2Endpoints::new(authorization_url, token_url).userinfo_url(url), ..)` es cualquier otro proveedor (sin una URL de userinfo, el grant no tiene perfil). `OAuth2Options::endpoints(..)` reemplaza las URL de un preset (GitHub Enterprise, un proxy, un doble de pruebas). Registra en el proveedor el redirect URI que configuraste. Google necesita un scope; el `user:email` de GitHub da las direcciones de e-mail verificadas.

### Qué recibe el validador

`OAuth2Grant` tiene `access_token` y `refresh_token: Option<_>` (ambos `OAuth2Token`: sin `Display`, sin `Serialize`, `Debug` no imprime nada; llama a `expose()` a propósito), `params` (la respuesta del token sin los tokens: `get(key)`, `expires_in()`, `scope()`, `id_token()`) y `profile: Option<OAuth2Profile>`. El framework no guarda ningún token. Si conservas uno para llamar al proveedor más tarde, guárdalo cifrado en reposo junto al registro del usuario, nunca en una cookie ni en la sesión del navegador.

`OAuth2Profile` se serializa con las claves del original (`provider`, `id`, `displayName`, `username`, `name`, `emails`, `photos`, `profileUrl`, `_json`) y añade una marca `verified` a cada e-mail (y `primary` para GitHub). **Identifica a un usuario por `(provider, id)`**: el `sub` de Google, el id numérico de GitHub como cadena. **Vincula una cuenta por e-mail solo mediante `verified_email()`**: la dirección principal que el proveedor informa como verificada (Google: `email_verified: true`; GitHub: la entrada de `/user/emails` que es a la vez `primary` y `verified`). Vincular por una dirección sin verificar permite que cualquiera que la registre en el proveedor tome la cuenta.

::: warning El `id_token` no se verifica
Con el scope `openid`, Google devuelve un ID token. `grant.params.id_token()` lo entrega tal como llegó: su firma, emisor, audiencia, vencimiento y nonce **no** se comprueban, así que no leas ningún claim de él. El perfil viene del endpoint de userinfo sobre TLS, autenticado por el token que el servidor obtuvo con su client secret y su verificador PKCE; usa ese.
:::

### `state`, PKCE y dónde viven

Cada login genera un `state` y un verificador PKCE, de 32 bytes cada uno del generador del sistema operativo (43 caracteres base64url), y envía `code_challenge` con `code_challenge_method=S256` (`plain` no se puede expresar). `state` no se puede desactivar. `.pkce(false)` desactiva el challenge para un proveedor que rechaza parámetros desconocidos y registra una advertencia al arrancar.

Tú **eliges dónde vive el estado de un login**; no elegir nada detiene `init()`:

| | `state_cookie_secrets(..)` / `state_cookie(keys)` | `state_in_session()` |
|---|---|---|
| Se guarda en | una cookie firmada con `CookieKeys` (secretos de al menos 32 bytes; usa uno propio, no el de la sesión) | la sesión de la petición |
| Estado en el servidor | ninguno: un visitante que inicia un login no crea nada | un registro de sesión por cada inicio de login (limita la tasa de la ruta de login) |
| Necesita | nada más | `SessionModule` instalado, cookie de sesión que no sea `SameSite=Strict` (de lo contrario, error al arrancar) |
| Un solo uso | la cookie se borra y vence; un cliente que la conserva puede reproducir su propio callback dentro de la vigencia, y el proveedor rechaza entonces el código reutilizado | el registro se elimina antes de cualquier petición al proveedor: una repetición falla sea lo que sea que envíe el cliente |
| Verificador PKCE | se deriva en el servidor a partir del `state` y de la primera clave de la cookie: nunca está en la cookie | se guarda en el registro de sesión |

La cookie de estado es `__Host-oauth2.<name>`, `Path=/`, `HttpOnly`, `Secure`, `SameSite=Lax` (el callback es una navegación de nivel superior entre sitios, así que `Strict` nunca se enviaría), sin `Domain`, y vive tanto como el estado (600 s por defecto). El prefijo `__Host-` hace que un navegador rechace una cookie puesta por un subdominio hermano, lo que cierra el CSRF de login por cookie tossing. `.state_cookie_secure(false)` (desarrollo en HTTP plano) quita `Secure` y el prefijo y registra una advertencia al arrancar. Tras una rotación de claves, un login iniciado antes falla una vez (el verificador se deriva con la primera clave).

Con la sesión como almacén, el mismo ataque necesita que la cookie de sesión la ponga otra persona (un subdominio hermano, o un atacante de red en HTTP plano): asigna a la cookie de sesión el prefijo `__Host-` (`SessionOptions::name("__Host-sid")`) y mantenla `Secure`.

Un callback se comprueba en este orden, y nada llega al proveedor hasta que pasa la última comprobación: `code`, `state` y `error` están cada uno ausente o son una sola cadena dentro de sus límites (`code` y `error` juntos fallan); el flujo de ese `state` se saca del almacén (un state falsificado no gasta nada de los otros flujos del navegador); el flujo debe existir para esta estrategia y estar dentro de su vigencia; su `state` es igual al de la query (en tiempo constante). Un navegador puede tener hasta cuatro logins en curso por estrategia (dos pestañas no se rompen entre sí); el más antiguo se descarta cuando empieza un quinto, y un flujo sin reclamar vence con su vigencia.

### El redirect URI

`redirect_uri` es un argumento obligatorio: una URL absoluta (`https`, o `http` para un host de loopback), sin fragmento ni información de usuario. La misma cadena va en la petición de autorización y en la del token. **No se construye nada a partir de la petición**: el original resuelve una URL de callback relativa contra la cabecera `Host`, así que una cabecera falsificada decide adónde envía el proveedor el código. Corvis rechaza un URI relativo al arrancar y no tiene un redirect URI por petición; una aplicación con varios dominios ejecuta una estrategia por dominio.

### `return_to` y `failure_redirect`

`.return_to_param("return_to")` hace que el **inicio del login** lea ese parámetro de query y lo conserve solo cuando es una sola cadena de a lo sumo 1024 bytes que empieza con una sola `/` (el segundo carácter no es `/` ni `\`), sin barra invertida, carácter de control ni byte no ASCII (la regla de [las URL de redirección de una estrategia](#escribir-una-estrategia); `is_safe_relative_path` es público). Cualquier otra cosa se descarta y el login continúa. El valor viaja en el registro de estado, no en ninguna URL que vea el proveedor, y `oauth2::return_to(&req)` lo devuelve en el handler del callback de un login exitoso (nunca desde la query del propio callback). `.failure_redirect(url)` responde cada callback fallido con un 302 a una URL fija (una ruta relativa bajo la misma regla, o una URL `https` absoluta), sin añadir nada de la petición; los errores del proveedor o de la configuración (500, 503) no se redirigen.

### Fallos

| El callback | Respuesta |
|---|---|
| no es un `GET`; parámetros repetidos, sobredimensionados o mal formados; `code` con `error` | 401 |
| sin registro de estado, registro de otra estrategia, vencido, `state` distinto | 401 |
| `error=access_denied` | 401 |
| cualquier otro `error` | 401 (el original: 500) |
| el endpoint del token responde 400 o 401 (`invalid_grant`, un código reutilizado), o 200 con un `error` | 401 (el original: 500) |
| límite de concurrencia alcanzado | 503 `Service Unavailable` |
| timeout, fallo de conexión o de TLS, redirección, 5xx u otro estado, respuesta sobredimensionada o que no se puede interpretar, sin `access_token`, `token_type` desconocido, fallo de la petición del perfil | 500 `Internal server error` |
| `validate` devuelve `None` / una excepción | 401 / esa excepción |

El texto del proveedor nunca llega a una respuesta: el challenge de un fallo (`Request::auth_info()`, `AuthGuard::on_failure`) es `{"message": <motivo fijo>}` con uno de `state mismatch`, `access denied`, `provider error`, `code rejected`, `invalid callback`, más `"error": <code>` solo cuando el código del proveedor tiene de 1 a 64 caracteres de `[A-Za-z0-9_.-]`. `error_description` y `error_uri` se descartan sin leerse. La respuesta del callback lleva `Cache-Control: no-store` y `Referrer-Policy: no-referrer`; redirige desde el handler para que el `code` salga de la barra de direcciones.

### El cliente del proveedor y sus límites

Las peticiones de token, refresh y perfil pasan por un cliente privado sobre `hyper` y `tokio-rustls`: TLS 1.2 y 1.3 con las raíces web PKI incluidas (más `OAuth2HttpOptions::extra_root_certificate(der)` para una CA privada), nombre de host verificado, **sin forma de desactivar la verificación**. `http` se acepta solo para un host de loopback cuyas direcciones resueltas son todas de loopback (un proveedor simulado en `127.0.0.1`). Una redirección es una petición fallida y nunca se sigue, las variables de proxy se ignoran, las respuestas nunca se descomprimen, y un error nunca contiene texto del proveedor.

| Ajuste | Por defecto | Límites |
|---|---|---|
| `state_ttl` | 600 s | de 60 s a 3600 s |
| PKCE | `S256`, activo | `.pkce(false)` avisa |
| `max_code_bytes` (`code` del callback) | 2048 | de 16 a 8192 |
| `state` del callback | 43 caracteres base64url | fijo |
| timeout de conexión (TCP y TLS) | 5 s | de 1 s a 30 s |
| timeout total por petición | 10 s | de 1 s a 60 s |
| respuesta del token | 64 KiB | de 1 KiB a 1 MiB |
| cada respuesta de perfil | 256 KiB | de 1 KiB a 4 MiB |
| bloque de cabeceras de la respuesta | 16 KiB | fijo |
| un access, refresh o ID token | 16 KiB | fijo |
| peticiones al proveedor en curso, por estrategia | 32 | de 1 a 1024 (por encima: 503 de inmediato) |
| scope | ninguno (Google: obligatorio) | 64 valores de 1 a 128 caracteres ASCII visibles sin espacio, `"` ni `\` (GitHub: tampoco `,`) |
| parámetros de autorización adicionales | ninguno | 16, nombres y valores de a lo sumo 256 bytes; `response_type`, `client_id`, `redirect_uri`, `scope`, `state`, `code_challenge`, `code_challenge_method` rechazados |
| autenticación del cliente | cuerpo de la petición | `ClientAuth::Basic` (id y secret codificados como formulario, nada en el cuerpo) |
| flujos en curso por navegador y estrategia | 4 | fijo |

Una opción fuera de su límite detiene `init()` con un error que nombra la estrategia y la opción y nunca contiene el client secret.

### Refresh

`OAuth2Client::new(options)?` valida las opciones como la estrategia y ofrece `refresh(refresh_token)`: `grant_type=refresh_token` con las credenciales del cliente (y `refresh_scope(..)` si está definido) a través del mismo cliente y de los mismos límites, y devuelve un `OAuth2TokenSet`. Constrúyelo con las mismas `options()` que tu validador. No se guarda nada y nada se refresca por sí solo. Un refresh rechazado es un error cuyo texto contiene solo el código saneado del proveedor (`invalid_grant`).

### Logs, proxies y límites de tasa

El callback registra `code` y `state` como parámetros de query de credenciales, así que `LoggingInterceptor` y `Request::redacted_url()` los ocultan (un middleware que registra `Request::url()` no: usa `redacted_url()`). El log de acceso de un proxy inverso sigue registrando la URL del callback; el `code` es de un solo uso y está ligado al verificador PKCE. Las líneas de log contienen el nombre de la estrategia, el host del proveedor, el estado y el tipo de error, como máximo una vez por minuto por motivo, nunca una query, cuerpo, cabecera, token ni secreto. **Limita la tasa de la ruta de login** (ver [limitación de tasa](/es/security/rate-limiting)): cada inicio de login es una petición que cualquiera puede hacer, y con la sesión como almacén cada uno crea un registro de sesión.

### Diferencias con `passport-oauth2`

- `state` y PKCE siempre están activos; el original no envía ninguno a menos que se configuren `state: true` y `pkce: true` con una sesión.
- El redirect URI es absoluto y está configurado; el original resuelve uno relativo a partir de `Host`.
- Un `error` del proveedor distinto de `access_denied`, y un endpoint del token que rechaza el código, responden 401 (el original 500).
- Las peticiones de perfil envían el token en una cabecera `Authorization: Bearer`; el original envía el de Google en la URL. También envían `Accept: application/json` y el `User-Agent` propio del cliente.
- El scope de GitHub se une con comas, como hace `passport-github2`; los e-mails conservan todas las direcciones de `/user/emails` como `{ value, primary, verified }` (el original: solo la principal como `{ value }`, o todas con `allRawEmails`, añadiendo `visibility`). Una petición de e-mails fallida hace fallar el login, y una lista vacía también.
- Todo e-mail tiene una marca `verified` y existe `verified_email()`; el original omite la marca para GitHub.
- La respuesta del token se entrega sin `access_token`, `refresh_token` ni `id_token` en `params` (son valores tipados propios); el ID token no está verificado.
- Un callback con un state que no es de este navegador responde 401 a través de `AuthGuard` (el original responde el 403 que dio la estrategia, a través de `passport` simple).

### Pruebas contra el original

`crates/corvis/tests/fixtures/auth/oauth2.mjs` ejecuta los cuatro paquetes contra un proveedor falso y graba las redirecciones de autorización, las peticiones de token y de perfil, los perfiles entregados a `validate()`, 14 resultados de callback y la petición de refresh (`oauth2.json`); `crates/corvis/tests/auth_oauth2_original.rs` los reproduce contra un proveedor simulado en `127.0.0.1` sin Node ni red, y comprueba las divergencias marcadas a partir de la especificación. `examples/cats-app` tiene un login con GitHub cuyos endpoints vienen de la configuración, probado contra un proveedor simulado local.

## Pruebas diferenciales

Cada caso de arriba está grabado del original: `crates/corvis/tests/fixtures/auth/strategies.mjs` ejecuta la integración original de passport (12.0.1) sobre `passport` 0.7.0 con estrategias propias de `passport-strategy` 1.0.0 llevadas a cada resultado, y `tests/auth_strategies.rs` reproduce las respuestas grabadas contra las mismas rutas en Corvis, sin necesitar Node al ejecutar los tests. Cada tipo de estrategia que llegue después añade sus propios casos.

## Alternativa: `JwtModule` + guards

La estrategia JWT de arriba es el camino principal. Las piezas que hay debajo siguen disponibles cuando prefieres escribir el guard tú mismo, y cubren el caso más común, el de una API con JWT:

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

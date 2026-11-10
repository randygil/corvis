---
status: partial
---

# Passport (autenticación)

Corvis tiene el **modelo de estrategias** de Passport y de su integración con el framework original: estrategias registradas por nombre, un `AuthGuard` que las ejecuta, el usuario en la request y el `401` que responde el original cuando todas las estrategias fallan. Su comportamiento se comprueba contra la integración original de passport (12.0.1) sobre `passport` 0.7.0 con respuestas grabadas.

Lo que hay ahora es el núcleo: el trait `Strategy` que implementas para tus propias estrategias, `AuthGuard`, `PassportModule` y el parámetro `User<T>`, y la [estrategia JWT](#la-estrategia-jwt) y la [estrategia local](#estrategia-local-usuario-y-contrasena) (usuario y contraseña, con la feature `auth-local`). Las demás estrategias llegan una a una: API keys, sesiones y OAuth2 tienen su propio issue (#45, #44 y #43). Un [guard propio sobre `JwtModule`](#alternativa-jwtmodule-guards) sigue disponible.

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

**Las URL de redirección se envían tal cual.** Construye la URL con datos de confianza (la dirección configurada del proveedor de identidad, una ruta fija). Si el destino de la redirección viene de la petición (un parámetro `return_to` o `next`), valídalo contra una lista de permitidos, o acepta solo una ruta relativa que empiece por una sola `/`, que no vaya seguida de `/` ni de `\` y que no contenga barras invertidas ni caracteres de control (los navegadores tratan `\` como `/`, así que `/\evil.com` se convierte en `//evil.com`), antes de ponerlo en `Redirect`: de lo contrario el login se convierte en una redirección abierta.

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

- **Errores al arrancar.** El original responde `500 Internal server error` en cada petición cuando un guard nombra una estrategia desconocida, y solo registra un error cuando `AuthGuard()` no tiene estrategia por defecto. Corvis detiene `init()` antes de `configure` del middleware y de los hooks de inicio, con un único error que lista todas las rutas (método y ruta) y el nombre que falta. Cubre también un `AuthGuard::chain` vacío, un valor por defecto no registrado y dos llamadas a `PassportModule::register` con valores por defecto distintos. Un guard propio que llama al `Authenticator` con un nombre desconocido sigue recibiendo el `500` del original en tiempo de petición. Un guard que envuelve un `AuthGuard` (un guard compuesto) lo lista en `CanActivate::inner_guards`, que la comprobación recorre a cualquier profundidad, así que un error de escritura en el nombre envuelto también detiene `init()`: `fn inner_guards(&self) -> Vec<&dyn CanActivate> { vec![&self.jwt] }`. No devuelvas el guard interno desde `as_any`: ese método es para el guard mismo, y haría saltar el aviso de gateways de abajo en un wrapper que deja pasar los destinos públicos.
- **Sin `AuthGuard` en gateways de WebSocket.** Un `AuthGuard` global también llega a los mensajes de los gateways por defecto, donde el original tampoco funciona. Deniega el mensaje con `Forbidden resource` sin ejecutar una estrategia, y la aplicación registra un aviso por gateway al arrancar. Autentica los sockets en `handle_connection`, o llama a `apply_global_enhancers_to_gateways(false)`, que quita todos los enhancers globales de los gateways, no solo el `AuthGuard`.
- **El usuario tipado vive en su propio espacio.** `User<T>` refleja solo lo que guardó un `AuthGuard`, nunca otra extensión de la petición de tipo `T`, y `Request::set_user` lo elimina, así que si un guard posterior fija su propio usuario `User<T>` responde `401`. Cuando una estrategia hace pass, o un guard opcional no encuentra credenciales, se eliminan tanto el usuario JSON como el tipado (`req.user` es `undefined` o `null` en el original).
- **Sin login de sesión.** `AuthGuard` nunca inicia sesión por sí solo; las sesiones llegan con su propio issue.
- **Sin `property`, `failureRedirect` ni mensajes flash.** El usuario siempre se guarda para `User<T>` y `request.user()`.

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
use corvis::auth::local::{LocalStrategy, LocalValidator, PasswordHasher};
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
            .map_err(|_| InternalServerErrorException::empty())?;
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

Las credenciales se leen como lo hace `passport-local`: cada campo se busca en el body ya parseado (JSON o `application/x-www-form-urlencoded`) y, si el body no tiene valor, en el query string. Un nombre de campo como `user[email]` es una ruta (`body.user.email`); un segmento de un array es un índice; una cadena que aparece antes del final de la ruta es el valor; un objeto o un array al final no da valor. Un valor ausente, vacío o que no es una cadena cuenta como ausente, y cuando falta cualquiera de las dos credenciales la estrategia falla sin llamar a `validate`. `AuthGuard` responde el mismo `401` para cualquier fallo; el fallo en sí (`{"message":"Missing credentials"}`, estado `400`) está disponible en `.on_failure(..)`. Un nombre de campo vacío detiene la aplicación al arrancar.

No envíes credenciales en el query string salvo que no haya otra opción: las URL acaban en los logs. Se lee el query porque el original lo lee.

### Diferencias con passport-local

- **Las credenciales que no son cadenas cuentan como ausentes.** El original pasa un número o un booleano a `validate()`; Corvis los trata como ausentes, de modo que una búsqueda de usuario nunca recibe un tipo que no espera.
- **Las credenciales tienen un límite.** Un usuario o una contraseña de más de 1024 bytes (configurable) falla sin llamar a `validate`. El original no tiene más límite que el tamaño del body. Sube los límites con `.max_password_bytes(4096)` si tus usuarios lo necesitan; el hasher tiene su propio `max_password_bytes`.
- **Los fallos son uniformes.** Credenciales ausentes, contraseñas incorrectas y usuarios desconocidos dan el mismo `401`, y el validador de arriba gasta una verificación de hash en todos los casos.
- **El límite de intentos de login es aparte.** Combina la ruta de login con el [throttler](/es/security/rate-limiting) (el ejemplo de gatos permite cinco intentos por minuto y cliente); el hashing es caro a propósito, y una avalancha hace cola en los huecos `max_concurrent` del hasher.


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

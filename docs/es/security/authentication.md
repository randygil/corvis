---
status: documented
---

# Autenticación

La autenticación responde *quién hace la petición*. En Corvis se resuelve con **estrategias**: piezas pequeñas que leen una
credencial de una petición (un token bearer, un usuario y una contraseña, una clave de API, una cookie de sesión, la respuesta
de un proveedor OAuth2) y la convierten en un usuario. Un [guard](/es/guards) llamado `AuthGuard` ejecuta las estrategias que
nombres en una ruta, guarda el usuario para el handler y responde el `401` simple cuando ninguna acepta la petición. El modelo
es el de Passport y el de su integración para el framework original, y está comprobado contra ellos: consulta
[Passport](/es/recipes/passport) como referencia y [Migrar desde Passport](/es/recipes/passport-migration) si traes una
configuración existente.

Esta página es la guía: cómo funciona, qué estrategia elegir, un camino corto para cada una, cómo se combinan y qué no se
incluye. Cada opción, valor por defecto y diferencia está en la referencia, y cada amenaza en el
[modelo de amenazas](/es/security/authentication-threat-model); esta página enlaza a ellos en lugar de repetirlos. Lo que debes
hacer tú antes de desplegar está en [Asegurar la autenticación](/es/security/authentication-security).

Los ejemplos vienen de `examples/auth-app`, una aplicación pequeña que trata solo de autenticación y se ejecuta con
`cargo run -p auth-app -- --dev`, sin ninguna cuenta externa. Cada ejemplo empieza con un comentario que nombra su archivo;
`// ...` marca código omitido.

## Cómo funciona

```text
petición ──> ThrottlerGuard ──> AuthGuard::new("jwt") ──> tu handler
             (global, primero)       │
                                     ├─ la estrategia "jwt" lee la credencial  ──> validador ──> usuario
                                     ├─ éxito: el usuario se guarda (User<T>, Request::user())
                                     └─ todas las estrategias fallaron: 401 {"message":"Unauthorized","statusCode":401}
```

- Una **estrategia** lee un tipo de credencial. No la escribes tú: escribes el **validador**, el único método que decide si la
  credencial pertenece a un usuario (el callback `verify` de Passport), y registras la estrategia con
  `Provider::strategy::<JwtStrategy<YourValidator>>()`.
- `AuthGuard::new("jwt")` va en un controlador o en una ruta (o de forma global, ver [más abajo](#proteger-todo-por-defecto)) y
  ejecuta la estrategia con ese nombre.
- El handler lee el usuario con el parámetro `User<T>`.
- Un validador devuelve `Ok(Some(user))`, `Ok(None)` (la credencial se rechaza) o un error. **Di "no hay usuario" con `None`,
  nunca con un valor que se serialice como `null`, `false`, `0` o `""`**: ese usuario nunca se autentica, y el guard registra un
  aviso que indica qué validador corregir ([por qué](/es/recipes/passport#escribir-una-estrategia)).

También puedes [escribir una estrategia](/es/recipes/passport#escribir-una-estrategia) propia para cualquier otra credencial.

## Elegir una estrategia

| Estrategia | Para | La credencial viaja en | Feature de Cargo | Referencia |
|---|---|---|---|---|
| `jwt` | aplicaciones de una sola página, aplicaciones móviles y APIs que envían un token | la cabecera `Authorization: Bearer` | `jwt` (activada por defecto) | [JWT](/es/recipes/passport#la-estrategia-jwt) |
| `local` | el formulario de login: cambia un usuario y una contraseña por un token o una sesión | el cuerpo de la petición de login | `auth-local` | [Local](/es/recipes/passport#estrategia-local-usuario-y-contrasena) |
| `api-key` | clientes máquina: webhooks, servicios internos, socios | una cabecera como `X-Api-Key` | `auth-api-key` | [Claves de API](/es/recipes/passport#api-keys) |
| `session` | navegadores en el mismo sitio que el servidor | una cookie firmada | `auth-session` | [Sesiones](/es/recipes/passport#sesiones) |
| OAuth2 (cualquier proveedor, con ajustes preconfigurados para Google y GitHub) | "Iniciar sesión con ..." | una redirección al proveedor y de vuelta | `auth-oauth2` | [OAuth2](/es/recipes/passport#oauth2-google-github-y-otros) |

Algunas reglas prácticas:

- Una API consumida por código que no controlas usa `jwt`, y `api-key` para los servicios que la llaman.
- Una aplicación de navegador servida desde tu propio dominio usa `session`: el servidor puede terminar una sesión, mientras que
  un token no se puede retirar antes de que caduque. Una cookie es una credencial ambiental, así que necesita el cuidado de
  [CSRF](#cookies-y-csrf) que se explica más abajo.
- `local` y OAuth2 son estrategias de *login*. Comprueban quién es alguien una vez y luego pasan el relevo a un token o una
  sesión; el ejemplo muestra ambos resultados.
- Rara vez usas solo una. [Combinar estrategias](#combinar-estrategias) muestra cómo una ruta acepta varias.

## JWT

No actives nada: la feature `jwt` viene activada por defecto. El validador recibe el payload de un token cuya firma, algoritmo
y caducidad ya fueron verificados:

```rust
// examples/auth-app/src/auth/jwt.rs
#[injectable]
pub struct JwtAuth {
    settings: Arc<Settings>,
}

#[async_trait]
impl JwtValidator for JwtAuth {
    type User = Account;

    fn options(&self) -> JwtStrategyOptions {
        JwtStrategyOptions::new()
            .jwt_from_request(ExtractJwt::from_auth_header_as_bearer_token())
            .secret_or_key(JwtKey::secret(self.settings.jwt_secret.clone()))
            .algorithms([JwtAlgorithm::HS256])
    }

    async fn validate(&self, _req: &Request, payload: Value) -> Result<Option<Account>, HttpException> {
        // ...
        let claims: Option<Claims> = corvis::serde_json::from_value(payload).ok();
        Ok(claims.map(|c| Account { id: c.sub, username: c.username, roles: c.roles }))
    }
}
```

Registra la estrategia junto a su validador y pon el guard en una ruta:

```rust
// examples/auth-app/src/app_module.rs
#[module(
    // ...
    providers = [
        // ...
        JwtAuth,
        Provider::strategy::<JwtStrategy<JwtAuth>>(),
        // ...
    ],
)]
pub struct AppModule;
```

```rust
// examples/auth-app/src/controllers.rs
    #[get("me")]
    #[use_guards(AuthGuard::new("jwt"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }
```

```sh
curl -s http://127.0.0.1:3000/me -H "Authorization: Bearer $TOKEN"
# {"id":1,"username":"john","roles":["admin"]}
curl -s http://127.0.0.1:3000/me
# {"message":"Unauthorized","statusCode":401}
```

Los tokens los firma `JwtService` (el ejemplo les da una vida corta; un token robado funciona hasta que caduca y no hay
revocación). De dónde se lee el token, los algoritmos, los proveedores de claves, las comprobaciones de issuer y audience y cada
diferencia con `passport-jwt` están en [la referencia de JWT](/es/recipes/passport#la-estrategia-jwt). Lo que el propio
`JwtService` no comprueba por defecto se sigue en #174; para autenticar peticiones usa la
estrategia, no `JwtService::verify`.

## Usuario y contraseña

La estrategia `local` lee el usuario y la contraseña del cuerpo de la petición de login. Tu validador los comprueba contra el
hash almacenado y devuelve el usuario. Debe gastar una verificación real exista o no el usuario, para que el tiempo de
respuesta no revele qué usuarios existen, y reemplaza los hashes hechos con un algoritmo anterior después de un login correcto:

```rust
// examples/auth-app/src/auth/local.rs
#[async_trait]
impl LocalValidator for LocalAuth {
    type User = Account;

    async fn validate(&self, _req: &Request, username: &str, password: &str) -> Result<Option<Account>, HttpException> {
        let found = self.users.credentials(username);
        let stored = found.as_ref().map(|(_, hash)| hash.as_str());
        // ...
        let ok = self.hasher.verify_or_dummy(password, stored).await.map_err(PasswordError::into_http_exception)?;
        let Some((account, hash)) = found.filter(|_| ok) else { return Ok(None) };
        if self.hasher.needs_rehash(&hash) {
            let upgraded = self.hasher.hash(password).await.map_err(PasswordError::into_http_exception)?;
            self.users.set_password_hash(account.id, upgraded);
        }
        Ok(Some(account))
    }
}
```

La ruta de login pone la estrategia primero y el [throttler](/es/security/rate-limiting#proteger-una-ruta-de-login) antes de
ella. El handler se ejecuta solo cuando las credenciales eran correctas, y emite el token:

```rust
// examples/auth-app/src/controllers.rs
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(AuthGuard::new("local"))]
    fn login(&self, user: User<Account>) -> HttpResult<Value> {
        Ok(json!({ "access_token": issue_token(&self.jwt, &user)? }))
    }
```

```sh
curl -s -X POST http://127.0.0.1:3000/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"john","password":"john-demo-pass"}'
# {"access_token":"eyJ..."}
```

Una contraseña incorrecta, un usuario desconocido y un campo ausente responden el mismo `401`. Los algoritmos de hash,
`needs_rehash`, qué configurar mientras la mayoría de los hashes guardados siguen siendo bcrypt, y los métodos bloqueantes que
son solo para código fuera del runtime de Tokio están en [Cifrado y hashing](/es/security/encryption-and-hashing); los lugares
de las credenciales, los límites y las diferencias con `passport-local` están en
[la referencia de local](/es/recipes/passport#estrategia-local-usuario-y-contrasena).

## Claves de API

Un cliente máquina envía una clave larga y aleatoria en una cabecera. El validador la compara como un digest, en tiempo
constante, y responde `None` para una clave incorrecta (`then_some`, nunca un `Some(false)`):

```rust
// examples/auth-app/src/auth/api_key.rs
#[async_trait]
impl ApiKeyValidator for ServiceKeyAuth {
    type User = ServiceCaller;

    async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<ServiceCaller>, HttpException> {
        // ...
        Ok(self.keys.expected.matches(key).then_some(ServiceCaller { service: "reports" }))
    }
}
```

El ejemplo la registra como `Provider::strategy::<ApiKeyStrategy<ServiceKeyAuth>>()` y la acepta en una ruta para personas y
máquinas a la vez (una [cadena](#combinar-estrategias)):

```rust
// examples/auth-app/src/controllers.rs
    #[get("reports")]
    #[use_guards(AuthGuard::chain(["jwt", "api-key"]))]
    fn reports(&self, req: Request) -> Value {
        json!({ "caller": req.user() })
    }
```

```sh
curl -s http://127.0.0.1:3000/reports -H "X-Api-Key: $KEY"
# {"caller":{"service":"reports"}}
```

Mantén la clave en una cabecera: una clave en la query string es opcional (opt-in), termina en los logs de acceso y se oculta
del log de peticiones solo para los parámetros que una estrategia declara. Guarda solo un digest de cada clave. De dónde se
pueden leer las claves, los límites y cómo buscar muchas claves están en
[la referencia de claves de API](/es/recipes/passport#api-keys).

## Sesiones

Una sesión mantiene al usuario en el servidor y entrega al navegador una cookie firmada. Son dos piezas: un
`SessionSerializer` que dice qué se guarda del usuario (el id, nunca el usuario completo) y cómo leerlo de vuelta, y una ruta
de login que inicia sesión con `.log_in()`:

```rust
// examples/auth-app/src/auth/session.rs
#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else { return Ok(None) };
        Ok(self.users.find_by_id(id))
    }
}
```

```rust
// examples/auth-app/src/controllers.rs
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[apply_decorators(throttle("default").limit(10), throttle("login").limit(5).tracker("login-account"))]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(HttpStatus::NO_CONTENT)]
    #[use_guards(SameOriginGuard::new(), AuthGuard::new("session"))]
    async fn logout(&self, req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
```

```sh
curl -s -c jar -X POST http://127.0.0.1:3000/session/login -H 'Content-Type: application/json' \
  -d '{"username":"maria","password":"maria-demo-pass"}'
# {"username":"maria"}
curl -s -b jar http://127.0.0.1:3000/session/me
# {"id":2,"username":"maria","roles":["user"]}
```

Iniciar sesión cambia el identificador de sesión, lo que evita la fijación de sesión. Importan dos reglas:

- **Un login hecho por un guard se conserva solo cuando la petición no se rechaza después.** Una ruta con un segundo paso (un
  segundo factor, una página de términos) no debe usar `.log_in()`: autentica con la estrategia y llama a
  `Authenticator::log_in` después del último paso, ver [Sesión](/es/http/session#login-y-logout).
- **El usuario de sesión es visible en rutas que no tienen `AuthGuard`.** Un guard propio que lea `Request::user()` ahí queda
  autenticado solo por la cookie. Ver [Combinar estrategias](#combinar-estrategias) y
  [CSRF](#cookies-y-csrf).

El almacén del ejemplo es la memoria del proceso, que se pierde al reiniciar y no se comparte entre instancias. Las opciones de
la cookie, los secretos y su rotación, el contrato del almacén y los tiempos de espera están en [Sesión](/es/http/session).

## OAuth2

"Iniciar sesión con un proveedor" es el flujo de código de autorización, con `state` y PKCE siempre activos. Dos rutas llevan el
mismo guard: la que redirige al proveedor y el callback. El validador recibe lo que entregó el proveedor y devuelve el usuario:

```rust
// examples/auth-app/src/auth/provider.rs
#[async_trait]
impl OAuth2Validator for ProviderAuth {
    const NAME: &'static str = "provider";
    type User = Account;

    fn options(&self) -> OAuth2Options {
        let client = &self.settings.oauth;
        let endpoints = OAuth2Endpoints::new(&client.authorize_url, &client.token_url).userinfo_url(&client.userinfo_url);
        OAuth2Options::new(endpoints, &client.client_id, &client.client_secret, &client.redirect_uri)
            .scope(["openid", "email", "profile"])
            // ...
            .state_cookie_secrets([self.settings.oauth_state_secret.clone()])
            .state_cookie_secure(!self.settings.dev)
    }

    async fn validate(&self, _req: &Request, grant: OAuth2Grant) -> Result<Option<Account>, HttpException> {
        let Some(profile) = grant.profile else { return Ok(None) };
        // ...
        let name = profile.display_name.as_deref().unwrap_or("user");
        Ok(Some(self.users.for_provider_user(&profile.provider, &profile.id, profile.verified_email(), name)))
    }
}
```

```rust
// examples/auth-app/src/controllers.rs
    #[get]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn start(&self) -> Value {
        json!({})
    }

    #[get("callback")]
    #[apply_decorators(throttle("default").limit(10))]
    #[use_guards(AuthGuard::new("provider").log_in())]
    fn callback(&self, user: User<Account>) -> Value {
        json!({ "username": user.username })
    }
```

Las reglas que sigue este código, y que debes seguir en el tuyo:

- **Identifica a un usuario por `(provider, id)`, nunca solo por correo electrónico.** Una dirección es una afirmación, y una
  que cambia de manos. Vincula un usuario del proveedor a una cuenta que ya existe solo mediante `verified_email()`, que
  contiene la dirección únicamente cuando el proveedor dice que la verificó. El ejemplo hace exactamente eso en
  `src/users.rs`, y una prueba fija ambos casos.
- **Ejecuta los mismos guards en la ruta que inicia un flujo y en el callback.** Un flujo se completa solo bajo la sesión, y
  para el usuario, que lo inició. Esto es lo que impide que un login (o un enlace de cuenta) iniciado por un usuario se complete
  como otro en un navegador compartido.
- Guarda el estado de un login en una cookie firmada con un secreto propio, ponle un límite de tasa a ambas rutas y mantén las
  estrategias OAuth2 fuera de las cadenas (inician un login para cualquier petición que no sea un callback).
- El `id_token` que devuelven algunos proveedores se entrega sin verificar: no leas ningún claim de él.

El ejemplo no necesita ninguna cuenta externa: `--dev` inicia un proveedor simulado en `127.0.0.1`, y al apuntar los ajustes
`OAUTH_*` a uno real se ejecuta el mismo código (ver su README). La mecánica de `state` y PKCE, las reglas del redirect URI,
`return_to`, lo que responde cada fallo y los límites están en
[la referencia de OAuth2](/es/recipes/passport#oauth2-google-github-y-otros).

## Combinar estrategias

**Una cadena** prueba las estrategias en orden: la primera que tiene éxito decide el usuario, un fallo pasa a la siguiente, y un
error (una base de datos caída, un proveedor que falló) detiene la cadena y es la respuesta, de modo que una caída nunca cae
en una estrategia más débil.

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

**Una cookie de sesión y un token en la misma petición.** Deciden el usuario las estrategias que lista el `AuthGuard` de la
ruta: `AuthGuard::new("jwt")` responde `401` a una cookie por sí sola, `AuthGuard::chain(["jwt", "session"])` acepta cualquiera
de las dos (el token primero), y un `AuthGuard` que tiene éxito reemplaza al usuario que la sesión había restaurado. Indica en
cada ruta qué credenciales acepta.

**La autenticación opcional** personaliza una ruta que cualquiera puede llamar. Un guard opcional deja pasar la petición cuando
todas las estrategias *fallaron*, y un error aún la termina. No es control de acceso, así que la ruta se marca también
`#[public]`:

```rust
// examples/auth-app/src/controllers.rs
    #[get("feed")]
    #[public]
    #[use_guards(AuthGuard::chain(["jwt", "session"]).optional())]
    fn feed(&self, req: Request) -> Value {
        match req.user_as::<Account>() {
            Some(user) => json!({ "message": format!("Hello, {}", user.username) }),
            None => json!({ "message": "Hello, stranger" }),
        }
    }
```

**Una estrategia por defecto.** `PassportModule::register(PassportModuleOptions::new().default_strategy("jwt"))` permite que
una ruta escriba `AuthGuard::default()`. Una aplicación con una estrategia principal la usa; una con varias las nombra en cada
ruta.

**Orden de los guards.** Los guards se ejecutan primero los globales, luego los del controlador y después los de la ruta, cada
uno de izquierda a derecha. De ahí se siguen dos reglas:

- Un `AuthGuard` va **antes** de cualquier guard que lea el usuario. El guard de roles del ejemplo se lista después de la
  cadena, y [Autorización](/es/security/authorization) muestra por qué: en una ruta sin `AuthGuard`, `Request::user()` es lo que
  el middleware de sesión restauró desde la cookie, algo que nada de la petición ha comprobado.
- El throttler va **antes** del `AuthGuard`, para que una petición que excede el límite no cueste un hash de contraseña.
  Registra `ThrottlerGuard` con `Provider::app_guard` y el arranque avisa cuando una ruta lo ejecuta después de un
  `AuthGuard`. Ver [Límite de tasa](/es/security/rate-limiting#proteger-una-ruta-de-login).

**¿Token o sesión para las rutas de login?** La misma estrategia `local` sirve para ambos. Una ruta de login que emite un
token lo devuelve en el cuerpo y no inicia ninguna sesión; una que abre una sesión usa `.log_in()` y responde con una cookie.
Una aplicación de una sola página que además necesita la revocación de la cookie usa la sesión; un cliente móvil usa el token.

## Proteger todo por defecto

La mayoría de las aplicaciones quieren todas las rutas autenticadas salvo unas pocas. Registra un `AuthGuard` global creado con
`.skip_public()` y marca las rutas abiertas con `#[public]`:

```rust
app.use_global_guards(AuthGuard::new("jwt").skip_public());
```

```rust
#[get]
#[public]
fn find_all(&self) -> Vec<Cat> { /* ... */ }
```

`.skip_public()` es lo que hace que `#[public]` signifique algo para el guard: sin él, las rutas públicas se autentican como
cualquier otra. Un guard global confía en cada marca `#[public]`, así que revísalas. Para que la aplicación se niegue a
arrancar cuando una ruta no tiene guard ni la marca, activa la [postura de rutas](/es/security/route-posture).
`examples/auth-app` no tiene guard global y aplica la postura en su lugar, de modo que cada estrategia se muestra en su propia
ruta; `examples/cats-app` muestra el patrón global.

`AuthGuard` deniega los mensajes de WebSocket (ver [más abajo](#websockets)), así que un guard global no es la forma de
proteger un gateway.

## Leer el usuario

- `User<T>` en los parámetros del handler es el valor tipado que guardó el guard. En una ruta donde no hay ninguno (sin guard, o
  con un guard opcional que no encontró credenciales) responde el mismo `401`.
- `Request::user()` es la forma JSON, y `Request::user_as::<T>()` la deserializa. Esto es lo que lee un guard propio.
- `Request::auth_info()` es el motivo de un intento fallido o la información de un éxito. Nunca se la devuelvas al cliente.

## Autorización

La autenticación dice quién; lo que el usuario puede hacer es [Autorización](/es/security/authorization): un guard listado
después del `AuthGuard` que lee el usuario, como hace `RolesGuard` en el ejemplo.

## Cookies y CSRF

Una cookie de sesión, o un token que guardas en una cookie, lo envía el navegador por sí solo. Eso es una credencial
ambiental, y trae dos cosas por hacer:

- **Rechaza las peticiones que cambian estado y vienen de otro sitio.** `SameSite=Lax` no detiene el CSRF de *login*: un `POST`
  de formulario entre sitios a tu ruta de login recibe respuesta, y el navegador guarda la cookie que inicia la sesión de la
  víctima como el atacante. El ejemplo pone un `SameOriginGuard` (en `src/same_origin.rs`) antes de que se comprueben las
  credenciales; su prueba envía la petición entre sitios. Ver [Protección CSRF](/es/security/csrf).
- **Prefija las cookies con `__Host-`.** El prefijo hace que un navegador rechace una cookie fijada desde un subdominio hermano
  (cookie tossing). La cookie de sesión, la cookie de estado de OAuth2 y una cookie que contenga un JWT deberían tenerlo, y una
  petición que lleve el mismo nombre de cookie dos veces se trata como si no tuviera credencial. Solo `--dev`, que se ejecuta
  sobre HTTP plano, lo omite.

## WebSockets

`AuthGuard` deniega todos los mensajes de WebSocket con `Forbidden resource` y no ejecuta ninguna estrategia: un `AuthGuard`
global bloquea por tanto un gateway público, y la aplicación avisa una vez por gateway al arrancar. Tampoco se ejecutan
estrategias en el handshake de WebSocket (#173): un navegador no puede enviar cabeceras al abrir un socket, así que el gateway
autentica en `handle_connection`, como hace `examples/tertulia`, y cierra la conexión cuando el token no es válido:

```rust
// examples/tertulia/src/chat/chat_gateway.rs
    async fn handle_connection(&self, client: WsClient) {
        let handshake = client.handshake();
        let token = handshake.query_param("token").map(str::to_string).or_else(|| extract_bearer_token(handshake));
        let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
            // ...
            client.disconnect_with(CLOSE_UNAUTHORIZED, "Unauthorized");
            return;
        };
        client.insert(user.clone());
        // ...
    }
```

Mientras las estrategias no se ejecuten en el handshake, `JwtService::verify` es más permisivo que la estrategia JWT (#174):
aplica las reglas de la estrategia (una caducidad, una longitud mínima del secreto) en el handler. Un token en la query string
llega a los logs de tu proxy, así que prefiere una cabecera o un primer mensaje. En Socket.IO, un middleware de conexión en el
namespace principal no cubre los demás namespaces (#178): regístralo en cada namespace que tenga handlers.
La página de [guards de WebSockets](/es/websockets/guards) cubre los guards en los mensajes.

## Sin estrategias

Las estrategias son el camino principal, pero las piezas que hay debajo siguen disponibles: `JwtModule`, `JwtService::sign` y
`verify`, `extract_bearer_token` y un `CanActivate` propio que deje el usuario en la petición. Ese camino se describe en
[Passport: la alternativa](/es/recipes/passport#alternativa-jwtmodule-guards). Prefiere una estrategia: te da gratis las
comprobaciones de arranque, el `401` uniforme, los plazos, las cadenas y los logs censurados.

## Qué no se incluye

- **OpenID Connect.** La estrategia OAuth2 hace solo el flujo de código de autorización, y el ID token se entrega sin verificar.
- **Tokens CSRF.** Nada genera ni comprueba tokens sincronizados; ver [Protección CSRF](/es/security/csrf).
- **Autenticación multifactor, bloqueo de cuentas, políticas de contraseñas y comprobación de contraseñas filtradas.** Son
  política de la aplicación. Un segundo factor es un segundo paso que llama a `Authenticator::log_in` al final; el throttler
  puede limitar los intentos por cuenta pero no bloquea nada.
- **Refresh tokens.** No hay almacén: firma un segundo token con una vida más larga y un endpoint que lo canjee. `OAuth2Client`
  puede refrescar el token de un proveedor, y nada se refresca por sí solo.
- **Un almacén de sesiones distinto de la memoria.** Los almacenes de Redis y de ORM son trabajo posterior (#170,
  #171); mientras tanto escribe el contrato del almacén (ver [Sesión](/es/http/session#escribir-un-almacen)).
- **Una lista de redirect URI permitidos** para OAuth2 (#172).
- **Estrategias en el handshake de WebSocket** (#173).
- **Una caché de respuestas que sea segura para rutas autenticadas.** No registres `CacheInterceptor` para toda la aplicación
  cuando tiene rutas `GET` autenticadas: usa la URL como clave, así que reproduce la respuesta de un usuario a otro
  (#177, ver [Caché](/es/data/caching)).

Siguiente: [Asegurar la autenticación](/es/security/authentication-security) antes de desplegar, el
[modelo de amenazas](/es/security/authentication-threat-model) si evalúas el módulo, y
[Migrar desde Passport](/es/recipes/passport-migration) si traes una configuración existente.

---
status: documented
---

# Autenticación

Corvis no impone una estrategia de autenticación, pero trae las piezas para la más habitual en APIs:
tokens JWT firmados por el servidor y verificados por un [guard](/es/guards). Esta página arma ese flujo
completo con `JwtModule`, `JwtService` y un `CanActivate` propio, siguiendo el ejemplo
`examples/cats-app`.

El flujo es el siguiente:

1. El cliente envía sus credenciales a `POST /auth/login`.
2. Un servicio comprueba las credenciales y firma un JWT con `JwtService::sign`.
3. En cada petición posterior el cliente envía `Authorization: Bearer <token>`.
4. Un guard extrae el token, lo verifica y deja los *claims* en la request para el handler.

::: tip Requisito
`JwtModule` y `JwtService` están detrás de la feature `jwt` del crate `corvis`, que viene activada
por defecto.
:::

## Registrar `JwtModule`

`JwtModule::register` recibe un `JwtModuleOptions` y devuelve un módulo dinámico que exporta
`JwtService`. Lo normal es leer el secreto de la configuración con `register_async`, que recibe
primero si el módulo es global y después una fábrica asíncrona con acceso al contenedor:

```rust
use corvis::prelude::*;
use corvis::{JwtModule, JwtModuleOptions};

#[module(
    imports = [
        UsersModule,
        JwtModule::register_async(true, |ctx| async move {
            let config = ctx.get::<ConfigService>().await?;
            let secret = config
                .get::<String>("JWT_SECRET")
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
            Ok(JwtModuleOptions::new(secret).expires_in(config.get_or("JWT_EXPIRES_IN", "1h".to_string())))
        }),
    ],
    controllers = [AuthController],
    providers = [AuthService],
    exports = [AuthService],
)]
pub struct AuthModule;
```

Con el primer argumento en `true`, `JwtService` queda visible en toda la aplicación sin importar el
módulo en cada sitio (ver [Módulos](/es/modules)). Si la fábrica devuelve un error, la aplicación no
arranca: es preferible a firmar tokens con un secreto conocido.

Opciones de `JwtModuleOptions`:

| Método | Efecto |
|---|---|
| `JwtModuleOptions::new(secret)` | Secreto HMAC (`HS256` por defecto) |
| `.expires_in("15m")` | Añade `exp`. Acepta la gramática del paquete `ms` (`"60s"`, `"1.5h"`, `"2 days"`); un número sin unidad son segundos. Un valor que no se entiende hace fallar `sign` |
| `.algorithm(JwtAlgorithm::RS256)` | Algoritmo (`corvis::jwt::JwtAlgorithm`, reexportado de `jsonwebtoken`) |
| `.private_key(pem)` / `.public_key(pem)` | Claves PEM para `RS*`, `PS*`, `ES*` y `EdDSA`. Usa `JwtModuleOptions::new("")`: un secreto no vacío tiene prioridad sobre las claves |
| `.issuer("api")` / `.audience("web")` | Añade `iss`/`aud` al firmar y los exige al verificar |
| `.global(true)` | Solo para `register`: registra el módulo como global |

## Firmar el token

`JwtService` se inyecta como cualquier provider. `sign` serializa el payload, añade `iat` (y `exp`
si configuraste `expires_in`) y devuelve el token:

```rust
use corvis::JwtService;
use corvis::prelude::*;

#[injectable]
pub struct AuthService {
    users_service: Arc<UsersService>,
    jwt_service: Arc<JwtService>,
}

impl AuthService {
    pub async fn sign_in(&self, username: &str, pass: &str) -> HttpResult<Value> {
        let user = self.users_service.find_one(username).await;
        let Some(user) = user.filter(|u| u.password == pass) else {
            return Err(UnauthorizedException.into());
        };
        let payload = JwtPayload { sub: user.user_id, username: user.username, roles: user.roles };
        let token = self.jwt_service.sign(&payload).map_err(HttpException::from_anyhow)?;
        Ok(json!({ "access_token": token }))
    }
}
```

::: warning Contraseñas
El ejemplo compara contraseñas en texto plano para no distraer del flujo. En una aplicación real
guarda un hash y compáralo con una función de derivación lenta; ver
[Cifrado y hashing](/es/security/encryption-and-hashing).
:::

Para opciones por llamada existe `sign_with(&payload, JwtSignOptions { .. })`, con `expires_in`,
`secret`, `subject` y `private_key`. También están `sign_async` y `verify_async`, que hacen lo mismo
que sus versiones síncronas.

## El endpoint de login

```rust
#[controller("auth")]
pub struct AuthController {
    auth_service: Arc<AuthService>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[http_code(HttpStatus::OK)]
    #[public]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        self.auth_service.sign_in(&dto.username, &dto.password).await
    }

    #[get("profile")]
    fn get_profile(&self, user: CurrentUser) -> JwtPayload {
        user.0
    }
}
```

`#[http_code(HttpStatus::OK)]` cambia el `201` que un `POST` devuelve por defecto. El parámetro
`user: CurrentUser` no lleva atributo: se resuelve con `FromContext` (ver más abajo).

## El guard de autenticación

El guard lee la cabecera con `extract_bearer_token`, verifica el token y adjunta el payload a la
request:

```rust
use corvis::prelude::*;
use corvis::{JwtService, extract_bearer_token};

#[injectable]
pub struct AuthGuard {
    jwt_service: Arc<JwtService>,
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let is_public: bool = self
            .reflector
            .get_all_and_override(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()])
            .unwrap_or(false);
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

- `verify::<T>(token)` comprueba la firma (solo con el algoritmo configurado), `nbf`, `exp`
  (caducado cuando `ahora >= exp`), `iss`/`aud` si los configuraste, y deserializa los *claims* en
  `T`. Devuelve un `corvis::Result<T>`.
- `sign` y `verify` fallan con `secretOrPrivateKey must have a value` si no hay clave. `verify` también rechaza un token que aún no es válido (`jwt not active`, por `nbf`) y uno cuyo `exp`, `nbf` o `iat` no sea un número.
- `verify_or_unauthorized::<T>(token)` hace lo mismo y convierte cualquier fallo en un `401`.
- `decode(token)` devuelve el payload como `Value` **sin verificar la firma**. No lo uses para
  decidir permisos.

Hay dos formas de guardar el usuario en la request:

- `request.set_user(&payload)` lo serializa a JSON; se lee con `request.user_as::<JwtPayload>()` o
  `request.user()`.
- `request.insert(valor)` guarda un valor tipado (debe ser `Clone`); se lee con
  `request.get::<T>()`. Es lo que hace `examples/tertulia`.

## Activar la autenticación en todas las rutas

Registrado como provider con `Provider::app_guard`, el guard se aplica a todas las rutas de los
controladores y puede recibir dependencias por inyección:

```rust
#[module(
    imports = [AuthModule, UsersModule, CatsModule],
    providers = [Provider::app_guard::<AuthGuard>()],
)]
pub struct AppModule;
```

Las rutas abiertas se marcan con `#[public]`, que guarda la metadata `"isPublic"` (`PUBLIC_METADATA`) que el guard
consulta mediante `Reflector`:

```rust
#[get]
#[public]
fn find_all(&self) -> Vec<Cat> { /* ... */ }
```

`#[public]` también puede ir debajo de `#[controller]` para abrir un controlador completo. Para que la aplicación se
niegue a arrancar cuando una ruta no tiene guard ni `#[public]`, consulta [Postura de rutas](/es/security/route-posture).

::: warning Alcance de los guards globales
`Provider::app_guard` y `app.use_global_guards` protegen las rutas de los controladores y se ejecutan en los
mensajes de los gateways de WebSocket. No se aplican a los listeners `upgrade` de `corvis::ws`, a los archivos
estáticos ni a las rutas de Swagger/Scalar. Protege esas superficies por separado; [Postura de rutas](/es/security/route-posture#lo-que-no-se-comprueba)
las lista.
:::

## Leer el usuario en el handler

Un tipo propio que implementa `FromContext` funciona como un decorador de parámetro: basta con
declararlo como argumento del handler, sin atributo.

```rust
pub struct CurrentUser(pub JwtPayload);

#[async_trait]
impl FromContext for CurrentUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ctx.request()
            .user_as::<JwtPayload>()
            .map(CurrentUser)
            .ok_or_else(|| UnauthorizedException.into())
    }
}
```

Ver [Decoradores personalizados](/es/custom-decorators) para más detalles.

## Autenticación en WebSockets

Un navegador no puede enviar cabeceras al abrir un WebSocket, así que el token suele viajar en la
query. `examples/tertulia` lo verifica en `handle_connection` y cierra la conexión si no es válido:

```rust
async fn handle_connection(&self, client: WsClient) {
    let handshake = client.handshake();
    let token = handshake.query_param("token").map(str::to_string).or_else(|| extract_bearer_token(handshake));
    let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
        client.disconnect_with(4401, "Unauthorized");
        return;
    };
    client.insert(user);
}
```

El mismo `AuthGuard` de tertulia sirve para HTTP y para WebSocket: consulta `ctx.get_type()` y, en
`ContextType::Ws`, lee el usuario guardado en el cliente. Ver [Guards de WebSockets](/es/websockets/guards).

## Lo que no incluye Corvis

- No hay estrategias tipo Passport (local, OAuth, OpenID Connect) ni sesiones de servidor. Ver
  [Passport](/es/recipes/passport) y [Sesión](/es/http/session).
- No hay *refresh tokens* integrados: se implementan firmando un segundo token con otra vigencia
  (`sign_with` con `JwtSignOptions { expires_in: Some("7d".into()), .. }`) y un endpoint que lo canjea.

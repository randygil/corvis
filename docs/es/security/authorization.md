---
status: documented
---

# Autorización

La autenticación responde *quién* hace la petición; la autorización decide *qué* puede hacer. En
Corvis la herramienta para lo segundo es la misma que para lo primero: un [guard](/es/guards) que lee
metadata de la ruta con `Reflector` y compara con el usuario que
[`corvis::auth::AuthGuard`](/es/security/authentication) dejó en la petición. Los ejemplos de esta página vienen de
`examples/auth-app`.

## Control de acceso basado en roles

Define los roles y una clave de metadata. Como la metadata se guarda como JSON, cualquier tipo
`Serialize + Deserialize` sirve como valor. El usuario es lo que devuelven tus validadores; en el ejemplo, cada estrategia
devuelve un `Account` que lleva sus roles:

```rust
// examples/auth-app/src/users.rs
pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

// ...
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: u32,
    pub username: String,
    pub roles: Vec<Role>,
}
```

Marca las rutas con `#[set_metadata]`:

```rust
#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
#[set_metadata(ROLES_KEY, [Role::Admin])]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

Y escribe el guard:

```rust
// examples/auth-app/src/auth/roles.rs
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().typed_user::<Account>();
        Ok(user.is_some_and(|u| required.iter().any(|r| u.roles.contains(r))))
    }

    // ...
    fn controls_access(&self) -> bool {
        false
    }
}
```

`get_all_and_override` busca la clave primero en el handler y después en la clase, y devuelve el
primer valor que encuentra; así un método puede sobrescribir lo que declara su controlador. Si la
ruta no declara roles, el guard deja pasar, y por eso devuelve `false` en
`controls_access`: no es lo que mantiene fuera las peticiones anónimas, así que la [postura de rutas](/es/security/route-posture) no
debe contarlo como control de acceso. Un guard escrito para HTTP lee el usuario tipado con `Request::typed_user::<T>()` (sin JSON que deserializar); un handler lo
lee con el parámetro `User<T>`.

::: warning Un guard de roles necesita un `AuthGuard` antes
`Request::user()` no siempre es lo que un guard estableció. El middleware de sesión restaura el usuario desde la cookie en
**todas** las rutas, incluidas las rutas sin `AuthGuard` y las `#[public]`, antes de que se ejecute cualquier guard. Un guard de
roles que lee el usuario en una ruta que no tiene un `AuthGuard` antes queda por tanto autenticado solo por la cookie, sin que
se haya comprobado nada de la petición misma (un `POST` de formulario entre sitios lleva la cookie). Lista siempre un
`AuthGuard` que nombre las credenciales que la ruta acepta antes del guard de roles, como hacen los ejemplos siguientes. Ver
[el usuario de sesión](/es/security/authentication-threat-model#el-usuario-de-sesion-en-rutas-sin-authguard) y
[Protección CSRF](/es/security/csrf).
:::

Cuando un guard devuelve `Ok(false)`, el cliente recibe:

```json
{ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }
```

Si quieres otro estado o mensaje, devuelve un error en lugar de `false`, por ejemplo
`Err(ForbiddenException::new("Admins only"))`.

## Registro y orden

El guard de roles necesita al usuario que adjunta el guard de autenticación, así que debe ejecutarse
después. Aplica ambos en la ruta o en el controlador; en una lista se ejecutan de izquierda a derecha. El `AuthGuard`
viene de `corvis::auth` y nombra las estrategias que la ruta acepta:

```rust
// examples/auth-app/src/controllers.rs
    #[get("admin/users")]
    #[set_metadata(ROLES_KEY, [Role::Admin])]
    #[use_guards(AuthGuard::chain(["jwt", "session"]), RolesGuard)]
    fn admin_users(&self) -> Vec<Account> {
        self.users.accounts()
    }
```

Una petición anónima recibe `401` del `AuthGuard`, y un usuario autenticado sin el rol recibe `403` del guard de roles.

Cuida el orden cuando registres guards de forma global. Los guards registrados con `Provider::app_guard` se ejecutan antes
que los que se pasan a `app.use_global_guards(..)`, sin importar el orden de las llamadas, así que un guard de roles registrado
como `Provider::app_guard::<RolesGuard>()` se ejecuta *antes* que un `AuthGuard::new("jwt")` global entregado a
`use_global_guards` y no vería ningún usuario. O aplicas el guard de roles en los controladores y rutas que lo necesitan (como
arriba), o registras el guard de autenticación también a través del contenedor (un tipo que envuelve el `AuthGuard` y se
registra con `Provider::app_guard`, listado antes del guard de roles, como hace `examples/cats-app`).

El orden completo entre guards globales, de controlador y de método se describe en
[Ciclo de vida de la petición](/es/faq/request-lifecycle).

## Agrupar la metadata en un decorador

Para no repetir `#[set_metadata(ROLES_KEY, ...)]`, agrupa la metadata en un `CustomDecorator` y
aplícalo con `#[apply_decorators]`:

```rust
use corvis::CustomDecorator;

fn admin_only() -> CustomDecorator {
    CustomDecorator::new().set_metadata(ROLES_KEY, [Role::Admin])
}

#[delete(":id")]
#[apply_decorators(admin_only())]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

Un `CustomDecorator` solo transporta metadata: no puede registrar guards ni interceptors. El guard se
sigue aplicando con `#[use_guards]` o de forma global.

## Reglas por recurso

Cuando la decisión depende del recurso (por ejemplo, "solo el autor puede editar su post"), el guard
no siempre tiene la información necesaria. Hay dos enfoques:

- Leer el parámetro de ruta en el guard (`ctx.request().param("id")`) y consultar un servicio
  inyectado en el propio guard.
- Hacer la comprobación en el servicio y devolver `ForbiddenException` desde allí.

```rust
pub async fn update(&self, id: u32, user: &JwtPayload, dto: UpdatePostDto) -> HttpResult<Post> {
    let post = self.find_one(id).await?;
    if post.author_id != user.sub && !user.roles.contains(&Role::Admin) {
        return Err(ForbiddenException::new("You can only edit your own posts"));
    }
    // ...
}
```

## Autorización en WebSockets

Los guards funcionan también en los handlers de un gateway. `RolesGuard` de `examples/tertulia`
obtiene el usuario de la request HTTP o del cliente WebSocket según `ctx.get_type()`, de modo que
la misma regla protege un endpoint REST y un mensaje. (`corvis::auth::AuthGuard` deniega los mensajes de WebSocket, así que un
gateway autentica en `handle_connection` y un guard propio lee el usuario guardado en el cliente; ver
[Autenticación](/es/security/authentication#websockets).)

```rust
#[subscribe_message("clear")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(RolesGuard, RoomMemberGuard)]
async fn clear(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
    // ...
}
```

Los guards globales también corren en los mensajes de los gateways; marca los gateways públicos con metadatos o comprueba `ctx.get_type()`. Ver
[Guards de WebSockets](/es/websockets/guards).

## Lo que no incluye Corvis

No hay un módulo de políticas o permisos (tipo CASL) ni un lenguaje de reglas. Los guards con
`Reflector` cubren los casos de roles y *claims*; para reglas más complejas, encapsúlalas en un
servicio inyectable y llámalo desde el guard.

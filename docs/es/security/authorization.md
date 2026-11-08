---
status: documented
---

# Autorización

La autenticación responde *quién* hace la petición; la autorización decide *qué* puede hacer. En
Corvis la herramienta para lo segundo es la misma que para lo primero: un [guard](/es/guards) que lee
metadata de la ruta con `Reflector` y compara con los datos del usuario que dejó el guard de
autenticación.

## Control de acceso basado en roles

Define los roles y una clave de metadata. Como la metadata se guarda como JSON, cualquier tipo
`Serialize + Deserialize` sirve como valor:

```rust
use serde::{Deserialize, Serialize};

pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtPayload {
    pub sub: u32,
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
#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let required: Option<Vec<Role>> =
            self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(required) = required else { return Ok(true) };
        let user = ctx.switch_to_http().get_request().user_as::<JwtPayload>();
        Ok(user.map(|u| required.iter().any(|r| u.roles.contains(r))).unwrap_or(false))
    }
}
```

`get_all_and_override` busca la clave primero en el handler y después en la clase, y devuelve el
primer valor que encuentra; así un método puede sobrescribir lo que declara su controlador. Si la
ruta no declara roles, el guard deja pasar.

Cuando un guard devuelve `Ok(false)`, el cliente recibe:

```json
{ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }
```

Si quieres otro estado o mensaje, devuelve un error en lugar de `false`, por ejemplo
`Err(ForbiddenException::new("Admins only"))`.

## Registro y orden

El guard de roles necesita al usuario que adjunta el guard de autenticación, así que debe ejecutarse
después. Los guards registrados con `Provider::app_guard` se ejecutan en el orden en que aparecen en
los módulos:

```rust
#[module(
    imports = [AuthModule, UsersModule, CatsModule],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
        Provider::app_guard::<RolesGuard>(),
    ],
)]
pub struct AppModule;
```

También puedes aplicarlos solo donde hagan falta, en el controlador o en el método. En una lista, se
ejecutan de izquierda a derecha:

```rust
#[post(":room/announcements")]
#[set_metadata(ROLES_KEY, [MODERATOR])]
#[use_guards(AuthGuard, RolesGuard)]
async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Value> {
    // ...
}
```

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
la misma regla protege un endpoint REST y un mensaje:

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

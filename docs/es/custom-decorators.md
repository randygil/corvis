---
status: partial
---

# Decoradores personalizados

Rust no tiene decoradores; Corvis usa atributos de procedimiento. No puedes crear un atributo nuevo sin escribir tu propio crate de macros, pero sí puedes cubrir los usos habituales de los decoradores propios con tres herramientas:

| Necesidad | Herramienta |
|---|---|
| Un parámetro calculado a partir de la petición (`@User()`) | Implementar `FromContext` para un tipo |
| Marcar rutas o controladores con un dato (`@Roles('admin')`, `@Public()`) | Una constante + `#[set_metadata(CLAVE, valor)]` |
| Agrupar varios metadatos bajo un nombre | `CustomDecorator` + `#[apply_decorators(...)]` |

## Decoradores de parámetro: `FromContext`

Todo parámetro de un handler **sin** atributo se resuelve con el trait `FromContext`:

```rust
#[async_trait]
pub trait FromContext: Sized + Send {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException>;
}
```

Implementarlo para un tipo propio es el equivalente de un decorador de parámetro. Este ejemplo de `examples/tertulia` devuelve el usuario que dejó `AuthGuard` en la petición (o en el socket, si el contexto es de WebSocket):

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUser {
    pub username: String,
    pub role: String,
}

impl ChatUser {
    /// El usuario de una petición HTTP o de un mensaje de WebSocket.
    pub fn of(ctx: &ExecutionContext) -> Option<ChatUser> {
        match ctx.get_type() {
            corvis::ContextType::Ws => ctx.switch_to_ws().get_client::<WsClient>().and_then(|c| c.get::<ChatUser>()),
            _ => ctx.request().get::<ChatUser>(),
        }
    }
}

#[async_trait]
impl FromContext for ChatUser {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        ChatUser::of(ctx).ok_or_else(|| UnauthorizedException::new("Unauthorized"))
    }
}
```

Uso en el controlador, sin ningún atributo:

```rust
#[post]
#[use_guards(AuthGuard)]
async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> {
    Ok(self.rooms.create(dto, &user.username).await?)
}
```

Detalles:

- Si `from_context` devuelve `Err`, la petición termina con esa excepción y el handler no se ejecuta.
- Declarar el parámetro como `Option<ChatUser>` lo vuelve opcional: un error se convierte en `None`.
- Un *newtype* sirve para dar otro significado al mismo dato: `pub struct CurrentUser(pub JwtPayload);` en `examples/cats-app/src/common/decorators.rs`.
- Corvis ya trae implementaciones para `Request`, `Res`, `ExecutionContext`, `ModuleRef` y `Option<T>`.
- Los atributos `#[req]`, `#[request]` y `#[ctx]` también resuelven por `FromContext`; puedes usarlos para documentar la intención.

::: warning Sin pipes
Los parámetros resueltos con `FromContext` no pasan por pipes (ni globales ni de ruta). Valida dentro de `from_context` si lo necesitas.
:::

### Leer datos con argumentos

Un decorador de parámetro que recibe un argumento (por ejemplo, "dame solo el campo `email` del usuario") no tiene equivalente directo: `FromContext` no recibe parámetros. Usa un tipo por cada variante, o devuelve el objeto completo y elige el campo en el handler.

## Metadatos: `#[set_metadata]`

Para "marcar" un handler o un controlador, define una clave y úsala con `#[set_metadata]`. Un guard o un interceptor la lee después con `Reflector`.

Corvis define un marcador por ti: `#[public]` guarda `PUBLIC_METADATA` (`"isPublic"`) con el valor `true`, y la verificación de postura de rutas lo lee (ver [Postura de rutas](/es/security/route-posture#marcar-rutas-como-publicas)). Tus propias claves funcionan igual con `#[set_metadata]`:

```rust
pub const ROLES_KEY: &str = "roles";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}
```

```rust
#[get]
#[public]
fn find_all(&self, #[query] query: ListCatsQuery) -> Vec<Cat> { ... }

#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
#[set_metadata(ROLES_KEY, [Role::Admin])]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> { ... }
```

(`examples/cats-app/src/cats/cats_controller.rs`)

El valor puede ser cualquier cosa que implemente `Serialize`: se guarda como JSON. Quien lo lee elige el tipo al que deserializarlo:

```rust
let required: Option<Vec<Role>> =
    self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
```

`#[set_metadata]` se puede usar sobre un método de `#[routes]`, debajo de `#[controller]`, debajo de `#[websocket_gateway]` o sobre un `#[subscribe_message]`.

## Componer metadatos: `CustomDecorator`

`CustomDecorator` es una lista reutilizable de pares `(clave, valor)`. Se construye con una función y se aplica con `#[apply_decorators(...)]`:

```rust
use corvis::CustomDecorator;

fn admin_only() -> CustomDecorator {
    CustomDecorator::new()
        .set_metadata(ROLES_KEY, [Role::Admin])
        .set_metadata("audit", true)
}

#[routes]
impl UsersController {
    #[delete(":id")]
    #[apply_decorators(admin_only())]
    fn remove(&self, #[param("id")] id: String) { ... }
}
```

- `CustomDecorator::new().set_metadata(k, v)` agrega un par; `.and(otro)` concatena otro decorador.
- `apply_decorators([a(), b()])` combina varios en uno.
- `#[apply_decorators(a(), b())]` acepta varias expresiones y se puede usar sobre un handler o debajo de `#[controller]`.
- Las entradas conservan el orden de declaración.

`CustomDecorator::new().set_metadata(PUBLIC_METADATA, true)` compone el marcador público con otros metadatos, con el mismo efecto que `#[public]`:

```rust
use corvis::{CustomDecorator, PUBLIC_METADATA};

fn public_health_check() -> CustomDecorator {
    CustomDecorator::new().set_metadata(PUBLIC_METADATA, true).set_metadata("audit", false)
}
```

Un `CustomDecorator` también puede fijar el código de estado de la ruta con la clave `HTTP_CODE_METADATA`, como haría `#[http_code]`:

```rust
fn created() -> CustomDecorator {
    CustomDecorator::new().set_metadata(corvis::HTTP_CODE_METADATA, 201)
}
```

Así funcionan los atributos `#[api_*]` de `corvis-swagger`: cada `#[api_tags("cats")]` se traduce a la macro `api_tags!("cats")`, que produce un `CustomDecorator`. Cualquier atributo cuyo nombre empiece con `api_` sigue esa convención, así que puedes publicar tus propias macros `api_*!` que devuelvan un `CustomDecorator`.

::: warning Límite actual
Un `CustomDecorator` solo transporta **metadatos** (y el código de estado). No puede agregar guards, interceptors, pipes ni filtros. Para combinar, por ejemplo, un rol con su guard, escribe ambos atributos (`#[set_metadata(ROLES_KEY, ...)]` y `#[use_guards(RolesGuard)]`) o registra el guard como global y deja que lea el metadato.
:::

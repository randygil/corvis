---
status: documented
---

# Controladores

Un controlador recibe las peticiones HTTP de un conjunto de rutas y devuelve la respuesta. En Corvis un controlador son dos piezas:

- un `struct` marcado con `#[controller]`, cuyos campos son sus dependencias, y
- un bloque `impl` marcado con `#[routes]`, donde cada método con un atributo de verbo (`#[get]`, `#[post]`, ...) es un handler.

```rust
use corvis::prelude::*;

#[controller("cats")]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[get]
    fn find_all(&self) -> Vec<Cat> {
        self.cats_service.find_all()
    }

    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats_service.find_one(id)
    }
}
```

Para que el router lo conozca, el controlador se declara en el módulo que lo contiene:

```rust
#[module(controllers = [CatsController], providers = [CatsService])]
pub struct CatsModule;
```

`#[controller]` también hace del `struct` un provider: sus campos `Arc<T>` se inyectan igual que en un `#[injectable]` (ver [Providers](/es/providers)). El controlador se crea una sola vez y se comparte entre todas las peticiones, así que cualquier estado mutable que guarde debe ir protegido (`Mutex`, `RwLock`, atómicos).

## Rutas

### Prefijo del controlador

| Forma | Efecto |
|---|---|
| `#[controller]` | Sin prefijo: las rutas cuelgan de `/`. |
| `#[controller("cats")]` | Prefijo `/cats`. |
| `#[controller(["cats", "gatos"])]` | Varios prefijos: cada ruta se monta en todos. |
| `#[controller(path = "cats", version = "1", host = "admin.example.com")]` | Forma con nombre. `version` y `host` se explican más abajo. |

Si la aplicación usa `app.set_global_prefix("api")`, ese prefijo se antepone a todas las rutas (ver [Prefijo global](/es/faq/global-prefix)).

### Verbos

Los atributos de verbo van sobre los métodos del bloque `#[routes]`:

| Atributo | Método HTTP |
|---|---|
| `#[get]`, `#[get(":id")]` | `GET` |
| `#[post]` | `POST` |
| `#[put]` | `PUT` |
| `#[patch]` | `PATCH` |
| `#[delete]` | `DELETE` |
| `#[options]` | `OPTIONS` |
| `#[head]` | `HEAD` |
| `#[search]` | `SEARCH` |
| `#[all]` | Cualquier método |
| `#[sse("stream")]` | `GET` que responde un flujo de Server-Sent Events (ver [Server-Sent Events](/es/http/server-sent-events)) |

Sin argumento la ruta es la del controlador. También acepta varias rutas a la vez: `#[get(["a", "b"])]`.

Todos los handlers reciben `&self` como primer parámetro (el macro rechaza los que no lo tienen). Pueden ser `fn` o `async fn`.

### Patrones de ruta

- `:nombre` captura un segmento: `#[get(":id")]`, `#[get(":room/messages")]`.
- `*`, `*nombre` o `{*nombre}` al final capturan el resto de la ruta.
- Las rutas no distinguen mayúsculas (`/CATS` = `/cats`) y admiten una sola barra final opcional (`/cats/` sí, `/cats//` no), como el router de Express 5.
- Los segmentos estáticos tienen prioridad sobre los parámetros, sin importar el orden de declaración: `#[get("count")]` y `#[get(":id")]` pueden convivir en cualquier orden.
- Los parámetros se decodifican con la semántica de `decodeURIComponent`; uno mal codificado responde `400 Failed to decode param '...'`.

### Enrutamiento por host

`#[controller(host = "admin.example.com")]` hace que las rutas del controlador solo respondan cuando el `Host` de la petición coincide (sin distinguir mayúsculas). El valor es un nombre fijo: no hay parámetros de host.

## Parámetros del handler

Cada parámetro lleva un atributo que indica de dónde sale su valor. El valor se extrae como JSON, pasa por los [pipes](/es/pipes) y al final se deserializa en el tipo declarado con `serde`.

| Atributo | Valor |
|---|---|
| `#[body] dto: T` | El cuerpo completo |
| `#[body("name")] name: String` | Una propiedad del cuerpo |
| `#[param("id")] id: u32` | Un parámetro de ruta |
| `#[param] params: HashMap<String, String>` | Todos los parámetros de ruta |
| `#[query] q: T` | Todo el query string como objeto |
| `#[query("page")] page: Option<u32>` | Una clave del query string |
| `#[headers("authorization")] auth: Option<String>` | Una cabecera (también `#[header("...")]`); el nombre se normaliza a minúsculas |
| `#[headers] h: HashMap<String, String>` | Todas las cabeceras |
| `#[ip] ip: String` | La IP del cliente (respeta `set_trust_proxy`) |
| `#[req] req: Request` | La petición (también `#[request]`) |
| `#[res] res: Res` | La respuesta de estilo Express (ver más abajo) |
| `#[uploaded_file]`, `#[uploaded_files]` | Archivos subidos (ver [Subida de archivos](/es/http/file-upload)) |

Después del nombre puedes listar pipes, por tipo o como expresión: `#[param("id", ParseIntPipe)]`, `#[query("page", DefaultValuePipe::new(1), ParseIntPipe)]`.

```rust
#[get(":room/messages")]
async fn messages(&self, #[param("room")] room: String, #[query] q: HistoryQuery) -> HttpResult<Vec<ChatMessage>> {
    self.find(&room).await?;
    Ok(self.rooms.history(&room, q.limit.unwrap_or(50).clamp(1, 200)).await?)
}
```

(`examples/tertulia/src/rooms/rooms_controller.rs`)

**Conversión de tipos.** Los valores de ruta, query y cabeceras llegan como texto; al deserializarlos Corvis convierte los strings a números o booleanos cuando el tipo lo pide (`#[query("page")] page: Option<u32>` con `?page=2` da `Some(2)`). El cuerpo se deserializa de forma estricta. Si la deserialización falla, la respuesta es `400 Bad Request` con el error de `serde`.

**Parámetros sin atributo.** Un parámetro sin atributo se resuelve con el trait `FromContext`. Así se escriben los decoradores de parámetro propios, por ejemplo el usuario autenticado:

```rust
#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

`ChatUser` implementa `FromContext` en `examples/tertulia/src/auth/user.rs`. Ver [Decoradores personalizados](/es/custom-decorators). Corvis ya implementa `FromContext` para `Request`, `Res`, `ExecutionContext`, `ModuleRef` y `Option<T>`.

## Respuestas

El valor de retorno del handler se convierte en la respuesta:

| Tipo devuelto | Respuesta |
|---|---|
| Cualquier `T: Serialize` (structs, `Vec`, `Value`, ...) | JSON |
| `String`, `&'static str` | Texto con `Content-Type: text/html; charset=utf-8`, como `res.send(string)` en Express |
| `()` | Cuerpo vacío |
| `Json(v)`, `Html(s)` | JSON o HTML explícitos |
| `Redirect::to(url)`, `Redirect::permanent(url)` | Redirección |
| `Response` | Control total: `Response::json(..)`, `Response::text(..)`, `Response::html(..)`, `Response::bytes(datos, tipo)`, `Response::stream(..)`, `Response::redirect(url, código)`, con `.with_status(..)`, `.with_header(..)`, `.with_cookie(..)` |
| `StreamableFile` | Un archivo o flujo de bytes (ver [Respuestas en streaming](/es/http/streaming-responses)) |
| `Sse` | Server-Sent Events |
| `(StatusCode, T)` o `(u16, T)` | `T` con otro estado |
| `Bytes`, `Vec<u8>` | Bytes sin procesar |
| `Option<T>`, `Result<T, E>` | Según el contenido |

Los cuerpos de texto y JSON reciben `ETag`, `Content-Length` y la respuesta `304` a peticiones condicionales, igual que en Express.

### Código de estado

Por defecto la respuesta es `200`, salvo en `POST`, que responde `201`. Para cambiarlo:

```rust
#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

`#[http_code(...)]` acepta un número o una constante de `HttpStatus` (`HttpStatus::CREATED`, `HttpStatus::NOT_FOUND`, ...). Para un estado que depende de la lógica, devuelve una tupla `(código, valor)` o un `Response` con `.with_status(...)`.

### Cabeceras

```rust
#[get("whoami/role")]
#[header("Cache-Control", "none")]
async fn whoami(&self, role: CurrentRole) -> String {
    role.0
}
```

Una cabecera que el handler ya puso en su `Response` tiene prioridad sobre `#[header]`. Si la cabecera es `Content-Type` de texto, se le agrega `charset=utf-8`.

### Redirecciones

```rust
#[get("legacy/redirect")]
#[redirect("https://example.com", 301)]
fn legacy(&self) {}
```

El código es opcional (`302` por defecto). Si el handler devuelve un objeto JSON con `url` (y opcionalmente `statusCode`), esos valores reemplazan los del atributo.

### Errores

Un handler que devuelve `HttpResult<T>` (alias de `Result<T, HttpException>`) responde el error como JSON con su estado:

```rust
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
    self.cats_service.find_one(id).ok_or_else(|| NotFoundException::new(format!("Cat #{id} not found")))
}
```

```json
{ "message": "Cat #42 not found", "error": "Not Found", "statusCode": 404 }
```

Cualquier `Result` cuyo error se convierta en `HttpException` o en `anyhow::Error` sirve; un error que no es una `HttpException` produce un `500` genérico y queda registrado en el log. La lista de excepciones y cómo personalizarlas está en [Filtros de excepción](/es/exception-filters).

## Handlers asíncronos

Un handler `async fn` puede esperar cualquier futuro. Corvis lo ejecuta en el runtime de Tokio:

```rust
#[get]
async fn list(&self) -> HttpResult<Value> {
    let rooms = self.rooms.list().await?;
    Ok(self.chat.with_members(rooms))
}
```

## El objeto de respuesta (`#[res]`)

Si un handler necesita manejar la respuesta a la manera de Express, declara un parámetro `#[res] res: Res`:

```rust
#[get("magic")]
async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
    if let Some(s) = self.auth.redeem(token).await {
        res.set_header("Set-Cookie", format!("sid={s}; Path=/; HttpOnly"));
    }
    res.redirect_with(302, "/");
}
```

- Con `#[res]` el handler es dueño de la respuesta: el valor de retorno se ignora y la petición queda abierta hasta que llama a `send`, `json`, `end`, `end_empty`, `redirect`, `redirect_with` o `send_status`.
- Con `#[res(passthrough)]` (o `#[res(passthrough = true)]`) Corvis sigue serializando el valor de retorno y conserva el estado, las cabeceras y las cookies que el handler fijó en `res`. Es la forma cómoda de poner una cookie sin renunciar al resto:

```rust
#[post("login")]
#[http_code(204)]
async fn login(&self, #[body] b: Value, #[res(passthrough)] res: Res) -> HttpResult<()> {
    res.cookie("sid", &self.auth.create().await, CookieOptions::new().http_only(true).same_site("lax"));
    Ok(())
}
```

`Res` tiene los métodos de Express en `snake_case`: `status`, `status_code`, `send_status`, `set`, `header`, `set_header`, `append`, `get`, `get_header`, `get_headers`, `has_header`, `remove_header`, `type_`, `content_type`, `vary`, `links`, `attachment`, `location`, `cookie`, `cookie_json`, `clear_cookie`, `redirect`, `redirect_with`, `send`, `json`, `end`, `end_empty`, `headers_sent` y `writable_ended`.

::: warning
Prefiere devolver valores. Con `#[res]` en modo completo pierdes la conversión automática del valor de retorno, y lo que un interceptor haga con la respuesta no llega al cliente: se envía lo que el handler escribió en `res`.
:::

## Versionado

`#[version("2")]` en un handler, o `version = "1"` en `#[controller(...)]`, asigna versiones cuando la aplicación activa el versionado:

```rust
#[get(":id")]
#[version("2")]
fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
    let cat = self.cats_service.find_one(id)?;
    Ok(json!({ "cat": cat, "apiVersion": 2 }))
}
```

`#[version(VERSION_NEUTRAL)]` marca una ruta que responde a cualquier versión. Los detalles están en [Versionado](/es/http/versioning).

## Atributos de clase

Debajo de `#[controller]` puedes poner atributos que se aplican a todas sus rutas:

```rust
#[controller("cats")]
#[use_interceptors(TimingInterceptor)]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}
```

Los disponibles son `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]`, `#[version]`, `#[apply_decorators]` y los atributos `#[api_*]` de OpenAPI.

::: warning Orden de los atributos
Los atributos de clase deben ir **debajo** de `#[controller]`. Rust expande los atributos de arriba hacia abajo y `#[controller]` solo ve los que vienen después.
:::

## Un ejemplo completo

El controlador REST de `examples/tertulia` combina inyección, guards, metadatos, parámetros y un decorador de parámetro propio:

```rust
#[controller("rooms")]
pub struct RoomsController {
    rooms: Arc<RoomsService>,
    chat: Arc<ChatGateway>,
}

#[routes]
impl RoomsController {
    #[get]
    async fn list(&self) -> HttpResult<Value> {
        let rooms = self.rooms.list().await?;
        Ok(self.chat.with_members(rooms))
    }

    #[post]
    #[use_guards(AuthGuard)]
    async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> {
        Ok(self.rooms.create(dto, &user.username).await?)
    }

    #[post(":room/announcements")]
    #[set_metadata(ROLES_KEY, [MODERATOR])]
    #[use_guards(AuthGuard, RolesGuard)]
    async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Value> {
        self.find(&room).await?;
        let delivered = self.chat.announce(&room, &user, &dto.text);
        Ok(json!({ "room": room, "delivered": delivered }))
    }
}
```

Los métodos fuera del bloque `#[routes]` (como `find` en el mismo archivo) son métodos normales de Rust: no generan rutas.

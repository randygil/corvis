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

Un método sin atributo de verbo es un helper común y no se registra como ruta. Ponerle enhancers u otros atributos de ruta (`#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]`, `#[http_code]`, `#[header]`, `#[redirect]`, `#[version]`, `#[apply_decorators]`, `#[api_*]`) es un error de compilación, así que un guard nunca se descarta en silencio.

### Patrones de ruta

- `:nombre` captura un segmento: `#[get(":id")]`, `#[get(":room/messages")]`.
- `:nombre?` hace opcional el segmento: `#[get(":id?")]` en `#[controller("cats")]` responde `GET /cats` y `GET /cats/7`, y `#[get("files/:dir?/list")]` responde `/files/list` y `/files/docs/list`. Cuando el segmento falta, el parámetro no existe: `#[param("id")] id: Option<String>` recibe `None`. Una ruta puede tener hasta 4 segmentos opcionales (más es un error de arranque); con varios seguidos (`:a?/:b?`) los valores se asignan de izquierda a derecha. Las rutas estáticas conservan prioridad sobre una opcional. OpenAPI documenta el parámetro como parámetro de ruta obligatorio, como exige OpenAPI para los parámetros de ruta.
- `*`, `*nombre` o `{*nombre}` al final capturan el resto de la ruta.
- Las rutas no distinguen mayúsculas (`/CATS` = `/cats`) y admiten una sola barra final opcional (`/cats/` sí, `/cats//` no), como el router de Express 5.
- Los segmentos estáticos tienen prioridad sobre los parámetros, sin importar el orden de declaración: `#[get("count")]` y `#[get(":id")]` pueden convivir en cualquier orden.
- Los parámetros se decodifican con la semántica de `decodeURIComponent`; uno mal codificado responde `400 Failed to decode param '...'`. Los escapes `%XX` exigen dos dígitos hexadecimales: `%+A` no es válido.

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

El valor de retorno del handler se convierte en la respuesta. Lo habitual es devolver un struct que deriva `Serialize`: el framework lo responde como JSON con los campos en el orden de declaración.

```rust
#[derive(Serialize)]
struct CatDto {
    id: u32,
    name: String,
}

#[get(":id")]
async fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<CatDto> {
    Ok(self.cats_service.find_dto(id)?)
}
```

```json
{ "id": 1, "name": "Tom" }
```

Un struct produce los mismos bytes que el objeto `json!` equivalente, y además tiene un nombre y un tipo que el compilador comprueba. `Value` sigue disponible cuando la forma es realmente dinámica (ver la tabla).

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

Cada respuesta pasa además por los valores por defecto de Express:

- Los cuerpos enviados como texto, JSON o HTML llevan un `ETag` débil (`W/"<largo hex>-<sha1 base64>"`, idéntico al paquete `etag`) y `Content-Length`. Un `GET` o `HEAD` con `If-None-Match` o `If-Modified-Since` vigente responde `304` sin cuerpo. `app.set_etag(EtagMode::Strong)` o `app.disable("etag")` cambian esto.
- Un `Content-Type` explícito en un cuerpo de texto o JSON recibe `charset=utf-8` (como `res.send(string)`), y `#[header("Content-Type", "text/plain")]` también agrega el charset por defecto (`text/plain; charset=utf-8`).
- Los cuerpos crudos (`Response::bytes`, `StreamableFile`, streams, cuerpo vacío) se escriben sin tocar, como `res.end(buffer)`: sin `ETag` y con el `Content-Type` exacto (`text/plain; charset=windows-1252` se mantiene).
- `HEAD` conserva el `Content-Length` y el `ETag` del `GET`, sin cuerpo.
- `X-Powered-By: corvis` se envía por defecto. `app.set_x_powered_by("Express")` reproduce byte a byte lo que envía una app de Express; `app.disable("x-powered-by")` lo quita.

### Código de estado

Por defecto la respuesta es `200`, salvo en `POST`, que responde `201`. Para cambiarlo:

```rust
#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

`#[http_code(...)]` acepta un número o una constante de `HttpStatus` (`HttpStatus::CREATED`, `HttpStatus::NOT_FOUND`, ...). También funciona con un handler que devuelve un struct. Para un estado que depende de la lógica, devuelve una tupla `(código, Json(valor))` (la tupla necesita `Json` alrededor de un struct simple) o un `Response` con `.with_status(...)`:

```rust
#[post("jobs")]
async fn start(&self) -> (u16, Json<JobDto>) {
    (HttpStatus::ACCEPTED, Json(JobDto { id: 7 }))
}
```

El estado puede ser un `u16` (las constantes de `HttpStatus` son `u16`) o un `StatusCode`.

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

`Response::redirect` y `Redirect::to` codifican `Location` como `res.redirect` (el paquete `encodeurl`). Siguen aceptando URLs absolutas, así que redirigir a una URL que envía el cliente es una redirección abierta: valídala en tu aplicación.

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

Cualquier `Result` cuyo error se convierta en `HttpException` o en `anyhow::Error` sirve; un error que no es una `HttpException` produce un `500` genérico y queda registrado en el log. Para darle un estado a tu propio tipo de error, implementa `HttpError` y devuelve `Result<T, AppError>` directamente (ver [Errores de dominio](/es/exception-filters#errores-de-dominio)). La lista de excepciones y cómo personalizarlas está en [Filtros de excepción](/es/exception-filters).

## Handlers asíncronos

Un handler `async fn` puede esperar cualquier futuro. Corvis lo ejecuta en el runtime de Tokio:

```rust
#[get]
async fn list(&self) -> HttpResult<Vec<RoomWithMembers>> {
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

- `res` ya trae el estado de la ruta (`#[http_code]`, `201` en `POST`, `200` en el resto) y los `#[header(...)]` antes de que corran los interceptors, y el handler puede cambiarlos.
- Si el handler falla antes de responder, el filtro de excepciones arma la respuesta y se mantienen las cabeceras fijadas en `res` (las del filtro tienen prioridad).
- `send(string)` usa `text/html` si no hay tipo y `send(Vec<u8>)` usa `application/octet-stream`; ambos (igual que `json`) llevan `ETag`, `Content-Length`, el charset y `304`. `end(bytes)` escribe los bytes sin tocar, sin `ETag` ni charset. `set` y `header` agregan el charset por defecto a `Content-Type`.
- `redirect` codifica la URL, agrega `Vary: Accept` y un cuerpo negociado con `Accept` (`Found. Redirecting to /` como `text/plain`, `<p>...</p>` como `text/html` o vacío).
- `cookie` y `clear_cookie` siguen al paquete `cookie` 0.7 y validan el nombre, el valor, `path` y `domain` (ver [Cookies](/es/http/cookies)).
- Los interceptors y filtros pueden leer el objeto con `ctx.switch_to_http().get_response()`.

`Res` tiene los métodos de Express en `snake_case`: `status`, `status_code`, `send_status`, `set`, `header`, `set_header`, `append`, `get`, `get_header`, `get_header_values`, `get_headers`, `has_header`, `remove_header`, `type_`, `content_type`, `vary`, `links`, `attachment`, `location`, `cookie`, `cookie_json`, `clear_cookie`, `redirect`, `redirect_with`, `send`, `json`, `end`, `end_empty`, `headers_sent` y `writable_ended`.

::: warning
Prefiere devolver valores. Con `#[res]` en modo completo pierdes la conversión automática del valor de retorno, y lo que un interceptor haga con la respuesta no llega al cliente: se envía lo que el handler escribió en `res`.
:::

## Versionado

`#[version("2")]` en un handler, o `version = "1"` en `#[controller(...)]`, asigna versiones cuando la aplicación activa el versionado:

```rust
#[get(":id")]
#[version("2")]
fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<CatV2Dto> {
    let cat = self.cats_service.find_one(id)?;
    Ok(CatV2Dto { cat, api_version: 2 })
}
```

```rust
#[derive(Serialize)]
pub struct CatV2Dto {
    pub cat: Cat,
    #[serde(rename = "apiVersion")]
    pub api_version: u32,
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
    async fn list(&self) -> HttpResult<Vec<RoomWithMembers>> {
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
    async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Delivery> {
        self.find(&room).await?;
        let delivered = self.chat.announce(&room, &user, &dto.text);
        Ok(Delivery { room, delivered })
    }
}
```

Los métodos fuera del bloque `#[routes]` (como `find` en el mismo archivo) son métodos normales de Rust: no generan rutas.

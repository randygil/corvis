---
status: documented
---

# Controllers

A controller receives the HTTP requests for a set of routes and returns the response. In Corvis a controller is made of two pieces:

- a `struct` marked with `#[controller]`, whose fields are its dependencies, and
- an `impl` block marked with `#[routes]`, where each method with a verb attribute (`#[get]`, `#[post]`, ...) is a handler.

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

For the router to know about it, the controller is declared in the module that contains it:

```rust
#[module(controllers = [CatsController], providers = [CatsService])]
pub struct CatsModule;
```

`#[controller]` also turns the `struct` into a provider: its `Arc<T>` fields are injected just like in an `#[injectable]` (see [Providers](/providers)). The controller is created once and shared across all requests, so any mutable state it holds must be protected (`Mutex`, `RwLock`, atomics).

## Routes

### Controller prefix

| Form | Effect |
|---|---|
| `#[controller]` | No prefix: routes hang from `/`. |
| `#[controller("cats")]` | Prefix `/cats`. |
| `#[controller(["cats", "gatos"])]` | Several prefixes: each route is mounted under all of them. |
| `#[controller(path = "cats", version = "1", host = "admin.example.com")]` | Named form. `version` and `host` are explained below. |

If the application uses `app.set_global_prefix("api")`, that prefix is prepended to all routes (see [Global prefix](/faq/global-prefix)).

### Verbs

Verb attributes go on the methods of the `#[routes]` block:

| Attribute | HTTP method |
|---|---|
| `#[get]`, `#[get(":id")]` | `GET` |
| `#[post]` | `POST` |
| `#[put]` | `PUT` |
| `#[patch]` | `PATCH` |
| `#[delete]` | `DELETE` |
| `#[options]` | `OPTIONS` |
| `#[head]` | `HEAD` |
| `#[search]` | `SEARCH` |
| `#[all]` | Any method |
| `#[sse("stream")]` | `GET` that responds with a Server-Sent Events stream (see [Server-Sent Events](/http/server-sent-events)) |

Without an argument, the route is the controller's own. It also accepts several routes at once: `#[get(["a", "b"])]`.

All handlers receive `&self` as their first parameter (the macro rejects those that don't have it). They can be `fn` or `async fn`.

### Route patterns

- `:name` captures one segment: `#[get(":id")]`, `#[get(":room/messages")]`.
- `:name?` makes the segment optional: `#[get(":id?")]` in `#[controller("cats")]` answers `GET /cats` and `GET /cats/7`, and `#[get("files/:dir?/list")]` answers `/files/list` and `/files/docs/list`. When the segment is absent the parameter is missing: `#[param("id")] id: Option<String>` receives `None`. A path may have up to 4 optional segments (more is a startup error); with several in a row (`:a?/:b?`) the values bind from left to right. Static routes keep priority over an optional one. OpenAPI documents the parameter as a required path parameter, as OpenAPI requires for path parameters.
- `*`, `*name`, or `{*name}` at the end capture the rest of the path.
- Routes are case-insensitive (`/CATS` = `/cats`) and accept a single optional trailing slash (`/cats/` works, `/cats//` does not), like the Express 5 router.
- Static segments take priority over parameters regardless of declaration order: `#[get("count")]` and `#[get(":id")]` can coexist in either order.
- Parameters are decoded with `decodeURIComponent` semantics; a badly encoded one responds `400 Failed to decode param '...'`. `%XX` escapes need two hexadecimal digits: `%+A` is not valid.

### Host-based routing

`#[controller(host = "admin.example.com")]` makes the controller's routes respond only when the request's `Host` matches (case-insensitive). The value is a fixed name: there are no host parameters.

## Handler parameters

Each parameter carries an attribute that says where its value comes from. The value is extracted as JSON, goes through the [pipes](/pipes), and is finally deserialized into the declared type with `serde`.

| Attribute | Value |
|---|---|
| `#[body] dto: T` | The whole body |
| `#[body("name")] name: String` | One property of the body |
| `#[param("id")] id: u32` | A route parameter |
| `#[param] params: HashMap<String, String>` | All route parameters |
| `#[query] q: T` | The whole query string as an object |
| `#[query("page")] page: Option<u32>` | One key of the query string |
| `#[headers("authorization")] auth: Option<String>` | One header (also `#[header("...")]`); the name is normalized to lowercase |
| `#[headers] h: HashMap<String, String>` | All headers |
| `#[ip] ip: String` | The client IP (honors `set_trust_proxy`) |
| `#[req] req: Request` | The request (also `#[request]`) |
| `#[res] res: Res` | The Express-style response (see below) |
| `#[uploaded_file]`, `#[uploaded_files]` | Uploaded files (see [File upload](/http/file-upload)) |

After the name you can list pipes, by type or as an expression: `#[param("id", ParseIntPipe)]`, `#[query("page", DefaultValuePipe::new(1), ParseIntPipe)]`.

```rust
#[get(":room/messages")]
async fn messages(&self, #[param("room")] room: String, #[query] q: HistoryQuery) -> HttpResult<Vec<ChatMessage>> {
    self.find(&room).await?;
    Ok(self.rooms.history(&room, q.limit.unwrap_or(50).clamp(1, 200)).await?)
}
```

(`examples/tertulia/src/rooms/rooms_controller.rs`)

**Type conversion.** Route, query, and header values arrive as text; when deserializing them, Corvis converts the strings to numbers or booleans when the type asks for it (`#[query("page")] page: Option<u32>` with `?page=2` gives `Some(2)`). The body is deserialized strictly. If deserialization fails, the response is `400 Bad Request` with the `serde` error.

**Parameters without an attribute.** A parameter without an attribute is resolved with the `FromContext` trait. This is how you write your own parameter decorators, for example the authenticated user:

```rust
#[get("me")]
#[use_guards(AuthGuard)]
fn me(&self, user: ChatUser) -> ChatUser {
    user
}
```

`ChatUser` implements `FromContext` in `examples/tertulia/src/auth/user.rs`. See [Custom decorators](/custom-decorators). Corvis already implements `FromContext` for `Request`, `Res`, `ExecutionContext`, `ModuleRef`, and `Option<T>`.

## Responses

The handler's return value is converted into the response:

| Returned type | Response |
|---|---|
| Any `T: Serialize` (structs, `Vec`, `Value`, ...) | JSON |
| `String`, `&'static str` | Text with `Content-Type: text/html; charset=utf-8`, like `res.send(string)` in Express |
| `()` | Empty body |
| `Json(v)`, `Html(s)` | Explicit JSON or HTML |
| `Redirect::to(url)`, `Redirect::permanent(url)` | Redirect |
| `Response` | Full control: `Response::json(..)`, `Response::text(..)`, `Response::html(..)`, `Response::bytes(data, type)`, `Response::stream(..)`, `Response::redirect(url, code)`, with `.with_status(..)`, `.with_header(..)`, `.with_cookie(..)` |
| `StreamableFile` | A file or byte stream (see [Streaming responses](/http/streaming-responses)) |
| `Sse` | Server-Sent Events |
| `(StatusCode, T)` or `(u16, T)` | `T` with a different status |
| `Bytes`, `Vec<u8>` | Raw bytes |
| `Option<T>`, `Result<T, E>` | Depends on the content |

Every response also goes through Express's defaults:

- Bodies sent as text, JSON or HTML carry a weak `ETag` (`W/"<hex length>-<base64 sha1>"`, the same as the `etag` package) and `Content-Length`. A `GET` or `HEAD` with a still-valid `If-None-Match` or `If-Modified-Since` gets `304` without a body. `app.set_etag(EtagMode::Strong)` or `app.disable("etag")` change this.
- An explicit `Content-Type` on a text or JSON body gets `charset=utf-8` (like `res.send(string)`), and `#[header("Content-Type", "text/plain")]` adds the default charset too (`text/plain; charset=utf-8`).
- Raw bodies (`Response::bytes`, `StreamableFile`, streams, an empty body) are written untouched, like `res.end(buffer)`: no `ETag` and the exact `Content-Type` (`text/plain; charset=windows-1252` is kept).
- `HEAD` keeps the `Content-Length` and `ETag` of the `GET`, without a body.
- `X-Powered-By: corvis` is sent by default. `app.set_x_powered_by("Express")` reproduces byte for byte what an Express app sends; `app.disable("x-powered-by")` removes it.

### Status code

By default the response is `200`, except for `POST`, which responds `201`. To change it:

```rust
#[delete(":id")]
#[http_code(HttpStatus::NO_CONTENT)]
fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
    self.cats_service.remove(id)
}
```

`#[http_code(...)]` accepts a number or an `HttpStatus` constant (`HttpStatus::CREATED`, `HttpStatus::NOT_FOUND`, ...). For a status that depends on logic, return a `(code, value)` tuple or a `Response` with `.with_status(...)`.

### Headers

```rust
#[get("whoami/role")]
#[header("Cache-Control", "none")]
async fn whoami(&self, role: CurrentRole) -> String {
    role.0
}
```

A header that the handler already set on its `Response` takes priority over `#[header]`. If the header is a text `Content-Type`, `charset=utf-8` is appended.

### Redirects

```rust
#[get("legacy/redirect")]
#[redirect("https://example.com", 301)]
fn legacy(&self) {}
```

The code is optional (`302` by default). If the handler returns a JSON object with `url` (and optionally `statusCode`), those values replace the ones from the attribute.

`Response::redirect` and `Redirect::to` encode `Location` like `res.redirect` does (the `encodeurl` package). They still accept absolute URLs, so redirecting to a URL sent by the client is an open redirect: validate it in your application.

### Errors

A handler that returns `HttpResult<T>` (an alias of `Result<T, HttpException>`) responds with the error as JSON with its status:

```rust
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
    self.cats_service.find_one(id).ok_or_else(|| NotFoundException::new(format!("Cat #{id} not found")))
}
```

```json
{ "message": "Cat #42 not found", "error": "Not Found", "statusCode": 404 }
```

Any `Result` whose error converts into `HttpException` or `anyhow::Error` works; an error that is not an `HttpException` produces a generic `500` and is recorded in the log. The list of exceptions and how to customize them is in [Exception filters](/exception-filters).

## Async handlers

An `async fn` handler can await any future. Corvis runs it on the Tokio runtime:

```rust
#[get]
async fn list(&self) -> HttpResult<Value> {
    let rooms = self.rooms.list().await?;
    Ok(self.chat.with_members(rooms))
}
```

## The response object (`#[res]`)

If a handler needs to manage the response the Express way, declare a `#[res] res: Res` parameter:

```rust
#[get("magic")]
async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
    if let Some(s) = self.auth.redeem(token).await {
        res.set_header("Set-Cookie", format!("sid={s}; Path=/; HttpOnly"));
    }
    res.redirect_with(302, "/");
}
```

- With `#[res]` the handler owns the response: the return value is ignored and the request stays open until it calls `send`, `json`, `end`, `end_empty`, `redirect`, `redirect_with`, or `send_status`.
- With `#[res(passthrough)]` (or `#[res(passthrough = true)]`) Corvis keeps serializing the return value and preserves the status, headers, and cookies the handler set on `res`. It is the convenient way to set a cookie without giving up the rest:

```rust
#[post("login")]
#[http_code(204)]
async fn login(&self, #[body] b: Value, #[res(passthrough)] res: Res) -> HttpResult<()> {
    res.cookie("sid", &self.auth.create().await, CookieOptions::new().http_only(true).same_site("lax"));
    Ok(())
}
```

- `res` already carries the route's status (`#[http_code]`, `201` on `POST`, `200` otherwise) and the `#[header(...)]` values before the interceptors run, and the handler can change them.
- If the handler fails before responding, the exception filter builds the response and the headers set on `res` are kept (those from the filter take priority).
- `send(string)` uses `text/html` when there is no type and `send(Vec<u8>)` uses `application/octet-stream`; both (like `json`) carry `ETag`, `Content-Length`, the charset and `304`. `end(bytes)` writes the bytes untouched, without `ETag` or charset. `set` and `header` add the default charset to `Content-Type`.
- `redirect` encodes the URL, adds `Vary: Accept` and a body negotiated with `Accept` (`Found. Redirecting to /` as `text/plain`, `<p>...</p>` as `text/html`, or empty).
- `cookie` and `clear_cookie` follow the `cookie` 0.7 package and validate the name, value, `path` and `domain` (see [Cookies](/http/cookies)).
- Interceptors and filters can read the object with `ctx.switch_to_http().get_response()`.

`Res` has the Express methods in `snake_case`: `status`, `status_code`, `send_status`, `set`, `header`, `set_header`, `append`, `get`, `get_header`, `get_header_values`, `get_headers`, `has_header`, `remove_header`, `type_`, `content_type`, `vary`, `links`, `attachment`, `location`, `cookie`, `cookie_json`, `clear_cookie`, `redirect`, `redirect_with`, `send`, `json`, `end`, `end_empty`, `headers_sent`, and `writable_ended`.

::: warning
Prefer returning values. With `#[res]` in full mode you lose the automatic conversion of the return value, and whatever an interceptor does with the response does not reach the client: what the handler wrote to `res` is what gets sent.
:::

## Versioning

`#[version("2")]` on a handler, or `version = "1"` in `#[controller(...)]`, assigns versions when the application enables versioning:

```rust
#[get(":id")]
#[version("2")]
fn find_one_v2(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> {
    let cat = self.cats_service.find_one(id)?;
    Ok(json!({ "cat": cat, "apiVersion": 2 }))
}
```

`#[version(VERSION_NEUTRAL)]` marks a route that responds to any version. The details are in [Versioning](/http/versioning).

## Class attributes

Below `#[controller]` you can put attributes that apply to all of its routes:

```rust
#[controller("cats")]
#[use_interceptors(TimingInterceptor)]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}
```

The available ones are `#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[use_filters]`, `#[set_metadata]`, `#[version]`, `#[apply_decorators]`, and the OpenAPI `#[api_*]` attributes.

::: warning Attribute order
Class attributes must go **below** `#[controller]`. Rust expands attributes from top to bottom, and `#[controller]` only sees the ones that come after it.
:::

## A complete example

The REST controller in `examples/tertulia` combines injection, guards, metadata, parameters, and a custom parameter decorator:

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

Methods outside the `#[routes]` block (like `find` in the same file) are regular Rust methods: they don't generate routes.

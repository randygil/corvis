---
status: documented
---

# Operations

In OpenAPI, an *operation* is the combination of a path and an HTTP method (`GET /cats`, `POST /cats`). `corvis-swagger` creates one for each handler in your controllers. The attributes on this page complete it with text, responses and content types.

All `#[api_*]` attributes go below `#[controller]` (they affect all the controller's operations) or below `#[get]`, `#[post]`... (only that operation). Remember to import `corvis_swagger::*` in the file.

## Operation data

```rust
#[get(":id")]
#[api_operation(
    summary = "Find a cat",
    description = "Returns the cat with that id or a 404.",
    operation_id = "findCat",
    deprecated = false,
)]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> { /* ... */ }
```

`#[api_operation]` accepts `summary`, `description`, `operation_id`, `deprecated`, `tags` and `external_docs`. Without `operation_id`, the identifier is built as `<Controller>_<method>`, with the method name in camelCase (`CatsController_findOne`) and, if the route has a version, `_<version>` at the end.

## Tags

`#[api_tags("cats", "admin")]` assigns tags to the operations. A controller without `#[api_tags]` automatically gets a tag with its name without the `Controller` suffix (`CatsController` → `Cats`); this is disabled with `auto_tag_controllers: false` in `SwaggerDocumentOptions`.

## Responses

Without response attributes, each operation declares only the route's default response: `201` on `POST`, `200` on the rest (or the `#[http_code]` code), with an empty description and no content. The type the handler returns in Rust is **not** inferred: declare it.

```rust
#[post]
#[api_created_response(type = CatDto, description = "The created cat")]
#[api_bad_request_response(description = "Invalid data")]
#[api_conflict_response(description = "Already exists")]
fn create(&self, #[body] dto: CreateCatDto) -> HttpResult<CatDto> { /* ... */ }

#[get]
#[api_ok_response(type = [CatDto])]
fn list(&self) -> Vec<CatDto> { /* ... */ }
```

As soon as you declare a response, the default response is no longer added: declare the success one too.

| Attribute | Use |
|---|---|
| `#[api_response(status = 418, description = "...", type = T)]` | Any code |
| `#[api_ok_response(...)]`, `#[api_created_response(...)]`, `#[api_not_found_response(...)]`... | A shortcut per HTTP code (full list in [Decorators](/openapi/decorators)) |
| `#[api_default_response(...)]` | The `default` response |

Response options: `description`, `type` (a type, `[Type]` for an array or `"string"`), `is_array`, `schema` (a JSON object), `headers`, `links`, `content`, `example` and `examples`. Two responses with the same code are merged: their descriptions are concatenated.

Responses declared on the controller apply to all its operations; in the [introduction](/openapi/introduction) example, `#[api_unauthorized_response]` on `CatsController` adds the `401` to each route.

### Responses validated with zod

`corvis_swagger::decorators::zod_response` does three things at once: it sets the route's status code, registers the DTO so that `ZodSerializerInterceptor` serializes the response with its schema, and documents the response with the output schema (`<Dto>_Output`):

```rust
#[get(":id")]
#[apply_decorators(corvis_swagger::decorators::zod_response::<CatDto>(Some(202), Some("The cat")))]
fn one(&self, #[param("id")] id: String) -> Value {
    json!({ "id": id, "name": "Tom", "extra": true }) // `extra` does not reach the client
}
```

It needs the interceptor to be registered, for example as a global provider:

```rust
#[module(
    controllers = [CatsController],
    providers = [Provider::app_interceptor::<z::ZodSerializerInterceptor>()],
)]
pub struct AppModule;
```

`zod_response_array::<CatDto>(...)` does the same for a list.

## Request body

The body is documented on its own from the type of `#[body]` (see [Types and parameters](/openapi/types-and-parameters)). `#[api_body(...)]` replaces or completes it, with the options `type`, `description`, `required`, `is_array`, `schema` and `examples`:

```rust
#[post("import")]
#[api_body(type = [CreateCatDto], description = "Several cats at once")]
fn import(&self, #[body] cats: Vec<CreateCatDto>) -> Value { /* ... */ }
```

## Content types

`#[api_produces("application/pdf")]` and `#[api_consumes("multipart/form-data")]` change the MIME types of the responses and the body (`application/json` by default). For a file upload, combine `#[api_consumes]` with an explicit `schema`:

```rust
#[post("upload")]
#[use_interceptors(FileInterceptor::new("file"))]
#[api_consumes("multipart/form-data")]
#[api_body(schema = json!({
    "type": "object",
    "properties": { "file": { "type": "string", "format": "binary" } }
}))]
fn upload(&self, #[uploaded_file] file: Option<UploadedFile>) -> Value { /* ... */ }
```

## Headers

`#[api_header(name = "X-Tenant", description = "...", required = true)]` documents a request header. For several at once: `#[api_headers(json!([{ "name": "X-A" }, { "name": "X-B" }]))]`.

## Extensions

`#[api_extension("x-rate-limit", json!({ "limit": 100 }))]` adds an `x-...` key to the operation (or to all the controller's operations).

## Excluding operations

| Attribute | Effect |
|---|---|
| `#[api_exclude_endpoint]` | The handler does not appear in the document |
| `#[api_exclude_controller]` | No handler of the controller appears |
| `#[api_include_endpoint]` | With `only_include_decorated_endpoints: true`, only the marked handlers appear |

All three accept an optional boolean (`#[api_exclude_endpoint(false)]`).

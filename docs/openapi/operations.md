---
status: documentado
---

# Operaciones

En OpenAPI, una *operación* es la combinación de una ruta y un método HTTP (`GET /cats`, `POST /cats`). `corvis-swagger` crea una por cada handler de tus controladores. Los atributos de esta página la completan con texto, respuestas y tipos de contenido.

Todos los atributos `#[api_*]` van debajo de `#[controller]` (afectan a todas las operaciones del controlador) o debajo de `#[get]`, `#[post]`... (solo a esa operación). Recuerda importar `corvis_swagger::*` en el archivo.

## Datos de la operación

```rust
#[get(":id")]
#[api_operation(
    summary = "Busca un gato",
    description = "Devuelve el gato con ese id o un 404.",
    operation_id = "findCat",
    deprecated = false,
)]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Value> { /* ... */ }
```

`#[api_operation]` acepta `summary`, `description`, `operation_id`, `deprecated`, `tags` y `external_docs`. Sin `operation_id`, el identificador se forma como `<Controlador>_<método>`, con el nombre del método en camelCase (`CatsController_findOne`) y, si la ruta tiene versión, `_<versión>` al final.

## Tags

`#[api_tags("cats", "admin")]` asigna tags a las operaciones. Un controlador sin `#[api_tags]` recibe automáticamente un tag con su nombre sin el sufijo `Controller` (`CatsController` → `Cats`); se desactiva con `auto_tag_controllers: false` en `SwaggerDocumentOptions`.

## Respuestas

Sin atributos de respuesta, cada operación declara solo la respuesta por defecto de la ruta: `201` en `POST`, `200` en el resto (o el código de `#[http_code]`), con descripción vacía y sin contenido. El tipo que devuelve el handler en Rust **no** se infiere: decláralo.

```rust
#[post]
#[api_created_response(type = CatDto, description = "El gato creado")]
#[api_bad_request_response(description = "Datos inválidos")]
#[api_conflict_response(description = "Ya existe")]
fn create(&self, #[body] dto: CreateCatDto) -> HttpResult<CatDto> { /* ... */ }

#[get]
#[api_ok_response(type = [CatDto])]
fn list(&self) -> Vec<CatDto> { /* ... */ }
```

En cuanto declaras una respuesta, la respuesta por defecto deja de añadirse: declara también la de éxito.

| Atributo | Uso |
|---|---|
| `#[api_response(status = 418, description = "...", type = T)]` | Cualquier código |
| `#[api_ok_response(...)]`, `#[api_created_response(...)]`, `#[api_not_found_response(...)]`... | Un atajo por código HTTP (lista completa en [Decoradores](/openapi/decorators)) |
| `#[api_default_response(...)]` | La respuesta `default` |

Opciones de una respuesta: `description`, `type` (un tipo, `[Tipo]` para un array o `"string"`), `is_array`, `schema` (un objeto JSON), `headers`, `links`, `content`, `example` y `examples`. Dos respuestas con el mismo código se fusionan: sus descripciones se concatenan.

Las respuestas declaradas en el controlador se aplican a todas sus operaciones; en el ejemplo de la [introducción](/openapi/introduction), `#[api_unauthorized_response]` en `CatsController` añade el `401` a cada ruta.

### Respuestas validadas con zod

`corvis_swagger::decorators::zod_response` hace tres cosas a la vez: fija el código de estado de la ruta, registra el DTO para que `ZodSerializerInterceptor` serialice la respuesta con su schema, y documenta la respuesta con el schema de salida (`<Dto>_Output`):

```rust
#[get(":id")]
#[apply_decorators(corvis_swagger::decorators::zod_response::<CatDto>(Some(202), Some("The cat")))]
fn one(&self, #[param("id")] id: String) -> Value {
    json!({ "id": id, "name": "Tom", "extra": true }) // `extra` no llega al cliente
}
```

Necesita el interceptor registrado, por ejemplo como provider global:

```rust
#[module(
    controllers = [CatsController],
    providers = [Provider::app_interceptor::<z::ZodSerializerInterceptor>()],
)]
pub struct AppModule;
```

`zod_response_array::<CatDto>(...)` hace lo mismo para una lista.

## Cuerpo de la petición

El body se documenta solo a partir del tipo de `#[body]` (ver [Tipos y parámetros](/openapi/types-and-parameters)). `#[api_body(...)]` lo reemplaza o lo completa, con las opciones `type`, `description`, `required`, `is_array`, `schema` y `examples`:

```rust
#[post("import")]
#[api_body(type = [CreateCatDto], description = "Varios gatos a la vez")]
fn import(&self, #[body] cats: Vec<CreateCatDto>) -> Value { /* ... */ }
```

## Tipos de contenido

`#[api_produces("application/pdf")]` y `#[api_consumes("multipart/form-data")]` cambian los tipos MIME de las respuestas y del cuerpo (por defecto `application/json`). Para una subida de archivos, combina `#[api_consumes]` con un `schema` explícito:

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

## Cabeceras

`#[api_header(name = "X-Tenant", description = "...", required = true)]` documenta una cabecera de la petición. Para varias a la vez: `#[api_headers(json!([{ "name": "X-A" }, { "name": "X-B" }]))]`.

## Extensiones

`#[api_extension("x-rate-limit", json!({ "limit": 100 }))]` añade una clave `x-...` a la operación (o a todas las del controlador).

## Excluir operaciones

| Atributo | Efecto |
|---|---|
| `#[api_exclude_endpoint]` | El handler no aparece en el documento |
| `#[api_exclude_controller]` | Ningún handler del controlador aparece |
| `#[api_include_endpoint]` | Con `only_include_decorated_endpoints: true`, solo aparecen los handlers marcados |

Los tres aceptan un booleano opcional (`#[api_exclude_endpoint(false)]`).

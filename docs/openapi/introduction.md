---
status: documented
---

# Introduction

[OpenAPI](https://swagger.io/specification/) is a standard format for describing HTTP APIs. The `corvis-swagger` crate generates that document from your controllers, serves it as JSON and YAML and mounts two interfaces to explore it: Swagger UI and the Scalar reference.

The information comes from three sources:

- **the routes**: methods, paths, versions and parameters (`#[param]`, `#[query]`, `#[body]`, `#[headers]`);
- **the parameter types**: DTOs with a `corvis::z` schema (`zod_dto!`) or with `#[derive(ApiSchema)]`;
- **the `#[api_*]` attributes** you add to controllers and handlers: tags, descriptions, responses, security.

## Installation

Add the crate to your project (it is not on crates.io yet, so a local path is used):

```toml
[dependencies]
corvis = { path = "../corvis/crates/corvis" }
corvis-swagger = { path = "../corvis/crates/corvis-swagger" }
```

The `#[api_*]` attributes are `corvis-swagger` macros; to make them available, import the crate in every file that uses them:

```rust
use corvis_swagger::*;
```

## Bootstrap

After creating the application, build the document and mount it:

```rust
use corvis::prelude::*;
use corvis_swagger::*;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.set_global_prefix("api");

    let config = DocumentBuilder::new()
        .set_title("Cats")
        .set_description("The cats API")
        .set_version("1.0")
        .add_tag("cats", "Operations on cats")
        .add_bearer_auth()
        .build();
    let document = cleanup_open_api_doc(SwaggerModule::create_document(&app, &config));
    SwaggerModule::setup("docs", &mut app, document);

    app.listen(3000).await
}
```

With the application running:

| Path | Content |
|---|---|
| `http://localhost:3000/docs` | Swagger UI |
| `http://localhost:3000/docs-json` | The document as JSON |
| `http://localhost:3000/docs-yaml` | The document as YAML |

`DocumentBuilder` defines the fixed part of the document (title, version, servers, security schemes). `SwaggerModule::create_document` walks the application's controllers and fills in `paths` and `components.schemas`. `cleanup_open_api_doc` converts the schemas of the `corvis::z` DTOs to the OpenAPI format; if you do not use those DTOs it is not needed, but it does no harm either.

::: tip Flat routes
The routes mounted by `SwaggerModule::setup` (the page, `swagger-ui-init.js`, `-json` and `-yaml`, plus the `swagger-ui-dist` files) are plain Express-style routes, not controller routes: they do not appear in the `Mapped {...}` logs and do not go through guards, pipes or interceptors. If the documentation must not be public, protect it with a middleware or do not mount it in production.
:::

## A documented controller

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use corvis_swagger::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCatDto {
    pub name: String,
    pub lives: i64,
}
zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), lives: z::int().min(1).default(9) });

#[derive(Debug, Deserialize, Serialize)]
pub struct CatDto {
    pub id: String,
    pub name: String,
}
zod_dto!(CatDto, z::object! { id: z::uuid(), name: z::string() });

#[controller("cats")]
#[api_tags("cats")]
#[api_bearer_auth]
#[api_unauthorized_response(description = "Missing token")]
pub struct CatsController;

#[routes]
impl CatsController {
    #[post]
    #[api_operation(summary = "Create a cat")]
    #[api_created_response(type = CatDto)]
    #[api_conflict_response(description = "Duplicated")]
    fn create(&self, #[body] dto: CreateCatDto) -> Value {
        json!({ "id": "00000000-0000-4000-8000-000000000000", "name": dto.name })
    }

    #[get]
    #[api_ok_response(type = [CatDto])]
    fn list(&self) -> Value {
        json!([])
    }
}
```

This controller is the one in `crates/corvis-swagger/tests/swagger_app.rs`, which checks the generated document field by field. `POST /api/cats` ends up like this:

```json
{
  "operationId": "CatsController_create",
  "parameters": [],
  "requestBody": {
    "required": true,
    "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateCatDto" } } }
  },
  "responses": {
    "201": { "description": "", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CatDto" } } } },
    "401": { "description": "Missing token" },
    "409": { "description": "Duplicated" }
  },
  "security": [{ "bearer": [] }],
  "summary": "Create a cat",
  "tags": ["cats"]
}
```

Note that the attributes go **below** `#[controller]` and `#[get]`/`#[post]`, in the order in which you want the responses to appear.

## Scalar reference

Besides Swagger UI you can mount the [Scalar](https://scalar.com) reference:

```rust
scalar::mount(&mut app, "/reference", ApiReferenceOptions::new().document(&document));
```

`mount` serves the page at that path and everything under it, for any method. `ApiReferenceOptions` lets you change the theme (`.theme("purple")`), the title (`.page_title(..)`), the CDN (`.cdn(..)`), a `nonce` for CSP or any key of Scalar's configuration with `.set(key, value)`. Instead of the embedded document you can point to a URL with `.url("/docs-json")`.

The page is the one generated by `@scalar/client-side-rendering` 0.3, with its default theme unless you set `.theme(..)`. It does not send the `_integration` field of Scalar's server integrations; if you need it (for example to serve a page identical to an existing one), set it with `ApiReferenceOptions::integration("...")`. The JSON configuration embedded in the page (which includes the document) escapes `<`, `>`, `&`, U+2028 and U+2029, so no text in the document can close the `<script>`. `{"$js": "..."}` values set with `.set(..)` are emitted as code, because the developer writes them.

## Example

`examples/cats-app` in the repository is a complete application with an OpenAPI document: `#[derive(Validate)]` DTOs described with `#[derive(ApiSchema)]`, tagged controllers, bearer authentication and `SwaggerModule::setup("docs", ..)` in `create_app()`. Its test (`examples/cats-app/tests/openapi.rs`) checks the document.

## Next steps

- [Types and parameters](/openapi/types-and-parameters): how DTOs and parameters are described.
- [Operations](/openapi/operations): responses, bodies, content types.
- [Security](/openapi/security): authentication schemes.
- [Decorators](/openapi/decorators): the full list of attributes.
- [Other features](/openapi/other-features): global prefix, multiple documents, Swagger UI options.

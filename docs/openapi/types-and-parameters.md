---
status: documented
---

# Types and parameters

`SwaggerModule::create_document` looks at each handler's parameters to document the request. Corvis's parameter attributes translate like this:

| Handler parameter | In the document |
|---|---|
| `#[body] dto: T` | `requestBody` with the schema of `T` |
| `#[param("id")] id: T` | `in: path` parameter named `id` |
| `#[query("page")] page: T` | `in: query` parameter named `page` |
| `#[query] q: T` | If `T` is a DTO, one `in: query` parameter per property |
| `#[headers("x-tenant")] t: T` | `in: header` parameter |
| `#[req]`, `#[ip]`, `FromContext` parameters... | Do not appear |

The Rust type decides the parameter's `type`: integers and floats are `number`, `String` and `&str` are `string`, `bool` is `boolean`, `Vec` is `array`, `Value` and maps are `object`.

## DTOs with `corvis::z`

The most complete way to describe a DTO is to give it a schema with `zod_dto!`. The same schema validates the request (with `ZodValidationPipe`) and generates the document:

```rust
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCatDto {
    pub name: String,
    pub lives: i64,
}
zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), lives: z::int().min(1).default(9) });
```

The body is referenced as `#/components/schemas/CreateCatDto`, and the schema ends up in `components`:

```json
{
  "type": "object",
  "properties": {
    "name": { "type": "string", "minLength": 1 },
    "lives": { "default": 9, "type": "integer", "minimum": 1, "maximum": 9007199254740991 }
  },
  "required": ["name"]
}
```

`lives` is not required because it has a `default`. Remember to pass the document through `cleanup_open_api_doc` before serving it: that is what converts these schemas to the OpenAPI dialect.

A query DTO is expanded into parameters. With

```rust
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct ListCatsQuery {
    #[serde(default)]
    pub q: Option<String>,
}
zod_dto!(ListCatsQuery, z::object! { q: z::string().max(20).optional() });

#[get]
fn list(&self, #[query] q: ListCatsQuery) -> Vec<CatDto> { /* ... */ }
```

the document contains:

```json
[{ "name": "q", "required": false, "in": "query", "schema": { "maxLength": 20, "type": "string" } }]
```

### Input and output schema

A zod schema can describe different things on input and on output (for example, a field with `default` is optional on input and is always present on output). When you reference a DTO as a response with [`zod_response`](/openapi/operations#responses-validated-with-zod), the document uses its output version, registered as `<Dto>_Output`. To refer to it in other attributes use the `corvis_swagger::types::Output<Dto>` type:

```rust
#[api_ok_response(type = corvis_swagger::types::Output<CatDto>)]
```

## DTOs with `#[derive(ApiSchema)]`

If a DTO has no zod schema (for example, because you validate it with `#[derive(Validate)]`), describe it with `#[derive(ApiSchema)]`. Each named field becomes a property, with no need to annotate it:

```rust
use corvis_swagger::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
#[api_schema(description = "A shelter cat")]
pub struct CatResponse {
    #[api_property(description = "Identifier", example = 42)]
    pub id: u32,
    pub name: String,
    pub owner_email: Option<String>,
    pub tags: Vec<String>,
    #[api_hide_property]
    pub internal_notes: String,
}
```

| Field type | Property |
|---|---|
| `Option<T>` | `T` with `required: false` |
| `Vec<T>`, `HashSet<T>`, `[T; N]`... | Array of `T` |
| `Box<T>`, `Arc<T>` | `T` |
| Another type with `ApiSchema` or `zod_dto!` | Reference to its schema |

`#[serde(rename)]`, `#[serde(rename_all)]` and `#[serde(skip)]` are respected.

| Attribute | Where | Effect |
|---|---|---|
| `#[api_property(...)]` | Field | Property options: `description`, `example`, `enum`, `minimum`, `type`, `required`... |
| `#[api_property_optional(...)]` | Field | Same, with `required: false` |
| `#[api_response_property(...)]` | Field | Same, with `readOnly: true` |
| `#[api_hide_property]` | Field | Excludes the field |
| `#[api_schema(name = "...", ...)]` | Struct | Schema name in `components` and other schema keys (`description`...) |

Option keys are written in snake_case or camelCase; they are converted to camelCase (`read_only` → `readOnly`). Values are Rust expressions (`example = 42`, `enum = ["a", "b"]`) except `type`, which takes a type: `type = CatDto`, `type = [CatDto]` for an array or `type = "string"`.

::: warning
`#[derive(ApiSchema)]` does not have tests of its own in the repository yet (the `corvis-swagger` integration test uses `corvis::z` DTOs). Check the generated document if you depend on uncommon options.
:::

## Explicit parameters

To add information the type does not provide, use `#[api_param]`, `#[api_query]` or `#[api_header]` with the same keys as OpenAPI's parameter object:

```rust
#[get(":id")]
#[api_param(name = "id", description = "Cat id", example = "abc")]
#[api_query(name = "fields", required = false, enum = ["name", "age"], is_array = true)]
#[api_header(name = "X-Tenant", required = true)]
fn one(&self, #[param("id")] id: String) -> Value { /* ... */ }
```

An `#[api_param]` with the same name as a `#[param]` is merged with it. As in the Node.js ecosystem's generators, the `example` of an explicit parameter goes into the `schema`:

```json
[{ "name": "id", "required": true, "in": "path", "schema": { "example": "abc", "type": "string" } }]
```

## Custom types

For a scalar type of your own (a decimal, an identifier) that must appear as `string`, implement `SwaggerType`:

```rust
use corvis_swagger::{SwaggerType, ctor_marker};

pub struct Money(String);

impl SwaggerType for Money {
    fn swagger_type() -> serde_json::Value {
        ctor_marker("String")
    }
}
```

`ctor_marker` takes the name of a JavaScript constructor, such as `"String"`, `"Number"` or `"Boolean"`: these are the same markers the crate uses for Rust's primitive types.

## Extra models

A schema that is not referenced from any route does not appear in `components`. To include it, use `#[api_extra_models(A, B)]` on a controller or handler, or `extra_models` in `SwaggerDocumentOptions` (see [Other features](/openapi/other-features)).

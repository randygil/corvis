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

### Validation constraints

The constraints declared for `#[derive(Validate)]` on the same field also become OpenAPI keywords, with the values the original framework's OpenAPI compiler plugin gives to the equivalent class-validator decorators. Nothing else to write:

```rust
#[derive(Deserialize, Validate, ApiSchema)]
pub struct CreateCatDto {
    #[max_length(50)]
    pub name: String,                 // { "type": "string", "maxLength": 50 }
    #[min(0)]
    #[max(40)]
    pub age: u32,                     // { "type": "number", "minimum": 0, "maximum": 40 }
    #[is_in("persian", "siamese")]
    pub breed: String,                // { "type": "string", "enum": ["persian", "siamese"] }
}
```

| Field attribute | Keywords |
|---|---|
| `#[min(n)]`, `#[max(n)]` | `minimum`, `maximum` |
| `#[min_length(n)]`, `#[max_length(n)]` | `minLength`, `maxLength` |
| `#[length(a)]`, `#[length(a, b)]` | `minLength: a`, and `maxLength: b` when given |
| `#[matches(re)]` | `pattern: re` |
| `#[contains(s)]` | `pattern`: `s` with the regular expression characters escaped |
| `#[is_email]`, `#[is_url]`, `#[is_uuid]` | `format`: `email`, `uri`, `uuid` |
| `#[is_hex_color]` | `pattern` of a hexadecimal color |
| `#[is_positive]`, `#[is_negative]` | `minimum: 1`, `maximum: -1` (`exclusiveMinimum: 0`, `exclusiveMaximum: 0` on fields typed `f32` / `f64` directly or inside `Option` / `Vec`, not through a type alias) |
| `#[array_not_empty]`, `#[array_min_size(n)]`, `#[array_max_size(n)]` | `minItems`, `maxItems` |
| `#[array_unique]` | `uniqueItems: true` |
| `#[is_in(a, b)]` | `enum: [a, b]`, plus `isArray: true` with `each` |
| `#[is_enum]`, `#[is_enum(T)]` | `enum` with the variant names serde accepts for the enum |

Values are the arguments serialized as JSON. Rules that are not in the table (`is_int`, `is_date_string`, `is_not_empty`, `is_optional`...) add nothing: `is_int` does not change `type`, and `is_optional` does not change `required` (an `Option<T>` field is the optional one).

- **Precedence.** When two attributes of a field produce the same keyword, the keyword written later in this order wins: `is_enum`, `is_in`, `min`, `max`, `min_length`, `max_length`, `array_not_empty`, `is_positive`, `is_negative`, `array_unique`, `is_email`, `is_url`, `is_uuid`, `is_hex_color`, `array_min_size`, `array_max_size`, `contains`, `length`, `matches` (so `#[min(5)]` with `#[is_positive]` gives `minimum: 1`). For a rule written twice, the upper attribute counts.
- **Explicit options win.** A key in `#[api_property(...)]` replaces the derived one: `#[max(40)]` with `#[api_property(maximum = 30)]` gives `maximum: 30`.
- **Arrays.** For `Vec<T>` fields, value constraints (`maxLength`, `minimum`, `pattern`, `format`...) go to `items`, and `minItems`, `maxItems` and `uniqueItems` stay on the array. `each` only matters for `is_in` and `is_enum`, as in the original.
- **`pattern`** carries the regular expression as written, in Rust syntax; a tool that reads it as ECMAScript may reject Rust-only constructs.

Differences from the original plugin, on purpose: `#[is_positive]` and `#[is_negative]` on a float produce an exclusive bound instead of `minimum: 1` (which excluded the values between 0 and 1), `#[contains]` escapes the text instead of using it as a regular expression, and `#[is_enum]` adds `enum` (the original takes it from the TypeScript type). The fixture `crates/corvis-swagger/tests/fixtures/class_validator_shim` records the original's output and the test lists these differences.

Clients generated from the document become stricter (a generator may turn `maxLength` into a check). Validation itself does not change. The `cats-app` example tests the generated document (`examples/cats-app/tests/openapi.rs`).

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

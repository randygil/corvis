---
status: partial
---

# CLI plugin

In TypeScript frameworks, a compiler plugin reads the types of classes and adds only the OpenAPI annotations, so you do not have to repeat them on every property. Corvis **has no compile-time plugin** and no CLI options for this, but it does not need one for the same purpose: Rust macros already see the types.

What the plugin does in those frameworks, macros do in Corvis:

| Need | How it is solved |
|---|---|
| Document all the properties of a DTO without annotating them | `#[derive(ApiSchema)]` turns every named field into a property |
| Mark optional fields as optional | An `Option<T>` field comes out with `required: false` |
| Detect arrays | `Vec<T>`, `HashSet<T>`, `[T; N]`... come out as arrays of `T` |
| Respect JSON names | `#[serde(rename)]`, `#[serde(rename_all)]` and `#[serde(skip)]` are applied |
| Document the body, the parameters and their type | `#[routes]` registers the types of `#[body]`, `#[param]`, `#[query]` and `#[headers]` |
| Reflect class-validator constraints | `#[derive(ApiSchema)]` reads the `#[derive(Validate)]` attributes of each field ([Validation constraints](/openapi/types-and-parameters#validation-constraints)) |
| Reuse validation rules | With `zod_dto!`, the same schema validates and documents (lengths, ranges, formats) |

```rust
#[derive(Debug, Deserialize, Serialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoomSummary {
    pub name: String,            // required, string
    pub max_members: i64,        // "maxMembers", number
    pub topic: Option<String>,   // required: false
    pub members: Vec<String>,    // array of string
}
```

What is **not** inferred:

- **The response type**: the handler's return value is not documented on its own. Use `#[api_ok_response(type = ...)]` or `zod_response` (see [Operations](/openapi/operations#responses)).
- **Descriptions from comments**: `///` comments on fields do not make it into the document. Write `#[api_property(description = "...")]`.
- **Examples**: they are declared with `example = ...`.

See [Types and parameters](/openapi/types-and-parameters).

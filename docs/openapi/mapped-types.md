---
status: unavailable
---

# Mapped types

`corvis-swagger` **has no mapped types** (functions that derive one DTO from another, such as "the same DTO with all fields optional" or "only these fields"). In Rust a type cannot be generated from another at runtime, and there is no macro yet that does it.

## Alternative: compose `corvis::z` schemas

If you describe your DTOs with `zod_dto!`, the schema can be derived. `corvis::z` objects have zod's composition methods:

| Method | Result |
|---|---|
| `.partial()` | All properties optional |
| `.partial_fields(&["name"])` | Only those optional |
| `.required()`, `.required_fields(&[..])` | The reverse |
| `.pick(&["name", "age"])` | Only those properties |
| `.omit(&["password"])` | All except those |
| `.extend(other)`, `.merge(other)` | The union of two objects |

You write the Rust struct yourself, with `Option` where appropriate; the schema is built from the original:

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

fn create_cat_schema() -> z::ZObject {
    z::object! {
        name: z::string().trim().min(1),
        age: z::int().nonnegative(),
        breed: z::string(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateCatDto {
    pub name: String,
    pub age: u32,
    pub breed: String,
}
zod_dto!(CreateCatDto, create_cat_schema());

/// Equivalent to "CreateCatDto with everything optional".
#[derive(Debug, Deserialize)]
pub struct UpdateCatDto {
    pub name: Option<String>,
    pub age: Option<u32>,
    pub breed: Option<String>,
}
zod_dto!(UpdateCatDto, create_cat_schema().partial());
```

The OpenAPI document and validation (`ZodValidationPipe`) use the derived schema, so each field's rules (`min(1)`, `nonnegative()`) are kept in a single place.

With `#[derive(Validate)]` or `#[derive(ApiSchema)]` DTOs there is no composition: declare the second struct in full. That is what `corvis g resource` does, which generates `Update<Entity>Dto` with `Option` fields and `#[is_optional]`.

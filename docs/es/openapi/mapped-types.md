---
status: unavailable
---

# Tipos mapeados

`corvis-swagger` **no tiene tipos mapeados** (funciones que derivan un DTO de otro, como "el mismo DTO con todos los campos opcionales" o "solo estos campos"). En Rust un tipo no se puede generar a partir de otro en tiempo de ejecución, y no hay todavía una macro que lo haga.

## Alternativa: componer schemas de `corvis::z`

Si describes tus DTOs con `zod_dto!`, el schema sí se puede derivar. Los objetos de `corvis::z` tienen los métodos de composición de zod:

| Método | Resultado |
|---|---|
| `.partial()` | Todas las propiedades opcionales |
| `.partial_fields(&["name"])` | Solo esas opcionales |
| `.required()`, `.required_fields(&[..])` | Al revés |
| `.pick(&["name", "age"])` | Solo esas propiedades |
| `.omit(&["password"])` | Todas menos esas |
| `.extend(otro)`, `.merge(otro)` | La unión de dos objetos |

El struct de Rust lo escribes tú, con `Option` donde corresponda; el schema se construye a partir del original:

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

/// Equivale a "CreateCatDto con todo opcional".
#[derive(Debug, Deserialize)]
pub struct UpdateCatDto {
    pub name: Option<String>,
    pub age: Option<u32>,
    pub breed: Option<String>,
}
zod_dto!(UpdateCatDto, create_cat_schema().partial());
```

El documento OpenAPI y la validación (`ZodValidationPipe`) usan el schema derivado, así que las reglas de cada campo (`min(1)`, `nonnegative()`) se mantienen en un solo lugar.

Con DTOs de `#[derive(Validate)]` o `#[derive(ApiSchema)]` no hay composición: declara el segundo struct completo. Es lo que hace `corvis g resource`, que genera `Update<Entidad>Dto` con los campos `Option` y `#[is_optional]`.

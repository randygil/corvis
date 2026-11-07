---
status: parcial
---

# Plugin de la CLI

En los frameworks de TypeScript, un plugin del compilador lee los tipos de las clases y añade solo las anotaciones de OpenAPI, para no tener que repetirlas en cada propiedad. Corvis **no tiene un plugin de compilación** ni opciones de la CLI para esto, pero no lo necesita para lo mismo: las macros de Rust ya ven los tipos.

Lo que en esos frameworks hace el plugin, en Corvis lo hacen las macros:

| Necesidad | Cómo se resuelve |
|---|---|
| Documentar todas las propiedades de un DTO sin anotarlas | `#[derive(ApiSchema)]` convierte cada campo con nombre en una propiedad |
| Marcar como opcionales los campos opcionales | Un campo `Option<T>` sale con `required: false` |
| Detectar arrays | `Vec<T>`, `HashSet<T>`, `[T; N]`... salen como arrays de `T` |
| Respetar los nombres JSON | Se aplican `#[serde(rename)]`, `#[serde(rename_all)]` y `#[serde(skip)]` |
| Documentar el body, los parámetros y su tipo | `#[routes]` registra los tipos de `#[body]`, `#[param]`, `#[query]` y `#[headers]` |
| Reutilizar las reglas de validación | Con `zod_dto!`, el mismo schema valida y documenta (longitudes, rangos, formatos) |

```rust
#[derive(Debug, Deserialize, Serialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoomSummary {
    pub name: String,            // required, string
    pub max_members: i64,        // "maxMembers", number
    pub topic: Option<String>,   // required: false
    pub members: Vec<String>,    // array de string
}
```

Lo que **no** se infiere:

- **El tipo de respuesta**: el valor de retorno del handler no se documenta solo. Usa `#[api_ok_response(type = ...)]` o `zod_response` (ver [Operaciones](/openapi/operations#respuestas)).
- **Las descripciones a partir de comentarios**: los comentarios `///` de los campos no pasan al documento. Escribe `#[api_property(description = "...")]`.
- **Los ejemplos**: se declaran con `example = ...`.

Ver [Tipos y parámetros](/openapi/types-and-parameters).

---
status: documentado
---

# Tipos y parámetros

`SwaggerModule::create_document` mira los parámetros de cada handler para documentar la petición. Los atributos de parámetro de Corvis se traducen así:

| Parámetro del handler | En el documento |
|---|---|
| `#[body] dto: T` | `requestBody` con el schema de `T` |
| `#[param("id")] id: T` | Parámetro `in: path` llamado `id` |
| `#[query("page")] page: T` | Parámetro `in: query` llamado `page` |
| `#[query] q: T` | Si `T` es un DTO, un parámetro `in: query` por cada propiedad |
| `#[headers("x-tenant")] t: T` | Parámetro `in: header` |
| `#[req]`, `#[ip]`, parámetros `FromContext`... | No aparecen |

El tipo Rust decide el `type` del parámetro: los enteros y flotantes son `number`, `String` y `&str` son `string`, `bool` es `boolean`, `Vec` es `array`, `Value` y los mapas son `object`.

## DTOs con `corvis::z`

La forma más completa de describir un DTO es darle un schema con `zod_dto!`. El mismo schema valida la petición (con `ZodValidationPipe`) y genera el documento:

```rust
#[derive(Debug, Deserialize, Serialize)]
pub struct CreateCatDto {
    pub name: String,
    pub lives: i64,
}
zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), lives: z::int().min(1).default(9) });
```

El body se referencia como `#/components/schemas/CreateCatDto`, y el schema queda en `components`:

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

`lives` no es obligatoria porque tiene `default`. Recuerda pasar el documento por `cleanup_open_api_doc` antes de servirlo: es lo que convierte estos schemas al dialecto de OpenAPI.

Un DTO de query se expande en parámetros. Con

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

el documento contiene:

```json
[{ "name": "q", "required": false, "in": "query", "schema": { "maxLength": 20, "type": "string" } }]
```

### Esquema de entrada y de salida

Un schema de zod puede describir cosas distintas a la entrada y a la salida (por ejemplo, un campo con `default` es opcional al entrar y siempre está al salir). Cuando referencias un DTO como respuesta con [`zod_response`](/openapi/operations#respuestas-validadas-con-zod), el documento usa su versión de salida, registrada como `<Dto>_Output`. Para referirte a ella en otros atributos usa el tipo `corvis_swagger::types::Output<Dto>`:

```rust
#[api_ok_response(type = corvis_swagger::types::Output<CatDto>)]
```

## DTOs con `#[derive(ApiSchema)]`

Si un DTO no tiene schema de zod (por ejemplo, porque lo validas con `#[derive(Validate)]`), descríbelo con `#[derive(ApiSchema)]`. Cada campo con nombre se convierte en una propiedad, sin necesidad de anotarlo:

```rust
use corvis_swagger::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, ApiSchema)]
#[serde(rename_all = "camelCase")]
#[api_schema(description = "Un gato del refugio")]
pub struct CatResponse {
    #[api_property(description = "Identificador", example = 42)]
    pub id: u32,
    pub name: String,
    pub owner_email: Option<String>,
    pub tags: Vec<String>,
    #[api_hide_property]
    pub internal_notes: String,
}
```

| Tipo del campo | Propiedad |
|---|---|
| `Option<T>` | `T` con `required: false` |
| `Vec<T>`, `HashSet<T>`, `[T; N]`... | Array de `T` |
| `Box<T>`, `Arc<T>` | `T` |
| Otro tipo con `ApiSchema` o `zod_dto!` | Referencia a su schema |

Se respetan `#[serde(rename)]`, `#[serde(rename_all)]` y `#[serde(skip)]`.

| Atributo | Dónde | Efecto |
|---|---|---|
| `#[api_property(...)]` | Campo | Opciones de la propiedad: `description`, `example`, `enum`, `minimum`, `type`, `required`... |
| `#[api_property_optional(...)]` | Campo | Igual, con `required: false` |
| `#[api_response_property(...)]` | Campo | Igual, con `readOnly: true` |
| `#[api_hide_property]` | Campo | Excluye el campo |
| `#[api_schema(name = "...", ...)]` | Struct | Nombre del schema en `components` y otras claves del schema (`description`...) |

Las claves de las opciones se escriben en snake_case o camelCase; se convierten a camelCase (`read_only` → `readOnly`). Los valores son expresiones de Rust (`example = 42`, `enum = ["a", "b"]`) salvo `type`, que recibe un tipo: `type = CatDto`, `type = [CatDto]` para un array o `type = "string"`.

::: warning
`#[derive(ApiSchema)]` no tiene tests propios en el repositorio todavía (el test de integración de `corvis-swagger` usa DTOs de `corvis::z`). Revisa el documento generado si dependes de opciones poco comunes.
:::

## Parámetros explícitos

Para añadir información que el tipo no da, usa `#[api_param]`, `#[api_query]` o `#[api_header]` con las mismas claves que el objeto de parámetro de OpenAPI:

```rust
#[get(":id")]
#[api_param(name = "id", description = "Id del gato", example = "abc")]
#[api_query(name = "fields", required = false, enum = ["name", "age"], is_array = true)]
#[api_header(name = "X-Tenant", required = true)]
fn one(&self, #[param("id")] id: String) -> Value { /* ... */ }
```

Un `#[api_param]` con el mismo nombre que un `#[param]` se fusiona con él. Como en los generadores del ecosistema de Node.js, el `example` de un parámetro explícito pasa al `schema`:

```json
[{ "name": "id", "required": true, "in": "path", "schema": { "example": "abc", "type": "string" } }]
```

## Tipos propios

Para un tipo escalar propio (un decimal, un identificador) que debe aparecer como `string`, implementa `SwaggerType`:

```rust
use corvis_swagger::{SwaggerType, ctor_marker};

pub struct Money(String);

impl SwaggerType for Money {
    fn swagger_type() -> serde_json::Value {
        ctor_marker("String")
    }
}
```

`ctor_marker` recibe el nombre de un constructor de JavaScript, como `"String"`, `"Number"` o `"Boolean"`: son los mismos marcadores que usa la crate para los tipos primitivos de Rust.

## Modelos adicionales

Un schema que no se referencia desde ninguna ruta no aparece en `components`. Para incluirlo, usa `#[api_extra_models(A, B)]` en un controlador o handler, o `extra_models` en `SwaggerDocumentOptions` (ver [Otras características](/openapi/other-features)).

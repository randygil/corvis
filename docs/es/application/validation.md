---
status: documented
---

# Validación

Todo dato que entra por HTTP es sospechoso hasta que se comprueba. Corvis ofrece dos formas de validar los argumentos de un handler, y pueden convivir en la misma aplicación:

- **DTOs con atributos** (`#[derive(Validate)]` + `ValidationPipe`): reglas declarativas sobre cada campo, con los mensajes de class-validator.
- **Schemas** (`corvis::z` + `ZodValidationPipe`): un port de zod 4 que valida, transforma y aplica valores por defecto.

Ambos son [pipes](/es/pipes): reciben el valor JSON del argumento antes de que se deserialice en el tipo del parámetro y pueden rechazarlo con un `400`.

## DTOs con `#[derive(Validate)]`

```rust
use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(40)]
    pub age: u32,
    #[is_string]
    #[is_in("persian", "siamese", "maine coon", "mixed")]
    pub breed: String,
}
```

(Tomado de `examples/cats-app/src/cats/dto/mod.rs`.) Activa la validación con un pipe global en `main`:

```rust
app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

Con eso, cualquier `#[body]`, `#[query]` o `#[param]` cuyo tipo derive `Validate` se comprueba antes de llegar al handler. Si falla, la respuesta es:

```json
{ "message": ["name should not be empty", "age must not be less than 0"], "error": "Bad Request", "statusCode": 400 }
```

Los tipos que no derivan `Validate` pasan sin cambios.

### Reglas disponibles

| Grupo | Atributos |
|---|---|
| Tipo | `is_string`, `is_int`, `is_number`, `is_boolean`, `is_array`, `is_object`, `is_enum` |
| Presencia | `is_defined`, `is_optional`, `is_not_empty`, `is_empty` |
| Formato de texto | `is_email`, `is_url`, `is_uuid`, `is_alpha`, `is_alphanumeric`, `is_number_string` (alias `is_numeric_string`), `is_date_string` (alias `is_iso8601`), `is_hex_color`, `is_jwt`, `is_mongo_id`, `is_ip`, `is_lowercase`, `is_uppercase`, `matches("regex")` |
| Longitud | `min_length(n)`, `max_length(n)`, `length(min, max)` |
| Números | `min(n)`, `max(n)`, `is_positive`, `is_negative` |
| Contenido | `contains("s")`, `not_contains("s")`, `is_in(a, b, ...)`, `is_not_in(a, b, ...)`, `equals(v)`, `not_equals(v)` |
| Arreglos | `array_min_size(n)`, `array_max_size(n)`, `array_not_empty`, `array_unique` |
| Composición | `validate_nested`, `validate_if(f)`, `validate_with(f)` |

Detalles útiles:

- **Mensaje propio**: cualquier regla acepta `message = "..."`, por ejemplo `#[min_length(3, message = "too short")]`.
- **Cada elemento**: `each` aplica la regla a cada elemento de un arreglo (`#[max_length(20, each)]`); el mensaje empieza con `each value in ...`.
- **Opcionales**: `#[is_optional]` omite las demás reglas si el campo falta o es `null`. Úsalo con campos `Option<T>`.
- **Reglas implícitas**: un campo sin atributos se comprueba según su tipo de Rust (texto, entero, número, booleano).
- **Anidados**: `#[validate_nested]` valida un campo cuyo tipo también deriva `Validate` (o cada elemento de un `Vec` de ese tipo).
- **Condición**: `#[validate_if(|obj| ...)]` recibe el objeto JSON completo (`&serde_json::Map<String, Value>`) y solo valida el campo si devuelve `true`.
- **Regla libre**: `#[validate_with(f)]` recibe el valor del campo (`Option<&Value>`) y devuelve `bool`; el mensaje por defecto es `<campo> is invalid`.
- **Enums**: `#[is_enum]` comprueba que el valor sea una de las variantes de un enum que implementa `Deserialize` (el del propio campo o el indicado: `#[is_enum(Role)]`).
- Los nombres de los campos en los mensajes respetan `#[serde(rename = "...")]` y `#[serde(rename_all = "...")]`.

### Opciones del pipe

`ValidationPipe::new()` se configura con métodos encadenados (o con `ValidationPipe::with_options(ValidationPipeOptions { .. })`):

| Método | Efecto |
|---|---|
| `whitelist(true)` | Elimina las propiedades que el DTO no declara |
| `forbid_non_whitelisted(true)` | En lugar de eliminarlas, responde error |
| `skip_missing_properties(true)` | No valida propiedades ausentes o `null` |
| `stop_at_first_error(true)` | Deja de validar una propiedad tras su primer error |
| `disable_error_messages(true)` | Responde sin la lista de mensajes |
| `error_http_status_code(422)` | Cambia el código de estado (por defecto `400`) |
| `exception_factory(f)` | `fn(Vec<ValidationError>) -> HttpException` para construir tu propia respuesta |
| `transform(bool)` | Se acepta por compatibilidad; en Corvis la conversión al tipo declarado siempre ocurre |

En `#[query]` y `#[param]` el pipe convierte primero los textos numéricos y booleanos según el tipo del campo (`"2"` pasa a `2` en un `u32`). En `#[body]` no hay conversión: un JSON con `"age": "2"` falla en `#[is_int]`.

Para validar un DTO ya construido fuera de un handler, usa `corvis::validation::validate(&dto)`, que devuelve la lista de `ValidationError`.

## Schemas con `corvis::z`

`corvis::z` reproduce la API de zod 4 con funciones de Rust. Un DTO se asocia a su schema con la macro `zod_dto!`:

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRoomDto {
    pub name: String,
    pub topic: String,
    pub max_members: i64,
}

zod_dto!(
    CreateRoomDto,
    z::strict_object! {
        name: z::string().min(1).max(24).regex("^[a-z0-9][a-z0-9-]*$"),
        topic: z::string().trim().max(120).default(""),
        maxMembers: z::int().min(1.0).max(500.0).default(50),
    }
);
```

(Adaptado de `examples/tertulia/src/rooms/dto.rs`.) Con `ZodValidationPipe::auto()` como pipe global, cada parámetro cuyo tipo tiene schema se valida, y lo que se deserializa en el struct es la **salida** del schema: con los `default` aplicados, los textos recortados y las claves desconocidas eliminadas o rechazadas según el tipo de objeto.

```rust
app.use_global_pipes(ZodValidationPipe::auto());
```

Un error responde así:

```json
{
  "statusCode": 400,
  "message": "Validation failed",
  "errors": [
    { "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true,
      "path": ["name"], "message": "Too small: expected string to have >=1 characters" }
  ]
}
```

Otras formas de usar el pipe:

- `ZodValidationPipe::new(schema)` valida un argumento concreto con un schema explícito, por ejemplo para query strings: `#[query(ZodValidationPipe::new(z::object! { page: z::coerce::number().int().min(1.0).default(1) }))] q: ListQuery`.
- `ZodValidationPipe::dto::<Dto>()` usa el schema de otro DTO.
- `z::create_zod_validation_pipe(ZodValidationPipeOptions::new().create_validation_exception(|e| ...).strict_schema_declaration(true))` personaliza la excepción y, con `strict_schema_declaration`, rechaza (500) los parámetros cuyo tipo no tiene schema.

### Query strings y `coerce`

Los valores de la query llegan como texto. Usa `z::coerce::number()` para convertirlos antes de validar. Ten cuidado con `z::coerce::boolean()`: sigue la regla de JavaScript (`Boolean(x)`), así que `"false"` da `true`. Para banderas de texto usa `z::stringbool()`.

### Validar sin pipe

Un schema se puede usar directamente:

```rust
let schema = z::object! { name: z::string().min(2), email: z::email() };
let result = schema.safe_parse(&json!({ "name": "A" }));
if let Some(error) = result.error {
    let flat = z::flatten_error(&error); // { formErrors, fieldErrors }
}
```

`parse`, `safe_parse`, `parse_as::<T>` (valida y deserializa), `flatten_error`, `treeify_error` y `prettify_error` están disponibles. El catálogo completo de tipos y métodos sigue la API de zod 4; la tabla de equivalencias está en el `README.md` del repositorio.

### Mensajes en español

Los mensajes de zod usan el locale `en` por defecto. `corvis::z` incluye también `es`:

```rust
z::configure(|c| c.locale_error = Some(z::locales::es()));
```

## Qué ven los guards y los interceptors

Los pipes validan el argumento del handler; no reemplazan `req.body()`. Un guard o un interceptor que lee el cuerpo ve el valor **sin validar**. Si un guard necesita datos ya validados, valida ahí mismo o mueve la lógica al handler.

## Límites de seguridad

- Los schemas recursivos (`z::lazy`, `z::json()`) admiten como mucho 128 niveles por parse; se cambia con `z::configure(|c| c.max_depth = n)`.
- Los `regex` sin lookarounds ni backreferences corren en tiempo lineal; los que los usan tienen un tope de pasos. Evítalos en reglas que reciben texto del cliente.
- `z::url()` acepta cualquier esquema (`javascript:` incluido); para enlaces que se van a mostrar usa `z::http_url()`.

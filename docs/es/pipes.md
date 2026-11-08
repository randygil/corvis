---
status: documented
---

# Pipes

Un pipe procesa el valor de un parámetro del handler **antes** de que el handler se ejecute. Sirve para dos cosas:

- **transformar** el valor (el texto `"42"` de la URL en el número `42`, un valor ausente en uno por defecto), y
- **validar**: si el valor no es aceptable, el pipe devuelve un error y el handler no llega a ejecutarse.

## Cómo viaja el valor

En Corvis los pipes trabajan sobre `serde_json::Value`. Para cada parámetro con atributo (`#[body]`, `#[param]`, `#[query]`, `#[headers]`, `#[ip]`, `#[uploaded_file]`, ...):

1. Corvis extrae el valor crudo de la petición como JSON (`null` si falta).
2. Lo pasa por los pipes que le corresponden, en orden. Cada pipe recibe la salida del anterior.
3. Deserializa el resultado final en el tipo declarado del parámetro.

Los parámetros que se resuelven con `FromContext` (sin atributo, o con `#[req]`) no pasan por pipes.

## Pipes incluidos

| Pipe | Qué hace | Error |
|---|---|---|
| `ParseIntPipe` | Convierte un string numérico en entero | `Validation failed (numeric string is expected)` |
| `ParseFloatPipe` | Convierte un string numérico en número | `Validation failed (numeric string is expected)` |
| `ParseBoolPipe` | `"true"`/`"false"` a booleano | `Validation failed (boolean string is expected)` |
| `ParseUUIDPipe` (alias `ParseUuidPipe`) | Valida un UUID | `Validation failed (uuid is expected)`; con `.version(4)`, `Validation failed (uuid v 4 is expected)`; un valor que no es string, `The value passed as UUID is not a string` |
| `ParseArrayPipe` | Parte `"1,2,3"` en un array | `Validation failed (parsable array expected)` |
| `ParseEnumPipe` | Acepta solo los valores dados | `Validation failed (enum string is expected)` |
| `DefaultValuePipe` | Pone un valor cuando falta | — |
| `ValidationPipe` | Valida DTOs con `#[derive(Validate)]` | Ver [Validación](/es/application/validation) |
| `ZodValidationPipe` | Valida con schemas de `corvis::z` | Ver [Validación](/es/application/validation) |
| `ParseFilePipe` | Valida archivos subidos | Ver [Subida de archivos](/es/http/file-upload) |

Todos responden `400 Bad Request` con el formato estándar:

```json
{ "message": "Validation failed (numeric string is expected)", "error": "Bad Request", "statusCode": 400 }
```

### Opciones

`ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe` y `ParseUUIDPipe` comparten:

- `.optional()`: deja pasar `null` (parámetro ausente) sin error.
- `.error_http_status_code(406)`: cambia el estado del error.

Además:

```rust
ParseUUIDPipe::new().version(4)           // solo UUID v4 (acepta 3, 4, 5 o 7)
ParseArrayPipe::new().separator(";").numbers().optional()
ParseEnumPipe::new(["persian", "siamese"]) // valores explícitos
ParseEnumPipe::of::<Breed>()               // variantes de un enum con serde
DefaultValuePipe::new(1)                   // reemplaza null, "" y NaN
```

## Usar un pipe en un parámetro

Los pipes se listan en el atributo del parámetro, después del nombre:

```rust
#[get(":id")]
fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
    self.cats_service.find_one(id)
}
```

Puedes pasar el **tipo** del pipe (se resuelve con inyección de dependencias) o una **expresión** que construya la instancia, por ejemplo para configurarlo:

```rust
#[get(":id")]
fn find_one(&self, #[param("id", ParseIntPipe::new().error_http_status_code(406))] id: u32) -> HttpResult<Cat> { ... }

#[get]
fn list(&self, #[query("page", DefaultValuePipe::new(1), ParseIntPipe)] page: u32) -> Vec<Cat> { ... }
```

En el segundo ejemplo el orden importa: `DefaultValuePipe` pone `1` si falta `page` y después `ParseIntPipe` convierte el valor.

::: tip Pipes por tipo
Los pipes referenciados por tipo deben implementar `Injectable` (los tuyos, con `#[injectable]`). `ParseIntPipe`, `ParseFloatPipe`, `ParseBoolPipe`, `ParseUUIDPipe`, `ParseArrayPipe`, `ValidationPipe`, `ZodValidationPipe` y `ParseFilePipe` ya lo hacen. `ParseEnumPipe` y `DefaultValuePipe` necesitan argumentos, así que se usan siempre como expresión.
:::

Sin pipe, los parámetros de ruta, query y cabeceras ya se convierten al tipo declarado cuando es posible (`#[param("id")] id: u32` funciona con `/cats/42`). La diferencia es el mensaje de error: un pipe produce el mensaje estándar de validación y la deserialización produce el de `serde`.

## Pipes en otros niveles

| Alcance | Forma |
|---|---|
| Un parámetro | `#[param("id", ParseIntPipe)]` |
| Todos los parámetros de una ruta | `#[use_pipes(...)]` sobre el método |
| Todas las rutas de un controlador | `#[use_pipes(...)]` debajo de `#[controller]` |
| Toda la aplicación | `app.use_global_pipes(ValidationPipe::new())` |
| Toda la aplicación, con DI | `Provider::app_pipe::<MiPipe>()` en `providers` |

Para cada parámetro el orden es: globales (primero los registrados como provider, después los de `use_global_pipes`), de controlador, de ruta y, al final, los del propio parámetro.

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.use_global_pipes(ValidationPipe::new().whitelist(true).forbid_non_whitelisted(true));
```

Los pipes de alcance amplio se aplican a **todos** los parámetros con atributo. Por eso los pipes de validación revisan el tipo del parámetro y dejan pasar los que no les corresponden: `ValidationPipe` solo actúa sobre tipos con `#[derive(Validate)]` y `ZodValidationPipe::auto()` solo sobre tipos declarados con `zod_dto!`. Pueden convivir.

Si fallan varios parámetros, gana el error del pipe que falla antes en su cadena (los globales primero) y, si empatan, el del último parámetro declarado.

## Pipes propios

Un pipe implementa `PipeTransform`:

```rust
#[async_trait]
pub trait PipeTransform: Send + Sync + 'static {
    async fn transform(&self, value: Value, metadata: &ArgumentMetadata) -> Result<Value, HttpException>;
}
```

`ArgumentMetadata` describe el parámetro:

| Campo | Contenido |
|---|---|
| `kind` | `ParamKind::Body`, `Query`, `Param`, `Headers`, `Ip`, `Custom`, `File` o `Files` |
| `data` | El nombre pasado al atributo (`Some("id")` en `#[param("id")]`) |
| `metatype.name` | El tipo declarado, como texto (`"u32"`, `"CreateCatDto"`) |
| `metatype.validator` | Presente si el tipo deriva `Validate` |
| `metatype.zod` | Presente si el tipo tiene schema `zod_dto!` |

Un pipe que normaliza un texto y rechaza los vacíos:

```rust
use corvis::prelude::*;

#[injectable]
pub struct TrimPipe;

#[async_trait]
impl PipeTransform for TrimPipe {
    async fn transform(&self, value: Value, meta: &ArgumentMetadata) -> Result<Value, HttpException> {
        match value {
            Value::String(s) if !s.trim().is_empty() => Ok(Value::String(s.trim().to_string())),
            Value::String(_) => Err(BadRequestException::new(format!("{} must not be empty", meta.data.unwrap_or("value")))),
            other => Ok(other),
        }
    }
}
```

```rust
#[get("search")]
fn search(&self, #[query("q", TrimPipe)] q: String) -> Vec<Cat> { ... }
```

Como el pipe es `#[injectable]`, puede tener dependencias (`Arc<ConfigService>`, un repositorio, ...). Se resuelven en el módulo del controlador que lo usa.

## Validar el cuerpo

Los casos más comunes de validación de DTOs ya están cubiertos por dos pipes:

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    #[max_length(50)]
    pub name: String,
    #[is_int]
    #[min(0)]
    pub age: u32,
}
```

```rust
use corvis::zod_dto;

#[derive(Debug, Deserialize)]
pub struct LoginDto {
    pub username: String,
}

zod_dto!(
    LoginDto,
    z::strict_object! {
        username: z::string().trim().min(2).max(20).regex("^[A-Za-z0-9_.-]+$"),
    }
);
```

Con `ValidationPipe` o `ZodValidationPipe::auto()` como pipe global, basta con declarar el parámetro `#[body] dto: CreateCatDto`. Todas las reglas y opciones están en [Validación](/es/application/validation).

::: info
Los guards y los interceptors ven el cuerpo **sin validar**: los pipes validan el argumento del handler, y su salida no reemplaza `req.body()`.
:::

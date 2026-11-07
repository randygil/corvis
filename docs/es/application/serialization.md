---
status: documented
---

# Serialización

Serializar es decidir qué forma tiene el JSON que sale de un handler: qué campos se envían, con qué nombres y con qué tipos. En Corvis el valor de retorno de un handler se convierte con `serde`, así que buena parte de esta tarea se resuelve con los atributos de `serde` en tus tipos. Para los casos en que la forma de la respuesta debe cumplir un contrato, existe un interceptor que la pasa por un schema de `corvis::z`.

## Con `serde`

Cualquier tipo que implemente `Serialize` se puede devolver desde un handler. Los atributos de `serde` cubren las necesidades habituales:

```rust
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    pub id: i64,
    pub display_name: String,          // sale como "displayName"
    #[serde(skip_serializing)]
    pub password_hash: String,         // nunca se envía
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,    // se omite si es None
}
```

Una práctica sana es separar la entidad de la base de datos del tipo que devuelves, y construir el segundo a partir del primero con un `From`. Así un campo nuevo en la tabla no se filtra en la API por accidente.

Los modelos de `corvis-orm` usan la misma convención: `#[serde(rename_all = "camelCase")]` para las columnas y `#[serde(default, skip_serializing_if = "Option::is_none")]` en las relaciones, para que una relación no incluida no aparezca en el JSON (ver [corvis-orm](/es/data/corvis-orm)).

## Transformar la respuesta en un interceptor

Un [interceptor](/es/interceptors) recibe la respuesta del handler y puede reescribir su cuerpo JSON con `map_data`. `examples/cats-app` envuelve así todas las respuestas de una ruta:

```rust
#[injectable]
pub struct TransformInterceptor;

#[async_trait]
impl CorvisInterceptor for TransformInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
    }
}
```

## Respuestas validadas con un schema

Cuando un DTO declara su schema con `zod_dto!`, puedes exigir que la respuesta cumpla ese schema. El interceptor `z::ZodSerializerInterceptor` lee la metadata `z::ZOD_SERIALIZER_DTO_OPTIONS` del handler (o del controlador) y pasa el cuerpo por el schema antes de enviarlo:

- Las claves que el schema no declara se eliminan (con `z::object!`), así que un campo interno no sale aunque el handler lo devuelva.
- Si el valor no cumple el schema, la respuesta es un `500` (`ZodSerializationException`) y el error de zod queda como causa para que lo registres.

```rust
use corvis::prelude::*;
use corvis::zod_dto;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDto {
    pub id: u32,
    pub email: String,
}

zod_dto!(UserDto, z::object! { id: z::int(), email: z::email() });

#[routes]
impl UsersController {
    #[get("me")]
    #[set_metadata(z::ZOD_SERIALIZER_DTO_OPTIONS, z::ZodSerializerDto::dto::<UserDto>())]
    fn me(&self) -> Value {
        // `passwordHash` se elimina de la respuesta.
        json!({ "id": 1, "email": "a@b.co", "passwordHash": "x" })
    }
}

// En main:
app.use_global_interceptors(z::ZodSerializerInterceptor::new());
```

(Basado en `crates/corvis/tests/zod_pipe.rs`.) Variantes de la metadata:

| Valor | Uso |
|---|---|
| `ZodSerializerDto::dto::<T>()` | La respuesta es un `T` |
| `ZodSerializerDto::dto_array::<T>()` | La respuesta es un arreglo de `T` |
| `ZodSerializerDto::schema(s)` / `schema_array(s)` | Un schema suelto, sin DTO |

`ZodSerializerInterceptor::create(Some(true))` incluye el valor de entrada en los issues del error (`reportInput`). Las respuestas que no son JSON (archivos, streams) pasan sin cambios.

Si el DTO se declaró como codec (`zod_dto!(Dto, schema, codec = true)`), el interceptor usa `encode` en lugar de `parse`, de modo que las transformaciones del codec se aplican en sentido inverso al serializar.

Con `corvis-swagger`, `corvis_swagger::decorators::zod_response::<Dto>(Some(201), None)` combina en un solo atributo el código de estado, esta metadata y la respuesta documentada en OpenAPI (ver [OpenAPI](/es/openapi/introduction)).

## Lo que no existe

No hay un equivalente de los decoradores `@Exclude()` / `@Expose()` de class-transformer ni de grupos de serialización. Usa los atributos de `serde`, tipos de salida separados o un schema con `ZodSerializerInterceptor`.

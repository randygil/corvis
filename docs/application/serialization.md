---
status: documented
---

# Serialization

Serializing means deciding what shape the JSON leaving a handler has: which fields are sent, with what names and with what types. In Corvis a handler's return value is converted with `serde`, so a good part of this task is solved with `serde` attributes on your types. For cases where the shape of the response must meet a contract, there is an interceptor that runs it through a `corvis::z` schema.

## With `serde`

Any type that implements `Serialize` can be returned from a handler. `serde` attributes cover the usual needs:

```rust
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    pub id: i64,
    pub display_name: String,          // comes out as "displayName"
    #[serde(skip_serializing)]
    pub password_hash: String,         // never sent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,    // omitted if None
}
```

A healthy practice is to separate the database entity from the type you return, and build the second from the first with a `From`. That way a new column in the table does not leak into the API by accident.

The `corvis-orm` models use the same convention: `#[serde(rename_all = "camelCase")]` for columns and `#[serde(default, skip_serializing_if = "Option::is_none")]` on relations, so a relation that was not included does not appear in the JSON (see [corvis-orm](/data/corvis-orm)).

## Transforming the response in an interceptor

An [interceptor](/interceptors) receives the handler's response and can rewrite its JSON body with `map_data`. `examples/cats-app` wraps all responses of a route this way:

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

## Responses validated with a schema

When a DTO declares its schema with `zod_dto!`, you can require the response to meet that schema. The `z::ZodSerializerInterceptor` interceptor reads the `z::ZOD_SERIALIZER_DTO_OPTIONS` metadata from the handler (or the controller) and runs the body through the schema before sending it:

- Keys that the schema does not declare are removed (with `z::object!`), so an internal field does not go out even if the handler returns it.
- If the value does not meet the schema, the response is a `500` (`ZodSerializationException`) and the zod error is kept as the cause so you can log it.

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
        // `passwordHash` is removed from the response.
        json!({ "id": 1, "email": "a@b.co", "passwordHash": "x" })
    }
}

// In main:
app.use_global_interceptors(z::ZodSerializerInterceptor::new());
```

(Based on `crates/corvis/tests/zod_pipe.rs`.) Metadata variants:

| Value | Use |
|---|---|
| `ZodSerializerDto::dto::<T>()` | The response is a `T` |
| `ZodSerializerDto::dto_array::<T>()` | The response is an array of `T` |
| `ZodSerializerDto::schema(s)` / `schema_array(s)` | A standalone schema, without a DTO |

`ZodSerializerInterceptor::create(Some(true))` includes the input value in the error's issues (`reportInput`). Responses that are not JSON (files, streams) pass through unchanged. A handler result with no value (an empty body) is validated as `null`, so the schema must accept `null` (`.nullable()`) for handlers that can return `None`.

If the DTO was declared as a codec (`zod_dto!(Dto, schema, codec = true)`), the interceptor uses `encode` instead of `parse`, so the codec's transformations are applied in reverse when serializing.

With `corvis-swagger`, `corvis_swagger::decorators::zod_response::<Dto>(Some(201), None)` combines in a single attribute the status code, this metadata and the response documented in OpenAPI (see [OpenAPI](/openapi/introduction)).

## What does not exist

There is no equivalent of class-transformer's `@Exclude()` / `@Expose()` decorators or of serialization groups. Use `serde` attributes, separate output types or a schema with `ZodSerializerInterceptor`.

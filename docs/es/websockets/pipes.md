---
status: documentado
---

# Pipes

Los [pipes](/es/pipes) de un gateway funcionan igual que en HTTP: reciben el valor de un argumento como `serde_json::Value`, lo validan o lo transforman y devuelven el resultado, que después se deserializa en el tipo del parámetro. La diferencia está en qué argumentos procesan: en un gateway solo hay uno con datos de entrada, el `data` del frame.

## Dónde se declaran

| Nivel | Cómo | Se aplica a |
|---|---|---|
| Gateway | `#[use_pipes(...)]` debajo de `#[websocket_gateway]` | Cada `#[message_body]` de todos los handlers |
| Handler | `#[use_pipes(...)]` encima del método | Cada `#[message_body]` de ese handler |
| Parámetro | `#[message_body("prop", MiPipe)]` | Solo ese argumento |

El orden es: pipes del gateway, después los del handler y por último los del parámetro. Los pipes se indican por tipo (`ParseIntPipe`), y entonces se instancian con inyección de dependencias en el contexto del módulo del gateway, o como expresión (`ZodValidationPipe::auto()`).

Los pipes globales (`app.use_global_pipes`, `Provider::app_pipe`) **no** se aplican a los gateways.

## Validar los mensajes

Tertulia valida todos los mensajes del chat con DTOs de `corvis::z` declarados con `zod_dto!` (`src/chat/dto.rs`):

```rust
use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

pub fn room_name() -> z::ZString {
    z::string().min(1).max(24).regex("^[a-z0-9][a-z0-9-]*$")
}

#[derive(Debug, Deserialize)]
pub struct SendMessageDto {
    pub room: String,
    pub text: String,
}

zod_dto!(
    SendMessageDto,
    z::strict_object! {
        room: room_name(),
        text: z::string().trim().min(1).max(500),
    }
);
```

El gateway activa la validación para todos sus handlers:

```rust
#[websocket_gateway(path = "/chat")]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway { /* ... */ }
```

`ZodValidationPipe::auto()` valida cada argumento cuyo tipo tiene un schema (`zod_dto!`) y deja pasar los demás. Como el pipe devuelve la salida del schema, el handler recibe el texto ya recortado por `trim()`.

Cuando la validación falla, el cliente recibe un frame `exception` con los issues de zod:

```json
{
  "event": "exception",
  "data": {
    "status": "error",
    "message": "Validation failed",
    "statusCode": 400,
    "errors": [{ "code": "too_small", "path": ["text"], "message": "..." }]
  }
}
```

`ValidationPipe` (los DTOs con `#[derive(Validate)]`) también funciona en gateways; ver [Validación](/es/application/validation).

## Una propiedad con su propio pipe

```rust
#[subscribe_message("kick")]
fn kick(&self, #[message_body("id", ParseIntPipe)] id: i64) -> HttpResult<()> {
    // ...
    Ok(())
}
```

`#[message_body("id")]` toma solo `data.id` y le aplica los pipes que siguen al nombre.

## Guards antes que pipes

Los guards corren **antes** que los pipes, así que ven el payload sin validar. Si un guard necesita un campo, debe leerlo de `ctx.switch_to_ws().get_data()` y tolerar que falte o tenga otra forma; el `RoomMemberGuard` de tertulia deja pasar los mensajes sin `room` para que sea el pipe quien informe el error. Ver [Guards](/es/websockets/guards).

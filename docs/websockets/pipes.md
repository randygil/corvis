---
status: documented
---

# Pipes

A gateway's [pipes](/pipes) work the same as in HTTP: they receive an argument's value as a `serde_json::Value`, validate or transform it, and return the result, which is then deserialized into the parameter's type. The difference is in which arguments they process: in a gateway there is only one with input data, the frame's `data`.

Pipes only run on `#[message_body]` (and `#[payload]`) parameters. `#[connected_socket]` and `#[ip]` parameters come from the server, not from the frame, so no pipe runs on them.

## Where they are declared

| Level | How | Applies to |
|---|---|---|
| Gateway | `#[use_pipes(...)]` below `#[websocket_gateway]` | Every `#[message_body]` of all handlers |
| Handler | `#[use_pipes(...)]` on top of the method | Every `#[message_body]` of that handler |
| Parameter | `#[message_body("prop", MyPipe)]` | That argument only |

The order is: the global pipes, the gateway's pipes, then the handler's, and finally the parameter's. Pipes are given by type (`ParseIntPipe`), and then they are instantiated with dependency injection in the context of the gateway's module, or as an expression (`ZodValidationPipe::auto()`).

Global pipes (`app.use_global_pipes`, `Provider::app_pipe`) apply to gateways too, to every `#[message_body]` before the other pipes. Repeating a global validation pipe on the gateway runs it twice, which is harmless. `app.apply_global_enhancers_to_gateways(false)` keeps global pipes HTTP-only (see [Gateways](/websockets/gateways#global-enhancers)).

## Validating messages

Tertulia validates all chat messages with `corvis::z` DTOs declared with `zod_dto!` (`src/chat/dto.rs`):

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

The gateway enables validation for all its handlers:

```rust
#[websocket_gateway(path = "/chat")]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway { /* ... */ }
```

`ZodValidationPipe::auto()` validates every argument whose type has a schema (`zod_dto!`) and lets the others through. Since the pipe returns the schema's output, the handler receives the text already trimmed by `trim()`.

When validation fails, the client receives an `exception` frame with the zod issues:

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

`ValidationPipe` (DTOs with `#[derive(Validate)]`) also works in gateways; see [Validation](/application/validation).

## One property with its own pipe

```rust
#[subscribe_message("kick")]
fn kick(&self, #[message_body("id", ParseIntPipe)] id: i64) -> HttpResult<()> {
    // ...
    Ok(())
}
```

`#[message_body("id")]` takes only `data.id` and applies the pipes that follow the name.

## Guards before pipes

Guards run **before** pipes, so they see the unvalidated payload. If a guard needs a field, it must read it from `ctx.switch_to_ws().get_data()` and tolerate it being missing or having a different shape; Tertulia's `RoomMemberGuard` lets messages without `room` through so that the pipe is the one to report the error. See [Guards](/websockets/guards).

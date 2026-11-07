//! Payloads of the chat messages, validated by `ZodValidationPipe::auto()` (declared on the
//! gateway). A failure answers an `exception` frame with the zod issues:
//! `{ status: "error", message: "Validation failed", statusCode: 400, errors: [...] }`.

use corvis::prelude::*;
use corvis::zod_dto;
use serde::Deserialize;

/// Room names: lowercase slugs (`general`, `rust-es`).
pub fn room_name() -> z::ZString {
    z::string().min(1).max(24).regex("^[a-z0-9][a-z0-9-]*$")
}

#[derive(Debug, Deserialize)]
pub struct RoomDto {
    pub room: String,
}

zod_dto!(RoomDto, z::strict_object! { room: room_name() });

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

#[derive(Debug, Deserialize)]
pub struct TypingDto {
    pub room: String,
    pub typing: bool,
}

zod_dto!(TypingDto, z::strict_object! { room: room_name(), typing: z::boolean() });

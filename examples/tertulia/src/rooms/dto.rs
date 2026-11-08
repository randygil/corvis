use corvis::prelude::*;
use corvis::zod_dto;
use serde::{Deserialize, Serialize};

use crate::auth::ChatUser;
use crate::chat::room_name;
use crate::models::Room;

/// A room with the users connected to it right now (`GET /api/rooms` and the WebSocket `rooms` message).
#[derive(Debug, Serialize)]
pub struct RoomWithMembers {
    #[serde(flatten)]
    pub room: Room,
    pub members: Vec<ChatUser>,
}

/// Answer of `POST /api/rooms/:room/announcements`.
#[derive(Debug, Serialize)]
pub struct Delivery {
    pub room: String,
    pub delivered: usize,
}

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
        name: room_name(),
        topic: z::string().trim().max(120).default(""),
        maxMembers: z::int().min(1.0).max(500.0).default(50),
    }
);

#[derive(Debug, Deserialize)]
pub struct AnnouncementDto {
    pub text: String,
}

zod_dto!(AnnouncementDto, z::strict_object! { text: z::string().trim().min(1).max(280) });

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

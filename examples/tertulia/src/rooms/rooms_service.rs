use corvis::prelude::*;
use corvis_orm::{OrmError, OrmService};

use super::dto::CreateRoomDto;
use crate::models::{ChatMessage, ChatMessageDelegate, Room, RoomDelegate};

/// Rooms created on startup when missing: (name, topic).
pub const DEFAULT_ROOMS: [(&str, &str); 3] = [
    ("general", "Anything goes"),
    ("rust", "Ownership, lifetimes and other joys"),
    ("random", "Memes and off-topic"),
];

/// Rooms and message history (SQLite through corvis-orm). Injected into the gateway and
/// into the REST controller.
#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn list(&self) -> Result<Vec<Room>, OrmError> {
        self.db.room().find_many(json!({ "orderBy": [{ "createdAt": "asc" }, { "name": "asc" }] })).await
    }

    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(json!({ "where": { "name": name } })).await
    }

    /// Creates a room (P2002 -> 409 if the name is taken) and emits `room.created`.
    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let room: Room = self
            .db
            .room()
            .create(json!({
                "data": { "name": dto.name, "topic": dto.topic, "maxMembers": dto.max_members, "createdBy": created_by }
            }))
            .await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }

    /// The last `limit` messages of a room, oldest first.
    pub async fn history(&self, room: &str, limit: i64) -> Result<Vec<ChatMessage>, OrmError> {
        let mut messages: Vec<ChatMessage> = self
            .db
            .chat_message()
            .find_many(json!({ "where": { "room": room }, "orderBy": { "id": "desc" }, "take": limit }))
            .await?;
        messages.reverse();
        Ok(messages)
    }

    pub async fn save_message(&self, room: &str, username: &str, text: &str) -> Result<ChatMessage, OrmError> {
        self.db.chat_message().create(json!({ "data": { "room": room, "username": username, "text": text } })).await
    }

    /// Deletes the history of a room; returns how many messages were removed.
    pub async fn clear(&self, room: &str) -> Result<i64, OrmError> {
        let result = self.db.chat_message().delete_many(json!({ "where": { "room": room } })).await?;
        Ok(result.count)
    }

    pub async fn message_count(&self) -> Result<i64, OrmError> {
        self.db.chat_message().count(json!({})).await
    }
}

#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db
                    .room()
                    .create(json!({ "data": { "name": name, "topic": topic, "createdBy": "tertulia" } }))
                    .await?;
            }
        }
        Ok(())
    }
}

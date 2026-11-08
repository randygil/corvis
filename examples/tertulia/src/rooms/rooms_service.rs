use corvis::prelude::*;
use corvis_orm::{Data, OrmError, OrmService, SortOrder};
use serde::Serialize;

use super::dto::CreateRoomDto;
use crate::models::{ChatMessage, ChatMessageDelegate, Room, RoomDelegate, chat_message, room};

/// Rooms created on startup when missing: (name, topic).
pub const DEFAULT_ROOMS: [(&str, &str); 3] =
    [("general", "Anything goes"), ("rust", "Ownership, lifetimes and other joys"), ("random", "Memes and off-topic")];

/// The data of a new room; `max_members` is left out when absent so the column default (50) applies.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewRoom<'a> {
    name: &'a str,
    topic: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_members: Option<i64>,
    created_by: &'a str,
}

#[derive(Serialize)]
struct NewMessage<'a> {
    room: &'a str,
    username: &'a str,
    text: &'a str,
}

/// Rooms and message history (SQLite through corvis-orm). Injected into the gateway and
/// into the REST controller.
#[injectable]
pub struct RoomsService {
    db: Arc<OrmService>,
    events: Arc<EventEmitter>,
}

impl RoomsService {
    pub async fn list(&self) -> Result<Vec<Room>, OrmError> {
        self.db.room().find_many(()).order_by(room::created_at::order(SortOrder::Asc)).order_by(room::name::order(SortOrder::Asc)).await
    }

    pub async fn find(&self, name: &str) -> Result<Option<Room>, OrmError> {
        self.db.room().find_unique(room::name::equals(name)).await
    }

    /// Creates a room (P2002 -> 409 if the name is taken) and emits `room.created`.
    pub async fn create(&self, dto: CreateRoomDto, created_by: &str) -> Result<Room, OrmError> {
        let new = NewRoom { name: &dto.name, topic: &dto.topic, max_members: Some(dto.max_members), created_by };
        let room: Room = self.db.room().create(Data(&new)).await?;
        self.events.emit("room.created", room.clone());
        Ok(room)
    }

    /// The last `limit` messages of a room, oldest first.
    pub async fn history(&self, room: &str, limit: i64) -> Result<Vec<ChatMessage>, OrmError> {
        let mut messages: Vec<ChatMessage> = self
            .db
            .chat_message()
            .find_many(chat_message::room::equals(room))
            .order_by(chat_message::id::order(SortOrder::Desc))
            .take(limit)
            .await?;
        messages.reverse();
        Ok(messages)
    }

    pub async fn save_message(&self, room: &str, username: &str, text: &str) -> Result<ChatMessage, OrmError> {
        self.db.chat_message().create(Data(NewMessage { room, username, text })).await
    }

    /// Deletes the history of a room; returns how many messages were removed.
    pub async fn clear(&self, room: &str) -> Result<i64, OrmError> {
        let result = self.db.chat_message().delete_many(chat_message::room::equals(room)).await?;
        Ok(result.count)
    }

    pub async fn message_count(&self) -> Result<i64, OrmError> {
        self.db.chat_message().count(()).await
    }
}

#[async_trait]
impl OnApplicationBootstrap for RoomsService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        for (name, topic) in DEFAULT_ROOMS {
            if self.find(name).await?.is_none() {
                self.db.room().create(Data(NewRoom { name, topic, max_members: None, created_by: "tertulia" })).await?;
            }
        }
        Ok(())
    }
}

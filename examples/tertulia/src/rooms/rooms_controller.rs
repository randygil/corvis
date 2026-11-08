use corvis::prelude::*;

use super::dto::{AnnouncementDto, CreateRoomDto, Delivery, HistoryQuery, RoomWithMembers};
use super::rooms_service::RoomsService;
use crate::auth::{AuthGuard, ChatUser, MODERATOR, ROLES_KEY, RolesGuard};
use crate::chat::ChatGateway;
use crate::models::{ChatMessage, Room};

/// REST view of the chat. `RoomsService` is shared with the gateway; the gateway itself is
/// injected to read the live WebSocket state (members) and to push to it (announcements).
#[controller("rooms")]
pub struct RoomsController {
    rooms: Arc<RoomsService>,
    chat: Arc<ChatGateway>,
}

#[routes]
impl RoomsController {
    /// `GET /api/rooms`: every room with the users connected to it right now.
    #[get]
    #[public]
    async fn list(&self) -> HttpResult<Vec<RoomWithMembers>> {
        let rooms = self.rooms.list().await?;
        Ok(self.chat.with_members(rooms))
    }

    /// `POST /api/rooms { name, topic?, maxMembers? }`: WebSocket clients get `room_created`.
    #[post]
    #[use_guards(AuthGuard)]
    async fn create(&self, #[body] dto: CreateRoomDto, user: ChatUser) -> HttpResult<Room> {
        Ok(self.rooms.create(dto, &user.username).await?)
    }

    /// `GET /api/rooms/:room/messages?limit=20`: the persisted history.
    #[get(":room/messages")]
    #[public]
    async fn messages(&self, #[param("room")] room: String, #[query] q: HistoryQuery) -> HttpResult<Vec<ChatMessage>> {
        self.find(&room).await?;
        Ok(self.rooms.history(&room, q.limit.unwrap_or(50).clamp(1, 200)).await?)
    }

    /// `GET /api/rooms/:room/members`: read straight from the gateway's sockets.
    #[get(":room/members")]
    #[public]
    async fn members(&self, #[param("room")] room: String) -> HttpResult<Vec<ChatUser>> {
        self.find(&room).await?;
        Ok(self.chat.members(&room))
    }

    /// `POST /api/rooms/:room/announcements { text }` (moderators): pushed over WebSocket.
    #[post(":room/announcements")]
    #[set_metadata(ROLES_KEY, [MODERATOR])]
    #[use_guards(AuthGuard, RolesGuard)]
    async fn announce(&self, #[param("room")] room: String, #[body] dto: AnnouncementDto, user: ChatUser) -> HttpResult<Delivery> {
        self.find(&room).await?;
        let delivered = self.chat.announce(&room, &user, &dto.text);
        Ok(Delivery { room, delivered })
    }
}

impl RoomsController {
    async fn find(&self, room: &str) -> HttpResult<Room> {
        self.rooms.find(room).await?.ok_or_else(|| NotFoundException::new(format!("Room #{room} does not exist")))
    }
}

//! The chat gateway. Protocol (JSON text frames, `{ event, data, id? }`):
//!
//! | client -> server | reply | others in the room receive |
//! |---|---|---|
//! | `join { room }` | `joined { room, members, history }` | `user_joined { room, user, members }` |
//! | `leave { room }` | `left { room }` | `user_left { room, user, members }` |
//! | `message { room, text }` | `sent { ...message }` (the ack) | `message { ...message }` |
//! | `typing { room, typing }` | - | `typing { room, user, typing }` |
//! | `clear { room }` (moderators) | - | everyone in the room: `cleared { room, by }` |
//! | `rooms` | `rooms [ ...rooms with members ]` | - |
//! | `ping` | `ping "pong"` | - |
//!
//! Server pushes: `welcome` on connect, `room_created` (REST `POST /api/rooms`, to everyone),
//! `announcement` (REST `POST /api/rooms/:room/announcements`). A frame with an `id` gets the
//! same `id` back in its reply or `exception` frame.

use std::collections::BTreeMap;

use corvis::prelude::*;
use corvis::{
    JwtService, OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, WsClient, WsException, WsResponse, WsServer,
    extract_bearer_token, websocket_gateway,
};
use corvis_orm::OrmResultExt;

use super::dto::{RoomDto, SendMessageDto, TypingDto};
use super::room_member_guard::RoomMemberGuard;
use super::ws_logging_interceptor::WsLoggingInterceptor;
use crate::auth::{AuthGuard, ChatUser, MODERATOR, ROLES_KEY, RolesGuard};
use crate::firehose::FirehoseService;
use crate::models::Room;
use crate::rooms::RoomsService;

/// Close code sent to sockets without a valid token (4000-4999 are application codes).
pub const CLOSE_UNAUTHORIZED: u16 = 4401;

#[websocket_gateway(path = "/chat")]
#[use_guards(AuthGuard)]
#[use_interceptors(WsLoggingInterceptor)]
#[use_pipes(ZodValidationPipe::auto())]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
    jwt: Arc<JwtService>,
    rooms: Arc<RoomsService>,
    firehose: Arc<FirehoseService>,
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("join")]
    async fn join(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<WsResponse> {
        let user = current_user(&client)?;
        let room = self.rooms.find(&dto.room).await.http()?.ok_or_else(|| {
            WsException::object(json!({
                "status": "error",
                "code": "ROOM_NOT_FOUND",
                "message": format!("Room #{} does not exist", dto.room),
                "room": dto.room,
            }))
        })?;
        if !client.in_room(&room.name) {
            let connected = self.server.clients().iter().filter(|c| c.in_room(&room.name)).count() as i64;
            if connected >= room.max_members {
                return Err(WsException::object(json!({
                    "status": "error",
                    "code": "ROOM_FULL",
                    "message": format!("#{} is full", room.name),
                    "room": room.name,
                    "max": room.max_members,
                })));
            }
            client.join(&room.name);
            let members = self.members(&room.name);
            self.server.to(&room.name).except(client.id()).emit("user_joined", json!({ "room": room.name, "user": user, "members": members }));
        }
        let history = self.rooms.history(&room.name, 50).await.http()?;
        Ok(WsResponse::new("joined", json!({ "room": room, "members": self.members(&room.name), "history": history })))
    }

    #[subscribe_message("leave")]
    #[use_guards(RoomMemberGuard)]
    fn leave(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<WsResponse> {
        let user = current_user(&client)?;
        client.leave(&dto.room);
        self.notify_left(&dto.room, &user);
        Ok(WsResponse::new("left", json!({ "room": dto.room })))
    }

    #[subscribe_message("message")]
    #[use_guards(RoomMemberGuard)]
    async fn message(&self, #[message_body] dto: SendMessageDto, #[connected_socket] client: WsClient) -> HttpResult<WsResponse> {
        let user = current_user(&client)?;
        let saved = self.rooms.save_message(&dto.room, &user.username, &dto.text).await.http()?;
        // Everyone else in the room; the sender gets the `sent` acknowledgement instead.
        self.server.to(&dto.room).except(client.id()).emit("message", &saved);
        self.firehose.publish(json!({ "type": "message", "message": saved }));
        Ok(WsResponse::new("sent", saved))
    }

    #[subscribe_message("typing")]
    #[use_guards(RoomMemberGuard)]
    fn typing(&self, #[message_body] dto: TypingDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
        let user = current_user(&client)?;
        self.server
            .to(&dto.room)
            .except(client.id())
            .emit("typing", json!({ "room": dto.room, "user": user.username, "typing": dto.typing }));
        Ok(())
    }

    #[subscribe_message("clear")]
    #[set_metadata(ROLES_KEY, [MODERATOR])]
    #[use_guards(RolesGuard, RoomMemberGuard)]
    async fn clear(&self, #[message_body] dto: RoomDto, #[connected_socket] client: WsClient) -> HttpResult<()> {
        let user = current_user(&client)?;
        let deleted = self.rooms.clear(&dto.room).await.http()?;
        self.server.to(&dto.room).emit("cleared", json!({ "room": dto.room, "by": user.username, "deleted": deleted }));
        self.firehose.publish(json!({ "type": "cleared", "room": dto.room, "by": user.username }));
        Ok(())
    }

    #[subscribe_message("rooms")]
    async fn rooms(&self) -> HttpResult<Value> {
        let rooms = self.rooms.list().await.http()?;
        Ok(self.with_members(rooms))
    }

    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "pong"
    }

    /// `RoomsService::create` emits `room.created`: tell every connected client.
    #[on_event("room.created")]
    fn on_room_created(&self, room: Room) {
        self.server.emit("room_created", json!({ "room": room, "members": [] }));
    }
}

/// The user attached on connect (the gateway's `AuthGuard` already checked it is there).
fn current_user(client: &WsClient) -> HttpResult<ChatUser> {
    client.get::<ChatUser>().ok_or_else(|| WsException::new("Unauthorized"))
}

/// Live state for the REST controllers and the stats stream.
impl ChatGateway {
    /// Users in `room` (a user with two tabs open is listed once).
    pub fn members(&self, room: &str) -> Vec<ChatUser> {
        let mut by_name = BTreeMap::new();
        for c in self.server.clients().into_iter().filter(|c| c.in_room(room)) {
            if let Some(u) = c.get::<ChatUser>() {
                by_name.insert(u.username.clone(), u);
            }
        }
        by_name.into_values().collect()
    }

    /// Distinct authenticated users connected to the gateway.
    pub fn online_users(&self) -> Vec<String> {
        let mut names: Vec<String> = self.server.clients().iter().filter_map(|c| c.get::<ChatUser>()).map(|u| u.username).collect();
        names.sort();
        names.dedup();
        names
    }

    /// Open sockets (several per user if they have several tabs).
    pub fn connections(&self) -> usize {
        self.server.count()
    }

    /// Rooms as JSON with their live `members`.
    pub fn with_members(&self, rooms: Vec<Room>) -> Value {
        Value::Array(
            rooms
                .into_iter()
                .map(|r| {
                    let members = self.members(&r.name);
                    let mut v = serde_json::to_value(&r).unwrap_or(Value::Null);
                    v["members"] = json!(members);
                    v
                })
                .collect(),
        )
    }

    /// Pushes an announcement to everyone in `room`; returns how many sockets got it.
    pub fn announce(&self, room: &str, from: &ChatUser, text: &str) -> usize {
        let payload = json!({ "room": room, "from": from.username, "text": text });
        self.firehose.publish(json!({ "type": "announcement", "announcement": payload }));
        self.server.to(room).emit("announcement", payload)
    }

    fn notify_left(&self, room: &str, user: &ChatUser) {
        self.server.to(room).emit("user_left", json!({ "room": room, "user": user, "members": self.members(room) }));
    }
}

#[async_trait]
impl OnGatewayInit for ChatGateway {
    async fn after_init(&self, _server: WsServer) {
        self.logger.log("Chat gateway listening on /chat");
    }
}

#[async_trait]
impl OnGatewayConnection for ChatGateway {
    /// Browsers cannot set headers on a WebSocket, so the JWT comes in `?token=`
    /// (`Authorization: Bearer` also works for other clients).
    async fn handle_connection(&self, client: WsClient) {
        let handshake = client.handshake();
        let token = handshake.query_param("token").map(str::to_string).or_else(|| extract_bearer_token(handshake));
        let Some(user) = token.and_then(|t| self.jwt.verify::<ChatUser>(&t).ok()) else {
            self.logger.warn(format!("Rejected socket {} from {}: missing or invalid token", client.id(), handshake.ip()));
            client.emit("exception", json!({ "status": "error", "message": "Unauthorized" }));
            client.disconnect_with(CLOSE_UNAUTHORIZED, "Unauthorized");
            return;
        };
        client.insert(user.clone());
        let rooms = match self.rooms.list().await {
            Ok(rooms) => self.with_members(rooms),
            Err(_) => json!([]),
        };
        client.emit("welcome", json!({ "id": client.id(), "user": user, "rooms": rooms }));
        self.logger.log(format!("{} connected ({} sockets)", user.username, self.server.count()));
    }
}

#[async_trait]
impl OnGatewayDisconnect for ChatGateway {
    /// The client is already gone from the server: `members()` no longer lists it.
    async fn handle_disconnect(&self, client: WsClient) {
        let Some(user) = client.get::<ChatUser>() else { return };
        for room in client.rooms() {
            self.notify_left(&room, &user);
        }
        self.logger.log(format!("{} disconnected ({} sockets)", user.username, self.server.count()));
    }
}

//! WebSocket gateway. Try it with `websocat ws://localhost:3000/chat` and send
//! `{"event":"join","data":"lobby"}` then `{"event":"message","data":"hi"}`.
//!
//! The gateway is public: the global `AuthGuard` runs on every message (global guards apply to
//! gateways) and lets the targets marked `#[public]` through, so no token is needed. The
//! global `ThrottlerGuard` also counts the messages (100 per minute per IP and handler).

use corvis::prelude::*;
use corvis::{OnGatewayConnection, OnGatewayDisconnect, WsClient, WsResponse, WsServer, websocket_gateway};

#[websocket_gateway(path = "/chat")]
#[public]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
    #[init(Logger::new("ChatGateway"))]
    logger: Logger,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] room: String, #[connected_socket] client: WsClient) -> WsResponse {
        client.join(&room);
        self.server.to(&room).except(client.id()).emit("joined", json!({ "id": client.id() }));
        WsResponse::new("joined", json!({ "room": room }))
    }

    #[subscribe_message("message")]
    fn message(&self, #[message_body] text: String, #[connected_socket] client: WsClient) {
        for room in client.rooms() {
            self.server.to(&room).emit("message", json!({ "from": client.id(), "text": text, "room": room }));
        }
    }

    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "pong"
    }
}

#[async_trait]
impl OnGatewayConnection for ChatGateway {
    async fn handle_connection(&self, client: WsClient) {
        self.logger.log(format!("Client connected: {} ({} online)", client.id(), self.server.count()));
    }
}

#[async_trait]
impl OnGatewayDisconnect for ChatGateway {
    async fn handle_disconnect(&self, client: WsClient) {
        self.logger.log(format!("Client disconnected: {}", client.id()));
    }
}

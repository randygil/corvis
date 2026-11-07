//! `/ws/firehose`: a plain WebSocket (`corvis::ws`, the `ws` package model) that streams
//! every chat event as a raw JSON text frame. Unlike a gateway, a raw `upgrade` listener can
//! reject the handshake itself: without a moderator token the client gets `401` and no
//! WebSocket is opened.

use std::collections::HashSet;
use std::sync::Mutex;

use corvis::JwtService;
use corvis::prelude::*;
use corvis::ws::{WebSocket, WebSocketServer, WebSocketServerOptions};

use crate::auth::ChatUser;

pub const FIREHOSE_PATH: &str = "/ws/firehose";

#[derive(Default)]
pub struct FirehoseState {
    clients: Mutex<HashSet<WebSocket>>,
}

#[injectable]
pub struct FirehoseService {
    adapter_host: Arc<HttpAdapterHost>,
    jwt: Arc<JwtService>,
    #[default]
    state: Arc<FirehoseState>,
}

impl FirehoseService {
    /// Sends `event` to every open firehose socket; returns how many got it.
    pub fn publish(&self, event: Value) -> usize {
        let clients: Vec<WebSocket> = self.state.clients.lock().unwrap().iter().cloned().collect();
        clients.iter().filter(|ws| ws.is_open() && ws.send(event.clone()).is_ok()).count()
    }

    pub fn subscribers(&self) -> usize {
        self.state.clients.lock().unwrap().len()
    }
}

#[async_trait]
impl OnApplicationBootstrap for FirehoseService {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        let server = self.adapter_host.http_adapter().get_http_server();
        let wss = WebSocketServer::new(WebSocketServerOptions::default().max_payload(1024));
        let (state, jwt) = (self.state.clone(), self.jwt.clone());
        server.on_upgrade(move |req| {
            if req.pathname() != FIREHOSE_PATH {
                req.destroy();
                return;
            }
            let user = req.query_param("token").and_then(|t| jwt.verify::<ChatUser>(t).ok());
            if !user.as_ref().is_some_and(ChatUser::is_moderator) {
                let _ = req.respond(Response::text("Moderator token required").with_status(401));
                return;
            }
            let state = state.clone();
            wss.handle_upgrade(&req, move |ws| {
                state.clients.lock().unwrap().insert(ws.clone());
                let (s, me) = (state.clone(), ws.clone());
                ws.on_close(move |_code, _reason| {
                    s.clients.lock().unwrap().remove(&me);
                });
                // Plain text frames both ways: `ping` -> `pong`.
                let peer = ws.clone();
                ws.on_message(move |m| {
                    if m.to_text() == "ping" {
                        let _ = peer.send("pong");
                    }
                });
                let _ = ws.send(json!({ "type": "hello", "user": user.map(|u| u.username) }));
            });
        });
        Ok(())
    }
}

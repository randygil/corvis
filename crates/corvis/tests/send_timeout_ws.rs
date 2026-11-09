//! `send_timeout` does not apply to an upgraded connection: a WebSocket client that stops reading for longer than the
//! timeout keeps its connection and then receives what the gateway queued (the queue is bounded by
//! `max_buffered_amount`).
#![cfg(feature = "websockets")]

use std::time::Duration;

use corvis::prelude::*;
use corvis::{OnGatewayConnection, ServerOptions, WsClient, websocket_gateway};
use futures::StreamExt;
use tokio::net::TcpSocket;
use tokio_tungstenite::tungstenite::Message;

const MESSAGES: usize = 24;
const PAYLOAD: usize = 128 * 1024;

#[websocket_gateway(path = "/feed")]
pub struct FeedGateway;

#[handlers]
impl FeedGateway {}

#[async_trait]
impl OnGatewayConnection for FeedGateway {
    /// 3 MiB at once: more than the kernel buffers of a client that does not read take, less than `max_buffered_amount`.
    async fn handle_connection(&self, client: WsClient) {
        for i in 0..MESSAGES {
            client.emit("chunk", json!({ "i": i, "pad": "x".repeat(PAYLOAD) }));
        }
        client.emit("done", json!({}));
    }
}

#[module(providers = [FeedGateway])]
pub struct AppModule;

#[corvis::test]
async fn a_websocket_client_that_stops_reading_keeps_its_connection() {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.set_server_options(ServerOptions::new().send_timeout(Duration::from_secs(1)));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();

    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let stream = socket.connect(addr).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://{addr}/feed"), stream).await.unwrap();

    // Not reading for three send timeouts.
    tokio::time::sleep(Duration::from_secs(3)).await;

    let mut chunks = 0usize;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next()).await.expect("a message in time");
        let Some(Ok(msg)) = msg else { panic!("the connection ended after {chunks} chunks") };
        if let Message::Text(t) = msg {
            let v: Value = serde_json::from_str(&t).unwrap();
            match v["event"].as_str() {
                Some("chunk") => chunks += 1,
                Some("done") => break,
                _ => {}
            }
        }
    }
    assert_eq!(chunks, MESSAGES);
}

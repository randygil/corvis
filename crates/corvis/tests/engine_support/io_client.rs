//! A raw Socket.IO client over the Engine.IO WebSocket transport, and an in-process application with an `IoServer`
//! attached. Servers bind to 127.0.0.1.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::time::Duration;

use corvis::prelude::*;
use corvis::socket_io::{IoOptions, IoServer};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

use super::{AppModule, WsClient};

pub struct IoFixture {
    pub app: CorvisApplication,
    pub addr: SocketAddr,
    pub io: IoServer,
}

/// An application with one Socket.IO server at `/socket.io/`; `configure` registers namespaces and handlers before it
/// listens.
pub async fn start(options: IoOptions, configure: impl FnOnce(&IoServer)) -> IoFixture {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let io = IoServer::new(options).unwrap();
    configure(&io);
    io.attach(&app.get_http_server()).unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    IoFixture { app, addr, io }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    Text(String),
    Binary(Vec<u8>),
}

pub struct Raw {
    ws: WsClient,
    pub open: serde_json::Value,
}

impl Raw {
    /// Opens an Engine.IO session over WebSocket and reads the open packet.
    pub async fn connect(addr: SocketAddr) -> Raw {
        let mut ws = super::ws_connect(addr, None).await;
        let open = super::next_text(&mut ws).await;
        let open = serde_json::from_str(open.strip_prefix('0').expect("an open packet")).unwrap();
        Raw { ws, open }
    }

    /// The same, from another local address (any `127.x.y.z` is loopback), so the server sees another client.
    pub async fn connect_from(addr: SocketAddr, local: std::net::IpAddr) -> Raw {
        let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
        socket.bind(SocketAddr::new(local, 0)).expect("bind the local address");
        let stream = socket.connect(addr).await.expect("connect");
        let (mut ws, _) = tokio_tungstenite::client_async(
            format!("ws://{addr}/socket.io/?EIO=4&transport=websocket"),
            tokio_tungstenite::MaybeTlsStream::Plain(stream),
        )
        .await
        .expect("WebSocket handshake");
        let open = super::next_text(&mut ws).await;
        let open = serde_json::from_str(open.strip_prefix('0').expect("an open packet")).unwrap();
        Raw { ws, open }
    }

    /// Sends a Socket.IO packet (as an Engine.IO message).
    pub async fn send(&mut self, packet: &str) {
        self.ws.send(Message::Text(format!("4{packet}"))).await.unwrap();
    }

    pub async fn send_binary(&mut self, data: &[u8]) {
        self.ws.send(Message::Binary(data.to_vec())).await.unwrap();
    }

    /// The next Socket.IO message within `wait`; pings are answered. `None` when the connection closed or nothing
    /// arrived in time.
    pub async fn recv_within(&mut self, wait: Duration) -> Option<Msg> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let next = tokio::time::timeout_at(deadline, self.ws.next()).await.ok()?;
            match next {
                Some(Ok(Message::Text(t))) if t == "2" => {
                    let _ = self.ws.send(Message::Text("3".into())).await;
                }
                Some(Ok(Message::Text(t))) => return Some(Msg::Text(t.strip_prefix('4').unwrap_or(&t).to_string())),
                Some(Ok(Message::Binary(b))) => return Some(Msg::Binary(b)),
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return None,
            }
        }
    }

    pub async fn recv(&mut self) -> Msg {
        self.recv_within(Duration::from_secs(10)).await.expect("a message within 10 s")
    }

    pub async fn recv_text(&mut self) -> String {
        match self.recv().await {
            Msg::Text(t) => t,
            other => panic!("expected text, got {other:?}"),
        }
    }

    /// Joins a namespace (`"/"`, `"/admin"`) and returns the socket id.
    pub async fn join(&mut self, nsp: &str, auth: Option<&str>) -> String {
        let auth = auth.unwrap_or("");
        let packet = if nsp == "/" { format!("0{auth}") } else { format!("0{nsp},{auth}") };
        self.send(&packet).await;
        let answer = self.recv_text().await;
        let json_at = answer.find('{').unwrap_or_else(|| panic!("not a CONNECT answer: {answer}"));
        assert!(answer.starts_with('0'), "{answer}");
        let body: serde_json::Value = serde_json::from_str(&answer[json_at..]).unwrap();
        body["sid"].as_str().unwrap().to_string()
    }

    /// Closes the WebSocket with a close frame, as a browser tab does.
    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }

    /// `true` when the server closed the connection within `wait` (data that arrives meanwhile is discarded).
    pub async fn closed_within(&mut self, wait: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, self.ws.next()).await {
                Err(_) => return false,
                Ok(None | Some(Err(_)) | Some(Ok(Message::Close(_)))) => return true,
                Ok(Some(Ok(Message::Text(t)))) if t == "2" => {
                    let _ = self.ws.send(Message::Text("3".into())).await;
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

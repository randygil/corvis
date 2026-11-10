//! The Corvis side of the differential harness: the same application as `parity/socket-io/reference/engine-server.mjs`.
//! The presets are mirrored option for option, because the scenarios pick a preset by its path. Servers bind to
//! 127.0.0.1.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use corvis::prelude::*;
use corvis::router::cors::CorsOptions;
use corvis::socket_io::EngineOptions;
use corvis::socket_io::engine::{EngineMessage, EngineServer, EngineSocket};
use futures::StreamExt;

static REASONS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[controller("harness")]
pub struct HarnessController;

#[routes]
impl HarnessController {
    /// The reason a session was closed with, or `open`.
    #[get("close-reason/:sid")]
    fn close_reason(&self, #[param("sid")] sid: String) -> String {
        REASONS.lock().unwrap().get(&sid).cloned().unwrap_or_else(|| "open".to_string())
    }

    /// What happened on the Socket.IO scenario servers, in order (loopback only: the servers bind to 127.0.0.1).
    #[get("log")]
    fn log(&self) -> serde_json::Value {
        serde_json::Value::Array(super::io_app::LOG.lock().unwrap().clone())
    }

    /// Clears the log.
    #[post("reset")]
    fn reset(&self) -> serde_json::Value {
        super::io_app::LOG.lock().unwrap().clear();
        serde_json::Value::Null
    }
}

#[module(controllers = [HarnessController])]
pub struct HarnessModule;

/// What the harness scenarios can pick by path.
pub struct Preset {
    pub name: &'static str,
    pub options: EngineOptions,
    pub burst: bool,
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset { name: "default", options: EngineOptions::new(), burst: false },
        Preset {
            name: "fast",
            options: EngineOptions::new().path("/fast/").ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200)),
            burst: false,
        },
        Preset {
            name: "cors",
            options: EngineOptions::new().path("/cors/").cors(CorsOptions::new().origin("https://app.example")),
            burst: false,
        },
        Preset { name: "burst", options: EngineOptions::new().path("/burst/"), burst: true },
        Preset { name: "small", options: EngineOptions::new().path("/small/").max_http_buffer_size(1000), burst: false },
        Preset { name: "cap", options: EngineOptions::new().path("/cap/").max_sessions(2), burst: false },
        Preset { name: "ipcap", options: EngineOptions::new().path("/ipcap/").max_sessions_per_ip(2), burst: false },
        Preset {
            name: "first",
            options: EngineOptions::new().path("/first/").first_request_timeout(Duration::from_millis(400)),
            burst: false,
        },
    ]
}

/// The echo behavior of the harness: what a session sends back and the commands it understands.
pub fn behave(socket: EngineSocket, burst: bool) {
    let id = socket.id().to_string();
    let closing = socket.clone();
    tokio::spawn(async move {
        let reason = closing.closed().await;
        REASONS.lock().unwrap().insert(id, reason.as_str().to_string());
    });
    if burst {
        for i in 0..100 {
            let _ = socket.send(format!("m{i}"));
        }
    }
    tokio::spawn(async move {
        let mut messages = socket.messages();
        while let Some(message) = messages.next().await {
            if let EngineMessage::Text(text) = &message {
                if text == "burst" {
                    let _ = socket.send("one");
                    let _ = socket.send(vec![1u8, 2, 3]);
                    let _ = socket.send("three");
                    continue;
                }
                if let Some(n) = text.strip_prefix("flood:").and_then(|n| n.parse::<usize>().ok()) {
                    let mut left = n;
                    while left > 0 {
                        let chunk = left.min(100_000);
                        if socket.send("x".repeat(chunk)).is_err() {
                            break;
                        }
                        left -= chunk;
                    }
                    continue;
                }
            }
            let _ = socket.send(message);
        }
    });
}

/// Starts the harness application on `127.0.0.1:0`.
pub async fn start() -> (CorvisApplication, SocketAddr, Vec<EngineServer>) {
    let mut app = CorvisFactory::create::<HarnessModule>().await.unwrap();
    let mut engines = Vec::new();
    for preset in presets() {
        let engine = EngineServer::new(preset.options).unwrap();
        let burst = preset.burst;
        engine.on_connection(move |socket| behave(socket, burst));
        engine.attach(&app.get_http_server()).unwrap();
        engines.push(engine);
    }
    // The Socket.IO scenario servers live on their own paths (`/sio/`, `/sio-short/`), apart from the Engine.IO presets.
    let _servers = super::io_app::attach_scenario_servers(&app);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr, engines)
}

/// The application the upstream `engine.io-protocol` suite requires: ping 300/200 ms, CORS for any origin and
/// echo, at `/engine.io/`.
pub async fn start_suite_app() -> (CorvisApplication, SocketAddr, EngineServer) {
    let mut app = CorvisFactory::create::<HarnessModule>().await.unwrap();
    let engine = EngineServer::new(
        EngineOptions::new()
            .path("/engine.io/")
            .ping_interval(Duration::from_millis(300))
            .ping_timeout(Duration::from_millis(200))
            .cors(CorsOptions::new().origin("*")),
    )
    .unwrap();
    engine.on_connection(|socket| behave(socket, false));
    engine.attach(&app.get_http_server()).unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr, engine)
}

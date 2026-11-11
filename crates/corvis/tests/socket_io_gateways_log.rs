#![cfg(feature = "socket-io")]
//! What the gateway adapter logs: the warning of a late `use_websocket_adapter`, the two risky configurations reported at
//! `init()`, and the failure of a reply for a reserved event. Its own test binary: the logger is process-wide, so every
//! test shares one capturing sink and the tests are serialized.

mod engine_support;

use std::sync::Mutex;

use corvis::Module;
use corvis::prelude::*;
use corvis::router::cors::{CorsOptions, CorsOrigin};
use corvis::socket_io::{EngineOptions, IoAdapter, IoOptions};
use corvis::{LogLevel, LoggerService, WsResponse, websocket_gateway};
use engine_support::io_client::Raw;

static LOGS: Mutex<Vec<(LogLevel, String)>> = Mutex::new(Vec::new());
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Capture;

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, _context: Option<&str>) {
        LOGS.lock().unwrap().push((level, message.to_string()));
    }
}

fn warnings() -> Vec<String> {
    LOGS.lock().unwrap().iter().filter(|(level, _)| *level == LogLevel::Warn).map(|(_, m)| m.clone()).collect()
}

#[websocket_gateway("/plain")]
pub struct PlainGateway;

#[handlers]
impl PlainGateway {
    #[subscribe_message("reserved")]
    fn reserved(&self) -> WsResponse {
        WsResponse::new("disconnect", 1)
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }
}

#[module(providers = [PlainGateway])]
pub struct PlainModule;

#[websocket_gateway(path = "/with-cors", cors = true)]
pub struct CorsGateway;

#[module(providers = [CorsGateway])]
pub struct CorsModule;

async fn app_of<M: Module>() -> CorvisApplication {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_logger(Capture);
    LOGS.lock().unwrap().clear();
    app
}

#[tokio::test]
async fn selecting_the_adapter_after_init_warns_and_changes_nothing() {
    let _serial = SERIAL.lock().await;
    let mut app = app_of::<PlainModule>().await;
    app.init().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    assert!(
        warnings().iter().any(|w| w.contains("use_websocket_adapter() has no effect after the application was initialized")),
        "{:?}",
        warnings()
    );
    // The gateways keep the adapter they were built with: the `ws` one.
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/plain")).await.is_ok());
    app.close().await.unwrap();
}

#[tokio::test]
async fn credentials_cors_without_an_origin_allowlist_is_reported_at_init() {
    let _serial = SERIAL.lock().await;
    let risky = CorsOptions { credentials: true, origin: CorsOrigin::Reflect, ..CorsOptions::default() };
    let mut app = app_of::<PlainModule>().await;
    app.use_websocket_adapter(IoAdapter::new().options(IoOptions::new().engine(EngineOptions::new().cors(risky.clone()))));
    app.init().await.unwrap();
    let found = warnings();
    assert!(
        found.iter().any(|w| w.contains("/socket.io/") && w.contains("allowed_origins") && w.contains("credentials")),
        "a warning names the path and the origin allowlist: {found:?}"
    );
    // With an allowlist there is nothing to report.
    let mut app = app_of::<PlainModule>().await;
    app.use_websocket_adapter(
        IoAdapter::new().options(IoOptions::new().engine(EngineOptions::new().cors(risky).allowed_origins(["https://app.example"]))),
    );
    app.init().await.unwrap();
    assert!(!warnings().iter().any(|w| w.contains("allowed_origins")), "{:?}", warnings());
}

#[tokio::test]
async fn cors_under_the_ws_adapter_warns_naming_the_gateway_and_serves_the_gateway_as_without_it() {
    let _serial = SERIAL.lock().await;
    let mut app = app_of::<CorsModule>().await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(warnings().iter().any(|w| w.contains("CorsGateway") && w.contains("cors") && w.contains("no effect")), "{:?}", warnings());
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/with-cors")).await.is_ok());
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_reply_for_a_reserved_event_is_logged_with_the_gateway_and_the_handler() {
    let _serial = SERIAL.lock().await;
    let mut app = app_of::<PlainModule>().await;
    app.use_websocket_adapter(IoAdapter::new());
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/plain", None).await;
    client.send(r#"2/plain,["reserved"]"#).await;
    client.send(r#"2/plain,["mark"]"#).await;
    assert_eq!(client.recv_text().await, r#"2/plain,["done",true]"#, "nothing was sent for the reserved event");
    assert!(
        warnings().iter().any(|w| w.contains("PlainGateway::reserved") && w.contains("reserved Socket.IO event name")),
        "{:?}",
        warnings()
    );
    app.close().await.unwrap();
}

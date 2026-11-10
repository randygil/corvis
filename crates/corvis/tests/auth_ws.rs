//! `AuthGuard` on a WebSocket gateway: it denies every message without running a strategy, and the application warns
//! once per gateway at startup. Its own test binary: the logger is process-wide.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::auth::{AuthGuard, AuthenticateOptions, Strategy, StrategyOutcome};
use corvis::prelude::*;
use corvis::{LogLevel, LoggerService, WsResponse, WsServer, websocket_gateway};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

static STRATEGY_CALLS: AtomicUsize = AtomicUsize::new(0);
static HANDLER_CALLS: AtomicUsize = AtomicUsize::new(0);
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[injectable]
pub struct CountingStrategy;

#[async_trait]
impl Strategy for CountingStrategy {
    fn name(&self) -> &str {
        "ws"
    }
    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        STRATEGY_CALLS.fetch_add(1, Ordering::SeqCst);
        StrategyOutcome::Pass
    }
}

#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        WsResponse::new("joined", text)
    }
}

#[websocket_gateway(path = "/quiet")]
pub struct QuietGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl QuietGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        WsResponse::new("joined", text)
    }
}

#[module(providers = [Provider::strategy::<CountingStrategy>(), ChatGateway, QuietGateway])]
pub struct AppModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

#[corvis::test]
async fn a_global_auth_guard_denies_gateway_messages_without_running_a_strategy() {
    let _s = SERIAL.lock().await;
    let captured: Captured = Arc::default();
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    app.use_global_guards(AuthGuard::new("ws"));
    app.init().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/chat")).await.unwrap();
    ws.send(Message::Text(json!({ "event": "join", "data": "lobby" }).to_string())).await.unwrap();
    let frame = loop {
        match tokio::time::timeout(Duration::from_secs(2), ws.next()).await.expect("a frame") {
            Some(Ok(Message::Text(t))) => break serde_json::from_str::<Value>(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("no frame: {other:?}"),
        }
    };
    assert_eq!(frame, json!({ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } }));
    assert_eq!(STRATEGY_CALLS.load(Ordering::SeqCst), 0);
    assert_eq!(HANDLER_CALLS.load(Ordering::SeqCst), 0);

    // One warning per gateway, logged at startup (not per message), naming the gateway and the guard.
    let warnings: Vec<String> = captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, _, ctx)| *l == LogLevel::Warn && ctx.as_deref() == Some("AuthGuard"))
        .map(|(_, m, _)| m.clone())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("ChatGateway") && w.contains("/chat") && w.contains("ws")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("QuietGateway")), "{warnings:?}");

    // Another message does not log again.
    ws.send(Message::Text(json!({ "event": "join", "data": "again" }).to_string())).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_millis(300), ws.next()).await;
    let after = captured.lock().unwrap().iter().filter(|(l, _, ctx)| *l == LogLevel::Warn && ctx.as_deref() == Some("AuthGuard")).count();
    assert_eq!(after, 2);
}

#[corvis::test]
async fn no_warning_when_the_gateway_does_not_see_the_guard() {
    let _s = SERIAL.lock().await;
    // Its own check of the opt-out: global enhancers turned off for gateways.
    let captured: Captured = Arc::default();
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    app.apply_global_enhancers_to_gateways(false);
    app.use_global_guards(AuthGuard::new("ws"));
    app.init().await.unwrap();
    let warned = captured.lock().unwrap().iter().any(|(l, _, ctx)| *l == LogLevel::Warn && ctx.as_deref() == Some("AuthGuard"));
    assert!(!warned);
}

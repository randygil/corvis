//! The startup warning of `ServerOptions::insecure_http_parser(true)`. Its own test binary: the logger is
//! process-wide, so the tests share one capturing sink and are serialized.

use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use corvis::{LogLevel, LoggerService, ServerOptions};

#[controller("health")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get]
    fn health(&self) -> Value {
        json!({ "status": "ok" })
    }
}

#[module(controllers = [HealthController])]
pub struct AppModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn make_app(options: Option<ServerOptions>) -> (CorvisApplication, Captured) {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let captured: Captured = Arc::default();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    if let Some(o) = options {
        app.set_server_options(o);
    }
    (app, captured)
}

fn parser_warnings(captured: &Captured) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Warn && ctx.as_deref() == Some("ServerOptions"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

#[tokio::test]
async fn warns_once_with_the_option_before_listen_returns() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = make_app(Some(ServerOptions::new().insecure_http_parser(true))).await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(addr.ip().is_loopback());
    let warnings = parser_warnings(&captured);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("insecure_http_parser(true)") && warnings[0].contains("framing checks are off"), "{warnings:?}");
    assert!(warnings[0].contains("https://corvis.randygil.com/security/http-parsing"));
}

#[tokio::test]
async fn does_not_warn_without_the_option() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = make_app(None).await;
    app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(parser_warnings(&captured).is_empty());
    let (mut app, captured) = make_app(Some(ServerOptions::new().insecure_http_parser(false))).await;
    app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(parser_warnings(&captured).is_empty());
}

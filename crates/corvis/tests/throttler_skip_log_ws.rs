//! A bare `SKIP_THROTTLE` that skips nothing is logged once per route, overrides for unknown throttlers are logged once
//! per route, invalid override values are logged as errors, and `skip_throttle_key` and `throttle_key` work on a gateway
//! handler. Its own test binary: the logger is process-wide, so the tests share one capturing sink and are
//! serialized.

use std::sync::Mutex;
use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{
    LogLevel, LoggerService, THROTTLE_LIMIT, ThrottlerGuard, ThrottlerModule, ThrottlerOptions, WsResponse, WsServer, websocket_gateway,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

static LOGS: Mutex<Vec<(LogLevel, String, Option<String>)>> = Mutex::new(Vec::new());
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Capture;

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        LOGS.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

fn guard_warnings() -> Vec<String> {
    LOGS.lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Warn && ctx.as_deref() == Some("ThrottlerGuard"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

fn errors() -> Vec<String> {
    LOGS.lock().unwrap().iter().filter(|(level, ..)| *level == LogLevel::Error).map(|(_, m, _)| m.clone()).collect()
}

fn throttler(name: &str, ttl: u64, limit: u32) -> ThrottlerOptions {
    ThrottlerOptions { name: name.into(), ttl, limit }
}

#[controller("r")]
#[use_guards(ThrottlerGuard)]
pub struct RoutesController;

#[routes]
impl RoutesController {
    #[get("bare")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn bare(&self) -> &'static str {
        "ok"
    }

    #[get("other")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn other(&self) -> &'static str {
        "ok"
    }

    #[get("plain")]
    fn plain(&self) -> &'static str {
        "ok"
    }

    #[get("misspelled")]
    #[apply_decorators(throttle("shrot").limit(1))]
    fn misspelled(&self) -> &'static str {
        "ok"
    }

    #[get("known")]
    #[apply_decorators(throttle("short").limit(4), throttle("long").limit(4))]
    fn known(&self) -> &'static str {
        "ok"
    }

    #[get("bare-limit")]
    #[set_metadata(THROTTLE_LIMIT, 1)]
    fn bare_limit(&self) -> &'static str {
        "ok"
    }

    #[get("invalid")]
    #[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), "abc")]
    fn invalid(&self) -> &'static str {
        "ok"
    }
}

#[websocket_gateway(path = "/skip")]
pub struct SkipGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl SkipGateway {
    #[subscribe_message("join")]
    #[set_metadata(skip_throttle_key("short"), true)]
    fn join(&self, #[message_body] text: String) -> WsResponse {
        WsResponse::new("joined", text)
    }
}

#[websocket_gateway(path = "/ovr")]
pub struct OverrideGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl OverrideGateway {
    #[subscribe_message("tight")]
    #[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), 1)]
    fn tight(&self, #[message_body] text: String) -> WsResponse {
        WsResponse::new("tight", text)
    }
}

#[module(imports = [ThrottlerModule::for_root(vec![throttler("short", 60_000, 5), throttler("long", 60_000, 5)])], controllers = [RoutesController])]
pub struct NamedModule;

#[module(imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 5), throttler("short", 60_000, 5)])], controllers = [RoutesController])]
pub struct DefaultModule;

#[module(
    imports = [ThrottlerModule::for_root(vec![throttler("short", 60_000, 1), throttler("long", 120_000, 2)])],
    providers = [SkipGateway, Provider::app_guard::<ThrottlerGuard>()]
)]
pub struct GatewayModule;

#[module(
    imports = [ThrottlerModule::for_root(vec![throttler("short", 60_000, 3), throttler("long", 120_000, 5)])],
    providers = [OverrideGateway, Provider::app_guard::<ThrottlerGuard>()]
)]
pub struct GatewayOverrideModule;

async fn start<M: corvis::Module>() -> (CorvisApplication, std::net::SocketAddr) {
    LOGS.lock().unwrap().clear();
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_logger(Capture);
    app.silence_route_logs();
    app.init().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

#[corvis::test]
async fn a_bare_skip_that_skips_nothing_warns_once_per_route() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<NamedModule>().await;
    let c = request(&app);
    for _ in 0..3 {
        c.get("/r/bare").send().await.expect_status(200).expect_header("x-ratelimit-limit-short", "5");
    }
    let warnings = guard_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("RoutesController") && warnings[0].contains("bare"), "{}", warnings[0]);
    assert!(warnings[0].contains("skip_throttle"), "{}", warnings[0]);
    c.get("/r/other").send().await.expect_status(200);
    c.get("/r/plain").send().await.expect_status(200);
    assert_eq!(guard_warnings().len(), 2, "another route warns once; a route without a skip does not");
}

#[corvis::test]
async fn no_warning_when_a_throttler_is_named_default() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<DefaultModule>().await;
    let c = request(&app);
    for _ in 0..3 {
        c.get("/r/bare").send().await.expect_status(200).expect_header("x-ratelimit-limit-short", "5");
    }
    assert_eq!(guard_warnings(), Vec::<String>::new());
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(ws: &mut Ws, frame: Value) -> Value {
    ws.send(Message::Text(frame.to_string())).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_millis(1_000), ws.next()).await.expect("a frame") {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[corvis::test]
async fn gateway_handler_skips_a_throttler_by_name() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<GatewayModule>().await;
    let mut ws = tokio_tungstenite::connect_async(format!("ws://{addr}/skip")).await.unwrap().0;
    let message = json!({ "event": "join", "data": "lobby" });
    // `short` allows one message and is skipped; `long` allows two.
    assert_eq!(send(&mut ws, message.clone()).await["event"], "joined");
    assert_eq!(send(&mut ws, message.clone()).await["event"], "joined");
    assert_eq!(send(&mut ws, message).await["event"], "exception");
}

#[corvis::test]
async fn overrides_for_an_unknown_throttler_warn_once_per_route() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<NamedModule>().await;
    let c = request(&app);
    for _ in 0..3 {
        c.get("/r/misspelled").send().await.expect_status(200).expect_header("x-ratelimit-limit-short", "5");
    }
    let warnings = guard_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("RoutesController") && warnings[0].contains("misspelled") && warnings[0].contains("shrot"),
        "{}",
        warnings[0]
    );
    assert!(warnings[0].contains("`short`") && warnings[0].contains("`long`"), "lists the configured names: {}", warnings[0]);
}

#[corvis::test]
async fn a_bare_override_without_a_default_throttler_warns_once() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<NamedModule>().await;
    let c = request(&app);
    for _ in 0..2 {
        c.get("/r/bare-limit").send().await.expect_status(200).expect_header("x-ratelimit-limit-short", "5");
    }
    let warnings = guard_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("bare_limit") && warnings[0].contains("THROTTLER:LIMIT"), "{}", warnings[0]);
}

#[corvis::test]
async fn no_warning_when_every_override_names_a_configured_throttler() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<NamedModule>().await;
    request(&app).get("/r/known").send().await.expect_status(200).expect_header("x-ratelimit-limit-short", "4");
    let (app, _addr) = start::<DefaultModule>().await;
    request(&app).get("/r/bare-limit").send().await.expect_status(200).expect_header("x-ratelimit-limit", "1");
    assert_eq!(guard_warnings(), Vec::<String>::new());
}

#[corvis::test]
async fn an_invalid_override_value_is_logged_with_the_option_the_throttler_and_the_value() {
    let _s = SERIAL.lock().await;
    let (app, _addr) = start::<NamedModule>().await;
    request(&app).get("/r/invalid").send().await.expect_status(500);
    let errors = errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("limit") && errors[0].contains("short") && errors[0].contains("abc"), "{}", errors[0]);
}

#[corvis::test]
async fn gateway_handler_overrides_a_throttler_by_name() {
    let _s = SERIAL.lock().await;
    let (_app, addr) = start::<GatewayOverrideModule>().await;
    let mut ws = tokio_tungstenite::connect_async(format!("ws://{addr}/ovr")).await.unwrap().0;
    let message = json!({ "event": "tight", "data": "x" });
    // `short` allows three messages but the handler tightens it to one; `long` is untouched.
    assert_eq!(send(&mut ws, message.clone()).await["event"], "tight");
    assert_eq!(send(&mut ws, message).await["event"], "exception");
}

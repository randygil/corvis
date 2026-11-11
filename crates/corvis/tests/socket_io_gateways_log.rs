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

#[websocket_gateway("/leaky")]
pub struct LeakyGateway;

#[handlers]
impl LeakyGateway {
    /// Panics with text the client chose.
    #[subscribe_message("boom")]
    fn boom(&self, #[message_body] text: String) {
        panic!("secret-panic: {text}");
    }
}

#[injectable]
pub struct Failing;

#[async_trait]
impl CanActivate for Failing {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Err(HttpException::from_anyhow(anyhow::anyhow!("secret connection detail")))
    }
}

#[websocket_gateway("/guarded-leaky")]
#[use_guards(Failing)]
pub struct GuardedLeakyGateway;

#[handlers]
impl GuardedLeakyGateway {}

#[module(providers = [LeakyGateway, Failing, GuardedLeakyGateway])]
pub struct LeakyModule;

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

#[tokio::test]
async fn panics_and_internal_errors_are_logged_rate_limited_and_never_with_client_text() {
    let _serial = SERIAL.lock().await;
    let mut app = app_of::<LeakyModule>().await;
    app.use_websocket_adapter(IoAdapter::new().guard_connections(true));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut client = Raw::connect(addr).await;
    client.send(r#"0/guarded-leaky,"#).await;
    assert_eq!(client.recv_text().await, r#"4/guarded-leaky,{"message":"Internal server error"}"#);
    app.close().await.unwrap();
    let mut app = app_of::<LeakyModule>().await;
    app.use_websocket_adapter(IoAdapter::new());
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut client = Raw::connect(addr).await;
    client.join("/leaky", None).await;
    for _ in 0..30 {
        client.send(r#"2/leaky,["boom","CLIENT-CHOSEN-TEXT"]"#).await;
    }
    for _ in 0..30 {
        let reply = client.recv_text().await;
        assert!(reply.contains("Internal server error") && !reply.contains("secret-panic"), "{reply}");
    }
    let lines: Vec<(LogLevel, String)> = LOGS.lock().unwrap().clone();
    assert!(lines.iter().all(|(_, m)| !m.contains("CLIENT-CHOSEN-TEXT") && !m.contains("secret connection detail")), "{lines:?}");
    let handler_lines =
        lines.iter().filter(|(_, m)| m.contains("internal error in a gateway handler") || m.contains("LeakyGateway::boom")).count();
    assert!((1..=2).contains(&handler_lines), "30 panics are logged by the rate limit, not one line each: {handler_lines} lines");
    app.close().await.unwrap();
}

// ------------------------------------------------------------------- a disconnect hook dropped at the application's cap

#[injectable]
pub struct Departures {
    #[default]
    started: std::sync::Arc<Mutex<Vec<String>>>,
    #[default]
    changed: std::sync::Arc<tokio::sync::Notify>,
    #[default]
    gate: std::sync::Arc<DepartureGate>,
}

pub struct DepartureGate(tokio::sync::Semaphore);

impl Default for DepartureGate {
    fn default() -> Self {
        DepartureGate(tokio::sync::Semaphore::new(0))
    }
}

impl Departures {
    async fn wait_started(&self, count: usize) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.started.lock().unwrap().len() >= count {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(30), wait).await.expect("the hooks to start");
    }
}

#[websocket_gateway("/departures")]
pub struct DeparturesGateway {
    departures: std::sync::Arc<Departures>,
}

#[handlers]
impl DeparturesGateway {}

#[async_trait]
impl corvis::OnGatewayDisconnect<corvis::socket_io::IoSocket> for DeparturesGateway {
    async fn handle_disconnect(&self, socket: corvis::socket_io::IoSocket) {
        let who = socket.handshake().auth().get("who").and_then(Value::as_str).unwrap_or("?").to_string();
        self.departures.started.lock().unwrap().push(who);
        self.departures.changed.notify_waiters();
        self.departures.gate.0.acquire().await.unwrap().forget();
    }
}

#[module(providers = [Departures, DeparturesGateway])]
pub struct DeparturesModule;

/// The one case in which a disconnect hook is dropped: the application is at its cap of pending hooks. Four may be
/// pending; five sockets leave, from five addresses (each within its share of one), while the hooks block: four hooks
/// run, the fifth is dropped, and the drop is logged (rate limited) without anything the client sent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnect_hook_is_dropped_only_at_the_applications_cap_and_the_drop_is_logged() {
    let _serial = SERIAL.lock().await;
    let mut app = app_of::<DeparturesModule>().await;
    app.use_websocket_adapter(IoAdapter::new().max_pending_disconnect_hooks(4));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let departures = app.get::<Departures>().unwrap();
    for n in 0..5u8 {
        let mut client = Raw::connect_from(addr, std::net::IpAddr::from([127, 0, 0, 40 + n])).await;
        client.join("/departures", Some(&format!(r#"{{"who":"secret-client-{n}"}}"#))).await;
        client.close().await;
        if n < 4 {
            // The first four are pending (and running) before the next leaves.
            departures.wait_started(n as usize + 1).await;
        }
    }
    let dropped = || warnings().iter().filter(|w| w.contains("was dropped")).cloned().collect::<Vec<_>>();
    let wait = async {
        while dropped().is_empty() {
            tokio::task::yield_now().await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), wait).await.expect("the drop to be logged");
    let lines = dropped();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("4 handle_disconnect hooks are pending") && !lines[0].contains("secret"), "{}", lines[0]);
    // The pending hooks run; the dropped one never did (a worker was free for it).
    departures.gate.0.add_permits(5);
    let mut started = departures.started.lock().unwrap().clone();
    started.sort();
    assert_eq!(started, ["secret-client-0", "secret-client-1", "secret-client-2", "secret-client-3"]);
    app.close().await.unwrap();
}

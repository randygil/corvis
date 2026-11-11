#![cfg(feature = "socket-io")]
//! Regression tests of the first security review of the gateway adapter: global connection guards for every gateway of
//! a namespace, first-in-first-out events behind a slow connection hook, bounded disconnect-hook workers, and the
//! startup errors for a root Engine.IO path, a reserved event name and a zero hook timeout. Servers bind to 127.0.0.1.

mod engine_support;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::Module;
use corvis::prelude::*;
use corvis::socket_io::{IoAdapter, IoSocket};
use corvis::{OnGatewayConnection, OnGatewayDisconnect, RoutePostureOptions, websocket_gateway};
use engine_support::io_client::Raw;
use tokio::sync::Notify;

#[injectable]
pub struct Log {
    #[default]
    entries: Arc<Mutex<Vec<String>>>,
    #[default]
    changed: Arc<Notify>,
    #[default]
    release: Arc<Notify>,
    #[default]
    running: Arc<AtomicUsize>,
    #[default]
    peak: Arc<AtomicUsize>,
    #[default]
    finished: Arc<AtomicUsize>,
}

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.entries.lock().unwrap().push(entry.into());
        self.changed.notify_waiters();
    }

    fn entries(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }

    async fn wait_until(&self, what: &str, done: impl Fn(&[String]) -> bool) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if done(&self.entries.lock().unwrap()) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(20), wait).await.is_err() {
            panic!("{what} did not happen within 20 s; log: {:?}", self.entries().len());
        }
    }
}

/// Honors the public marker: denies unless the handler or the gateway is `#[public]`.
#[injectable]
pub struct PublicOrDeny {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for PublicOrDeny {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(self.reflector.get_all_and_override::<bool>(PUBLIC_METADATA, &[ctx.get_handler(), ctx.get_class()]) == Some(true))
    }
}

// ---------------------------------------------------------------------------------------- M2: two gateways, one namespace

#[websocket_gateway("/shared")]
#[public]
pub struct SharedOpen;

#[handlers]
impl SharedOpen {}

#[websocket_gateway("/shared")]
pub struct SharedAdmin;

#[handlers]
impl SharedAdmin {}

#[module(providers = [PublicOrDeny, SharedOpen, SharedAdmin, Provider::app_guard::<PublicOrDeny>()])]
pub struct OpenFirst;

#[module(providers = [PublicOrDeny, SharedAdmin, SharedOpen, Provider::app_guard::<PublicOrDeny>()])]
pub struct AdminFirst;

async fn start<M: Module>(adapter: IoAdapter) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_websocket_adapter(adapter);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

async fn anonymous_answer<M: Module>() -> String {
    let (mut app, addr) = start::<M>(IoAdapter::new().guard_connections(true)).await;
    let mut client = Raw::connect(addr).await;
    client.send("0/shared,").await;
    let answer = client.recv_text().await;
    app.close().await.unwrap();
    answer
}

#[tokio::test]
async fn the_global_guards_run_for_every_gateway_of_a_namespace_whatever_the_provider_order() {
    // An anonymous client reaches both gateways of the namespace, so the one that is not public decides.
    let forbidden = r#"4/shared,{"message":"Forbidden resource"}"#;
    assert_eq!(anonymous_answer::<OpenFirst>().await, forbidden, "public gateway registered first");
    assert_eq!(anonymous_answer::<AdminFirst>().await, forbidden, "public gateway registered last");
}

#[tokio::test]
async fn the_posture_report_says_what_the_connection_runs_for_each_gateway() {
    for module in ["open-first", "admin-first"] {
        let adapter = IoAdapter::new().guard_connections(true);
        let mut app = if module == "open-first" {
            CorvisFactory::create::<OpenFirst>().await.unwrap()
        } else {
            CorvisFactory::create::<AdminFirst>().await.unwrap()
        };
        app.use_websocket_adapter(adapter);
        app.set_route_posture(RoutePostureOptions::enforce());
        // Enforce mode accepts both: the global guard covers `SharedAdmin::connect`, and `SharedOpen` is public.
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        let text = format!("{:?}", report.entries());
        assert!(text.contains("SharedAdmin") && text.contains("SharedOpen"), "{text}");
        assert!(report.unprotected().is_empty());
    }
}

// ------------------------------------------------------------------------------------------------ B1: first in, first out

#[websocket_gateway("/order")]
pub struct OrderGateway {
    log: Arc<Log>,
}

#[handlers]
impl OrderGateway {
    #[subscribe_message("ord")]
    fn ord(&self, #[message_body] body: Value) {
        self.log.push(format!(
            "ord:{}",
            body.as_array().map(|a| a.iter().map(Value::to_string).collect::<Vec<_>>().join(",")).unwrap_or_default()
        ));
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for OrderGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        if socket.handshake().auth().get("token").and_then(Value::as_str) == Some("slow") {
            self.log.push("order-hook-waiting");
            self.log.release.notified().await;
        }
    }
}

#[module(providers = [Log, OrderGateway])]
pub struct OrderModule;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn events_held_behind_a_slow_connection_hook_start_in_arrival_order() {
    let (mut app, addr) = start::<OrderModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    for round in 0..15 {
        let mut client = Raw::connect(addr).await;
        client.send(r#"0/order,{"token":"slow"}"#).await;
        let _connected = client.recv_text().await;
        let waiting = round + 1;
        log.wait_until("the hook to wait", |e| e.iter().filter(|x| *x == "order-hook-waiting").count() >= waiting).await;
        for n in 0..30 {
            client.send(&format!(r#"2/order,["ord",{round},{n}]"#)).await;
        }
        // Everything is on its way to the server while the hook still waits; then the hook returns.
        tokio::time::sleep(Duration::from_millis(30)).await;
        log.release.notify_one();
        let prefix = format!("ord:{round},");
        log.wait_until("30 events", |e| e.iter().filter(|x| x.starts_with(&prefix)).count() >= 30).await;
        let order: Vec<usize> = log.entries().iter().filter_map(|x| x.strip_prefix(&prefix).and_then(|n| n.parse().ok())).collect();
        assert_eq!(order, (0..30).collect::<Vec<_>>(), "round {round}: the events started in the order they arrived");
        client.close().await;
    }
    // Without a hook the first events race the task that opens the gate: still in order.
    for round in 100..115 {
        let mut client = Raw::connect(addr).await;
        client.send("0/order,").await;
        let _connected = client.recv_text().await;
        for n in 0..30 {
            client.send(&format!(r#"2/order,["ord",{round},{n}]"#)).await;
        }
        let prefix = format!("ord:{round},");
        log.wait_until("30 events", |e| e.iter().filter(|x| x.starts_with(&prefix)).count() >= 30).await;
        let order: Vec<usize> = log.entries().iter().filter_map(|x| x.strip_prefix(&prefix).and_then(|n| n.parse().ok())).collect();
        assert_eq!(order, (0..30).collect::<Vec<_>>(), "round {round} (no hook)");
        client.close().await;
    }
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------------------- B2: disconnect hook workers

#[websocket_gateway("/leaving")]
pub struct LeavingGateway {
    log: Arc<Log>,
}

#[handlers]
impl LeavingGateway {}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for LeavingGateway {
    async fn handle_disconnect(&self, _socket: IoSocket) {
        let now = self.log.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.log.peak.fetch_max(now, Ordering::SeqCst);
        self.log.release.notified().await;
        self.log.running.fetch_sub(1, Ordering::SeqCst);
        self.log.finished.fetch_add(1, Ordering::SeqCst);
        self.log.push("left");
    }
}

#[module(providers = [Log, LeavingGateway])]
pub struct LeavingModule;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnect_hooks_run_in_a_bounded_number_of_tasks_and_every_one_runs_once() {
    let (mut app, addr) = start::<LeavingModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    const SOCKETS: usize = 120;
    for _ in 0..SOCKETS {
        let mut client = Raw::connect(addr).await;
        client.join("/leaving", None).await;
        client.close().await;
    }
    // The hooks block: the workers are all busy and the rest wait in the queue.
    for _ in 0..200 {
        if log.running.load(Ordering::SeqCst) >= 16 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(log.running.load(Ordering::SeqCst), 16, "no more hooks run at once than there are workers");
    // Release them as they come: every hook runs exactly once.
    loop {
        log.release.notify_waiters();
        if log.finished.load(Ordering::SeqCst) >= SOCKETS {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(log.finished.load(Ordering::SeqCst), SOCKETS, "each socket's hook ran once");
    assert!(log.peak.load(Ordering::SeqCst) <= 16, "peak concurrency {}", log.peak.load(Ordering::SeqCst));
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------------------ startup errors

#[websocket_gateway(path = "/")]
pub struct RootPathGateway;

#[module(providers = [RootPathGateway])]
pub struct RootPathModule;

#[websocket_gateway(path = "")]
pub struct EmptyPathGateway;

#[module(providers = [EmptyPathGateway])]
pub struct EmptyPathModule;

#[websocket_gateway("/reserved")]
pub struct ReservedGateway;

#[handlers]
impl ReservedGateway {
    #[subscribe_message("disconnect")]
    fn on_disconnect(&self) {}
}

#[module(providers = [ReservedGateway])]
pub struct ReservedModule;

async fn init_error<M: Module>(adapter: Option<IoAdapter>) -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    if let Some(adapter) = adapter {
        app.use_websocket_adapter(adapter);
    }
    app.init().await
}

#[tokio::test]
async fn an_engine_path_of_root_is_refused_naming_the_gateway() {
    for message in [
        format!("{:#}", init_error::<RootPathModule>(Some(IoAdapter::new())).await.unwrap_err()),
        format!("{:#}", init_error::<EmptyPathModule>(Some(IoAdapter::new())).await.unwrap_err()),
    ] {
        assert!(message.contains("Gateway") && message.contains("every HTTP request"), "{message}");
    }
    // Under the default adapter `path = "/"` is the URL path it always was.
    init_error::<RootPathModule>(None).await.unwrap();
}

#[tokio::test]
async fn a_handler_for_a_reserved_event_name_is_a_startup_error_only_under_socket_io() {
    let message = format!("{:#}", init_error::<ReservedModule>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(
        message.contains("ReservedGateway::on_disconnect") && message.contains("\"disconnect\"") && message.contains("reserved"),
        "{message}"
    );
    init_error::<ReservedModule>(None).await.unwrap();
}

#[tokio::test]
async fn a_zero_hook_timeout_fails_init() {
    let message = format!("{:#}", init_error::<ReservedModule>(Some(IoAdapter::new().hook_timeout(Duration::ZERO))).await.unwrap_err());
    assert!(message.contains("hook_timeout") && message.contains("longer than zero"), "{message}");
}

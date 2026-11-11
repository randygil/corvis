#![cfg(feature = "socket-io")]
//! The security properties of gateways served over Socket.IO: connection guards (`guard_connections`) and their
//! context, what a denied connection gets, panics and slow hooks, disconnect hooks that run once on every path, the
//! namespaces a client can reach, the session cap taken from the adapter options, and route posture. Absence of a
//! reply is observed through a marker event; servers bind to 127.0.0.1.

mod engine_support;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::Module;
use corvis::prelude::*;
use corvis::socket_io::{ConnectError, EngineOptions, IoAdapter, IoGatewayServer, IoNamespace, IoOptions, IoSocket};
use corvis::{
    OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, RoutePostureOptions, UncheckedSurface, WsException, WsResponse,
    websocket_gateway,
};
use engine_support::http;
use engine_support::io_client::Raw;
use tokio::sync::Notify;

// -------------------------------------------------------------------------------------------------------- the log

#[injectable]
pub struct Log {
    #[default]
    entries: Arc<Mutex<Vec<String>>>,
    #[default]
    changed: Arc<Notify>,
    #[default]
    release: Arc<Notify>,
}

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.entries.lock().unwrap().push(entry.into());
        self.changed.notify_waiters();
    }

    fn entries(&self) -> Vec<String> {
        self.entries.lock().unwrap().clone()
    }

    fn count(&self, entry: &str) -> usize {
        self.entries.lock().unwrap().iter().filter(|e| *e == entry).count()
    }

    async fn wait_for(&self, entry: &str) {
        let wait = async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.entries.lock().unwrap().iter().any(|e| e == entry) {
                    return;
                }
                notified.await;
            }
        };
        if tokio::time::timeout(Duration::from_secs(10), wait).await.is_err() {
            panic!("`{entry}` was not logged within 10 s; log: {:?}", self.entries());
        }
    }
}

// ----------------------------------------------------------------------------------------------- enhancers

/// Records what it saw and denies unless the handshake's `auth.token` is set.
#[injectable]
pub struct NeedsToken {
    log: Arc<Log>,
}

#[async_trait]
impl CanActivate for NeedsToken {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        let socket = ws.get_client::<IoSocket>();
        self.log.push(format!(
            "guard:type={}:class={}:handler={}:data={}:client={}:path={}:meta={}",
            ctx.get_type().as_str(),
            ctx.get_class().name,
            ctx.get_handler().name,
            ws.get_data().cloned().unwrap_or(json!("none")),
            socket.is_some(),
            ctx.request().path(),
            ctx.get_class().get_metadata("gateway-meta").cloned().unwrap_or(Value::Null),
        ));
        Ok(socket.is_some_and(|s| s.handshake().auth().get("token").is_some()))
    }
}

#[injectable]
pub struct Expired;

#[async_trait]
impl CanActivate for Expired {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Err(WsException::object(json!({ "message": "Token expired", "code": "EXPIRED" })))
    }
}

#[injectable]
pub struct InternalFailure;

#[async_trait]
impl CanActivate for InternalFailure {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Err(HttpException::from_anyhow(anyhow::anyhow!("secret connection detail")))
    }
}

#[injectable]
pub struct PanicGuard;

#[async_trait]
impl CanActivate for PanicGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        panic!("secret guard detail")
    }
}

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

#[injectable]
pub struct DenyAll;

#[async_trait]
impl CanActivate for DenyAll {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(false)
    }
}

/// Panics in `catches`, `catch` for the exceptions of `fail`, and answers nothing else.
#[injectable]
pub struct PanicFilter;

#[async_trait]
impl ExceptionFilter for PanicFilter {
    async fn catch(&self, _e: HttpException, _host: &ArgumentsHost) -> Response {
        panic!("secret filter detail")
    }
}

// ---------------------------------------------------------------------------------------------------- gateways

#[websocket_gateway("/rules")]
#[use_guards(NeedsToken)]
#[set_metadata("gateway-meta", "rules-metadata")]
pub struct RulesGateway {
    #[websocket_server]
    server: IoGatewayServer,
    log: Arc<Log>,
}

#[handlers]
impl RulesGateway {
    #[subscribe_message("guarded")]
    fn guarded(&self) -> &'static str {
        self.log.push("rules-handler");
        "ok"
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for RulesGateway {
    async fn handle_connection(&self, _socket: IoSocket) {
        self.log.push("rules-connection");
    }
}

#[websocket_gateway("/expired")]
#[use_guards(Expired)]
pub struct ExpiredGateway {
    log: Arc<Log>,
}

#[handlers]
impl ExpiredGateway {}

#[async_trait]
impl OnGatewayConnection<IoSocket> for ExpiredGateway {
    async fn handle_connection(&self, _socket: IoSocket) {
        self.log.push("expired-connection");
    }
}

#[websocket_gateway("/internal")]
#[use_guards(InternalFailure)]
pub struct InternalGateway;

#[handlers]
impl InternalGateway {}

#[websocket_gateway("/panicky")]
#[use_guards(PanicGuard)]
pub struct PanickyGateway;

#[handlers]
impl PanickyGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        "hi"
    }
}

#[websocket_gateway("/lobby")]
#[use_guards(PublicOrDeny)]
#[public]
pub struct LobbyGateway;

#[handlers]
impl LobbyGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        "hi"
    }
}

#[websocket_gateway("/handler-guard")]
pub struct HandlerGuardGateway;

#[handlers]
impl HandlerGuardGateway {
    #[subscribe_message("secret")]
    #[use_guards(DenyAll)]
    fn secret(&self) -> &'static str {
        "never"
    }

    #[subscribe_message("open")]
    fn open(&self) -> &'static str {
        "open"
    }
}

#[websocket_gateway("/filtered")]
#[use_filters(PanicFilter)]
pub struct FilteredGateway;

#[handlers]
impl FilteredGateway {
    #[subscribe_message("fail")]
    fn fail(&self) -> Result<(), HttpException> {
        Err(WsException::new("raised"))
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }
}

/// Hooks whose behavior the test chooses through `auth.token`.
#[websocket_gateway("/life")]
pub struct LifecycleGateway {
    log: Arc<Log>,
}

#[handlers]
impl LifecycleGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        self.log.push("life-handler");
        "hi"
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for LifecycleGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        let token = socket.handshake().auth().get("token").and_then(Value::as_str).unwrap_or("").to_string();
        self.log.push(format!("life-connection:{token}"));
        match token.as_str() {
            "boom" => panic!("secret hook detail"),
            // Never returns on its own: the adapter's hook timeout ends it.
            "hang" => {
                self.log.push("life-hook-waiting");
                std::future::pending::<()>().await;
            }
            "kick" => {
                self.log.push("life-hook-waiting");
                self.log.release.notified().await;
                socket.disconnect(false);
            }
            _ => {}
        }
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for LifecycleGateway {
    async fn handle_disconnect(&self, _socket: IoSocket) {
        self.log.push("life-disconnect");
    }
}

/// `after_init` and `handle_disconnect` that panic.
#[websocket_gateway("/hooks-panic")]
pub struct PanickingHooksGateway {
    log: Arc<Log>,
}

#[handlers]
impl PanickingHooksGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        "hi"
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for PanickingHooksGateway {
    async fn handle_disconnect(&self, _socket: IoSocket) {
        self.log.push("panicking-disconnect");
        panic!("secret disconnect detail");
    }
}

#[websocket_gateway("/init-panic")]
pub struct InitPanicGateway;

#[async_trait]
impl OnGatewayInit<IoNamespace> for InitPanicGateway {
    async fn after_init(&self, _namespace: IoNamespace) {
        panic!("secret init detail");
    }
}

#[websocket_gateway("/init-hang")]
pub struct InitHangGateway;

#[async_trait]
impl OnGatewayInit<IoNamespace> for InitHangGateway {
    async fn after_init(&self, _namespace: IoNamespace) {
        std::future::pending::<()>().await;
    }
}

#[module(providers = [Log, NeedsToken, Expired, InternalFailure, PanicGuard, PublicOrDeny, DenyAll, PanicFilter,
    RulesGateway, ExpiredGateway, InternalGateway, PanickyGateway, LobbyGateway, HandlerGuardGateway, FilteredGateway, LifecycleGateway,
    PanickingHooksGateway])]
pub struct MainModule;

#[module(providers = [InitPanicGateway])]
pub struct InitPanicModule;

#[module(providers = [InitHangGateway])]
pub struct InitHangModule;

// ------------------------------------------------------------------------------------------------------ helpers

async fn start<M: Module>(adapter: IoAdapter) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_websocket_adapter(adapter);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

fn event_packet(nsp: &str, id: Option<u64>, body: &str) -> String {
    let nsp = if nsp == "/" { String::new() } else { format!("{nsp},") };
    format!("2{nsp}{}{body}", id.map(|i| i.to_string()).unwrap_or_default())
}

fn ack_packet(nsp: &str, id: u64, body: &str) -> String {
    let nsp = if nsp == "/" { String::new() } else { format!("{nsp},") };
    format!("3{nsp}{id}{body}")
}

async fn expect_marker(client: &mut Raw, nsp: &str) {
    client.send(&event_packet(nsp, None, r#"["mark"]"#)).await;
    assert_eq!(client.recv_text().await, event_packet(nsp, None, r#"["done",true]"#));
}

async fn connect(addr: SocketAddr, nsp: &str, auth: Option<&str>) -> Raw {
    let mut client = Raw::connect(addr).await;
    client.join(nsp, auth).await;
    client
}

/// What a client gets for a CONNECT to `nsp`: the text of the packet.
async fn try_connect(client: &mut Raw, nsp: &str, auth: &str) -> String {
    client.send(&format!("0{nsp},{auth}")).await;
    client.recv_text().await
}

// ------------------------------------------------------------------------------------- connection guards

#[tokio::test]
async fn connection_guards_are_off_by_default_and_only_messages_are_denied() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    // The guard of the gateway denies this client's messages, but it connects, and the connection hook runs.
    let mut client = connect(addr, "/rules", None).await;
    log.wait_for("rules-connection").await;
    client.send(&event_packet("/rules", Some(1), r#"["guarded"]"#)).await;
    assert_eq!(
        client.recv_text().await,
        event_packet("/rules", None, r#"["exception",{"status":"error","message":"Forbidden resource","cause":{"pattern":"guarded"}}]"#)
    );
    assert!(!log.entries().iter().any(|e| e.contains("handler=connect")), "{:?}", log.entries());
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_denied_connection_is_refused_with_connect_error_and_nothing_of_the_gateway_runs() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    assert_eq!(try_connect(&mut client, "/rules", "").await, r#"4/rules,{"message":"Forbidden resource"}"#);
    assert!(!log.entries().contains(&"rules-connection".to_string()));
    // It is in no namespace: a broadcast to the whole namespace reaches nobody.
    let namespace = app.get::<RulesGateway>().unwrap().server.namespace().unwrap();
    assert_eq!(namespace.socket_count(), 0);
    namespace.emit("news", ()).await.unwrap();
    assert!(client.recv_within(Duration::from_millis(200)).await.is_none(), "a denied socket receives no broadcast");
    // With a token it connects and the hook runs.
    let mut ok = Raw::connect(addr).await;
    assert!(try_connect(&mut ok, "/rules", r#"{"token":"t"}"#).await.starts_with("0/rules,"));
    log.wait_for("rules-connection").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_context_of_a_connection_guard() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    try_connect(&mut client, "/rules", r#"{"token":"t"}"#).await;
    let entry = log.entries().into_iter().find(|e| e.starts_with("guard:")).expect("the guard ran for the connection");
    assert_eq!(
        entry, r#"guard:type=ws:class=RulesGateway:handler=connect:data=null:client=true:path=/socket.io/:meta="rules-metadata""#,
        "a WebSocket context: the socket as client, the handshake as request, null data, the gateway as class, a handler named connect with the gateway's metadata"
    );
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_failing_connection_guard_answers_its_message_and_data_and_internal_errors_answer_nothing_else() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    assert_eq!(try_connect(&mut client, "/expired", "").await, r#"4/expired,{"message":"Token expired","data":{"code":"EXPIRED"}}"#);
    assert!(!log.entries().contains(&"expired-connection".to_string()));
    assert_eq!(try_connect(&mut client, "/internal", "").await, r#"4/internal,{"message":"Internal server error"}"#);
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_panicking_connection_guard_answers_an_internal_error_and_the_server_keeps_running() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let mut client = Raw::connect(addr).await;
    let answer = try_connect(&mut client, "/panicky", "").await;
    assert_eq!(answer, r#"4/panicky,{"message":"Internal server error"}"#);
    assert!(!answer.contains("secret"));
    // Another namespace of the same connection, and a new connection, still work.
    assert!(try_connect(&mut client, "/lobby", "").await.starts_with("0/lobby,"));
    connect(addr, "/lobby", None).await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn handler_level_guards_do_not_run_for_the_connection() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let mut client = connect(addr, "/handler-guard", None).await;
    client.send(&event_packet("/handler-guard", Some(1), r#"["open"]"#)).await;
    assert_eq!(client.recv_text().await, ack_packet("/handler-guard", 1, r#"["open"]"#));
    client.send(&event_packet("/handler-guard", Some(2), r#"["secret"]"#)).await;
    assert_eq!(
        client.recv_text().await,
        event_packet(
            "/handler-guard",
            None,
            r#"["exception",{"status":"error","message":"Forbidden resource","cause":{"pattern":"secret"}}]"#
        )
    );
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_public_gateway_connects_without_credentials_under_connection_guards() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().guard_connections(true)).await;
    let mut client = connect(addr, "/lobby", None).await;
    client.send(&event_packet("/lobby", Some(1), r#"["hello"]"#)).await;
    assert_eq!(client.recv_text().await, ack_packet("/lobby", 1, r#"["hi"]"#));
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------- namespaces and the cap

#[tokio::test]
async fn a_namespace_no_gateway_declares_is_refused_and_so_is_the_main_one_when_undeclared() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = Raw::connect(addr).await;
    assert_eq!(try_connect(&mut client, "/nope", "").await, r#"4/nope,{"message":"Invalid namespace"}"#);
    // Nobody declared `/` on this server.
    client.send("0").await;
    assert_eq!(client.recv_text().await, r#"4{"message":"Invalid namespace"}"#);
    // The declared ones are served on the same connection.
    assert!(try_connect(&mut client, "/lobby", "").await.starts_with("0/lobby,"));
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_session_cap_comes_from_the_adapter_options() {
    let options = IoOptions::new().engine(EngineOptions::new().max_sessions(1));
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().options(options)).await;
    let _first = Raw::connect(addr).await;
    let refused = http(addr, "GET", "/socket.io/?EIO=4&transport=polling", &[], b"").await;
    assert_eq!(refused.status, 503, "{}", refused.text());
    app.close().await.unwrap();
}

// ---------------------------------------------------------------------------------- panics and hooks

#[tokio::test]
async fn a_panicking_guard_or_filter_never_reaches_the_client_as_text_and_the_connection_survives() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    // A guard that panics on a message.
    let mut client = connect(addr, "/panicky", None).await;
    client.send(&event_packet("/panicky", Some(1), r#"["hello"]"#)).await;
    let text = client.recv_text().await;
    assert_eq!(
        text,
        event_packet("/panicky", None, r#"["exception",{"status":"error","message":"Internal server error","cause":{"pattern":"hello"}}]"#)
    );
    assert!(!text.contains("secret"));
    // A filter that panics.
    let mut client = connect(addr, "/filtered", None).await;
    client.send(&event_packet("/filtered", Some(1), r#"["fail"]"#)).await;
    let text = client.recv_text().await;
    assert_eq!(text, event_packet("/filtered", None, r#"["exception",{"status":"error","message":"Internal server error"}]"#));
    expect_marker(&mut client, "/filtered").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_hook_that_never_returns_is_bounded_by_the_hook_timeout() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new().hook_timeout(Duration::from_millis(150))).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    try_connect(&mut client, "/life", r#"{"token":"hang"}"#).await;
    log.wait_for("life-hook-waiting").await;
    assert!(client.closed_within(Duration::from_secs(10)).await, "the socket is disconnected and its connection closed");
    log.wait_for("life-disconnect").await;
    assert_eq!(log.count("life-disconnect"), 1);
    app.close().await.unwrap();
}

#[tokio::test]
async fn disconnect_hooks_run_once_on_every_path() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    // The client closes.
    let client = connect(addr, "/life", Some(r#"{"token":"a"}"#)).await;
    log.wait_for("life-connection:a").await;
    client.close().await;
    log.wait_for("life-disconnect").await;
    // The server disconnects the socket from inside the connection hook.
    let mut kicked = Raw::connect(addr).await;
    try_connect(&mut kicked, "/life", r#"{"token":"kick"}"#).await;
    log.wait_for("life-hook-waiting").await;
    log.release.notify_one();
    // The server shuts down with a socket connected.
    let _open = connect(addr, "/life", Some(r#"{"token":"b"}"#)).await;
    log.wait_for("life-connection:b").await;
    app.close().await.unwrap();
    for _ in 0..200 {
        if log.count("life-disconnect") >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(log.count("life-disconnect"), 3, "one per socket: {:?}", log.entries());
}

#[tokio::test]
async fn a_panicking_disconnect_hook_is_contained_and_runs_once() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = connect(addr, "/hooks-panic", None).await;
    client.send(&event_packet("/hooks-panic", Some(1), r#"["hello"]"#)).await;
    assert_eq!(client.recv_text().await, ack_packet("/hooks-panic", 1, r#"["hi"]"#));
    client.close().await;
    log.wait_for("panicking-disconnect").await;
    // The server is fine.
    connect(addr, "/lobby", None).await;
    assert_eq!(log.count("panicking-disconnect"), 1);
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_panicking_or_hanging_after_init_fails_init_instead_of_hanging_or_aborting() {
    let mut app = CorvisFactory::create::<InitPanicModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("InitPanicGateway") && message.contains("after_init panicked") && !message.contains("secret"), "{message}");
    let mut app = CorvisFactory::create::<InitHangModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new().hook_timeout(Duration::from_millis(100)));
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("InitHangGateway") && message.contains("hook_timeout"), "{message}");
}

// ------------------------------------------------------------------------------------------ posture

mod posture {
    use super::*;

    #[injectable]
    pub struct Access;

    #[async_trait]
    impl CanActivate for Access {
        async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
            Ok(true)
        }
    }

    macro_rules! gateway {
        ($name:ident, $namespace:literal, $path:literal, [$($attr:meta),*], { $($body:tt)* }) => {
            #[websocket_gateway(namespace = $namespace, path = $path)]
            $(#[$attr])*
            pub struct $name;

            #[handlers]
            impl $name {
                $($body)*
            }
        };
    }

    gateway!(Open, "/chat", "/socket.io", [], {
        #[subscribe_message("ping")]
        fn ping(&self) {}
    });
    #[module(providers = [Access, Open])]
    pub struct OpenModule;

    gateway!(FirstPath, "/chat", "/p1", [], {
        #[subscribe_message("ping")]
        fn ping(&self) {}
    });
    gateway!(SecondPath, "/chat", "/p2", [], {
        #[subscribe_message("ping")]
        fn ping(&self) {}
    });
    #[module(providers = [Access, FirstPath, SecondPath])]
    pub struct TwoPathsModule;

    gateway!(HandlerGuarded, "/chat", "/socket.io", [], {
        #[subscribe_message("a")]
        #[use_guards(Access)]
        fn a(&self) {}
    });
    #[module(providers = [Access, HandlerGuarded])]
    pub struct HandlerGuardedModule;

    gateway!(GatewayGuarded, "/chat", "/socket.io", [use_guards(Access)], {
        #[subscribe_message("a")]
        fn a(&self) {}
    });
    #[module(providers = [Access, GatewayGuarded])]
    pub struct GatewayGuardedModule;

    gateway!(PublicChat, "/chat", "/socket.io", [public], {
        #[subscribe_message("a")]
        fn a(&self) {}
    });
    #[module(providers = [Access, PublicChat])]
    pub struct PublicModule;

    #[websocket_gateway("/hooked")]
    #[use_guards(Access)]
    pub struct Hooked;

    #[handlers]
    impl Hooked {
        #[subscribe_message("a")]
        fn a(&self) {}
    }

    #[async_trait]
    impl OnGatewayInit<IoNamespace> for Hooked {
        async fn after_init(&self, namespace: IoNamespace) {
            namespace.use_middleware(|_socket| async { Err::<(), _>(ConnectError::new("closed")) });
        }
    }
    #[module(providers = [Access, Hooked])]
    pub struct HookedModule;

    pub async fn app_of<M: Module>(adapter: IoAdapter, posture: RoutePostureOptions) -> CorvisApplication {
        let mut app = CorvisFactory::create::<M>().await.unwrap();
        app.use_websocket_adapter(adapter);
        app.set_route_posture(posture);
        app
    }
}

#[tokio::test]
async fn an_unguarded_handler_under_socket_io_is_reported_with_its_namespace_event_and_handler() {
    let mut app = posture::app_of::<posture::OpenModule>(IoAdapter::new(), RoutePostureOptions::enforce()).await;
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("/chat") && message.contains("ping") && message.contains("Open::ping"), "{message}");
}

#[tokio::test]
async fn an_exemption_matches_the_namespace_and_one_exemption_accepts_the_same_namespace_on_two_paths() {
    let mut app = posture::app_of::<posture::OpenModule>(
        IoAdapter::new(),
        RoutePostureOptions::enforce().exempt_message("/chat", "ping", "health probe"),
    )
    .await;
    app.init().await.unwrap();
    // Two gateways, namespace `/chat` on two Engine.IO paths: the error lists both, told apart by name.
    let mut app = posture::app_of::<posture::TwoPathsModule>(IoAdapter::new(), RoutePostureOptions::enforce()).await;
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("FirstPath::ping") && message.contains("SecondPath::ping"), "{message}");
    let mut app = posture::app_of::<posture::TwoPathsModule>(
        IoAdapter::new(),
        RoutePostureOptions::enforce().exempt_message("/chat", "ping", "probe on both"),
    )
    .await;
    app.init().await.unwrap();
}

#[tokio::test]
async fn the_connection_is_listed_as_not_checked_with_its_namespace_path_and_whether_connection_guards_run() {
    let mut app = posture::app_of::<posture::HookedModule>(IoAdapter::new(), RoutePostureOptions::enforce()).await;
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    // A middleware registered in `after_init` is not claimed as protection.
    assert!(
        report
            .unchecked()
            .iter()
            .any(|s| matches!(s, UncheckedSurface::IoConnection { namespace, path, gateway, guard_connections: false }
            if namespace == "/hooked" && path == "/socket.io" && gateway == "Hooked")),
        "{:?}",
        report.unchecked()
    );
    let mut app = posture::app_of::<posture::HookedModule>(IoAdapter::new().guard_connections(true), RoutePostureOptions::enforce()).await;
    app.init().await.unwrap();
    assert!(
        app.route_posture()
            .unwrap()
            .unchecked()
            .iter()
            .any(|s| matches!(s, UncheckedSurface::IoConnection { guard_connections: true, .. }))
    );
}

#[tokio::test]
async fn with_connection_guards_the_connection_is_an_enforced_entry_named_connect() {
    // Handler-level guards only: the connection is the one entry that fails.
    let mut app =
        posture::app_of::<posture::HandlerGuardedModule>(IoAdapter::new().guard_connections(true), RoutePostureOptions::enforce()).await;
    let message = format!("{:#}", app.init().await.unwrap_err());
    assert!(message.contains("connect") && message.contains("/chat") && !message.contains("HandlerGuarded::a"), "{message}");
    // The same gateway without connection guards: nothing to enforce, and it starts.
    let mut app = posture::app_of::<posture::HandlerGuardedModule>(IoAdapter::new(), RoutePostureOptions::enforce()).await;
    app.init().await.unwrap();
    // A gateway-level guard, a public gateway, or an exemption accepts it.
    for module in ["guarded", "public"] {
        let mut app = match module {
            "guarded" => {
                posture::app_of::<posture::GatewayGuardedModule>(IoAdapter::new().guard_connections(true), RoutePostureOptions::enforce())
                    .await
            }
            _ => posture::app_of::<posture::PublicModule>(IoAdapter::new().guard_connections(true), RoutePostureOptions::enforce()).await,
        };
        app.init().await.unwrap();
    }
    let mut app = posture::app_of::<posture::HandlerGuardedModule>(
        IoAdapter::new().guard_connections(true),
        RoutePostureOptions::enforce().exempt_message("/chat", "connect", "anonymous lobby"),
    )
    .await;
    app.init().await.unwrap();
}

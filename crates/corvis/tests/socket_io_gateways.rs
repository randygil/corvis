#![cfg(feature = "socket-io")]
//! `#[websocket_gateway]` classes served over Socket.IO (`use_websocket_adapter(IoAdapter::new())`): adapter selection,
//! the gateway options, grouping by Engine.IO path, startup errors, message dispatch through the gateway pipeline,
//! results and exceptions, the handles, the lifecycle hooks and the connection gate. Absence of a reply is observed
//! through a marker event the gateway emits after the handler, never by waiting; servers bind to 127.0.0.1.

mod engine_support;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use corvis::Module;
use corvis::prelude::*;
use corvis::socket_io::{ConnectError, EmitError, EngineOptions, IoAdapter, IoGatewayServer, IoNamespace, IoOptions, IoSocket};
use corvis::{
    OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, WsAdapter, WsClient, WsException, WsResponse, WsServer, websocket_gateway,
};
use engine_support::http;
use engine_support::io_client::{Msg, Raw};
use tokio::sync::Notify;

// -------------------------------------------------------------------------------------------------------- the log

/// What happened on the server side, per application (it is a provider, so applications do not share it).
#[injectable]
pub struct Log {
    #[default]
    entries: Arc<Mutex<Vec<String>>>,
    #[default]
    changed: Arc<Notify>,
    /// Hooks that block wait for this.
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

/// Denies every message of the gateways that use it unless the handshake's `auth.token` is set.
#[injectable]
pub struct NeedsToken {
    log: Arc<Log>,
}

#[async_trait]
impl CanActivate for NeedsToken {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let socket = ctx.switch_to_ws().get_client::<IoSocket>().expect("the socket is the client of the context");
        let data = ctx.switch_to_ws().get_data().cloned().unwrap_or(Value::Null);
        self.log.push(format!("guard:{}:{}:{}:{}", ctx.get_class().name, ctx.get_handler().name, data, ctx.request().path()));
        // The request of the context is the handshake of the socket.
        assert_eq!(ctx.request().query_param("EIO"), socket.handshake().request().query_param("EIO"));
        Ok(socket.handshake().auth().get("token").is_some())
    }
}

/// Fails with a `WsException` object that carries a message and extra fields.
#[injectable]
pub struct Expired;

#[async_trait]
impl CanActivate for Expired {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Err(WsException::object(json!({ "message": "Token expired", "code": "EXPIRED" })))
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

/// Honors the public marker like a real access guard: denies unless the handler or the gateway is `#[public]`.
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

/// Accepts a number only.
#[injectable]
pub struct NumberPipe;

#[async_trait]
impl PipeTransform for NumberPipe {
    async fn transform(&self, value: Value, _meta: &ArgumentMetadata) -> Result<Value, HttpException> {
        if value.is_number() { Ok(value) } else { Err(BadRequestException::new("not a number")) }
    }
}

#[injectable]
pub struct WrapInterceptor;

#[async_trait]
impl CorvisInterceptor for WrapInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|d| json!({ "wrapped": d })))
    }
}

/// Catches the exceptions that carry a `code` field and answers a body of its own.
#[injectable]
pub struct CodeFilter;

#[async_trait]
impl ExceptionFilter for CodeFilter {
    fn catches(&self, e: &HttpException) -> bool {
        corvis::WsException::get_error(e).and_then(|v| v.get("code")).is_some()
    }
    async fn catch(&self, _e: HttpException, _host: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "code-filter" }))
    }
}

/// Catches `bad input` and answers with an empty body: nothing is sent.
#[injectable]
pub struct EmptyFilter;

#[async_trait]
impl ExceptionFilter for EmptyFilter {
    fn catches(&self, e: &HttpException) -> bool {
        e.message() == "quiet"
    }
    async fn catch(&self, _e: HttpException, _host: &ArgumentsHost) -> Response {
        Response::default()
    }
}

// ---------------------------------------------------------------------------------------------------- gateways

/// The main gateway: namespace `/chat`, every kind of handler.
#[websocket_gateway("/chat")]
pub struct ChatGateway {
    log: Arc<Log>,
    #[websocket_server]
    server: IoGatewayServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("echo")]
    fn echo(&self, #[message_body] body: Value) -> Value {
        body
    }

    #[subscribe_message("silent")]
    fn silent(&self) {}

    #[subscribe_message("null")]
    fn null(&self) -> Value {
        Value::Null
    }

    #[subscribe_message("reply")]
    fn reply(&self) -> WsResponse {
        WsResponse::new("reply", 1)
    }

    #[subscribe_message("duck")]
    fn duck(&self) -> Value {
        json!({ "event": "duck", "data": 2 })
    }

    #[subscribe_message("falsy")]
    fn falsy(&self) -> i32 {
        0
    }

    #[subscribe_message("reserved")]
    fn reserved(&self) -> WsResponse {
        WsResponse::new("disconnect", 1)
    }

    #[subscribe_message("args")]
    fn args(&self, #[message_body] body: Value) -> Value {
        json!({ "body": body })
    }

    #[subscribe_message("room")]
    fn room(&self, #[message_body("room")] room: String) -> String {
        room
    }

    #[subscribe_message("fail-str")]
    fn fail_str(&self) -> Result<(), HttpException> {
        Err(WsException::new("Room is full"))
    }

    #[subscribe_message("fail-obj")]
    fn fail_obj(&self) -> Result<(), HttpException> {
        Err(WsException::object(json!({ "code": "ROOM_FULL" })))
    }

    #[subscribe_message("fail-http")]
    fn fail_http(&self) -> Result<(), HttpException> {
        Err(BadRequestException::new("bad input"))
    }

    #[subscribe_message("fail-unknown")]
    fn fail_unknown(&self) -> Result<(), HttpException> {
        Err(HttpException::from_anyhow(anyhow::anyhow!("secret database detail")))
    }

    #[subscribe_message("boom")]
    fn boom(&self) {
        panic!("secret panic detail");
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }

    #[subscribe_message("whoami")]
    fn whoami(&self, #[connected_socket] socket: IoSocket, #[ip] ip: String) -> Value {
        let mut rooms: Vec<String> = socket.rooms().into_iter().filter(|r| r != socket.id()).collect();
        rooms.sort();
        json!({ "id_matches": socket.id().len() == 20, "ip": ip, "rooms": rooms, "auth": socket.handshake().auth(), "ns": socket.namespace().name() })
    }

    #[subscribe_message("join")]
    fn join(&self, #[connected_socket] socket: IoSocket, #[message_body("room")] room: String) {
        socket.join(room).unwrap();
    }

    #[subscribe_message("shout")]
    async fn shout(&self, #[message_body("room")] room: String) -> Result<(), HttpException> {
        self.server.to(room).emit("news", ("hello",)).await.map_err(|e| HttpException::from_anyhow(anyhow::anyhow!("{e}")))
    }

    /// Also registered on the socket by `handle_connection`, which wins.
    #[subscribe_message("manual")]
    fn manual(&self) -> &'static str {
        self.log.push("gateway-manual");
        "gateway"
    }

    #[subscribe_message("upload")]
    fn upload(&self, #[message_body] body: Value) -> Value {
        json!({ "body": body })
    }
}

#[async_trait]
impl OnGatewayInit<IoNamespace> for ChatGateway {
    async fn after_init(&self, namespace: IoNamespace) {
        self.log.push(format!("init:{}", namespace.name()));
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for ChatGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        self.log.push(format!("connection:{}", socket.handshake().auth()));
        let log = self.log.clone();
        socket.on("manual", move |_socket, _args, ack| {
            let log = log.clone();
            async move {
                log.push("socket-manual");
                if let Some(ack) = ack {
                    let _ = ack.send(("socket",));
                }
            }
        });
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for ChatGateway {
    async fn handle_disconnect(&self, socket: IoSocket) {
        self.log.push(format!("disconnect:rooms={}", socket.rooms().len()));
    }
}

/// Gateway-level guard, pipe, interceptor and filters.
#[websocket_gateway("/rules")]
#[use_guards(NeedsToken)]
#[use_filters(CodeFilter, EmptyFilter)]
pub struct RulesGateway {
    log: Arc<Log>,
}

#[handlers]
impl RulesGateway {
    #[subscribe_message("guarded")]
    fn guarded(&self) -> &'static str {
        self.log.push("rules-handler");
        "ok"
    }

    #[subscribe_message("piped")]
    fn piped(&self, #[message_body(NumberPipe)] n: Value) -> Value {
        json!({ "n": n })
    }

    #[subscribe_message("wrapped")]
    #[use_interceptors(WrapInterceptor)]
    fn wrapped(&self) -> i32 {
        7
    }

    #[subscribe_message("fail-code")]
    fn fail_code(&self) -> Result<(), HttpException> {
        Err(WsException::object(json!({ "code": "X", "secret": "no" })))
    }

    #[subscribe_message("fail-quiet")]
    fn fail_quiet(&self) -> Result<(), HttpException> {
        Err(BadRequestException::new("quiet"))
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }
}

/// A gateway whose guard fails, for the connection guards.
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

/// Public gateway under a guard that honors `#[public]`.
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

/// Only a handler-level guard.
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

/// Hooks with behaviors chosen by the handshake's `auth.token`.
#[websocket_gateway("/life")]
pub struct LifecycleGateway {
    log: Arc<Log>,
    #[websocket_server]
    server: IoGatewayServer,
}

#[handlers]
impl LifecycleGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        self.log.push("life-handler");
        "hi"
    }

    #[subscribe_message("join")]
    fn join(&self, #[connected_socket] socket: IoSocket) {
        socket.join("r").unwrap();
    }
}

#[async_trait]
impl OnGatewayInit<IoNamespace> for LifecycleGateway {
    async fn after_init(&self, namespace: IoNamespace) {
        namespace.use_middleware(|socket| async move {
            match socket.handshake().auth().get("token").and_then(Value::as_str) {
                Some("no") => Err(ConnectError::new("not allowed")),
                _ => Ok(()),
            }
        });
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for LifecycleGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        let token = socket.handshake().auth().get("token").and_then(Value::as_str).unwrap_or("").to_string();
        self.log.push(format!("life-connection:{token}"));
        match token.as_str() {
            "boom" => panic!("secret hook detail"),
            "slow" => {
                self.log.push("life-hook-waiting");
                self.log.release.notified().await;
                self.log.push("life-hook-released");
            }
            "kick" => {
                self.log.push("life-hook-waiting");
                self.log.release.notified().await;
                socket.disconnect(false);
            }
            "refuse" => {
                socket.disconnect(false);
                self.log.push("life-refused");
            }
            _ => {}
        }
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for LifecycleGateway {
    async fn handle_disconnect(&self, socket: IoSocket) {
        self.log.push(format!("life-disconnect:rooms={}", socket.rooms().len()));
    }
}

/// The handlers of `/a` and `/b` are two gateways of one Engine.IO path.
#[websocket_gateway("/a")]
pub struct AGateway {
    log: Arc<Log>,
}

#[handlers]
impl AGateway {
    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "a"
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for AGateway {
    async fn handle_connection(&self, _socket: IoSocket) {
        self.log.push("a-connection");
    }
}

#[websocket_gateway("/a")]
pub struct A2Gateway {
    log: Arc<Log>,
}

#[handlers]
impl A2Gateway {
    #[subscribe_message("pong")]
    fn pong(&self) -> &'static str {
        "a2"
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for A2Gateway {
    async fn handle_connection(&self, _socket: IoSocket) {
        self.log.push("a2-connection");
    }
}

#[websocket_gateway("/b")]
pub struct BGateway;

#[handlers]
impl BGateway {
    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "b"
    }
}

#[websocket_gateway(path = "/realtime", namespace = "/chat2")]
pub struct RealtimeChatGateway;

#[handlers]
impl RealtimeChatGateway {
    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "realtime"
    }
}

#[websocket_gateway(path = "/rootpath")]
pub struct RootPathGateway;

#[handlers]
impl RootPathGateway {
    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "root"
    }
}

#[websocket_gateway(path = "/cors-any", cors = true)]
pub struct CorsAnyGateway;

#[websocket_gateway(path = "/cors-one", cors = "https://app.example")]
pub struct CorsOneGateway;

#[websocket_gateway(path = "/cors-list", cors = ["https://a.example", "https://b.example"])]
pub struct CorsListGateway;

// ------------------------------------------------------------------------------------------------------ modules

#[module(providers = [Log, NeedsToken, Expired, PanicGuard, PublicOrDeny, DenyAll, NumberPipe, WrapInterceptor, CodeFilter, EmptyFilter,
    ChatGateway, RulesGateway, ExpiredGateway, PanickyGateway, LobbyGateway, HandlerGuardGateway, LifecycleGateway])]
pub struct MainModule;

#[module(providers = [Log, AGateway, A2Gateway, BGateway, RealtimeChatGateway, RootPathGateway, CorsAnyGateway, CorsOneGateway, CorsListGateway])]
pub struct PathsModule;

// ------------------------------------------------------------------------------------------------------ helpers

async fn start<M: Module>(adapter: IoAdapter) -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    app.use_websocket_adapter(adapter);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

/// `"2"` + namespace + ack id + the JSON array: the text of an EVENT packet (the client adds the Engine.IO prefix).
fn event_packet(nsp: &str, id: Option<u64>, body: &str) -> String {
    let nsp = if nsp == "/" { String::new() } else { format!("{nsp},") };
    format!("2{nsp}{}{body}", id.map(|i| i.to_string()).unwrap_or_default())
}

fn ack_packet(nsp: &str, id: u64, body: &str) -> String {
    let nsp = if nsp == "/" { String::new() } else { format!("{nsp},") };
    format!("3{nsp}{id}{body}")
}

/// Emits `body` (`["name", ...]`) with ack id 1.
async fn emit_ack(client: &mut Raw, nsp: &str, body: &str) {
    client.send(&event_packet(nsp, Some(1), body)).await;
}

async fn emit(client: &mut Raw, nsp: &str, body: &str) {
    client.send(&event_packet(nsp, None, body)).await;
}

/// The next packet is the marker: everything the previous events did has been sent before it.
async fn expect_marker(client: &mut Raw, nsp: &str) {
    emit(client, nsp, r#"["mark"]"#).await;
    assert_eq!(client.recv_text().await, event_packet(nsp, None, r#"["done",true]"#));
}

async fn connect(addr: SocketAddr, nsp: &str, auth: Option<&str>) -> Raw {
    let mut client = Raw::connect(addr).await;
    client.join(nsp, auth).await;
    client
}

// ------------------------------------------------------------------------------------- adapter selection

#[tokio::test]
async fn opting_in_serves_the_namespace_on_the_default_path_and_not_a_plain_websocket() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = Raw::connect(addr).await;
    let sid = client.join("/chat", None).await;
    assert_eq!(sid.len(), 20);
    // A plain WebSocket upgrade to the gateway's namespace as a URL path does not reach it.
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/chat")).await.is_err());
    // The log says where the gateway runs.
    let log = app.get::<Log>().unwrap();
    log.wait_for("init:/chat").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_same_declaration_under_the_ws_adapter_is_a_url_path() {
    #[websocket_gateway(namespace = "/wschat")]
    pub struct WsChat;

    #[handlers]
    impl WsChat {
        #[subscribe_message("echo")]
        fn echo(&self, #[message_body] data: Value) -> Value {
            data
        }
    }

    #[module(providers = [WsChat])]
    pub struct WsModule;

    use futures::{SinkExt, StreamExt};
    let mut app = CorvisFactory::create::<WsModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/wschat")).await.unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(r#"{"event":"echo","data":5}"#.into())).await.unwrap();
    let reply = ws.next().await.unwrap().unwrap();
    assert_eq!(reply.into_text().unwrap(), r#"{"event":"echo","data":5}"#);
    // The Socket.IO path does not exist under the default adapter.
    let resp = http(addr, "GET", "/socket.io/?EIO=4&transport=polling", &[], b"").await;
    assert_eq!(resp.status, 404);
    app.close().await.unwrap();
}

#[tokio::test]
async fn plain_websocket_servers_next_to_socket_io_gateways_are_untouched() {
    let mut app = CorvisFactory::create::<MainModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let log = app.get::<Log>().unwrap();
    let wss = corvis::ws::WebSocketServer::new(corvis::ws::WebSocketServerOptions::default());
    let seen = log.clone();
    app.get_http_server().on_upgrade(move |req| {
        let seen = seen.clone();
        if req.pathname() == "/ws/firehose" {
            wss.handle_upgrade(&req, move |_ws| seen.push("firehose"));
        }
    });
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let (_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/firehose")).await.unwrap();
    log.wait_for("firehose").await;
    // And Socket.IO still works on the same server.
    connect(addr, "/chat", None).await;
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------- options and grouping

#[tokio::test]
async fn a_custom_engine_path_serves_the_namespace_and_the_default_path_does_not_exist() {
    let (mut app, addr) = start::<PathsModule>(IoAdapter::new()).await;
    let mut client = Raw::connect_path(addr, "/realtime/").await;
    client.join("/chat2", None).await;
    emit_ack(&mut client, "/chat2", r#"["ping"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat2", 1, r#"["realtime"]"#));
    // Namespace `/` on that path is not declared by anyone.
    client.send("0").await;
    assert_eq!(client.recv_text().await, r#"4{"message":"Invalid namespace"}"#);
    // `/socket.io` is still the path of the other gateways here (`/a`, `/b`), so it answers.
    let resp = http(addr, "GET", "/socket.io/?EIO=4&transport=polling", &[], b"").await;
    assert_eq!(resp.status, 200);
    app.close().await.unwrap();
}

#[tokio::test]
async fn path_alone_serves_the_root_namespace() {
    let (mut app, addr) = start::<PathsModule>(IoAdapter::new()).await;
    let mut client = Raw::connect_path(addr, "/rootpath/").await;
    client.join("/", None).await;
    emit_ack(&mut client, "/", r#"["ping"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/", 1, r#"["root"]"#));
    app.close().await.unwrap();
}

#[tokio::test]
async fn two_gateways_are_namespaces_of_one_session() {
    let (mut app, addr) = start::<PathsModule>(IoAdapter::new()).await;
    let mut client = Raw::connect(addr).await;
    client.join("/a", None).await;
    client.join("/b", None).await;
    emit_ack(&mut client, "/a", r#"["ping"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/a", 1, r#"["a"]"#));
    emit_ack(&mut client, "/b", r#"["ping"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/b", 1, r#"["b"]"#));
    app.close().await.unwrap();
}

#[tokio::test]
async fn two_gateways_on_one_namespace_serve_both_handler_sets_and_run_both_hooks_in_order() {
    let (mut app, addr) = start::<PathsModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = connect(addr, "/a", None).await;
    emit_ack(&mut client, "/a", r#"["ping"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/a", 1, r#"["a"]"#));
    emit_ack(&mut client, "/a", r#"["pong"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/a", 1, r#"["a2"]"#));
    let hooks: Vec<String> = log.entries().into_iter().filter(|e| e.ends_with("-connection")).collect();
    assert_eq!(hooks, ["a-connection", "a2-connection"], "registration order");
    app.close().await.unwrap();
}

#[tokio::test]
async fn options_come_from_the_adapter_per_path() {
    let adapter = IoAdapter::new().options(IoOptions::new().engine(EngineOptions::new().ping_interval(Duration::from_millis(300))));
    let (mut app, addr) = start::<PathsModule>(adapter).await;
    let client = Raw::connect(addr).await;
    assert_eq!(client.open["pingInterval"], 300);
    // A path without options uses the defaults.
    let other = Raw::connect_path(addr, "/rootpath/").await;
    assert_eq!(other.open["pingInterval"], 25000);
    app.close().await.unwrap();
}

#[tokio::test]
async fn cors_preflight_and_any_origin() {
    let (mut app, addr) = start::<PathsModule>(IoAdapter::new()).await;
    let preflight = http(
        addr,
        "OPTIONS",
        "/cors-one/?EIO=4&transport=polling",
        &[("Origin", "https://app.example"), ("Access-Control-Request-Method", "GET")],
        b"",
    )
    .await;
    assert_eq!(preflight.status, 204);
    assert_eq!(preflight.header("access-control-allow-origin"), Some("https://app.example"));
    let any = http(addr, "GET", "/cors-any/?EIO=4&transport=polling", &[("Origin", "https://whoever.example")], b"").await;
    assert_eq!(any.status, 200);
    assert_eq!(any.header("access-control-allow-origin"), Some("*"));
    assert_eq!(any.header("access-control-allow-credentials"), None);
    // A list reflects the origin it matches.
    let listed = http(addr, "GET", "/cors-list/?EIO=4&transport=polling", &[("Origin", "https://b.example")], b"").await;
    assert_eq!(listed.header("access-control-allow-origin"), Some("https://b.example"));
    // Gateways without `cors` send no CORS headers.
    let none = http(addr, "GET", "/socket.io/?EIO=4&transport=polling", &[("Origin", "https://app.example")], b"").await;
    assert_eq!(none.header("access-control-allow-origin"), None);
    app.close().await.unwrap();
}

// -------------------------------------------------------------------------------------- startup errors

macro_rules! gateways_module {
    ($module:ident, [$($gateway:ident),*]) => {
        #[module(providers = [$($gateway),*])]
        pub struct $module;
    };
}

mod startup {
    use super::*;

    #[websocket_gateway(path = "/p", cors = "https://a.example")]
    pub struct CorsA;
    #[websocket_gateway(path = "/p", cors = "https://b.example")]
    pub struct CorsB;
    gateways_module!(ConflictingCors, [CorsA, CorsB]);

    #[websocket_gateway(path = "/p", cors = true)]
    pub struct CorsTwice;
    gateways_module!(CorsTwiceModule, [CorsTwice]);

    #[websocket_gateway(path = "/io")]
    pub struct IoPath;
    #[websocket_gateway(path = "/io/v2")]
    pub struct IoV2Path;
    gateways_module!(OverlapModule, [IoPath, IoV2Path]);

    #[websocket_gateway("/dup")]
    pub struct DupA;
    #[handlers]
    impl DupA {
        #[subscribe_message("message")]
        fn message(&self) {}
    }
    #[websocket_gateway("/dup")]
    pub struct DupB;
    #[handlers]
    impl DupB {
        #[subscribe_message("message")]
        fn message(&self) {}
    }
    gateways_module!(DupModule, [DupA, DupB]);

    #[websocket_gateway("/same")]
    pub struct SameA;
    #[handlers]
    impl SameA {
        #[subscribe_message("m")]
        fn m(&self) {}
    }
    #[websocket_gateway("/same")]
    pub struct SameB;
    #[handlers]
    impl SameB {
        #[subscribe_message("n")]
        fn n(&self) {}
    }
    gateways_module!(SameNamespaceModule, [SameA, SameB]);

    /// Written for the `ws` adapter.
    #[websocket_gateway("/wsonly")]
    pub struct WsOnlySocket;
    #[handlers]
    impl WsOnlySocket {
        #[subscribe_message("hello")]
        fn hello(&self, #[connected_socket] _client: WsClient) {}
    }
    gateways_module!(WsOnlySocketModule, [WsOnlySocket]);

    #[websocket_gateway("/ioonly")]
    pub struct IoOnlyServer {
        #[websocket_server]
        _server: IoGatewayServer,
    }
    gateways_module!(IoOnlyServerModule, [IoOnlyServer]);

    #[websocket_gateway("/wshook")]
    pub struct WsOnlyHook;
    #[async_trait]
    impl OnGatewayConnection for WsOnlyHook {
        async fn handle_connection(&self, _client: WsClient) {}
    }
    gateways_module!(WsOnlyHookModule, [WsOnlyHook]);

    #[websocket_gateway("/iohook")]
    pub struct IoOnlyHook;
    #[async_trait]
    impl OnGatewayInit<IoNamespace> for IoOnlyHook {
        async fn after_init(&self, _namespace: IoNamespace) {}
    }
    gateways_module!(IoOnlyHookModule, [IoOnlyHook]);

    #[websocket_gateway("/both")]
    pub struct Both {
        log: Arc<Log>,
    }
    #[handlers]
    impl Both {
        #[subscribe_message("ping")]
        fn ping(&self) -> &'static str {
            "pong"
        }
    }
    #[async_trait]
    impl OnGatewayConnection for Both {
        async fn handle_connection(&self, _client: WsClient) {
            self.log.push("ws-connection");
        }
    }
    #[async_trait]
    impl OnGatewayConnection<IoSocket> for Both {
        async fn handle_connection(&self, _socket: IoSocket) {
            self.log.push("io-connection");
        }
    }
    #[module(providers = [Log, Both])]
    pub struct BothModule;

    #[websocket_gateway("/nohooks")]
    pub struct NoHooks;
    gateways_module!(NoHooksModule, [NoHooks]);
}

async fn init_of<M: Module>(adapter: Option<IoAdapter>) -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    if let Some(adapter) = adapter {
        app.use_websocket_adapter(adapter);
    }
    app.init().await
}

#[tokio::test]
async fn conflicting_cors_on_one_path_names_both_gateways() {
    let message = format!("{:#}", init_of::<startup::ConflictingCors>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(message.contains("CorsA") && message.contains("CorsB") && message.contains("/p/") && message.contains("cors"), "{message}");
}

#[tokio::test]
async fn cors_declared_twice_names_the_gateway_and_the_path() {
    let options = IoOptions::new().engine(EngineOptions::new().path("/p").cors(CorsOptions::new()));
    let message = format!("{:#}", init_of::<startup::CorsTwiceModule>(Some(IoAdapter::new().options(options))).await.unwrap_err());
    assert!(message.contains("CorsTwice") && message.contains("/p/") && message.contains("EngineOptions::cors"), "{message}");
}

#[tokio::test]
async fn options_given_twice_for_one_path_name_the_path() {
    let adapter = IoAdapter::new().options(IoOptions::new()).options(IoOptions::new().max_inflight(3));
    let message = format!("{:#}", init_of::<startup::NoHooksModule>(Some(adapter)).await.unwrap_err());
    assert!(message.contains("/socket.io/") && message.contains("twice"), "{message}");
}

#[tokio::test]
async fn invalid_options_name_the_path_and_the_option() {
    let adapter = IoAdapter::new().options(IoOptions::new().max_inflight(0));
    let message = format!("{:#}", init_of::<startup::NoHooksModule>(Some(adapter)).await.unwrap_err());
    assert!(message.contains("/socket.io/") && message.contains("max_inflight"), "{message}");
}

#[tokio::test]
async fn overlapping_engine_paths_name_both() {
    let message = format!("{:#}", init_of::<startup::OverlapModule>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(message.contains("/io/") && message.contains("/io/v2/") && message.contains("overlap"), "{message}");
}

#[tokio::test]
async fn the_same_event_in_two_gateways_of_a_namespace_fails_naming_both_and_the_event() {
    let message = format!("{:#}", init_of::<startup::DupModule>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(
        message.contains("DupA") && message.contains("DupB") && message.contains("\"message\"") && message.contains("/dup"),
        "{message}"
    );
}

#[tokio::test]
async fn different_events_in_gateways_of_one_namespace_start() {
    init_of::<startup::SameNamespaceModule>(Some(IoAdapter::new())).await.unwrap();
}

#[tokio::test]
async fn a_socket_handle_of_the_wrong_adapter_fails_startup_naming_gateway_handler_and_handle() {
    let message = format!("{:#}", init_of::<startup::WsOnlySocketModule>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(message.contains("WsOnlySocket::hello") && message.contains("WsClient") && message.contains("IoSocket"), "{message}");
}

#[tokio::test]
async fn a_socket_io_server_field_under_the_default_adapter_fails_startup() {
    let message = format!("{:#}", init_of::<startup::IoOnlyServerModule>(None).await.unwrap_err());
    assert!(message.contains("IoOnlyServer") && message.contains("IoGatewayServer") && message.contains("WsServer"), "{message}");
}

#[tokio::test]
async fn a_hook_of_the_wrong_adapter_fails_startup_in_both_directions() {
    let message = format!("{:#}", init_of::<startup::WsOnlyHookModule>(Some(IoAdapter::new())).await.unwrap_err());
    assert!(message.contains("WsOnlyHook") && message.contains("OnGatewayConnection<IoSocket>"), "{message}");
    let message = format!("{:#}", init_of::<startup::IoOnlyHookModule>(None).await.unwrap_err());
    assert!(message.contains("IoOnlyHook") && message.contains("OnGatewayInit"), "{message}");
    let mut app = CorvisFactory::create::<startup::IoOnlyHookModule>().await.unwrap();
    app.use_websocket_adapter(WsAdapter);
    let explicit = format!("{:#}", app.init().await.unwrap_err());
    assert!(explicit.contains("IoOnlyHook"), "{explicit}");
}

#[tokio::test]
async fn a_gateway_with_each_hook_for_both_adapters_starts_under_either_and_the_matching_hook_runs() {
    use futures::{SinkExt, StreamExt};
    // Socket.IO.
    let (mut app, addr) = start::<startup::BothModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    connect(addr, "/both", None).await;
    log.wait_for("io-connection").await;
    assert!(!log.entries().contains(&"ws-connection".to_string()));
    app.close().await.unwrap();
    // The default adapter.
    let mut app = CorvisFactory::create::<startup::BothModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let log = app.get::<Log>().unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/both")).await.unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(r#"{"event":"ping","data":1}"#.into())).await.unwrap();
    assert_eq!(ws.next().await.unwrap().unwrap().into_text().unwrap(), r#"{"event":"ping","data":"pong"}"#);
    log.wait_for("ws-connection").await;
    assert!(!log.entries().contains(&"io-connection".to_string()));
    app.close().await.unwrap();
}

#[tokio::test]
async fn use_websocket_adapter_after_init_has_no_effect() {
    let mut app = CorvisFactory::create::<startup::NoHooksModule>().await.unwrap();
    app.init().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    // Still the `ws` adapter: the gateway is a URL path and Socket.IO does not exist.
    assert_eq!(http(addr, "GET", "/socket.io/?EIO=4&transport=polling", &[], b"").await.status, 404);
    assert!(tokio_tungstenite::connect_async(format!("ws://{addr}/nohooks")).await.is_ok());
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------------------ the handle

#[tokio::test]
async fn the_server_handle_fails_before_the_gateway_started_and_works_after() {
    let mut app = CorvisFactory::create::<MainModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let gateway = app.get::<ChatGateway>().unwrap();
    assert!(gateway.server.namespace().is_none());
    assert_eq!(gateway.server.to("room").emit("news", ("x",)).await, Err(EmitError::NotStarted));
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert_eq!(gateway.server.namespace().unwrap().name(), "/chat");
    // Exactly the sockets that joined the room receive the broadcast.
    let mut in_room = connect(addr, "/chat", None).await;
    let mut outside = connect(addr, "/chat", None).await;
    emit(&mut in_room, "/chat", r#"["join",{"room":"lobby"}]"#).await;
    expect_marker(&mut in_room, "/chat").await;
    emit(&mut in_room, "/chat", r#"["shout",{"room":"lobby"}]"#).await;
    assert_eq!(in_room.recv_text().await, event_packet("/chat", None, r#"["news","hello"]"#));
    expect_marker(&mut outside, "/chat").await;
    app.close().await.unwrap();
}

// ----------------------------------------------------------------------------- dispatch and the results

#[tokio::test]
async fn the_body_follows_the_arguments() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/chat", None).await;
    emit_ack(&mut client, "/chat", r#"["echo",{"a":1}]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"[{"a":1}]"#));
    emit_ack(&mut client, "/chat", r#"["args",1,"two",{"three":3}]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"[{"body":[1,"two",{"three":3}]}]"#));
    emit_ack(&mut client, "/chat", r#"["args"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"[{"body":null}]"#));
    emit_ack(&mut client, "/chat", r#"["room",{"room":"r1"}]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"["r1"]"#));
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_binary_argument_is_null_in_the_body() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/chat", None).await;
    client.send(r#"51-/chat,1["upload",{"_placeholder":true,"num":0}]"#).await;
    client.send_binary(&[1, 2, 3]).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"[{"body":null}]"#));
    app.close().await.unwrap();
}

#[tokio::test]
async fn every_row_of_the_result_table() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/chat", None).await;
    // A falsy value is still acknowledged.
    emit_ack(&mut client, "/chat", r#"["falsy"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, "[0]"));
    // Without a callback nothing is sent for a value.
    emit(&mut client, "/chat", r#"["echo","x"]"#).await;
    expect_marker(&mut client, "/chat").await;
    // Nothing returned: the callback is not called.
    emit_ack(&mut client, "/chat", r#"["silent"]"#).await;
    expect_marker(&mut client, "/chat").await;
    emit_ack(&mut client, "/chat", r#"["null"]"#).await;
    expect_marker(&mut client, "/chat").await;
    // A WsResponse is emitted, with or without a callback, and the callback stays unanswered.
    emit_ack(&mut client, "/chat", r#"["reply"]"#).await;
    assert_eq!(client.recv_text().await, event_packet("/chat", None, r#"["reply",1]"#));
    expect_marker(&mut client, "/chat").await;
    emit(&mut client, "/chat", r#"["reply"]"#).await;
    assert_eq!(client.recv_text().await, event_packet("/chat", None, r#"["reply",1]"#));
    // An object with an `event` field is emitted too.
    emit_ack(&mut client, "/chat", r#"["duck"]"#).await;
    assert_eq!(client.recv_text().await, event_packet("/chat", None, r#"["duck",2]"#));
    expect_marker(&mut client, "/chat").await;
    // A reply for a reserved event sends nothing and the socket stays connected.
    emit_ack(&mut client, "/chat", r#"["reserved"]"#).await;
    expect_marker(&mut client, "/chat").await;
    // An event nobody subscribes to is dropped: no reply, no exception.
    emit_ack(&mut client, "/chat", r#"["nobody-listens"]"#).await;
    expect_marker(&mut client, "/chat").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn exceptions_are_events_and_never_answer_the_callback() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/chat", None).await;
    let exception = |body: &str| event_packet("/chat", None, &format!(r#"["exception",{body}]"#));
    emit_ack(&mut client, "/chat", r#"["fail-str"]"#).await;
    assert_eq!(client.recv_text().await, exception(r#"{"status":"error","message":"Room is full","cause":{"pattern":"fail-str"}}"#));
    emit_ack(&mut client, "/chat", r#"["fail-obj"]"#).await;
    assert_eq!(client.recv_text().await, exception(r#"{"code":"ROOM_FULL"}"#));
    // An HTTP exception keeps its message (an expected difference from the reference).
    emit_ack(&mut client, "/chat", r#"["fail-http"]"#).await;
    let text = client.recv_text().await;
    assert!(text.starts_with(&exception("").replace("]", "")) && text.contains("bad input"), "{text}");
    // Unknown errors and panics say `Internal server error` and nothing of the original text.
    for event in ["fail-unknown", "boom"] {
        emit_ack(&mut client, "/chat", &format!(r#"["{event}"]"#)).await;
        let text = client.recv_text().await;
        assert_eq!(text, exception(&format!(r#"{{"status":"error","message":"Internal server error","cause":{{"pattern":"{event}"}}}}"#)));
        assert!(!text.contains("secret"));
    }
    // The connection survives a panic: the next event works.
    expect_marker(&mut client, "/chat").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn guards_pipes_interceptors_and_filters_run_in_the_gateway_pipeline() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    // No token: the guard denies.
    let mut denied = connect(addr, "/rules", None).await;
    emit_ack(&mut denied, "/rules", r#"["guarded"]"#).await;
    assert_eq!(
        denied.recv_text().await,
        event_packet("/rules", None, r#"["exception",{"status":"error","message":"Forbidden resource","cause":{"pattern":"guarded"}}]"#)
    );
    assert!(!log.entries().contains(&"rules-handler".to_string()));
    // With a token: the handler runs, the context was a WebSocket one with the gateway as class.
    let mut client = connect(addr, "/rules", Some(r#"{"token":"t"}"#)).await;
    emit_ack(&mut client, "/rules", r#"["guarded"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/rules", 1, r#"["ok"]"#));
    assert!(log.entries().iter().any(|e| e.starts_with("guard:RulesGateway:guarded:null:")), "{:?}", log.entries());
    // A failing pipe: the handler does not run and the client gets `exception`.
    emit_ack(&mut client, "/rules", r#"["piped","x"]"#).await;
    let text = client.recv_text().await;
    assert!(text.contains("exception") && text.contains("not a number"), "{text}");
    emit_ack(&mut client, "/rules", r#"["piped",4]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/rules", 1, r#"[{"n":4}]"#));
    // The interceptor replaces the result.
    emit_ack(&mut client, "/rules", r#"["wrapped"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/rules", 1, r#"[{"wrapped":7}]"#));
    // A gateway filter answers with its own body, and an empty body sends nothing.
    emit_ack(&mut client, "/rules", r#"["fail-code"]"#).await;
    assert_eq!(client.recv_text().await, event_packet("/rules", None, r#"["exception",{"by":"code-filter"}]"#));
    emit_ack(&mut client, "/rules", r#"["fail-quiet"]"#).await;
    expect_marker(&mut client, "/rules").await;
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_handler_registered_on_the_socket_wins_over_the_gateway_handler() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = connect(addr, "/chat", None).await;
    emit_ack(&mut client, "/chat", r#"["manual"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/chat", 1, r#"["socket"]"#));
    assert!(log.entries().contains(&"socket-manual".to_string()));
    assert!(!log.entries().contains(&"gateway-manual".to_string()));
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_socket_handle_has_id_handshake_rooms_and_the_client_address() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/chat", Some(r#"{"token":"abc"}"#)).await;
    emit(&mut client, "/chat", r#"["join",{"room":"r1"}]"#).await;
    emit_ack(&mut client, "/chat", r#"["whoami"]"#).await;
    assert_eq!(
        client.recv_text().await,
        ack_packet("/chat", 1, r#"[{"id_matches":true,"ip":"127.0.0.1","rooms":["r1"],"auth":{"token":"abc"},"ns":"/chat"}]"#)
    );
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_client_address_follows_trust_proxy() {
    let mut app = CorvisFactory::create::<MainModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    app.set_trust_proxy(true);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let (mut ws, _) = {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket").into_client_request().unwrap();
        request.headers_mut().insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        tokio_tungstenite::connect_async(request).await.unwrap()
    };
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    // Open packet, then join `/chat` and ask.
    let _open = ws.next().await.unwrap().unwrap();
    ws.send(Message::Text("40/chat,".into())).await.unwrap();
    let _connected = ws.next().await.unwrap().unwrap();
    ws.send(Message::Text(r#"42/chat,1["whoami"]"#.into())).await.unwrap();
    let answer = ws.next().await.unwrap().unwrap().into_text().unwrap();
    assert!(answer.contains(r#""ip":"203.0.113.9""#), "{answer}");
    app.close().await.unwrap();
}

// --------------------------------------------------------------------------------------- the lifecycle

#[tokio::test]
async fn hooks_run_in_order_and_disconnect_exactly_once() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    log.wait_for("init:/chat").await;
    let mut client = connect(addr, "/chat", Some(r#"{"token":"t"}"#)).await;
    emit(&mut client, "/chat", r#"["join",{"room":"r"}]"#).await;
    expect_marker(&mut client, "/chat").await;
    client.close().await;
    log.wait_for("disconnect:rooms=0").await;
    let chat: Vec<String> = log
        .entries()
        .into_iter()
        .filter(|e| e.starts_with("init:") || e.starts_with("connection:") || e.starts_with("disconnect:"))
        .collect();
    assert_eq!(
        chat,
        [r#"init:/chat"#, r#"connection:{"token":"t"}"#, "disconnect:rooms=0"],
        "init, then the connection (it saw the handshake auth), then one disconnect that reads no rooms"
    );
    app.close().await.unwrap();
}

/// Security review of #185, F1: a polling client that posts CONNECT and never polls must still go through
/// `handle_connection` (where a gateway authenticates its sockets): the hook runs when the socket is announced, not when
/// the client chooses to collect its CONNECT, and a socket it refused gets no broadcast.
#[tokio::test]
async fn the_connection_hook_runs_for_a_polling_client_that_does_not_poll_and_a_socket_it_refuses_gets_no_broadcast() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let life = app.get::<LifecycleGateway>().unwrap().server.namespace().expect("started");
    let mut member = Raw::connect(addr).await;
    member.join("/life", Some(r#"{"token":"member"}"#)).await;
    let (sid, _) = engine_support::open_polling(addr).await;
    // No GET is pending, and none comes until the hook ran.
    assert_eq!(engine_support::post(addr, &sid, r#"40/life,{"token":"refuse"}"#).await.status, 200);
    log.wait_for("life-refused").await;
    life.emit("news", ()).await.unwrap();
    assert_eq!(member.recv_text().await, r#"2/life,["news"]"#, "the broadcast ran");
    // The client polls now: its CONNECT (alone in its response), then the disconnect, and nothing of the broadcast.
    let first = engine_support::packets(&engine_support::poll(addr, &sid).await.text());
    assert!(matches!(&first[..], [connect] if connect.starts_with("40/life,{\"sid\"")), "{first:?}");
    assert_eq!(engine_support::packets(&engine_support::poll(addr, &sid).await.text()), ["41/life,"]);
    app.close().await.unwrap();
}

#[tokio::test]
async fn connect_middleware_registered_in_after_init_runs_before_the_connection_hook() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.send(r#"0/life,{"token":"no"}"#).await;
    assert_eq!(client.recv_text().await, r#"4/life,{"message":"not allowed"}"#);
    assert!(!log.entries().iter().any(|e| e.starts_with("life-connection")), "{:?}", log.entries());
    app.close().await.unwrap();
}

#[tokio::test]
async fn a_panicking_connection_hook_disconnects_only_that_socket() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut other = connect(addr, "/life", Some(r#"{"token":"fine"}"#)).await;
    let mut boom = Raw::connect(addr).await;
    boom.send(r#"0/life,{"token":"boom"}"#).await;
    assert!(boom.closed_within(Duration::from_secs(10)).await, "the connection of the socket whose hook panicked is closed");
    log.wait_for("life-disconnect:rooms=0").await;
    emit_ack(&mut other, "/life", r#"["hello"]"#).await;
    assert_eq!(other.recv_text().await, ack_packet("/life", 1, r#"["hi"]"#));
    app.close().await.unwrap();
}

#[tokio::test]
async fn an_early_event_waits_for_the_connection_hook_and_is_not_lost() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.send(r#"0/life,{"token":"slow"}"#).await;
    // CONNECT is answered before the hook finishes; the event is sent while the hook still waits.
    let connected = client.recv_text().await;
    assert!(connected.starts_with("0/life,"), "{connected}");
    log.wait_for("life-hook-waiting").await;
    emit_ack(&mut client, "/life", r#"["hello"]"#).await;
    log.release.notify_one();
    assert_eq!(client.recv_text().await, ack_packet("/life", 1, r#"["hi"]"#));
    let entries = log.entries();
    let position = |needle: &str| entries.iter().position(|e| e == needle).unwrap_or_else(|| panic!("{needle} in {entries:?}"));
    assert!(position("life-hook-released") < position("life-handler"), "the handler ran after the hook returned: {entries:?}");
    app.close().await.unwrap();
}

#[tokio::test]
async fn early_events_are_discarded_when_the_hook_disconnects_the_socket() {
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let log = app.get::<Log>().unwrap();
    let mut client = Raw::connect(addr).await;
    client.send(r#"0/life,{"token":"kick"}"#).await;
    let _connected = client.recv_text().await;
    log.wait_for("life-hook-waiting").await;
    emit_ack(&mut client, "/life", r#"["hello"]"#).await;
    log.release.notify_one();
    log.wait_for("life-disconnect:rooms=0").await;
    // Whatever the discarded event could have done, it did not: no handler ran. Absence of an effect on another task, with
    // nothing that would mark its end (a discarded event leaves no trace): a quiet period, not a condition.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!log.entries().iter().any(|e| e == "life-handler"), "{:?}", log.entries());
    app.close().await.unwrap();
}

#[tokio::test]
async fn the_handshake_is_the_request_of_the_context() {
    // Checked inside `NeedsToken` (it asserts the context's request is the socket's handshake request).
    let (mut app, addr) = start::<MainModule>(IoAdapter::new()).await;
    let mut client = connect(addr, "/rules", Some(r#"{"token":"t"}"#)).await;
    emit_ack(&mut client, "/rules", r#"["guarded"]"#).await;
    assert_eq!(client.recv_text().await, ack_packet("/rules", 1, r#"["ok"]"#));
    app.close().await.unwrap();
}

// ------------------------------------------------------------------------------------- the throttler

#[websocket_gateway("/throttled")]
pub struct ThrottledGateway;

#[handlers]
impl ThrottledGateway {
    #[subscribe_message("hit")]
    fn hit(&self) -> &'static str {
        "ok"
    }
}

#[module(
    imports = [corvis::ThrottlerModule::for_root(vec![corvis::ThrottlerOptions::new(60_000, 1)])],
    providers = [ThrottledGateway, Provider::app_guard::<corvis::ThrottlerGuard>()]
)]
pub struct ThrottledModule;

#[tokio::test]
async fn the_throttler_keys_a_socket_io_handler_by_the_client_address_with_trust_proxy() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let mut app = CorvisFactory::create::<ThrottledModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    app.set_trust_proxy(true);
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let connect = |forwarded: &'static str| async move {
        let mut request = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket").into_client_request().unwrap();
        request.headers_mut().insert("x-forwarded-for", forwarded.parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        ws.next().await.unwrap().unwrap(); // the open packet
        ws.send(Message::Text("40/throttled,".into())).await.unwrap();
        ws.next().await.unwrap().unwrap(); // CONNECT
        ws
    };
    let mut first = connect("203.0.113.1").await;
    first.send(Message::Text(r#"42/throttled,1["hit"]"#.into())).await.unwrap();
    assert_eq!(first.next().await.unwrap().unwrap().into_text().unwrap(), r#"43/throttled,1["ok"]"#);
    // The same address is over its budget.
    first.send(Message::Text(r#"42/throttled,2["hit"]"#.into())).await.unwrap();
    let denied = first.next().await.unwrap().unwrap().into_text().unwrap();
    assert!(denied.starts_with(r#"42/throttled,["exception""#) && denied.contains("429"), "{denied}");
    // Another address has its own budget.
    let mut second = connect("203.0.113.2").await;
    second.send(Message::Text(r#"42/throttled,1["hit"]"#.into())).await.unwrap();
    assert_eq!(second.next().await.unwrap().unwrap().into_text().unwrap(), r#"43/throttled,1["ok"]"#);
    app.close().await.unwrap();
}

#[allow(dead_code)]
fn unused(_: Msg, _: WsServer) {}

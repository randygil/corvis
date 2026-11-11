//! The Corvis side of the gateway scenarios: the same application as `parity/socket-io/gateway-reference/gateway-server.mjs`,
//! written with `#[websocket_gateway]` over the Socket.IO adapter. Change one and you change the other. Servers bind to
//! 127.0.0.1.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{LazyLock, Mutex};

use corvis::prelude::*;
use corvis::socket_io::{ConnectError, IoAdapter, IoGatewayServer, IoNamespace, IoSocket};
use corvis::{OnGatewayConnection, OnGatewayDisconnect, OnGatewayInit, WsException, WsResponse, websocket_gateway};
use serde_json::{Value, json};

/// What happened on the server side, in order: served by `GET /harness/log`.
static LOG: LazyLock<Mutex<Vec<Value>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn record(entry: Value) {
    LOG.lock().unwrap().push(entry);
}

// -------------------------------------------------------------------------------------------------------- enhancers

#[injectable]
pub struct NeedsToken;

#[async_trait]
impl CanActivate for NeedsToken {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let ws = ctx.switch_to_ws();
        record(json!({
            "event": "guard",
            "type": ctx.get_type().as_str(),
            "class": ctx.get_class().name,
            "handler": ctx.get_handler().name,
            "data": ws.get_data().cloned().unwrap_or(Value::Null),
        }));
        Ok(ws.get_client::<IoSocket>().is_some_and(|s| s.handshake().auth().get("token").is_some()))
    }
}

#[injectable]
pub struct NumberPipe;

#[async_trait]
impl PipeTransform for NumberPipe {
    async fn transform(&self, value: Value, _meta: &ArgumentMetadata) -> Result<Value, HttpException> {
        if value.is_number() { Ok(value) } else { Err(WsException::new("not a number")) }
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

#[injectable]
pub struct RoomErrorFilter;

#[async_trait]
impl ExceptionFilter for RoomErrorFilter {
    fn catches(&self, e: &HttpException) -> bool {
        e.message() == "room-error"
    }
    async fn catch(&self, _e: HttpException, _host: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "room-filter" }))
    }
}

#[injectable]
pub struct QuietFilter;

#[async_trait]
impl ExceptionFilter for QuietFilter {
    fn catches(&self, e: &HttpException) -> bool {
        e.message() == "quiet-error"
    }
    async fn catch(&self, _e: HttpException, _host: &ArgumentsHost) -> Response {
        Response::default()
    }
}

// -------------------------------------------------------------------------------------------------------- gateways

#[websocket_gateway]
pub struct MainGateway {
    #[websocket_server]
    server: IoGatewayServer,
}

#[handlers]
impl MainGateway {
    #[subscribe_message("echo")]
    fn echo(&self, #[message_body] body: Value) -> Value {
        record(json!({ "event": "message", "handler": "echo" }));
        body
    }

    #[subscribe_message("silent")]
    fn silent(&self) {}

    #[subscribe_message("nothing")]
    fn nothing(&self) -> Value {
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

    #[subscribe_message("args")]
    fn args(&self, #[message_body] body: Value) -> Value {
        json!({ "body": body })
    }

    #[subscribe_message("whoami")]
    fn whoami(&self, #[connected_socket] socket: IoSocket) -> Value {
        let mut rooms: Vec<String> = socket.rooms().into_iter().filter(|r| r != socket.id()).collect();
        rooms.sort();
        json!({ "idLength": socket.id().len(), "rooms": rooms, "auth": socket.handshake().auth(), "nsp": socket.namespace().name() })
    }

    #[subscribe_message("join")]
    fn join(&self, #[connected_socket] socket: IoSocket, #[message_body("room")] room: String) {
        socket.join(room).unwrap();
    }

    #[subscribe_message("shout")]
    async fn shout(&self, #[message_body("room")] room: String) {
        let _ = self.server.to(room).emit("news", ("hello",)).await;
    }

    #[subscribe_message("fail-ws")]
    fn fail_ws(&self) -> Result<(), HttpException> {
        Err(WsException::new("Room is full"))
    }

    #[subscribe_message("fail-ws-object")]
    fn fail_ws_object(&self) -> Result<(), HttpException> {
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

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }

    #[subscribe_message("upload")]
    fn upload(&self, #[message_body] body: Value) -> Value {
        json!({ "body": body })
    }
}

#[async_trait]
impl OnGatewayInit<IoNamespace> for MainGateway {
    async fn after_init(&self, namespace: IoNamespace) {
        record(json!({ "event": "init", "nsp": namespace.name() }));
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for MainGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        record(json!({ "event": "connection", "nsp": socket.namespace().name(), "auth": socket.handshake().auth() }));
    }
}

#[async_trait]
impl OnGatewayDisconnect<IoSocket> for MainGateway {
    async fn handle_disconnect(&self, socket: IoSocket) {
        record(json!({ "event": "disconnect", "nsp": socket.namespace().name() }));
    }
}

#[websocket_gateway("/rules")]
#[use_guards(NeedsToken)]
#[use_filters(RoomErrorFilter, QuietFilter)]
pub struct RulesGateway;

#[handlers]
impl RulesGateway {
    #[subscribe_message("guarded")]
    fn guarded(&self) -> &'static str {
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

    #[subscribe_message("fail-room")]
    fn fail_room(&self) -> Result<(), HttpException> {
        Err(HttpException::new("room-error", 400))
    }

    #[subscribe_message("fail-quiet")]
    fn fail_quiet(&self) -> Result<(), HttpException> {
        Err(HttpException::new("quiet-error", 400))
    }

    #[subscribe_message("mark")]
    fn mark(&self) -> WsResponse {
        WsResponse::new("done", true)
    }
}

#[websocket_gateway("/mw")]
pub struct MiddlewareGateway;

#[handlers]
impl MiddlewareGateway {
    #[subscribe_message("hello")]
    fn hello(&self) -> &'static str {
        "hi"
    }
}

#[async_trait]
impl OnGatewayInit<IoNamespace> for MiddlewareGateway {
    async fn after_init(&self, namespace: IoNamespace) {
        namespace.use_middleware(|socket| async move {
            if socket.handshake().auth().get("token").and_then(Value::as_str) == Some("no") {
                Err(ConnectError::new("not allowed"))
            } else {
                Ok(())
            }
        });
    }
}

#[async_trait]
impl OnGatewayConnection<IoSocket> for MiddlewareGateway {
    async fn handle_connection(&self, socket: IoSocket) {
        record(json!({ "event": "connection", "nsp": socket.namespace().name(), "auth": socket.handshake().auth() }));
    }
}

#[websocket_gateway("/a")]
pub struct AGateway;

#[handlers]
impl AGateway {
    #[subscribe_message("ping")]
    fn ping(&self) -> &'static str {
        "a"
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

#[websocket_gateway(path = "/cors-one", cors = "https://app.example")]
pub struct CorsOneGateway;

#[websocket_gateway(path = "/cors-any", cors = true)]
pub struct CorsAnyGateway;

// -------------------------------------------------------------------------------------------------- control routes

#[controller("harness")]
pub struct GatewayHarnessController;

#[routes]
impl GatewayHarnessController {
    /// What happened on the gateways, in order (loopback only: the server binds to 127.0.0.1).
    #[get("log")]
    fn log(&self) -> Value {
        Value::Array(LOG.lock().unwrap().clone())
    }

    #[post("reset")]
    fn reset(&self) -> Value {
        // What happened at startup (`init`) stays: it cannot happen again.
        LOG.lock().unwrap().retain(|entry| entry["event"] == "init");
        Value::Null
    }
}

#[module(
    controllers = [GatewayHarnessController],
    providers = [NeedsToken, NumberPipe, WrapInterceptor, RoomErrorFilter, QuietFilter, MainGateway, RulesGateway, MiddlewareGateway,
        AGateway, BGateway, CorsOneGateway, CorsAnyGateway],
)]
pub struct GatewayHarnessModule;

/// The application listening on 127.0.0.1:0 with the Socket.IO gateway adapter.
pub async fn start() -> (CorvisApplication, SocketAddr) {
    let mut app = CorvisFactory::create::<GatewayHarnessModule>().await.unwrap();
    app.use_websocket_adapter(IoAdapter::new());
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    (app, addr)
}

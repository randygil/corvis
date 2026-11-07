use std::time::Duration;

use futures::{SinkExt, StreamExt};
use corvis::prelude::*;
use corvis::{OnGatewayConnection, WsClient, WsResponse, WsServer, websocket_gateway};
use tokio_tungstenite::tungstenite::Message;

#[websocket_gateway(path = "/chat")]
pub struct ChatGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl ChatGateway {
    #[subscribe_message("echo")]
    async fn echo(&self, #[message_body] data: Value) -> Value {
        data
    }

    #[subscribe_message("join")]
    fn join(&self, #[message_body] room: String, #[connected_socket] client: WsClient) -> WsResponse {
        client.join(&room);
        WsResponse::new("joined", json!({ "room": room, "clients": self.server.count() }))
    }

    #[subscribe_message("shout")]
    fn shout(&self, #[message_body] text: String) {
        self.server.to("lobby").emit("shout", text);
    }

    #[subscribe_message("fail")]
    async fn fail(&self) -> HttpResult<()> {
        Err(corvis::WsException::new("nope"))
    }
}

#[async_trait]
impl OnGatewayConnection for ChatGateway {
    async fn handle_connection(&self, client: WsClient) {
        client.emit("welcome", json!({ "id": client.id() }));
    }
}

#[module(providers = [ChatGateway])]
pub struct AppModule;

async fn next_json<S>(ws: &mut S) -> Value
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timeout").unwrap().unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

#[corvis::test]
async fn gateway_roundtrip() {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{addr}/chat");
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    let welcome = next_json(&mut ws).await;
    assert_eq!(welcome["event"], "welcome");

    ws.send(Message::Text(json!({ "event": "echo", "data": { "a": 1 } }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "event": "echo", "data": { "a": 1 } }));

    ws.send(Message::Text(json!({ "event": "join", "data": "lobby" }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "event": "joined", "data": { "room": "lobby", "clients": 1 } }));

    ws.send(Message::Text(json!({ "event": "shout", "data": "hi" }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "event": "shout", "data": "hi" }));

    ws.send(Message::Text(json!({ "event": "fail" }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws).await, json!({ "event": "exception", "data": { "status": "error", "message": "nope" } }));
}

// ---------------------------------------------------------------------------
// Ordering and isolation.
// ---------------------------------------------------------------------------

#[websocket_gateway(path = "/ordered")]
pub struct OrderedGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl OrderedGateway {
    #[subscribe_message("join")]
    fn join(&self, #[message_body] room: String, #[connected_socket] client: WsClient) {
        client.join(&room);
    }

    #[subscribe_message("say")]
    fn say(&self, #[message_body] text: String) {
        self.server.to("r").emit("said", text);
    }

    #[subscribe_message("count")]
    fn count(&self) -> usize {
        self.server.count()
    }
}

#[module(providers = [OrderedGateway])]
pub struct OrderedModule;

/// Frames of one connection are dispatched in order: a `join` followed by a `say` in the same
/// TCP segment must see the room joined (handlers used to be spawned as separate tasks).
#[corvis::test]
async fn messages_of_a_connection_are_handled_in_order() {
    let mut app = CorvisFactory::create::<OrderedModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    for i in 0..100 {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ordered")).await.unwrap();
        ws.feed(Message::Text(json!({ "event": "join", "data": "r" }).to_string())).await.unwrap();
        ws.feed(Message::Text(json!({ "event": "say", "data": format!("m{i}") }).to_string())).await.unwrap();
        ws.flush().await.unwrap();
        assert_eq!(next_json(&mut ws).await, json!({ "event": "said", "data": format!("m{i}") }), "iteration {i}");
        ws.close(None).await.unwrap();
    }
}

/// Each application has its own gateway server: clients of one app are not seen by another
/// app mounting the same gateway path in the same process.
#[corvis::test]
async fn gateway_servers_are_per_application() {
    let mut a = CorvisFactory::create::<OrderedModule>().await.unwrap();
    let addr_a = a.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut b = CorvisFactory::create::<OrderedModule>().await.unwrap();
    let addr_b = b.listen_in_background("127.0.0.1:0").await.unwrap();
    let (mut ws_a, _) = tokio_tungstenite::connect_async(format!("ws://{addr_a}/ordered")).await.unwrap();
    let (mut ws_b, _) = tokio_tungstenite::connect_async(format!("ws://{addr_b}/ordered")).await.unwrap();
    ws_a.send(Message::Text(json!({ "event": "count" }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws_a).await, json!({ "event": "count", "data": 1 }));
    ws_b.send(Message::Text(json!({ "event": "count" }).to_string())).await.unwrap();
    assert_eq!(next_json(&mut ws_b).await, json!({ "event": "count", "data": 1 }));
}

// ---------------------------------------------------------------------------
// Guards, pipes, interceptors, exceptions, acknowledgements and disconnect.
// ---------------------------------------------------------------------------

const ROLE_KEY: &str = "role";

/// Authenticates the connection from `?user=` (class level): stores the user on the client.
#[injectable]
pub struct WsUserGuard;

#[async_trait]
impl CanActivate for WsUserGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        assert_eq!(ctx.get_type(), corvis::ContextType::Ws);
        let client = ctx.switch_to_ws().get_client::<WsClient>().expect("ws client");
        match ctx.request().query_param("user") {
            Some(u) => {
                client.insert(u.to_string());
                Ok(true)
            }
            None => Err(corvis::WsException::new("Unauthorized")),
        }
    }
}

/// Method level: compares `#[set_metadata("role", ...)]` with the message payload.
#[injectable]
pub struct RoleGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RoleGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let Some(role) = self.reflector.get::<String>(ROLE_KEY, ctx.get_handler()) else { return Ok(true) };
        let client = ctx.switch_to_ws().get_client::<WsClient>().unwrap();
        Ok(client.get::<String>().as_deref() == Some(role.as_str()) || ctx.switch_to_ws().get_data().and_then(|d| d.get("sudo")).is_some())
    }
}

/// Wraps the handler's reply.
#[injectable]
pub struct WrapInterceptor;

#[async_trait]
impl CorvisInterceptor for WrapInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let handler = ctx.get_handler().name.clone();
        Ok(next.handle().await?.map_data(|d| json!({ "handler": handler, "result": d })))
    }
}

/// Uppercases a string payload.
#[injectable]
pub struct UpperPipe;

#[async_trait]
impl PipeTransform for UpperPipe {
    async fn transform(&self, value: Value, _meta: &corvis::ArgumentMetadata) -> Result<Value, HttpException> {
        Ok(value.as_str().map(|s| Value::String(s.to_uppercase())).unwrap_or(value))
    }
}

fn message_schema() -> z::Schema {
    z::strict_object! { text: z::string().trim().min(1).max(10) }.into()
}

#[derive(serde::Deserialize)]
pub struct MessageDto {
    text: String,
}

#[websocket_gateway(path = "/secure")]
#[use_guards(WsUserGuard)]
pub struct SecureGateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl SecureGateway {
    #[subscribe_message("whoami")]
    fn whoami(&self, #[connected_socket] client: WsClient) -> String {
        client.get::<String>().unwrap_or_default()
    }

    #[subscribe_message("admin")]
    #[set_metadata(ROLE_KEY, "root")]
    #[use_guards(RoleGuard)]
    fn admin(&self) -> &'static str {
        "welcome, root"
    }

    #[subscribe_message("post")]
    fn post(&self, #[message_body(ZodValidationPipe::new(message_schema()))] dto: MessageDto) -> WsResponse {
        WsResponse::new("posted", json!({ "text": dto.text }))
    }

    #[subscribe_message("shout")]
    fn shout(&self, #[message_body("text", UpperPipe)] text: String, #[message_body("n")] n: u32) -> String {
        text.repeat(n as usize)
    }

    #[subscribe_message("wrapped")]
    #[use_interceptors(WrapInterceptor)]
    fn wrapped(&self, #[message_body] n: i64) -> i64 {
        n * 2
    }

    #[subscribe_message("wrapped_response")]
    #[use_interceptors(WrapInterceptor)]
    fn wrapped_response(&self) -> WsResponse {
        WsResponse::new("other", 1)
    }

    #[subscribe_message("room_full")]
    fn room_full(&self) -> HttpResult<()> {
        Err(corvis::WsException::object(json!({ "code": "ROOM_FULL", "max": 2 })))
    }

    #[subscribe_message("boom")]
    fn boom(&self) -> HttpResult<()> {
        Err(HttpException::from_anyhow(corvis::anyhow::anyhow!("database is on fire")))
    }

    #[subscribe_message("kick")]
    fn kick(&self, #[connected_socket] client: WsClient) {
        client.disconnect_with(4001, "kicked");
    }

    #[subscribe_message("ip")]
    fn ip(&self, #[connected_socket] client: WsClient) -> bool {
        client.handshake().remote_addr().is_some_and(|a| a.ip().is_loopback())
    }

    #[subscribe_message("count")]
    fn count(&self) -> usize {
        self.server.count()
    }
}

#[module(providers = [SecureGateway])]
pub struct SecureModule;

async fn secure_client(addr: std::net::SocketAddr, query: &str) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    tokio_tungstenite::connect_async(format!("ws://{addr}/secure{query}")).await.unwrap().0
}

async fn ask<S>(ws: &mut S, frame: Value) -> Value
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message, Error = tokio_tungstenite::tungstenite::Error>
        + Unpin,
{
    ws.send(Message::Text(frame.to_string())).await.unwrap();
    next_json(ws).await
}

#[corvis::test]
async fn gateway_guards_run_with_a_ws_execution_context() {
    let mut app = CorvisFactory::create::<SecureModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();

    let mut anon = secure_client(addr, "").await;
    assert_eq!(
        ask(&mut anon, json!({ "event": "whoami" })).await,
        json!({ "event": "exception", "data": { "status": "error", "message": "Unauthorized" } })
    );

    let mut ana = secure_client(addr, "?user=ana").await;
    assert_eq!(ask(&mut ana, json!({ "event": "whoami" })).await, json!({ "event": "whoami", "data": "ana" }));
    // A guard returning false: `WsException('Forbidden resource')`.
    assert_eq!(
        ask(&mut ana, json!({ "event": "admin" })).await,
        json!({ "event": "exception", "data": { "status": "error", "message": "Forbidden resource" } })
    );
    // Guards can read the payload (`switch_to_ws().get_data()`).
    assert_eq!(ask(&mut ana, json!({ "event": "admin", "data": { "sudo": true } })).await["data"], "welcome, root");

    let mut root = secure_client(addr, "?user=root").await;
    assert_eq!(ask(&mut root, json!({ "event": "admin" })).await, json!({ "event": "admin", "data": "welcome, root" }));
    assert_eq!(ask(&mut root, json!({ "event": "ip" })).await["data"], true);
}

#[corvis::test]
async fn message_body_pipes_and_properties() {
    let mut app = CorvisFactory::create::<SecureModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut ws = secure_client(addr, "?user=ana").await;

    assert_eq!(
        ask(&mut ws, json!({ "event": "post", "data": { "text": "  hola  " } })).await,
        json!({ "event": "posted", "data": { "text": "hola" } })
    );
    let err = ask(&mut ws, json!({ "event": "post", "data": { "text": "", "extra": 1 } })).await;
    assert_eq!(err["event"], "exception");
    assert_eq!(err["data"]["status"], "error");
    assert_eq!(err["data"]["message"], "Validation failed");
    assert_eq!(err["data"]["statusCode"], 400);
    let codes: Vec<&str> = err["data"]["errors"].as_array().unwrap().iter().map(|e| e["code"].as_str().unwrap()).collect();
    assert_eq!(codes, ["too_small", "unrecognized_keys"]);

    assert_eq!(
        ask(&mut ws, json!({ "event": "shout", "data": { "text": "ab", "n": 3 } })).await,
        json!({ "event": "shout", "data": "ABABAB" })
    );
    // A body that does not deserialize: 400 with the property name.
    let err = ask(&mut ws, json!({ "event": "shout", "data": { "text": "ab", "n": "x" } })).await;
    assert_eq!(err["data"]["status"], "error");
    assert!(err["data"]["message"].as_str().unwrap().starts_with("n: "), "{err}");
}

#[corvis::test]
async fn interceptors_exceptions_and_acknowledgements() {
    let mut app = CorvisFactory::create::<SecureModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut ws = secure_client(addr, "?user=ana").await;

    assert_eq!(
        ask(&mut ws, json!({ "event": "wrapped", "data": 21 })).await,
        json!({ "event": "wrapped", "data": { "handler": "wrapped", "result": 42 } })
    );
    // The interceptor saw `{ event, data }` and replaced it: sent with the handler's event.
    assert_eq!(
        ask(&mut ws, json!({ "event": "wrapped_response" })).await,
        json!({ "event": "wrapped_response", "data": { "handler": "wrapped_response", "result": { "event": "other", "data": 1 } } })
    );
    assert_eq!(
        ask(&mut ws, json!({ "event": "room_full" })).await,
        json!({ "event": "exception", "data": { "code": "ROOM_FULL", "max": 2 } })
    );
    assert_eq!(
        ask(&mut ws, json!({ "event": "boom" })).await,
        json!({ "event": "exception", "data": { "status": "error", "message": "Internal server error" } })
    );
    // `id` is echoed in replies and exception frames.
    assert_eq!(
        ask(&mut ws, json!({ "event": "whoami", "id": 7 })).await,
        json!({ "event": "whoami", "data": "ana", "id": 7 })
    );
    assert_eq!(
        ask(&mut ws, json!({ "event": "room_full", "id": "req-1" })).await,
        json!({ "event": "exception", "data": { "code": "ROOM_FULL", "max": 2 }, "id": "req-1" })
    );
}

#[corvis::test]
async fn disconnect_with_a_close_code_stops_dispatching() {
    let mut app = CorvisFactory::create::<SecureModule>().await.unwrap();
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    let mut watcher = secure_client(addr, "?user=w").await;
    let mut ws = secure_client(addr, "?user=ana").await;
    assert_eq!(ask(&mut watcher, json!({ "event": "count" })).await["data"], 2);

    ws.feed(Message::Text(json!({ "event": "kick" }).to_string())).await.unwrap();
    ws.feed(Message::Text(json!({ "event": "whoami" }).to_string())).await.unwrap();
    ws.flush().await.unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
    match frame {
        Message::Close(Some(f)) => {
            assert_eq!(u16::from(f.code), 4001);
            assert_eq!(f.reason, "kicked");
        }
        other => panic!("expected a close frame, got {other:?}"),
    }
    drop(ws);
    // The server forgets the client once the connection is gone.
    let mut n = 0;
    for _ in 0..50 {
        n = ask(&mut watcher, json!({ "event": "count" })).await["data"].as_u64().unwrap();
        if n == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(n, 1);
}

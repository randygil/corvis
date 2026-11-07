//! WebSockets (with a `ws`-style platform adapter): gateways, message handlers,
//! rooms and broadcasting.
//!
//! Messages are JSON frames `{ "event": "name", "data": ... }`, like the `ws` adapter.
//!
//! ```ignore
//! #[websocket_gateway(path = "/chat")]
//! pub struct ChatGateway {
//!     #[websocket_server]
//!     server: WsServer,
//! }
//!
//! #[handlers]
//! impl ChatGateway {
//!     #[subscribe_message("message")]
//!     async fn on_message(&self, #[message_body] text: String, #[connected_socket] client: WsClient) -> WsResponse {
//!         self.server.emit("message", json!({ "from": client.id(), "text": text }));
//!         WsResponse::new("ack", json!(true))
//!     }
//! }
//! ```

use std::any::Any;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role};

use crate::common::context::{ExecutionContext, Target};
use crate::common::enhancer::EnhancerRef;
use crate::common::guard::CanActivate;
use crate::common::interceptor::{CallHandler, CorvisInterceptor};
use crate::common::pipe::PipeTransform;
use crate::di::container::Container;
use crate::di::{DiError, ModuleRef, ResolveCtx};
use crate::discovery::{GatewayInfo, HandlerKind, WsCall};
use crate::http::{HttpException, Request, Response};
use crate::logger::Logger;
use crate::router::controller::ParamDef;
use crate::router::{AppShared, RouterConfig};
use crate::server::ResBody;

/// Implemented by `#[websocket_gateway]`.
pub trait Gateway: Send + Sync + 'static {
    fn __corvis_gateway(this: Arc<Self>) -> GatewayInfo;
}

/// `afterInit(server)`.
#[async_trait]
pub trait OnGatewayInit: Send + Sync {
    async fn after_init(&self, server: WsServer);
}

/// `handleConnection(client)`.
#[async_trait]
pub trait OnGatewayConnection: Send + Sync {
    async fn handle_connection(&self, client: WsClient);
}

/// `handleDisconnect(client)`.
#[async_trait]
pub trait OnGatewayDisconnect: Send + Sync {
    async fn handle_disconnect(&self, client: WsClient);
}

/// `WsException`. The client receives an `exception` frame:
/// `{ "status": "error", "message": "..." }` for [`WsException::new`], or the object itself
/// for [`WsException::object`] (`new WsException({ ... })`).
pub struct WsException;
impl WsException {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(message: impl Into<String>) -> HttpException {
        let message = message.into();
        HttpException::new(message.clone(), 400).with_cause(WsExceptionError(Value::String(message)))
    }
    /// `new WsException({ code: 'ROOM_FULL', message: '...' })`: the object is sent as the
    /// data of the `exception` frame, as-is.
    pub fn object(error: impl Serialize) -> HttpException {
        let error = serde_json::to_value(error).unwrap_or(Value::Null);
        HttpException::new(error.clone(), 400).with_cause(WsExceptionError(error))
    }
    /// `exception.getError()`: the error of an exception built by `WsException`.
    pub fn get_error(exception: &HttpException) -> Option<&Value> {
        exception.downcast_cause::<WsExceptionError>().map(|e| &e.0)
    }
}

/// The error carried by a [`WsException`] (its cause).
#[derive(Debug, Clone)]
pub struct WsExceptionError(pub Value);
impl std::fmt::Display for WsExceptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Value::String(s) => f.write_str(s),
            other => write!(f, "{other}"),
        }
    }
}
impl std::error::Error for WsExceptionError {}

/// Data of the `exception` frame for an error raised by a guard, a pipe or a handler.
///
/// - `WsException::new(msg)`: `{ status: "error", message }` (the base WebSocket exception filter).
/// - `WsException::object(obj)`: `obj`.
/// - Unknown errors (`is_internal()`): `{ status: "error", message: "Internal server error" }`.
/// - Any other `HttpException` (`BadRequestException`, `ZodValidationException`, ...):
///   `{ status: "error", message, ...the other fields of its response }` (e.g. `statusCode`
///   and the zod `errors`).
pub fn exception_payload(e: &HttpException) -> Value {
    if let Some(error) = WsException::get_error(e) {
        return match error {
            Value::Object(_) => error.clone(),
            other => json!({ "status": "error", "message": other }),
        };
    }
    if e.is_internal() {
        return json!({ "status": "error", "message": "Internal server error" });
    }
    let mut out = Map::new();
    out.insert("status".into(), Value::String("error".into()));
    out.insert("message".into(), Value::String(e.message().to_string()));
    if let Value::Object(o) = e.get_response() {
        for (k, v) in o {
            if k != "message" {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Frame sent back to the client: `{ event, data }`.
#[derive(Debug, Clone, Serialize)]
pub struct WsResponse {
    pub event: String,
    pub data: Value,
}

impl WsResponse {
    pub fn new(event: impl Into<String>, data: impl Serialize) -> Self {
        WsResponse { event: event.into(), data: serde_json::to_value(data).unwrap_or(Value::Null) }
    }
}

/// What a `#[subscribe_message]` handler returned.
pub enum WsReply {
    None,
    /// Data sent back with the same event name.
    Data(Value),
    Response(WsResponse),
}

pub trait IntoWsReply {
    fn into_ws_reply(self) -> WsReply;
}
impl IntoWsReply for () {
    fn into_ws_reply(self) -> WsReply {
        WsReply::None
    }
}
impl IntoWsReply for WsResponse {
    fn into_ws_reply(self) -> WsReply {
        WsReply::Response(self)
    }
}
impl IntoWsReply for WsReply {
    fn into_ws_reply(self) -> WsReply {
        self
    }
}
impl<T: IntoWsReply> IntoWsReply for Option<T> {
    fn into_ws_reply(self) -> WsReply {
        self.map(IntoWsReply::into_ws_reply).unwrap_or(WsReply::None)
    }
}

#[doc(hidden)]
pub struct WsReplyTag;
impl WsReplyTag {
    pub fn convert<T: IntoWsReply>(self, v: T) -> Option<Value> {
        encode_reply(v.into_ws_reply())
    }
}
#[doc(hidden)]
pub struct WsSerializeTag;
impl WsSerializeTag {
    pub fn convert<T: Serialize>(self, v: T) -> Option<Value> {
        encode_reply(WsReply::Data(serde_json::to_value(v).unwrap_or(Value::Null)))
    }
}
#[doc(hidden)]
pub trait WsReplyKind {
    fn __corvis_ws_kind(&self) -> WsReplyTag {
        WsReplyTag
    }
}
impl<T: IntoWsReply> WsReplyKind for T {}
#[doc(hidden)]
pub trait WsSerializeKind {
    fn __corvis_ws_kind(&self) -> WsSerializeTag {
        WsSerializeTag
    }
}
impl<T: Serialize> WsSerializeKind for &T {}

fn encode_reply(r: WsReply) -> Option<Value> {
    match r {
        WsReply::None => None,
        // `{"__same_event": data}` is resolved by the dispatcher.
        WsReply::Data(d) => Some(json!({ "__corvis_same_event": true, "data": d })),
        WsReply::Response(r) => Some(json!({ "event": r.event, "data": r.data })),
    }
}

static CLIENT_IDS: AtomicU64 = AtomicU64::new(1);

struct ClientInner {
    id: String,
    tx: mpsc::UnboundedSender<Message>,
    rooms: Mutex<HashSet<String>>,
    handshake: Request,
    server: WsServer,
    /// Set by `disconnect()`: frames received afterwards are not dispatched.
    closing: AtomicBool,
}

/// A connected socket (`@ConnectedSocket() client`).
#[derive(Clone)]
pub struct WsClient {
    inner: Arc<ClientInner>,
}

impl WsClient {
    pub fn id(&self) -> &str {
        &self.inner.id
    }
    /// `client.send(JSON.stringify({ event, data }))`.
    pub fn emit(&self, event: &str, data: impl Serialize) -> bool {
        let frame = json!({ "event": event, "data": serde_json::to_value(data).unwrap_or(Value::Null) });
        self.send_text(frame.to_string())
    }
    pub fn send_text(&self, text: impl Into<String>) -> bool {
        self.inner.tx.send(Message::Text(text.into())).is_ok()
    }
    pub fn join(&self, room: &str) {
        self.inner.rooms.lock().unwrap().insert(room.to_string());
    }
    pub fn leave(&self, room: &str) {
        self.inner.rooms.lock().unwrap().remove(room);
    }
    pub fn rooms(&self) -> Vec<String> {
        self.inner.rooms.lock().unwrap().iter().cloned().collect()
    }
    pub fn in_room(&self, room: &str) -> bool {
        self.inner.rooms.lock().unwrap().contains(room)
    }
    /// The HTTP upgrade request (headers, query string, ...).
    pub fn handshake(&self) -> &Request {
        &self.inner.handshake
    }
    /// Attach data to the client (e.g. the authenticated user).
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, v: T) {
        self.inner.handshake.insert(v);
    }
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.inner.handshake.get::<T>()
    }
    pub fn disconnect(&self) {
        self.inner.closing.store(true, Ordering::SeqCst);
        let _ = self.inner.tx.send(Message::Close(None));
    }
    /// Closes the connection with a close code and reason (e.g. `4401`, `"Unauthorized"`).
    /// Valid codes are 1000-1003, 1007-1014 and 3000-4999; others are sent as `1008`.
    /// Frames received after this call are not dispatched to the handlers.
    pub fn disconnect_with(&self, code: u16, reason: &str) {
        let code = if matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999) { code } else { 1008 };
        // The reason of a close frame is limited to 123 bytes.
        let mut end = reason.len().min(123);
        while !reason.is_char_boundary(end) {
            end -= 1;
        }
        self.inner.closing.store(true, Ordering::SeqCst);
        let frame = CloseFrame { code: CloseCode::from(code), reason: Cow::Owned(reason[..end].to_string()) };
        let _ = self.inner.tx.send(Message::Close(Some(frame)));
    }
    /// `true` once `disconnect()` / `disconnect_with()` was called.
    pub fn is_closing(&self) -> bool {
        self.inner.closing.load(Ordering::SeqCst)
    }
    pub fn server(&self) -> &WsServer {
        &self.inner.server
    }
}

/// Converts the type-erased client received by generated handlers.
pub fn client_from_any(c: &Arc<dyn Any + Send + Sync>) -> Result<WsClient, HttpException> {
    c.downcast_ref::<WsClient>().cloned().ok_or_else(|| HttpException::new("invalid socket", 500))
}

#[derive(Default)]
struct ServerInner {
    clients: RwLock<HashMap<String, WsClient>>,
}

/// The gateway server (`@WebSocketServer() server`): broadcast and rooms.
///
/// Each gateway instance (so each application) has its own server; the
/// `#[websocket_server]` field and `after_init(server)` receive it.
#[derive(Clone, Default)]
pub struct WsServer {
    inner: Arc<ServerInner>,
}

static SERVERS: LazyLock<Mutex<HashMap<String, WsServer>>> = LazyLock::new(Default::default);

impl WsServer {
    /// A new server with no clients.
    pub fn new() -> Self {
        Self::default()
    }
    /// The server of the gateway mounted at `path` by the most recently initialized
    /// application (a new, empty one before any application mounted it). Prefer injecting
    /// the gateway and using its `#[websocket_server]` field.
    pub fn for_path(path: &str) -> WsServer {
        SERVERS.lock().unwrap().entry(path.to_string()).or_default().clone()
    }
    pub fn clients(&self) -> Vec<WsClient> {
        self.inner.clients.read().unwrap().values().cloned().collect()
    }
    pub fn client(&self, id: &str) -> Option<WsClient> {
        self.inner.clients.read().unwrap().get(id).cloned()
    }
    pub fn count(&self) -> usize {
        self.inner.clients.read().unwrap().len()
    }
    /// Broadcast to every client: `server.emit(event, data)`.
    pub fn emit(&self, event: &str, data: impl Serialize) -> usize {
        let data = serde_json::to_value(data).unwrap_or(Value::Null);
        self.clients().iter().filter(|c| c.emit(event, &data)).count()
    }
    /// `server.to(room).emit(event, data)`.
    pub fn to(&self, room: &str) -> RoomEmitter {
        RoomEmitter { server: self.clone(), room: room.to_string(), except: None }
    }
    fn add(&self, c: WsClient) {
        self.inner.clients.write().unwrap().insert(c.id().to_string(), c);
    }
    fn remove(&self, id: &str) {
        self.inner.clients.write().unwrap().remove(id);
    }
}

pub struct RoomEmitter {
    server: WsServer,
    room: String,
    except: Option<String>,
}

impl RoomEmitter {
    /// Exclude a client (`client.broadcast.to(room)`).
    pub fn except(mut self, client_id: &str) -> Self {
        self.except = Some(client_id.to_string());
        self
    }
    pub fn emit(&self, event: &str, data: impl Serialize) -> usize {
        let data = serde_json::to_value(data).unwrap_or(Value::Null);
        self.server
            .clients()
            .iter()
            .filter(|c| c.in_room(&self.room) && Some(c.id()) != self.except.as_deref())
            .filter(|c| c.emit(event, &data))
            .count()
    }
}

/// A `#[subscribe_message]` handler with its resolved enhancers.
struct CompiledWsHandler {
    call: WsCall,
    guards: Vec<Arc<dyn CanActivate>>,
    interceptors: Arc<Vec<Arc<dyn CorvisInterceptor>>>,
    /// Gateway-level then handler-level pipes, applied to every `#[message_body]`.
    pipes: Vec<Arc<dyn PipeTransform>>,
    params: Vec<(ParamDef, Vec<Arc<dyn PipeTransform>>)>,
    target: Arc<Target>,
}

/// Runtime data of a mounted gateway.
pub(crate) struct GatewayRuntime {
    name: String,
    server: WsServer,
    handlers: HashMap<String, Arc<CompiledWsHandler>>,
    info: GatewayInfo,
    class_target: Arc<Target>,
    module_ref: ModuleRef,
}

async fn resolve_all<T: ?Sized + Send + Sync + 'static>(refs: &[EnhancerRef<T>], ctx: &ResolveCtx) -> Result<Vec<Arc<T>>, DiError> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        out.push(r.resolve(ctx.clone()).await?);
    }
    Ok(out)
}

pub(crate) async fn build_gateways(
    container: &Arc<Container>,
    _cfg: &RouterConfig,
) -> Result<HashMap<String, Arc<GatewayRuntime>>, DiError> {
    let mut out = HashMap::new();
    let logger = Logger::new("WebSocketsController");
    for module in &container.modules {
        for p in &module.providers {
            let Some(gw) = &p.hooks.gateway else { continue };
            let Some(instance) = container.instance(module.id, &p.token) else { continue };
            let Some(info) = gw(instance.clone()) else { continue };
            let ctx = ResolveCtx::for_module(container.clone(), module.id);
            let c = &info.enhancers;
            let c_guards = resolve_all(&c.guards, &ctx).await?;
            let c_interceptors = resolve_all(&c.interceptors, &ctx).await?;
            let c_pipes = resolve_all(&c.pipes, &ctx).await?;
            let mut handlers = HashMap::new();
            if let Some(explore) = &p.hooks.handlers {
                for h in explore(instance) {
                    if let HandlerKind::WsMessage { event, call, enhancers } = h.kind {
                        logger.log(format!("{} subscribed to the \"{}\" message", p.name, event));
                        let mut guards = c_guards.clone();
                        guards.extend(resolve_all(&enhancers.guards, &ctx).await?);
                        let mut interceptors = c_interceptors.clone();
                        interceptors.extend(resolve_all(&enhancers.interceptors, &ctx).await?);
                        let mut pipes = c_pipes.clone();
                        pipes.extend(resolve_all(&enhancers.pipes, &ctx).await?);
                        let mut params = Vec::with_capacity(enhancers.params.len());
                        for param in &enhancers.params {
                            let param_pipes = resolve_all(&param.pipes, &ctx).await?;
                            params.push((param.clone(), param_pipes));
                        }
                        let target =
                            Arc::new(Target { name: h.method_name.to_string(), metadata: enhancers.metadata.iter().cloned().collect() });
                        let compiled =
                            CompiledWsHandler { call, guards, interceptors: Arc::new(interceptors), pipes, params, target };
                        handlers.insert(event, Arc::new(compiled));
                    }
                }
            }
            let server = info.server.as_ref().and_then(|s| s.downcast_ref::<WsServer>().cloned()).unwrap_or_default();
            SERVERS.lock().unwrap().insert(info.path.clone(), server.clone());
            if let Some(init) = &info.on_init {
                init(Arc::new(server.clone())).await;
            }
            let class_target = Arc::new(Target { name: p.name.clone(), metadata: info.enhancers.metadata.iter().cloned().collect() });
            let module_ref = ModuleRef { container: container.clone(), module: module.id };
            out.insert(
                info.path.clone(),
                Arc::new(GatewayRuntime { name: p.name.clone(), server, handlers, info, class_target, module_ref }),
            );
        }
    }
    Ok(out)
}

/// If the request is a WebSocket upgrade for a gateway, returns its runtime.
pub(crate) fn match_upgrade(shared: &AppShared, headers: &http::HeaderMap, path: &str) -> Option<Arc<GatewayRuntime>> {
    let upgrade = headers.get(http::header::UPGRADE)?.to_str().ok()?;
    if !upgrade.eq_ignore_ascii_case("websocket") {
        return None;
    }
    let p = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    shared.gateways.get(p).cloned()
}

/// Performs the upgrade handshake and spawns the connection loop.
pub(crate) fn upgrade(gateway: Arc<GatewayRuntime>, mut req: hyper::Request<Incoming>, remote: SocketAddr) -> hyper::Response<ResBody> {
    use http_body_util::BodyExt;
    let empty = || http_body_util::Empty::<bytes::Bytes>::new().map_err(|never| match never {}).boxed_unsync();
    let Some(key) = req.headers().get("sec-websocket-key").map(|k| k.as_bytes().to_vec()) else {
        return hyper::Response::builder().status(400).body(empty()).unwrap();
    };
    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(&key);
    let on_upgrade = hyper::upgrade::on(&mut req);
    let (parts, _) = req.into_parts();
    let handshake = Request::new(parts.method, parts.uri, parts.headers, bytes::Bytes::new()).ok().map(|mut r| {
        r.version = parts.version;
        r.remote_addr = Some(remote);
        r
    });
    tokio::spawn(async move {
        let Ok(upgraded) = on_upgrade.await else { return };
        let ws = WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        if let Some(handshake) = handshake {
            run_connection(gateway, ws, handshake).await;
        }
    });
    hyper::Response::builder()
        .status(101)
        .header("upgrade", "websocket")
        .header("connection", "Upgrade")
        .header("sec-websocket-accept", accept)
        .body(empty())
        .unwrap()
}

const SHAPE_NONE: u8 = 0;
const SHAPE_DATA: u8 = 1;
const SHAPE_RESPONSE: u8 = 2;

/// Guards -> interceptors -> pipes -> handler for one message.
async fn run_handler(
    gateway: &GatewayRuntime,
    handler: &Arc<CompiledWsHandler>,
    client: &WsClient,
    data: Value,
) -> Result<Option<Value>, HttpException> {
    let ctx = ExecutionContext::for_ws(
        client.handshake().clone(),
        gateway.class_target.clone(),
        handler.target.clone(),
        Some(gateway.module_ref.clone()),
        Arc::new(client.clone()),
        data.clone(),
    );
    for g in &handler.guards {
        if !g.can_activate(&ctx).await? {
            return Err(WsException::new("Forbidden resource"));
        }
    }
    if handler.interceptors.is_empty() {
        return invoke_handler(handler.clone(), client.clone(), data).await;
    }
    // Interceptors see the handler's return value as the JSON body of a `Response`: the data
    // for a plain value, `{ event, data }` for a `WsResponse`, `null` for no reply.
    let shape = Arc::new(AtomicU8::new(SHAPE_NONE));
    let resp =
        call_interceptors(handler.interceptors.clone(), 0, ctx, handler.clone(), client.clone(), data, shape.clone()).await?;
    let body = resp.json_body().cloned().unwrap_or(Value::Null);
    Ok(match shape.load(Ordering::SeqCst) {
        SHAPE_RESPONSE if body.get("event").is_some_and(Value::is_string) => Some(body),
        SHAPE_NONE if body.is_null() => None,
        _ => Some(json!({ "__corvis_same_event": true, "data": body })),
    })
}

fn call_interceptors(
    list: Arc<Vec<Arc<dyn CorvisInterceptor>>>,
    index: usize,
    ctx: ExecutionContext,
    handler: Arc<CompiledWsHandler>,
    client: WsClient,
    data: Value,
    shape: Arc<AtomicU8>,
) -> BoxFuture<'static, Result<Response, HttpException>> {
    if let Some(interceptor) = list.get(index).cloned() {
        let ctx2 = ctx.clone();
        let next = CallHandler::new(move || call_interceptors(list, index + 1, ctx2, handler, client, data, shape));
        async move { interceptor.intercept(&ctx, next).await }.boxed()
    } else {
        async move {
            let reply = invoke_handler(handler, client, data).await?;
            let (s, body) = match reply {
                None => (SHAPE_NONE, Value::Null),
                Some(r) if r.get("__corvis_same_event").is_some() => (SHAPE_DATA, r.get("data").cloned().unwrap_or(Value::Null)),
                Some(r) => (SHAPE_RESPONSE, r),
            };
            shape.store(s, Ordering::SeqCst);
            Ok(Response::json(body))
        }
        .boxed()
    }
}

/// Applies the pipes to each `#[message_body]` argument and calls the handler.
async fn invoke_handler(handler: Arc<CompiledWsHandler>, client: WsClient, data: Value) -> Result<Option<Value>, HttpException> {
    let mut args = Vec::with_capacity(handler.params.len());
    for (param, param_pipes) in &handler.params {
        let mut value = match param.data {
            Some(key) => data.get(key).cloned().unwrap_or(Value::Null),
            None => data.clone(),
        };
        let meta = param.argument_metadata();
        for pipe in handler.pipes.iter().chain(param_pipes.iter()) {
            value = pipe.transform(value, &meta).await?;
        }
        args.push(value);
    }
    (handler.call)(Arc::new(client), args).await
}

/// Runs one message through its handler and sends the reply (or the `exception` frame).
/// When the incoming frame carries an `id`, the reply and the `exception` frame echo it, so a
/// client can match answers to requests (acknowledgements).
async fn dispatch(
    gateway: Arc<GatewayRuntime>,
    handler: Arc<CompiledWsHandler>,
    client: WsClient,
    event: String,
    data: Value,
    id: Option<Value>,
) {
    let result = match AssertUnwindSafe(run_handler(&gateway, &handler, &client, data)).catch_unwind().await {
        Ok(r) => r,
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "handler panicked".to_string());
            Err(HttpException::from_anyhow(anyhow::anyhow!("{msg}")))
        }
    };
    let with_id = |mut frame: Value| {
        if let (Some(id), Value::Object(o)) = (&id, &mut frame) {
            o.insert("id".into(), id.clone());
        }
        frame
    };
    match result {
        Ok(Some(reply)) => {
            let frame = if reply.get("__corvis_same_event").is_some() {
                json!({ "event": event, "data": reply.get("data").cloned().unwrap_or(Value::Null) })
            } else {
                reply
            };
            client.send_text(with_id(frame).to_string());
        }
        Ok(None) => {}
        Err(e) => {
            if e.is_internal() {
                Logger::new("WsExceptionsHandler").error(format!("{}: {}", gateway.name, e.message()));
            }
            client.send_text(with_id(json!({ "event": "exception", "data": exception_payload(&e) })).to_string());
        }
    }
}

async fn run_connection<S>(gateway: Arc<GatewayRuntime>, ws: WebSocketStream<S>, handshake: Request)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if sink.send(m).await.is_err() || close {
                break;
            }
        }
    });
    let id = format!("{:x}{:04x}", chrono::Utc::now().timestamp_millis(), CLIENT_IDS.fetch_add(1, Ordering::SeqCst));
    let client = WsClient {
        inner: Arc::new(ClientInner {
            id,
            tx: tx.clone(),
            rooms: Mutex::new(HashSet::new()),
            handshake,
            server: gateway.server.clone(),
            closing: AtomicBool::new(false),
        }),
    };
    gateway.server.add(client.clone());
    if let Some(h) = &gateway.info.on_connection {
        h(Arc::new(client.clone())).await;
    }
    // Handlers of a connection run concurrently (a slow handler does not block the socket)
    // but start in the order their frames arrived, like RxJS `mergeMap`: a `join` followed
    // by a `message` always sees the room joined.
    let mut inflight: FuturesUnordered<BoxFuture<'static, ()>> = FuturesUnordered::new();
    loop {
        let msg = tokio::select! {
            msg = stream.next() => msg,
            Some(()) = inflight.next(), if !inflight.is_empty() => continue,
        };
        let Some(msg) = msg else { break };
        let text = match msg {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).into_owned(),
            Ok(Message::Ping(p)) => {
                let _ = tx.send(Message::Pong(p));
                continue;
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => continue,
        };
        if client.is_closing() {
            continue;
        }
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(event) = parsed.get("event").and_then(|e| e.as_str()).map(String::from) else { continue };
        let data = parsed.get("data").cloned().unwrap_or(Value::Null);
        let frame_id = parsed.get("id").filter(|v| !v.is_null()).cloned();
        let Some(handler) = gateway.handlers.get(&event).cloned() else { continue };
        inflight.push(dispatch(gateway.clone(), handler, client.clone(), event, data, frame_id).boxed());
    }
    gateway.server.remove(client.id());
    // Handlers still running when the client left are completed in the background.
    if !inflight.is_empty() {
        tokio::spawn(async move { while inflight.next().await.is_some() {} });
    }
    if let Some(h) = &gateway.info.on_disconnect {
        h(Arc::new(client.clone())).await;
    }
    drop(tx);
    writer.abort();
}

#[doc(hidden)]
pub mod hooks {
    use super::*;
    use crate::__private::Probe;
    use crate::discovery::GatewayHook;

    macro_rules! detect {
        ($has:ident, $no:ident, $method:ident, $tr:ident, $call:ident, $arg:ty) => {
            pub trait $has<T> {
                fn $method(&self, this: Arc<T>) -> Option<GatewayHook>;
            }
            impl<T: $tr + Send + Sync + 'static> $has<T> for Probe<T> {
                fn $method(&self, this: Arc<T>) -> Option<GatewayHook> {
                    Some(Arc::new(move |arg: Arc<dyn Any + Send + Sync>| {
                        let this = this.clone();
                        Box::pin(async move {
                            if let Some(a) = arg.downcast_ref::<$arg>().cloned() {
                                this.$call(a).await;
                            }
                        })
                    }))
                }
            }
            pub trait $no<T> {
                fn $method(&self, _this: Arc<T>) -> Option<GatewayHook> {
                    None
                }
            }
            impl<T> $no<T> for &Probe<T> {}
        };
    }

    detect!(HasGwInit, NoGwInit, __corvis_gw_init, OnGatewayInit, after_init, WsServer);
    detect!(HasGwConn, NoGwConn, __corvis_gw_connection, OnGatewayConnection, handle_connection, WsClient);
    detect!(HasGwDisc, NoGwDisc, __corvis_gw_disconnect, OnGatewayDisconnect, handle_disconnect, WsClient);
}

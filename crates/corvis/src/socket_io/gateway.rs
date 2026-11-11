//! Gateways over Socket.IO: `#[websocket_gateway]` classes served to the official Socket.IO client.
//!
//! An application chooses the transport of all its gateways once, before `init()`:
//!
//! ```no_run
//! use corvis::prelude::*;
//! use corvis::Module;
//! use corvis::socket_io::IoAdapter;
//!
//! # async fn demo<M: Module>() -> corvis::Result<()> {
//! let mut app = CorvisFactory::create::<M>().await?;
//! app.use_websocket_adapter(IoAdapter::new());
//! app.listen("127.0.0.1:3000").await?;
//! # Ok(()) }
//! ```
//!
//! Under this adapter a gateway's `namespace` (or its literal) is the Socket.IO namespace, its `path` the Engine.IO
//! path and its `cors` the CORS of that path; message handlers run as event handlers through the gateway pipeline
//! (guards, interceptors, pipes, exception filters); the handles are [`IoSocket`], [`IoGatewayServer`] and, for
//! `after_init`, [`IoNamespace`].
//!
//! What authenticates a Socket.IO connection. Nothing does by default: the strategies of the authentication module run
//! on HTTP requests, `AuthGuard` denies outside HTTP, and guards run per message. A connection is checked by the
//! `allow_request` hook and `allowed_origins` of the Engine.IO options (before a session exists), by connect middleware
//! registered in `after_init` (before the socket connects), by `handle_connection` (after it connected; events wait for
//! it) and, opt in, by [`IoAdapter::guard_connections`], which runs the global and gateway-level guards when a socket
//! connects.
//!
//! Limits. This layer adds no table or queue of its own: events that wait for `handle_connection` count as handlers in
//! flight (`max_inflight`, 32 per connection, then reads pause), the namespaces a client can reach are the ones
//! gateways declare, and every other limit is the one of the Engine.IO and Socket.IO layers, set through
//! [`IoAdapter::options`]. Hooks are bounded by [`IoAdapter::hook_timeout`].

use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use futures::FutureExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::broadcast::{BroadcastAckError, BroadcastOperator, IntoRooms, RemoteSocket};
use super::engine::EngineOptions;
use super::namespace::{ConnectError, IoNamespace, NamespaceInner};
use super::options::IoOptions;
use super::server::IoServer;
use super::socket::{AckSender, EmitError, IoSocket, Phase};
use super::value::{Args, IntoArgs};
use crate::common::context::{ExecutionContext, Target};
use crate::common::guard::CanActivate;
use crate::di::DiError;
use crate::discovery::{CorsDecl, GatewayAdapterKind};
use crate::http::HttpException;
use crate::http_adapter::HttpServer;
use crate::logger::Logger;
use crate::router::cors::CorsOptions;
use crate::websockets::{
    CompiledWsHandler, GatewayAdapter, GatewayAdapterChoice, GatewayRuntime, GatewayServerField, GatewaySocket, contain_panic,
    exception_data, exception_payload, run_handler, run_hook_with, sealed,
};

/// How long a lifecycle hook (`after_init`, `handle_connection`, `handle_disconnect`) may run: 30 seconds.
pub const DEFAULT_HOOK_TIMEOUT: Duration = Duration::from_secs(30);

/// Serves the gateways of an application over Socket.IO. Pass it to
/// `CorvisApplication::use_websocket_adapter` before `init()`.
///
/// Gateways that share an Engine.IO path are namespaces of one Socket.IO server. The server of a path gets the
/// [`IoOptions`] given to [`options`](Self::options) for that path (the defaults for a path without them), with the
/// `cors` the path's gateways declare applied on top.
#[derive(Clone, Debug)]
pub struct IoAdapter {
    options: Vec<IoOptions>,
    /// The first path that [`options`](Self::options) was given twice for: `init()` fails naming it.
    duplicate_options: Option<String>,
    guard_connections: bool,
    hook_timeout: Duration,
}

impl Default for IoAdapter {
    fn default() -> Self {
        IoAdapter::new()
    }
}

impl IoAdapter {
    /// The defaults: [`IoOptions::default`] for every Engine.IO path, no connection guards, a hook timeout of
    /// [`DEFAULT_HOOK_TIMEOUT`].
    pub fn new() -> Self {
        IoAdapter { options: Vec::new(), duplicate_options: None, guard_connections: false, hook_timeout: DEFAULT_HOOK_TIMEOUT }
    }

    /// The options of the Engine.IO path the given options name (`EngineOptions::path`, `/socket.io` by default). Every
    /// limit of the Engine.IO, Socket.IO and rooms layers is set here, and so are `allow_request`, `allowed_origins`,
    /// the CORS of the path and the cluster adapter. Once per path: a second call for the same path, or options that
    /// do not validate, make `init()` fail with an error that names the path.
    pub fn options(mut self, options: IoOptions) -> Self {
        let path = options.engine.normalized_path();
        if self.options.iter().any(|o| o.engine.normalized_path() == path) {
            self.duplicate_options.get_or_insert(path);
        } else {
            self.options.push(options);
        }
        self
    }

    /// Off by default, as in the original framework: guards run for each message only, so a socket that never sends one
    /// is never checked. With `true`, the global guards (while global enhancers apply to gateways) and the guards
    /// declared on the gateways of a namespace also run when a socket connects to it, before it is connected; a denial
    /// answers `CONNECT_ERROR` (`Forbidden resource`, or the exception's message and data) and no hook runs for the
    /// socket. Handler-level guards run per message only. The context has `null` data and a handler named `connect`
    /// that carries the gateway's metadata.
    pub fn guard_connections(mut self, on: bool) -> Self {
        self.guard_connections = on;
        self
    }

    /// How long `after_init`, `handle_connection` and `handle_disconnect` may run (default 30 seconds). A
    /// `handle_connection` that exceeds it, or panics, disconnects the socket and closes its connection; one that
    /// exceeds it in `after_init` fails `init()`; a `handle_disconnect` is abandoned. Panics are always contained.
    pub fn hook_timeout(mut self, timeout: Duration) -> Self {
        self.hook_timeout = timeout;
        self
    }

    pub(crate) fn connection_guards(&self) -> bool {
        self.guard_connections
    }

    fn options_for(&self, path: &str) -> Option<&IoOptions> {
        self.options.iter().find(|o| o.engine.normalized_path() == path)
    }
}

impl sealed::Sealed for IoAdapter {}

impl GatewayAdapter for IoAdapter {
    fn __corvis_choice(self) -> GatewayAdapterChoice {
        GatewayAdapterChoice::SocketIo(Arc::new(self))
    }
}

impl GatewaySocket for IoSocket {
    const KIND: GatewayAdapterKind = GatewayAdapterKind::SocketIo;
}

// ---------------------------------------------------------------------------------------------------- the handle

/// The `#[websocket_server]` field of a gateway served over Socket.IO: the broadcast operators of the gateway's
/// namespace. Cheap to clone. It is empty until the application starts its gateways; before that every operation
/// fails with [`EmitError::NotStarted`], sends nothing and does not panic. Once bound it works from any provider that
/// injects the gateway.
///
/// ```
/// use corvis::prelude::*;
/// use corvis::socket_io::{IoGatewayServer, IoSocket};
/// use corvis::websocket_gateway;
///
/// #[websocket_gateway("/chat")]
/// pub struct Chat {
///     #[websocket_server]
///     server: IoGatewayServer,
/// }
///
/// #[handlers]
/// impl Chat {
///     #[subscribe_message("shout")]
///     async fn shout(&self, #[message_body] text: String, #[connected_socket] socket: IoSocket) {
///         let _ = socket.id();
///         let _ = self.server.to("lobby").emit("shout", (text,)).await;
///     }
/// }
/// ```
#[derive(Clone, Default)]
pub struct IoGatewayServer {
    /// Weak: the namespace holds the gateway (its handlers and hooks), and the gateway holds this handle.
    namespace: Arc<OnceLock<Weak<NamespaceInner>>>,
}

impl std::fmt::Debug for IoGatewayServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IoGatewayServer").field("started", &self.namespace().is_some()).finish()
    }
}

impl GatewayServerField for IoGatewayServer {
    const KIND: GatewayAdapterKind = GatewayAdapterKind::SocketIo;
    fn __corvis_unbound() -> Self {
        IoGatewayServer::default()
    }
}

impl IoGatewayServer {
    /// The gateway's namespace: `None` before the gateway started (and after its server was closed).
    pub fn namespace(&self) -> Option<IoNamespace> {
        self.namespace.get().and_then(Weak::upgrade).map(|inner| IoNamespace { inner })
    }

    pub(crate) fn bind(&self, namespace: &IoNamespace) {
        let _ = self.namespace.set(Arc::downgrade(&namespace.inner));
    }

    /// The operator that reaches every socket of the namespace.
    pub fn operator(&self) -> IoGatewayBroadcast {
        IoGatewayBroadcast { operator: self.namespace().map(|n| n.operator()) }
    }

    /// `server.emit(event, ...args)`: see [`BroadcastOperator::emit`].
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        self.operator().emit(event, args).await
    }

    /// See [`BroadcastOperator::emit_with_ack`].
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Vec<Args>, BroadcastAckError> {
        self.operator().emit_with_ack(event, args).await
    }

    /// `server.to(rooms)`.
    pub fn to(&self, rooms: impl IntoRooms) -> IoGatewayBroadcast {
        self.operator().to(rooms)
    }

    /// `server.in(rooms)`.
    pub fn within(&self, rooms: impl IntoRooms) -> IoGatewayBroadcast {
        self.operator().within(rooms)
    }

    /// `server.except(rooms)`.
    pub fn except(&self, rooms: impl IntoRooms) -> IoGatewayBroadcast {
        self.operator().except(rooms)
    }

    /// `server.local`.
    pub fn local(&self) -> IoGatewayBroadcast {
        self.operator().local()
    }

    /// `server.timeout(duration)`.
    pub fn timeout(&self, timeout: Duration) -> IoGatewayBroadcast {
        self.operator().timeout(timeout)
    }

    /// See [`BroadcastOperator::fetch_sockets`].
    pub async fn fetch_sockets(&self) -> Result<Vec<RemoteSocket>, EmitError> {
        self.operator().fetch_sockets().await
    }

    /// See [`BroadcastOperator::sockets_join`].
    pub async fn sockets_join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.operator().sockets_join(rooms).await
    }

    /// See [`BroadcastOperator::sockets_leave`].
    pub async fn sockets_leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.operator().sockets_leave(rooms).await
    }

    /// See [`BroadcastOperator::disconnect_sockets`].
    pub async fn disconnect_sockets(&self, close: bool) -> Result<(), EmitError> {
        self.operator().disconnect_sockets(close).await
    }
}

/// A broadcast operator of an [`IoGatewayServer`]: the same chain as [`BroadcastOperator`] (`to`, `within`, `except`,
/// `local`, `timeout`) and the same terminal operations. Built before the gateway started, it carries a deferred
/// [`EmitError::NotStarted`]: every terminal operation returns it and sends nothing.
#[derive(Clone, Debug)]
pub struct IoGatewayBroadcast {
    operator: Option<BroadcastOperator>,
}

impl IoGatewayBroadcast {
    fn map(&self, f: impl FnOnce(&BroadcastOperator) -> BroadcastOperator) -> Self {
        IoGatewayBroadcast { operator: self.operator.as_ref().map(f) }
    }

    pub fn to(&self, rooms: impl IntoRooms) -> Self {
        self.map(|o| o.to(rooms))
    }

    pub fn within(&self, rooms: impl IntoRooms) -> Self {
        self.map(|o| o.within(rooms))
    }

    pub fn except(&self, rooms: impl IntoRooms) -> Self {
        self.map(|o| o.except(rooms))
    }

    pub fn local(&self) -> Self {
        self.map(BroadcastOperator::local)
    }

    pub fn timeout(&self, timeout: Duration) -> Self {
        self.map(|o| o.timeout(timeout))
    }

    fn started(&self) -> Result<&BroadcastOperator, EmitError> {
        self.operator.as_ref().ok_or(EmitError::NotStarted)
    }

    /// See [`BroadcastOperator::emit`].
    pub async fn emit(&self, event: &str, args: impl IntoArgs) -> Result<(), EmitError> {
        self.started()?.emit(event, args).await
    }

    /// See [`BroadcastOperator::emit_with_ack`].
    pub async fn emit_with_ack(&self, event: &str, args: impl IntoArgs) -> Result<Vec<Args>, BroadcastAckError> {
        self.started()?.emit_with_ack(event, args).await
    }

    /// See [`BroadcastOperator::fetch_sockets`].
    pub async fn fetch_sockets(&self) -> Result<Vec<RemoteSocket>, EmitError> {
        self.started()?.fetch_sockets().await
    }

    /// See [`BroadcastOperator::sockets_join`].
    pub async fn sockets_join(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.started()?.sockets_join(rooms).await
    }

    /// See [`BroadcastOperator::sockets_leave`].
    pub async fn sockets_leave(&self, rooms: impl IntoRooms) -> Result<(), EmitError> {
        self.started()?.sockets_leave(rooms).await
    }

    /// See [`BroadcastOperator::disconnect_sockets`].
    pub async fn disconnect_sockets(&self, close: bool) -> Result<(), EmitError> {
        self.started()?.disconnect_sockets(close).await
    }
}

// ------------------------------------------------------------------------------------------------------ planning

fn fail<T>(message: String) -> Result<T, DiError> {
    Err(DiError::Other(message))
}

/// One Engine.IO path: the options of its server and its namespaces, each with the gateways that declare it (indexes
/// into the gateway list, in registration order).
struct ServerPlan {
    path: String,
    options: IoOptions,
    namespaces: Vec<(String, Vec<usize>)>,
}

fn cors_options(decl: &CorsDecl) -> CorsOptions {
    match decl {
        CorsDecl::Any => CorsOptions::new(),
        CorsDecl::Origins(origins) if origins.len() == 1 => CorsOptions::new().origin(origins[0].clone()),
        CorsDecl::Origins(origins) => CorsOptions::new().origins(origins.iter().cloned()),
    }
}

/// Groups the gateways by Engine.IO path and namespace and settles the options of each path. Pure: nothing is created,
/// so a configuration error is reported before any server or gateway code exists.
fn plan(gateways: &[&GatewayRuntime], adapter: &IoAdapter) -> Result<Vec<ServerPlan>, DiError> {
    if let Some(path) = &adapter.duplicate_options {
        return fail(format!("IoAdapter::options was given options twice for the Engine.IO path {path}; give each path one IoOptions"));
    }
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, gateway) in gateways.iter().enumerate() {
        let path = EngineOptions::new().path(gateway.io_path()).normalized_path();
        match groups.iter_mut().find(|(p, _)| *p == path) {
            Some((_, members)) => members.push(index),
            None => groups.push((path, vec![index])),
        }
    }
    for (i, (a, _)) in groups.iter().enumerate() {
        for (b, _) in &groups[i + 1..] {
            if a.starts_with(b.as_str()) || b.starts_with(a.as_str()) {
                return fail(format!(
                    "the Engine.IO paths {a} and {b} overlap: a path that is a prefix of another would take its requests"
                ));
            }
        }
    }
    let mut plans = Vec::with_capacity(groups.len());
    for (path, members) in groups {
        let mut options = match adapter.options_for(&path) {
            Some(options) => options.clone(),
            None => IoOptions::new().engine(EngineOptions::new().path(path.clone())),
        };
        // The `cors` of the path's gateways: all the same, and not also set on the options.
        let mut declared: Option<(&CorsDecl, &str)> = None;
        for &i in &members {
            let gateway = gateways[i];
            let Some(decl) = &gateway.info.io.cors else { continue };
            match declared {
                Some((other, by)) if other != decl => {
                    return fail(format!(
                        "the gateways {by} and {} declare different `cors` for the Engine.IO path {path}; declare it once, or set it with IoAdapter::options",
                        gateway.name
                    ));
                }
                Some(_) => {}
                None => declared = Some((decl, &gateway.name)),
            }
            if options.engine.cors.is_some() {
                return fail(format!(
                    "{} declares `cors`, but the options the Socket.IO adapter has for the Engine.IO path {path} already set EngineOptions::cors; configure it in one place",
                    gateway.name
                ));
            }
        }
        if let Some((decl, _)) = declared {
            options.engine = options.engine.cors(cors_options(decl));
        }
        if let Err(e) = options.validate() {
            return fail(format!("the Socket.IO options for the Engine.IO path {path} are not valid: {e}"));
        }
        let mut namespaces: Vec<(String, Vec<usize>)> = Vec::new();
        for &i in &members {
            let name = gateways[i].io_namespace();
            match namespaces.iter_mut().find(|(n, _)| *n == name) {
                Some((_, list)) => list.push(i),
                None => namespaces.push((name, vec![i])),
            }
        }
        for (namespace, list) in &namespaces {
            let mut seen: HashMap<&str, &str> = HashMap::new();
            for &i in list {
                for event in gateways[i].handlers.keys() {
                    if let Some(earlier) = seen.insert(event, &gateways[i].name) {
                        return fail(format!(
                            "{earlier} and {} both have a handler for the \"{event}\" event on the namespace {namespace}; a namespace can have only one handler per event",
                            gateways[i].name
                        ));
                    }
                }
            }
        }
        plans.push(ServerPlan { path, options, namespaces });
    }
    Ok(plans)
}

/// Checks the configuration of the gateways (Engine.IO paths, `cors`, options, one handler per event per namespace).
/// Runs before the route posture check and before any user code of a gateway.
pub(crate) fn validate(gateways: &[GatewayRuntime], adapter: &IoAdapter) -> Result<(), DiError> {
    let refs: Vec<&GatewayRuntime> = gateways.iter().collect();
    plan(&refs, adapter).map(|_| ())
}

// ----------------------------------------------------------------------------------------------------- runtime

/// The gateways of one namespace and the handlers of its events.
struct NamespaceRuntime {
    gateways: Vec<Arc<GatewayRuntime>>,
    /// event -> (gateway index, handler)
    events: HashMap<String, (usize, Arc<CompiledWsHandler>)>,
    /// The `connect` target of each gateway: its metadata under the name `connect` (for connection guards).
    connect_targets: Vec<Arc<Target>>,
    hook_timeout: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GateState {
    Closed,
    Open,
    Discard,
}

/// Holds the events of a socket until its `handle_connection` hooks returned.
struct Gate {
    state: watch::Sender<GateState>,
}

impl Gate {
    fn new() -> Gate {
        Gate { state: watch::channel(GateState::Closed).0 }
    }

    fn open(&self) {
        self.state.send_if_modified(|s| {
            let closed = *s == GateState::Closed;
            if closed {
                *s = GateState::Open;
            }
            closed
        });
    }

    fn discard(&self) {
        self.state.send_replace(GateState::Discard);
    }

    /// `true` when the hooks returned and the socket stays; `false` when it was disconnected meanwhile.
    async fn admitted(&self) -> bool {
        let mut rx = self.state.subscribe();
        loop {
            match *rx.borrow_and_update() {
                GateState::Open => return true,
                GateState::Discard => return false,
                GateState::Closed => {}
            }
            if rx.changed().await.is_err() {
                return false;
            }
        }
    }

    /// Resolves when the socket was disconnected.
    async fn discarded(&self) {
        let mut rx = self.state.subscribe();
        loop {
            if *rx.borrow_and_update() == GateState::Discard {
                return;
            }
            if rx.changed().await.is_err() {
                return futures::future::pending().await;
            }
        }
    }
}

/// What the gateway layer keeps on a socket: the gate of its events.
#[derive(Clone)]
struct GatewayConn {
    gate: Arc<Gate>,
}

fn conn_of(socket: &IoSocket) -> GatewayConn {
    if let Some(conn) = socket.get::<GatewayConn>() {
        return conn;
    }
    let conn = GatewayConn { gate: Arc::new(Gate::new()) };
    socket.insert(conn.clone());
    conn
}

/// The body of a message from its arguments: none gives `null`, one gives that argument, several give the array of them.
/// Binary values become `null`: the pipeline has no binary type.
fn body_of(args: &Args) -> Result<Value, ()> {
    let mut values = Vec::with_capacity(args.len());
    for arg in args.iter() {
        values.push(arg.to_json_binary_as_null().map_err(|_| ())?);
    }
    Ok(match values.len() {
        0 => Value::Null,
        1 => values.pop().unwrap_or(Value::Null),
        _ => Value::Array(values),
    })
}

/// What a handler's result does.
#[derive(Debug, PartialEq)]
enum Delivery {
    Nothing,
    Emit(String, Value),
    Ack(Value),
}

/// Maps a plain value like the reference adapter: `null` sends nothing, an object with a non-empty string `event`
/// is emitted as that event (`data` is its argument, `null` when absent), anything else answers the acknowledgement.
fn classify_value(value: Value) -> Delivery {
    match value {
        Value::Null => Delivery::Nothing,
        Value::Object(ref map) if map.get("event").and_then(Value::as_str).is_some_and(|e| !e.is_empty()) => {
            let event = map.get("event").and_then(Value::as_str).unwrap_or_default().to_string();
            Delivery::Emit(event, map.get("data").cloned().unwrap_or(Value::Null))
        }
        other => Delivery::Ack(other),
    }
}

/// Maps what `run_handler` returned (see `encode_reply`): the marker for a plain value, `{event, data}` for a
/// `WsResponse`.
fn classify(reply: Value) -> Delivery {
    if reply.get("__corvis_same_event").is_some() {
        return classify_value(reply.get("data").cloned().unwrap_or(Value::Null));
    }
    match reply.get("event").and_then(Value::as_str) {
        Some(event) if !event.is_empty() => Delivery::Emit(event.to_string(), reply.get("data").cloned().unwrap_or(Value::Null)),
        // A response with an empty event name is a value like any other, as in the reference.
        _ => Delivery::Ack(reply),
    }
}

/// Runs one event through the gateway pipeline and answers: the acknowledgement, an emitted event, or `exception`.
async fn dispatch_event(
    gateway: Arc<GatewayRuntime>,
    handler: Arc<CompiledWsHandler>,
    socket: IoSocket,
    event: String,
    args: Args,
    ack: Option<AckSender>,
) {
    let Some(conn) = socket.get::<GatewayConn>() else { return };
    // Events wait here for `handle_connection`; a socket that was disconnected meanwhile discards them.
    if !conn.gate.admitted().await {
        return;
    }
    let body = match body_of(&args) {
        Ok(body) => body,
        Err(()) => {
            let _ = socket.emit("exception", (json!({ "status": "error", "message": "Invalid message body" }),));
            return;
        }
    };
    let no_arguments = args.is_empty();
    drop(args);
    let client: Arc<dyn Any + Send + Sync> = Arc::new(socket.clone());
    let ctx = ExecutionContext::for_ws(
        socket.handshake().request_arc(),
        gateway.class_target.clone(),
        handler.target.clone(),
        Some(gateway.module_ref.clone()),
        client.clone(),
        body.clone(),
    );
    let result = contain_panic(run_handler(&handler, &client, &ctx, body)).await;
    match result {
        Ok(Some(reply)) => match classify(reply) {
            Delivery::Nothing => {}
            Delivery::Emit(event, data) => {
                if let Err(EmitError::ReservedEvent(_)) = socket.emit(&event, (data,)) {
                    socket.warn(
                        "a reserved event in a gateway reply",
                        &format!(
                            "{}::{} returned a reply for a reserved Socket.IO event name: nothing was sent",
                            gateway.name, handler.target.name
                        ),
                    );
                }
            }
            Delivery::Ack(value) => {
                if let Some(ack) = ack {
                    let _ = ack.send((value,));
                }
            }
        },
        Ok(None) => {}
        Err(e) => {
            // The message that raised the error, as the reference's `cause`: `data` is left out when the event had
            // no argument, where the reference has `undefined`.
            let mut cause = serde_json::Map::new();
            cause.insert("pattern".to_string(), Value::String(event));
            if !no_arguments {
                cause.insert("data".to_string(), ctx.switch_to_ws().get_data().cloned().unwrap_or(Value::Null));
            }
            if let Some(data) = exception_data(&gateway, &handler, &ctx, e, Some(Value::Object(cause))).await {
                let _ = socket.emit("exception", (data,));
            }
        }
    }
}

fn dispatcher(runtime: Arc<NamespaceRuntime>) -> super::namespace::Dispatcher {
    Arc::new(move |socket: IoSocket, event: &str, args: Args, ack: Option<AckSender>| {
        let (index, handler) = runtime.events.get(event)?;
        let gateway = runtime.gateways[*index].clone();
        Some(dispatch_event(gateway, handler.clone(), socket, event.to_string(), args, ack).boxed())
    })
}

/// The CONNECT_ERROR for a connection guard that failed: the exception's own message, with the rest of its payload as
/// data; internal errors say nothing but `Internal server error`.
fn connect_error_of(e: &HttpException) -> ConnectError {
    if e.is_internal() {
        return ConnectError::internal();
    }
    match exception_payload(e) {
        Value::Object(mut map) => match map.remove("message") {
            Some(Value::String(message)) => {
                let error = ConnectError::new(message);
                if map.is_empty() { error } else { error.data(Value::Object(map)) }
            }
            other => {
                if let Some(other) = other {
                    map.insert("message".to_string(), other);
                }
                ConnectError::new("Forbidden resource").data(Value::Object(map))
            }
        },
        other => ConnectError::new("Forbidden resource").data(other),
    }
}

/// The connect middleware of `guard_connections`: the global guards once (with the first gateway of the namespace as
/// class), then the guards of each gateway (with that gateway as class). Exception filters do not run.
async fn run_connection_guards(runtime: &NamespaceRuntime, socket: IoSocket) -> Result<(), ConnectError> {
    let client: Arc<dyn Any + Send + Sync> = Arc::new(socket.clone());
    for (index, gateway) in runtime.gateways.iter().enumerate() {
        let guards: &[Arc<dyn CanActivate>] = if index == 0 { &gateway.global_guards } else { &[] };
        for list in [guards, &gateway.gateway_guards[..]] {
            if list.is_empty() {
                continue;
            }
            let ctx = ExecutionContext::for_ws(
                socket.handshake().request_arc(),
                gateway.class_target.clone(),
                runtime.connect_targets[index].clone(),
                Some(gateway.module_ref.clone()),
                client.clone(),
                Value::Null,
            );
            for guard in list {
                match guard.can_activate(&ctx).await {
                    Ok(true) => {}
                    Ok(false) => return Err(ConnectError::new("Forbidden resource")),
                    Err(e) => return Err(connect_error_of(&e)),
                }
            }
        }
    }
    Ok(())
}

fn spawn_disconnect_hooks(runtime: &Arc<NamespaceRuntime>, socket: IoSocket) {
    if !runtime.gateways.iter().any(|g| g.info.io_on_disconnect.is_some()) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let runtime = runtime.clone();
    handle.spawn(async move {
        for gateway in &runtime.gateways {
            let Some(hook) = &gateway.info.io_on_disconnect else { continue };
            let arg: Arc<dyn Any + Send + Sync> = Arc::new(socket.clone());
            if tokio::time::timeout(runtime.hook_timeout, run_hook_with(&gateway.name, "handle_disconnect", hook, arg)).await.is_err() {
                Logger::new("WsExceptionsHandler")
                    .error(format!("{}: handle_disconnect took longer than {:?} and was abandoned", gateway.name, runtime.hook_timeout));
            }
        }
    });
}

/// A socket joined a gateway namespace (the connect middleware accepted it): its events wait for the connection hooks,
/// which run in registration order; its disconnect hooks run once, whichever way it leaves.
fn on_connected(runtime: &Arc<NamespaceRuntime>, socket: IoSocket) {
    let conn = conn_of(&socket);
    let once = Arc::new(AtomicBool::new(false));
    let leave: Arc<dyn Fn(&IoSocket) + Send + Sync> = {
        let (runtime, gate, once) = (runtime.clone(), conn.gate.clone(), once.clone());
        Arc::new(move |socket: &IoSocket| {
            gate.discard();
            if !once.swap(true, Ordering::SeqCst) {
                spawn_disconnect_hooks(&runtime, socket.clone());
            }
        })
    };
    {
        let leave = leave.clone();
        socket.on_disconnect(move |socket, _reason| leave(socket));
    }
    // The socket left before the handler was registered: nothing would call it.
    if socket.is_phase(Phase::Disconnected) {
        leave(&socket);
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let runtime = runtime.clone();
    handle.spawn(async move {
        let hooks = async {
            for gateway in &runtime.gateways {
                let Some(hook) = &gateway.info.io_on_connection else { continue };
                let arg: Arc<dyn Any + Send + Sync> = Arc::new(socket.clone());
                match tokio::time::timeout(runtime.hook_timeout, run_hook_with(&gateway.name, "handle_connection", hook, arg)).await {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(_) => {
                        Logger::new("WsExceptionsHandler").error(format!(
                            "{}: handle_connection took longer than {:?}: the socket was disconnected",
                            gateway.name, runtime.hook_timeout
                        ));
                        return false;
                    }
                }
            }
            true
        };
        tokio::select! {
            biased;
            // The socket left while a hook ran: the hook is dropped, its events are discarded.
            _ = conn.gate.discarded() => {}
            ok = hooks => {
                if ok {
                    conn.gate.open();
                } else {
                    conn.gate.discard();
                    socket.disconnect(true);
                }
            }
        }
    });
}

/// Creates the servers, namespaces and dispatchers of the gateways, runs `after_init` and attaches the servers.
/// Servers created before a failure are closed.
pub(crate) async fn start(
    compiled: Vec<GatewayRuntime>,
    adapter: &IoAdapter,
    http: &HttpServer,
) -> Result<Vec<Arc<GatewayRuntime>>, DiError> {
    let gateways: Vec<Arc<GatewayRuntime>> = compiled.into_iter().map(Arc::new).collect();
    let refs: Vec<&GatewayRuntime> = gateways.iter().map(|g| &**g).collect();
    let plans = plan(&refs, adapter)?;
    let mut servers: Vec<IoServer> = Vec::new();
    match build(&gateways, plans, adapter, http, &mut servers).await {
        Ok(()) => Ok(gateways),
        Err(error) => {
            for server in &servers {
                server.close();
            }
            Err(error)
        }
    }
}

async fn build(
    gateways: &[Arc<GatewayRuntime>],
    plans: Vec<ServerPlan>,
    adapter: &IoAdapter,
    http: &HttpServer,
    servers: &mut Vec<IoServer>,
) -> Result<(), DiError> {
    let log = Logger::new("SocketIoGateways");
    for unused in adapter.options.iter().filter(|o| !plans.iter().any(|p| p.path == o.engine.normalized_path())) {
        log.warn(format!(
            "IoAdapter::options were given for the Engine.IO path {}, which no gateway uses: they have no effect",
            unused.engine.normalized_path()
        ));
    }
    for ServerPlan { path, options, namespaces } in plans {
        if options.engine.cors.as_ref().is_some_and(|c| c.credentials) && options.engine.allowed_origins.is_none() {
            log.warn(format!(
                "the Engine.IO path {path} allows CORS with credentials and has no origin allowlist: CORS does not apply to the WebSocket transport, so a cookie-authenticated gateway is open to cross-site WebSocket hijacking unless EngineOptions::allowed_origins (or a connection check) is set"
            ));
        }
        let io = IoServer::new(options).map_err(|e| DiError::Other(format!("the Socket.IO server of the Engine.IO path {path}: {e}")))?;
        servers.push(io.clone());
        // The main namespace `/` exists on every server: when no gateway declares it, nobody may join it.
        if !namespaces.iter().any(|(name, _)| name == "/") {
            io.main().use_middleware(|_socket| async { Err(ConnectError::new("Invalid namespace")) });
        }
        let mut to_init: Vec<(IoNamespace, Arc<GatewayRuntime>)> = Vec::new();
        for (name, members) in namespaces {
            let namespace = io.of(&name).map_err(|e| DiError::Other(format!("{}: {e}", gateways[members[0]].name)))?;
            let members: Vec<Arc<GatewayRuntime>> = members.iter().map(|&i| gateways[i].clone()).collect();
            let mut events = HashMap::new();
            for (index, gateway) in members.iter().enumerate() {
                for (event, handler) in &gateway.handlers {
                    events.insert(event.clone(), (index, handler.clone()));
                }
            }
            let connect_targets = members
                .iter()
                .map(|g| Arc::new(Target { name: "connect".to_string(), metadata: g.class_target.metadata.clone() }))
                .collect();
            let runtime =
                Arc::new(NamespaceRuntime { gateways: members.clone(), events, connect_targets, hook_timeout: adapter.hook_timeout });
            namespace.set_dispatcher(dispatcher(runtime.clone()));
            if adapter.guard_connections {
                let guards = runtime.clone();
                namespace.use_middleware(move |socket| {
                    let guards = guards.clone();
                    async move { run_connection_guards(&guards, socket).await }
                });
            }
            let connected = runtime.clone();
            namespace.on_connection(move |socket| on_connected(&connected, socket));
            for gateway in &members {
                if let Some(server) = gateway.info.server.as_ref().and_then(|s| s.downcast_ref::<IoGatewayServer>()) {
                    server.bind(&namespace);
                }
                log.log(format!("{}: Socket.IO namespace {name} on {}", gateway.name, gateway.io_path()));
                to_init.push((namespace.clone(), gateway.clone()));
            }
        }
        for (namespace, gateway) in to_init {
            let Some(hook) = &gateway.info.io_on_init else { continue };
            let arg: Arc<dyn Any + Send + Sync> = Arc::new(namespace);
            match tokio::time::timeout(adapter.hook_timeout, run_hook_with(&gateway.name, "after_init", hook, arg)).await {
                Ok(true) => {}
                Ok(false) => return fail(format!("{}: after_init panicked", gateway.name)),
                Err(_) => {
                    return fail(format!(
                        "{}: after_init took longer than {:?} (IoAdapter::hook_timeout)",
                        gateway.name, adapter.hook_timeout
                    ));
                }
            }
        }
        io.attach(http).map_err(|e| DiError::Other(format!("the Engine.IO path {path} cannot be attached: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_has_the_shape_of_the_reference_adapter() {
        use super::super::value::IoValue;
        let one = IoValue::from("x");
        assert_eq!(body_of(&Args::new(vec![])), Ok(Value::Null));
        assert_eq!(body_of(&Args::new(vec![one.clone()])), Ok(json!("x")));
        assert_eq!(body_of(&Args::new(vec![one.clone(), IoValue::from(2)])), Ok(json!(["x", 2])));
    }

    #[test]
    fn a_binary_argument_is_null_in_the_body() {
        use super::super::value::{Binary, IoValue};
        let binary = IoValue::Binary(Binary::new(vec![1u8, 2, 3]));
        assert_eq!(body_of(&Args::new(vec![binary.clone()])), Ok(Value::Null));
        assert_eq!(body_of(&Args::new(vec![IoValue::from(1), binary])), Ok(json!([1, null])));
    }

    #[test]
    fn results_map_like_the_reference_adapter() {
        assert_eq!(classify_value(Value::Null), Delivery::Nothing);
        assert_eq!(classify_value(json!(0)), Delivery::Ack(json!(0)));
        assert_eq!(classify_value(json!(false)), Delivery::Ack(json!(false)));
        assert_eq!(classify_value(json!("")), Delivery::Ack(json!("")));
        assert_eq!(classify_value(json!({"event": "duck", "data": 2})), Delivery::Emit("duck".into(), json!(2)));
        assert_eq!(classify_value(json!({"event": "duck"})), Delivery::Emit("duck".into(), Value::Null));
        // An `event` that is not a non-empty string is not duck typing.
        assert_eq!(classify_value(json!({"event": 3})), Delivery::Ack(json!({"event": 3})));
        assert_eq!(classify_value(json!({"event": ""})), Delivery::Ack(json!({"event": ""})));
        assert_eq!(classify(json!({"__corvis_same_event": true, "data": 7})), Delivery::Ack(json!(7)));
        assert_eq!(classify(json!({"__corvis_same_event": true, "data": null})), Delivery::Nothing);
        assert_eq!(classify(json!({"event": "reply", "data": 1})), Delivery::Emit("reply".into(), json!(1)));
    }

    #[test]
    fn a_connection_guard_failure_is_a_connect_error_without_internal_detail() {
        use crate::websockets::WsException;
        let object = connect_error_of(&WsException::object(json!({"message": "Token expired", "code": "EXPIRED"})));
        assert_eq!(object.message(), "Token expired");
        assert_eq!(object.data, Some(super::super::value::IoValue::from_serialize(&json!({"code": "EXPIRED"})).unwrap()));
        let string = connect_error_of(&WsException::new("Room is full"));
        assert_eq!(string.message(), "Room is full");
        let no_message = connect_error_of(&WsException::object(json!({"code": "X"})));
        assert_eq!(no_message.message(), "Forbidden resource");
        assert!(no_message.data.is_some());
        let internal = connect_error_of(&HttpException::from_anyhow(anyhow::anyhow!("secret detail")));
        assert_eq!(internal.message(), "Internal server error");
        assert!(internal.data.is_none());
    }

    #[tokio::test]
    async fn the_gate_admits_after_open_and_refuses_after_discard() {
        let gate = Arc::new(Gate::new());
        let waiting = tokio::spawn({
            let gate = gate.clone();
            async move { gate.admitted().await }
        });
        tokio::task::yield_now().await;
        gate.open();
        assert!(waiting.await.unwrap());
        gate.discard();
        assert!(!gate.admitted().await, "a discarded gate never admits");
        let fresh = Gate::new();
        fresh.discard();
        fresh.open();
        assert!(!fresh.admitted().await, "open does not undo a discard");
    }

    #[tokio::test]
    async fn an_unbound_server_handle_fails_every_operation_and_sends_nothing() {
        let server = IoGatewayServer::default();
        assert!(server.namespace().is_none());
        assert_eq!(server.emit("news", ("x",)).await, Err(EmitError::NotStarted));
        assert_eq!(server.to("room").emit("news", ()).await, Err(EmitError::NotStarted));
        assert_eq!(
            server.to("a").except("b").within("c").local().timeout(Duration::from_secs(1)).emit("n", ()).await,
            Err(EmitError::NotStarted)
        );
        assert!(
            matches!(server.emit_with_ack("news", ()).await, Err(e) if matches!(e.kind(), super::super::BroadcastAckErrorKind::Emit(EmitError::NotStarted)))
        );
        assert!(matches!(server.fetch_sockets().await, Err(EmitError::NotStarted)));
        assert_eq!(server.sockets_join("r").await, Err(EmitError::NotStarted));
        assert_eq!(server.sockets_leave("r").await, Err(EmitError::NotStarted));
        assert_eq!(server.disconnect_sockets(true).await, Err(EmitError::NotStarted));
    }

    #[tokio::test]
    async fn a_bound_handle_gives_its_namespace_and_forgets_it_when_the_server_is_gone() {
        let io = IoServer::new(IoOptions::new()).unwrap();
        let namespace = io.of("/chat").unwrap();
        let server = IoGatewayServer::default();
        server.bind(&namespace);
        assert_eq!(server.namespace().unwrap().name(), "/chat");
        assert_eq!(server.emit("news", ()).await, Ok(()));
        // Binding twice keeps the first namespace.
        server.bind(&io.main());
        assert_eq!(server.namespace().unwrap().name(), "/chat");
        drop(namespace);
        io.close();
        drop(io);
        tokio::task::yield_now().await;
    }
}

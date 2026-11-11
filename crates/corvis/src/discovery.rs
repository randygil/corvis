//! Discovery of method-level handlers declared with `#[handlers]`
//! (`#[on_event]`, `#[cron]`, `#[interval]`, `#[timeout]`, `#[subscribe_message]`).

use std::any::Any;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;

use crate::di::container::{Container, ModuleId};

/// Payload of an event (`eventEmitter.emit('order.created', payload)`).
pub type Payload = Arc<dyn Any + Send + Sync>;

pub type EventCall = Arc<dyn Fn(Payload) -> BoxFuture<'static, crate::Result<()>> + Send + Sync>;
pub type TaskCall = Arc<dyn Fn() -> BoxFuture<'static, crate::Result<()>> + Send + Sync>;
/// A `#[subscribe_message]` handler: receives the client and the message body arguments
/// (one per `#[message_body]` parameter, after the pipes) and returns the reply frame.
pub type WsCall = Arc<
    dyn Fn(Arc<dyn Any + Send + Sync>, Vec<Value>) -> BoxFuture<'static, Result<Option<Value>, crate::http::HttpException>> + Send + Sync,
>;

/// Enhancers and metadata of a gateway (`#[use_guards]`, ... below `#[websocket_gateway]`)
/// or of a `#[subscribe_message]` handler.
#[derive(Clone, Default)]
pub struct WsEnhancers {
    pub guards: Vec<crate::common::enhancer::GuardRef>,
    pub interceptors: Vec<crate::common::enhancer::InterceptorRef>,
    pub pipes: Vec<crate::common::enhancer::PipeRef>,
    pub filters: Vec<crate::common::enhancer::FilterRef>,
    pub metadata: Vec<(String, Value)>,
    /// The `#[message_body]` parameters of a handler, in order (`kind` is `Body`).
    pub params: Vec<crate::router::controller::ParamDef>,
    /// The adapter each `#[connected_socket]` parameter of a handler belongs to (its type's kind), so a mismatch with
    /// the application's adapter is a startup error.
    pub socket_kinds: Vec<GatewayAdapterKind>,
}

/// Which transport a gateway handle belongs to: the `ws`-style adapter (`WsClient`, `WsServer`) or Socket.IO
/// (`IoSocket`, `IoGatewayServer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GatewayAdapterKind {
    Ws,
    SocketIo,
}

impl GatewayAdapterKind {
    /// How a message names the adapter.
    pub fn describe(self) -> &'static str {
        match self {
            GatewayAdapterKind::Ws => "default WebSocket adapter",
            GatewayAdapterKind::SocketIo => "Socket.IO adapter",
        }
    }
}

/// The `cors` option of `#[websocket_gateway]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorsDecl {
    /// `cors = true`: any origin, without credentials.
    Any,
    /// `cors = "https://a"` or `cors = ["https://a", "https://b"]`.
    Origins(Vec<String>),
}

/// What `#[websocket_gateway]` declared for the Socket.IO adapter: `namespace` (or the positional literal), `path`
/// (the Engine.IO path) and `cors`. The `ws` adapter reads `GatewayInfo::path` instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IoGatewayDecl {
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub cors: Option<CorsDecl>,
}

impl WsEnhancers {
    pub fn guard(mut self, g: crate::common::enhancer::GuardRef) -> Self {
        self.guards.push(g);
        self
    }
    pub fn interceptor(mut self, i: crate::common::enhancer::InterceptorRef) -> Self {
        self.interceptors.push(i);
        self
    }
    pub fn pipe(mut self, p: crate::common::enhancer::PipeRef) -> Self {
        self.pipes.push(p);
        self
    }
    /// Adds an exception filter. Filters are tried in the order they are added and the first whose
    /// `catches` returns `true` answers. `#[use_filters(A, B)]` adds `B` and then `A`, like the original
    /// framework, which tries the last listed filter first.
    pub fn filter(mut self, f: crate::common::enhancer::FilterRef) -> Self {
        self.filters.push(f);
        self
    }
    pub fn set_metadata(mut self, key: impl Into<String>, value: impl serde::Serialize) -> Self {
        self.metadata.push((key.into(), serde_json::to_value(value).unwrap_or(Value::Null)));
        self
    }
    pub fn param(mut self, p: crate::router::controller::ParamDef) -> Self {
        self.params.push(p);
        self
    }
    /// Records the adapter of a `#[connected_socket]` parameter.
    pub fn socket_kind(mut self, kind: GatewayAdapterKind) -> Self {
        self.socket_kinds.push(kind);
        self
    }
}

#[derive(Clone)]
pub enum HandlerKind {
    Event { events: Vec<String>, call: EventCall },
    Cron { cron_time: crate::modules::schedule::CronSource, options: crate::modules::schedule::CronOptions, call: TaskCall },
    Interval { millis: u64, name: Option<String>, call: TaskCall },
    Timeout { millis: u64, name: Option<String>, call: TaskCall },
    WsMessage { event: String, call: WsCall, enhancers: WsEnhancers },
}

/// A handler found on a provider instance.
#[derive(Clone)]
pub struct DiscoveredHandler {
    pub class_name: &'static str,
    pub method_name: &'static str,
    pub kind: HandlerKind,
}

/// Implemented by `#[handlers]`.
pub trait HandlersProvider: Send + Sync + 'static {
    fn __corvis_handlers(this: Arc<Self>) -> Vec<DiscoveredHandler>;
}

pub(crate) struct Discovered {
    #[allow(dead_code)]
    pub module: ModuleId,
    #[allow(dead_code)]
    pub type_id: std::any::TypeId,
    pub handler: DiscoveredHandler,
}

/// Collects handlers of every instantiated provider and controller.
pub(crate) fn discover(container: &Arc<Container>) -> Vec<Discovered> {
    let mut out = Vec::new();
    for module in &container.modules {
        let providers = module.providers.iter().chain(module.controllers.iter().map(|c| &c.provider));
        for p in providers {
            let Some(explore) = &p.hooks.handlers else { continue };
            let Some(instance) = container.hook_instance(module.id, &p.token) else { continue };
            let type_id = match &p.token {
                crate::di::Token::Type { id, .. } => *id,
                _ => std::any::TypeId::of::<()>(),
            };
            for h in explore(instance) {
                out.push(Discovered { module: module.id, type_id, handler: h });
            }
        }
    }
    out
}

pub type GatewayHook = Arc<dyn Fn(Arc<dyn Any + Send + Sync>) -> BoxFuture<'static, ()> + Send + Sync>;

/// Metadata of a `#[websocket_gateway]` instance.
#[derive(Clone)]
pub struct GatewayInfo {
    pub path: String,
    /// The Socket.IO view of the gateway's options (see [`IoGatewayDecl`]).
    pub io: IoGatewayDecl,
    pub on_init: Option<GatewayHook>,
    pub on_connection: Option<GatewayHook>,
    pub on_disconnect: Option<GatewayHook>,
    /// The hooks implemented for the Socket.IO handles (`OnGatewayInit<IoNamespace>`, `OnGatewayConnection<IoSocket>`,
    /// `OnGatewayDisconnect<IoSocket>`).
    pub io_on_init: Option<GatewayHook>,
    pub io_on_connection: Option<GatewayHook>,
    pub io_on_disconnect: Option<GatewayHook>,
    /// The `#[websocket_server]` field of this gateway instance (a `WsServer` or an `IoGatewayServer`), if any.
    pub server: Option<Arc<dyn Any + Send + Sync>>,
    /// The adapter the `#[websocket_server]` field's type belongs to.
    pub server_kind: Option<GatewayAdapterKind>,
    /// Class-level enhancers (`#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[set_metadata]`).
    pub enhancers: WsEnhancers,
}

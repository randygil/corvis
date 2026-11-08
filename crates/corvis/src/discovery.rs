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
    pub metadata: Vec<(String, Value)>,
    /// The `#[message_body]` parameters of a handler, in order (`kind` is `Body`).
    pub params: Vec<crate::router::controller::ParamDef>,
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
    pub fn set_metadata(mut self, key: impl Into<String>, value: impl serde::Serialize) -> Self {
        self.metadata.push((key.into(), serde_json::to_value(value).unwrap_or(Value::Null)));
        self
    }
    pub fn param(mut self, p: crate::router::controller::ParamDef) -> Self {
        self.params.push(p);
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
            let Some(instance) = container.instance(module.id, &p.token) else { continue };
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
    pub on_init: Option<GatewayHook>,
    pub on_connection: Option<GatewayHook>,
    pub on_disconnect: Option<GatewayHook>,
    /// The `#[websocket_server]` field of this gateway instance (a `WsServer`), if any.
    pub server: Option<Arc<dyn Any + Send + Sync>>,
    /// Class-level enhancers (`#[use_guards]`, `#[use_interceptors]`, `#[use_pipes]`, `#[set_metadata]`).
    pub enhancers: WsEnhancers,
}

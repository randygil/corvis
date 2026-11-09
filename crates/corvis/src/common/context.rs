//! `ExecutionContext` / `ArgumentsHost`: information about the current request
//! and the handler that is about to be executed.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::di::ModuleRef;
use crate::http::Request;

/// A handler or a controller class plus its metadata (`context.getHandler()` / `context.getClass()`).
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub name: String,
    pub metadata: HashMap<String, Value>,
}

impl Target {
    pub fn new(name: impl Into<String>) -> Self {
        Target { name: name.into(), metadata: HashMap::new() }
    }
    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }
}

/// Type of the current execution context (`context.getType()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextType {
    Http,
    Ws,
    Rpc,
}

impl ContextType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextType::Http => "http",
            ContextType::Ws => "ws",
            ContextType::Rpc => "rpc",
        }
    }
}

struct CtxInner {
    request: Request,
    handler: Arc<Target>,
    class: Arc<Target>,
    module_ref: Option<ModuleRef>,
    context_type: ContextType,
    ws_client: Option<Arc<dyn Any + Send + Sync>>,
    ws_data: Option<Value>,
}

/// The execution context passed to guards, interceptors and filters.
#[derive(Clone)]
pub struct ExecutionContext {
    inner: Arc<CtxInner>,
}

/// In corvis the arguments host (used by exception filters) is the execution context itself.
pub type ArgumentsHost = ExecutionContext;

impl ExecutionContext {
    pub fn new(request: Request, class: Arc<Target>, handler: Arc<Target>, module_ref: Option<ModuleRef>) -> Self {
        ExecutionContext {
            inner: Arc::new(CtxInner {
                request,
                handler,
                class,
                module_ref,
                context_type: ContextType::Http,
                ws_client: None,
                ws_data: None,
            }),
        }
    }

    /// The context of a WebSocket message (`context.getType() === 'ws'`): `request` is the
    /// handshake request of the connection, `client` the connected socket and `data` the
    /// message payload.
    pub fn for_ws(
        request: Request,
        class: Arc<Target>,
        handler: Arc<Target>,
        module_ref: Option<ModuleRef>,
        client: Arc<dyn Any + Send + Sync>,
        data: Value,
    ) -> Self {
        ExecutionContext {
            inner: Arc::new(CtxInner {
                request,
                handler,
                class,
                module_ref,
                context_type: ContextType::Ws,
                ws_client: Some(client),
                ws_data: Some(data),
            }),
        }
    }

    /// `context.switchToHttp()`.
    pub fn switch_to_http(&self) -> HttpArgumentsHost<'_> {
        HttpArgumentsHost { ctx: self }
    }

    /// `context.switchToWs()`.
    pub fn switch_to_ws(&self) -> WsArgumentsHost<'_> {
        WsArgumentsHost { ctx: self }
    }

    /// Shortcut for `switch_to_http().get_request()`.
    pub fn request(&self) -> &Request {
        &self.inner.request
    }

    /// `context.getHandler()`.
    pub fn get_handler(&self) -> &Target {
        &self.inner.handler
    }

    /// `context.getClass()`.
    pub fn get_class(&self) -> &Target {
        &self.inner.class
    }

    /// `context.getType()`.
    pub fn get_type(&self) -> ContextType {
        self.inner.context_type
    }

    /// The `ModuleRef` of the module that owns the controller.
    pub fn module_ref(&self) -> Option<&ModuleRef> {
        self.inner.module_ref.as_ref()
    }
}

pub struct HttpArgumentsHost<'a> {
    ctx: &'a ExecutionContext,
}

/// The status of the matched route (`#[http_code]`, else 201 for POST and 200 otherwise),
/// which is written to `res.statusCode` before guards and interceptors run.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RouteStatus(pub(crate) http::StatusCode);

/// Headers that frame the response or the connection; `set_response_header` refuses them.
const RESERVED_RESPONSE_HEADERS: [&str; 8] =
    ["content-length", "transfer-encoding", "connection", "keep-alive", "proxy-connection", "te", "trailer", "upgrade"];

/// Distinct header names an enhancer chain can record for one request.
const MAX_ENHANCER_HEADERS: usize = 128;

/// Response headers recorded by guards, interceptors and pipes (see
/// [`HttpArgumentsHost::set_response_header`]). Created on first use.
#[derive(Clone, Default)]
pub(crate) struct EnhancerHeaders(pub(crate) Arc<std::sync::Mutex<http::HeaderMap>>);

impl EnhancerHeaders {
    /// A snapshot of the recorded headers, if any enhancer set one.
    pub(crate) fn of(request: &Request) -> Option<http::HeaderMap> {
        request.get::<EnhancerHeaders>().map(|h| h.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

impl<'a> HttpArgumentsHost<'a> {
    /// Records a header for the response of the current HTTP request. It is added to the final response (success,
    /// error and filter responses) only when the response has no header with that name, and written to the `Res` of
    /// `#[res]` routes when absent there. A later call with the same name replaces the recorded value.
    ///
    /// Returns `false`, recording nothing, when
    /// - the name is not a valid header name, or the value has a character Node would refuse (anything but tab,
    ///   visible ASCII and U+0080..U+00FF, so CR and LF included);
    /// - the name is a framing or connection header the framework owns (`Content-Length`, `Transfer-Encoding`,
    ///   `Connection`, `Keep-Alive`, `Proxy-Connection`, `TE`, `Trailer`, `Upgrade`);
    /// - 128 distinct names are already recorded for the request (replacing a recorded name still works);
    /// - the context is a WebSocket one.
    ///
    /// A `Content-Type` recorded here applies only to a response without a body: a body (JSON, text, bytes, a
    /// stream) keeps its own type, and `#[res]` routes never receive it.
    pub fn set_response_header(&self, name: &str, value: &str) -> bool {
        if self.ctx.inner.context_type != ContextType::Http {
            return false;
        }
        let Ok(n) = http::HeaderName::from_bytes(name.as_bytes()) else { return false };
        if RESERVED_RESPONSE_HEADERS.contains(&n.as_str()) {
            return false;
        }
        if !value.chars().all(|c| matches!(c as u32, 0x09 | 0x20..=0x7e | 0x80..=0xff)) {
            return false;
        }
        let Ok(v) = http::HeaderValue::from_bytes(value.as_bytes()) else { return false };
        let req = &self.ctx.inner.request;
        let store = match req.get::<EnhancerHeaders>() {
            Some(s) => s,
            None => {
                let s = EnhancerHeaders::default();
                req.insert(s.clone());
                s
            }
        };
        {
            let mut headers = store.0.lock().unwrap_or_else(|e| e.into_inner());
            if headers.len() >= MAX_ENHANCER_HEADERS && !headers.contains_key(&n) {
                return false;
            }
            headers.insert(n.clone(), v.clone());
        }
        if n != http::header::CONTENT_TYPE
            && let Some(res) = req.get::<crate::http::Res>()
            && !res.headers_sent()
            && !res.has_header(n.as_str())
        {
            res.set_header_value(n, v);
        }
        true
    }

    pub fn get_request(&self) -> &'a Request {
        &self.ctx.inner.request
    }
    /// `getResponse().statusCode` as guards and interceptors see it: the route's status
    /// (`#[http_code]`, 201 for POST, 200 otherwise), or 200 outside a route.
    pub fn status_code(&self) -> u16 {
        self.ctx.inner.request.get::<RouteStatus>().map(|s| s.0.as_u16()).unwrap_or(200)
    }
    /// `getResponse()`: the Express response when the handler declares `#[res]`.
    pub fn get_response(&self) -> Option<crate::http::Res> {
        self.ctx.inner.request.get::<crate::http::Res>()
    }
}

pub struct WsArgumentsHost<'a> {
    ctx: &'a ExecutionContext,
}

impl<'a> WsArgumentsHost<'a> {
    /// The connected client (downcast it to `WsClient`).
    pub fn get_client<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.ctx.inner.ws_client.clone().and_then(|c| c.downcast::<T>().ok())
    }
    /// The message payload.
    pub fn get_data(&self) -> Option<&'a Value> {
        self.ctx.inner.ws_data.as_ref()
    }
}

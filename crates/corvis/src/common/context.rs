//! `ExecutionContext` / `ArgumentsHost`: information about the current request
//! and the handler that is about to be executed.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use crate::di::ModuleRef;
use crate::http::{Request, Response};

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
    request: Arc<Request>,
    handler: Arc<Target>,
    class: Arc<Target>,
    module_ref: Option<ModuleRef>,
    context_type: ContextType,
    ws_client: Option<Arc<dyn Any + Send + Sync>>,
    ws_data: Option<Arc<Value>>,
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
                request: Arc::new(request),
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
    ///
    /// The handshake is shared, not copied: a message costs a reference count, whatever the size
    /// of the handshake's query and headers.
    pub fn for_ws(
        request: impl Into<Arc<Request>>,
        class: Arc<Target>,
        handler: Arc<Target>,
        module_ref: Option<ModuleRef>,
        client: Arc<dyn Any + Send + Sync>,
        data: Value,
    ) -> Self {
        Self::for_ws_shared(request, class, handler, module_ref, client, Arc::new(data))
    }

    /// Like [`for_ws`](Self::for_ws) with a payload that is already shared: the context holds the reference, not a copy.
    pub fn for_ws_shared(
        request: impl Into<Arc<Request>>,
        class: Arc<Target>,
        handler: Arc<Target>,
        module_ref: Option<ModuleRef>,
        client: Arc<dyn Any + Send + Sync>,
        data: Arc<Value>,
    ) -> Self {
        ExecutionContext {
            inner: Arc::new(CtxInner {
                request: request.into(),
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

/// Set on a request when its pipeline produced an exception (a guard denied, a pipe or validation failed, the handler
/// returned `Err`, something panicked) or a guard ended the request, at the moment it happened: before any interceptor
/// or exception filter can turn the failure into another response. Middleware that must not act on a request the
/// application rejected (the session middleware, for a login a guard made) reads it instead of the final status.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestRejected;

impl RequestRejected {
    pub(crate) fn mark(req: &Request) {
        req.insert(RequestRejected);
    }

    #[cfg(feature = "auth-session")]
    pub(crate) fn is_marked(extensions: &std::sync::RwLock<http::Extensions>) -> bool {
        // A lock a panic poisoned still holds the mark (the router marks the request while that panic unwinds).
        extensions.read().unwrap_or_else(|e| e.into_inner()).get::<RequestRejected>().is_some()
    }
}

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

/// Validates a header an enhancer wants on the response: a valid name that is not a framing header and a value
/// without a character Node would refuse (anything but tab, visible ASCII and U+0080..U+00FF).
pub(crate) fn validate_response_header(name: &str, value: &str) -> Option<(http::HeaderName, http::HeaderValue)> {
    let n = http::HeaderName::from_bytes(name.as_bytes()).ok()?;
    if RESERVED_RESPONSE_HEADERS.contains(&n.as_str()) {
        return None;
    }
    if !value.chars().all(|c| matches!(c as u32, 0x09 | 0x20..=0x7e | 0x80..=0xff)) {
        return None;
    }
    let v = http::HeaderValue::from_bytes(value.as_bytes()).ok()?;
    Some((n, v))
}

/// Values recorded with [`HttpArgumentsHost::append_response_header`], in call order. At most
/// [`MAX_ENHANCER_HEADERS`] values per request. Created on first use.
#[derive(Clone, Default)]
pub(crate) struct AppendedHeaders(pub(crate) Arc<std::sync::Mutex<Vec<(http::HeaderName, http::HeaderValue)>>>);

impl AppendedHeaders {
    /// A snapshot of the appended values, if any enhancer appended one.
    pub(crate) fn of(request: &Request) -> Option<Vec<(http::HeaderName, http::HeaderValue)>> {
        request.get::<AppendedHeaders>().map(|h| h.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// The response a guard stored with [`HttpArgumentsHost::respond_with`].
#[derive(Clone, Default)]
pub(crate) struct PendingResponse(pub(crate) Arc<std::sync::Mutex<Option<Response>>>);

impl PendingResponse {
    /// Takes the stored response, if a guard stored one.
    pub(crate) fn take(request: &Request) -> Option<Response> {
        request.get::<PendingResponse>().and_then(|p| p.0.lock().unwrap_or_else(|e| e.into_inner()).take())
    }
}

/// Marks that the guards of the route are running: [`HttpArgumentsHost::respond_with`] is valid only then. The router
/// creates it before the first guard and sets the flag around each `can_activate` call.
#[derive(Clone, Default)]
pub(crate) struct GuardPhase(pub(crate) Arc<std::sync::atomic::AtomicBool>);

impl GuardPhase {
    /// Creates the marker (flag off) on the request, replacing an earlier one.
    pub(crate) fn install(request: &Request) -> GuardPhase {
        let phase = GuardPhase::default();
        request.insert(phase.clone());
        phase
    }

    pub(crate) fn set(&self, active: bool) {
        self.0.store(active, std::sync::atomic::Ordering::SeqCst);
    }

    /// Turns the phase on until the returned scope is dropped, also when the guard panics.
    pub(crate) fn enter(&self) -> GuardPhaseScope {
        self.set(true);
        GuardPhaseScope(self.clone())
    }

    pub(crate) fn is_active(request: &Request) -> bool {
        request.get::<GuardPhase>().is_some_and(|p| p.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}

/// Ends the guard phase on drop.
pub(crate) struct GuardPhaseScope(GuardPhase);

impl Drop for GuardPhaseScope {
    fn drop(&mut self) {
        self.0.set(false);
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
        let Some((n, v)) = validate_response_header(name, value) else { return false };
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

    /// Appends a value to a header of the response of the current HTTP request. Unlike
    /// [`set_response_header`](Self::set_response_header), values recorded for the same name accumulate in call
    /// order. Every `Set-Cookie` value is added to the final response next to the response's own cookies (recorded
    /// ones first), on every path: success, exception, filter, `#[res]` and a response stored with
    /// [`respond_with`](Self::respond_with). Any other name is added only when the response has no header with that
    /// name, and after the value recorded with `set_response_header` for it.
    ///
    /// Returns `false`, recording nothing, for an invalid name or value or a framing header (the rules of
    /// `set_response_header`), when 128 values are already recorded for the request, and in a WebSocket context.
    pub fn append_response_header(&self, name: &str, value: &str) -> bool {
        if self.ctx.inner.context_type != ContextType::Http {
            return false;
        }
        let Some((n, v)) = validate_response_header(name, value) else { return false };
        let req = &self.ctx.inner.request;
        let store = match req.get::<AppendedHeaders>() {
            Some(s) => s,
            None => {
                let s = AppendedHeaders::default();
                req.insert(s.clone());
                s
            }
        };
        let recorded_before = {
            let mut list = store.0.lock().unwrap_or_else(|e| e.into_inner());
            if list.len() >= MAX_ENHANCER_HEADERS {
                return false;
            }
            let before = list.iter().any(|(k, _)| *k == n)
                || req.get::<EnhancerHeaders>().is_some_and(|h| h.0.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&n));
            list.push((n.clone(), v.clone()));
            before
        };
        if n != http::header::CONTENT_TYPE
            && let Some(res) = req.get::<crate::http::Res>()
            && !res.headers_sent()
            && (n == http::header::SET_COOKIE || recorded_before || !res.has_header(n.as_str()))
        {
            res.append_header_value(n, v);
        }
        true
    }

    /// Ends the request with `response` from a guard: the router sends it right after the current guard returns, without running
    /// later guards, interceptors, pipes or the handler, and without exception filters, `#[header]` or route
    /// redirect settings. It still receives the headers recorded with `set_response_header` and
    /// `append_response_header`. The stored response wins whatever the guard returns (a discarded `Ok(false)` or
    /// error is logged); a second call replaces the first.
    ///
    /// Valid only while a guard runs: called from an interceptor, a handler or a hand-made context it stores nothing
    /// and returns `false`, because nothing would send the response. It returns `false` too in a WebSocket context.
    pub fn respond_with(&self, response: Response) -> bool {
        if self.ctx.inner.context_type != ContextType::Http || !GuardPhase::is_active(&self.ctx.inner.request) {
            return false;
        }
        let req = &self.ctx.inner.request;
        let store = match req.get::<PendingResponse>() {
            Some(s) => s,
            None => {
                let s = PendingResponse::default();
                req.insert(s.clone());
                s
            }
        };
        *store.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(response);
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
        self.ctx.inner.ws_data.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_ctx() -> ExecutionContext {
        ExecutionContext::new(Request::empty(), Arc::new(Target::new("C")), Arc::new(Target::new("h")), None)
    }

    fn ws_ctx() -> ExecutionContext {
        ExecutionContext::for_ws(Request::empty(), Arc::new(Target::new("G")), Arc::new(Target::new("h")), None, Arc::new(()), Value::Null)
    }

    #[test]
    fn append_validates_like_set() {
        let ctx = http_ctx();
        let http = ctx.switch_to_http();
        assert!(http.append_response_header("Link", "</a>"));
        assert!(!http.append_response_header("bad name", "v"));
        assert!(!http.append_response_header("X-A", "a\r\nX-B: b"));
        assert!(!http.append_response_header("X-A", "\u{20ac}"));
        for reserved in RESERVED_RESPONSE_HEADERS {
            assert!(!http.append_response_header(reserved, "1"), "{reserved}");
        }
        assert_eq!(AppendedHeaders::of(ctx.request()).unwrap().len(), 1);
    }

    #[test]
    fn append_stops_at_128_values_and_set_still_replaces() {
        let ctx = http_ctx();
        let http = ctx.switch_to_http();
        assert!((0..128).all(|_| http.append_response_header("X-Trace", "v")));
        assert!(!http.append_response_header("X-Trace", "v"));
        assert!(!http.append_response_header("X-Other", "v"));
        assert_eq!(AppendedHeaders::of(ctx.request()).unwrap().len(), 128);
        assert!(http.set_response_header("X-Trace", "one"));
        assert!(http.set_response_header("X-Trace", "two"));
        assert_eq!(EnhancerHeaders::of(ctx.request()).unwrap().get("x-trace").unwrap(), "two");
    }

    #[test]
    fn websocket_context_records_nothing() {
        let ctx = ws_ctx();
        let http = ctx.switch_to_http();
        assert!(!http.append_response_header("Set-Cookie", "a=1"));
        GuardPhase::install(ctx.request()).set(true);
        assert!(!http.respond_with(Response::new()));
        assert!(AppendedHeaders::of(ctx.request()).is_none());
        assert!(PendingResponse::take(ctx.request()).is_none());
    }

    #[test]
    fn respond_with_replaces_the_stored_response() {
        let ctx = http_ctx();
        let http = ctx.switch_to_http();
        GuardPhase::install(ctx.request()).set(true);
        assert!(http.respond_with(Response::new().with_status(302u16)));
        assert!(http.respond_with(Response::new().with_status(301u16)));
        assert_eq!(PendingResponse::take(ctx.request()).unwrap().status_code(), 301);
        assert!(PendingResponse::take(ctx.request()).is_none());
    }

    #[test]
    fn respond_with_is_refused_outside_the_guard_phase() {
        let ctx = http_ctx();
        let http = ctx.switch_to_http();
        assert!(!http.respond_with(Response::new().with_status(302u16)), "no phase at all");
        let phase = GuardPhase::install(ctx.request());
        assert!(!http.respond_with(Response::new().with_status(302u16)), "phase off");
        phase.set(true);
        assert!(http.respond_with(Response::new().with_status(302u16)));
        phase.set(false);
        assert!(!http.respond_with(Response::new().with_status(301u16)));
        assert_eq!(PendingResponse::take(ctx.request()).unwrap().status_code(), 302);
    }
}

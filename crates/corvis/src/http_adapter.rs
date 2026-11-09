//! `HttpAdapterHost`: access to the application's underlying HTTP
//! server from any provider, e.g. to attach raw `upgrade` listeners exactly like
//! `adapterHost.httpAdapter.getHttpServer().on('upgrade', (req, socket, head) => ...)`.
//!
//! ```ignore
//! #[injectable]
//! pub struct BridgeService {
//!     adapter_host: Arc<HttpAdapterHost>,
//! }
//!
//! #[async_trait]
//! impl OnApplicationBootstrap for BridgeService {
//!     async fn on_application_bootstrap(&self) -> corvis::Result<()> {
//!         let server = self.adapter_host.http_adapter().get_http_server();
//!         let wss = corvis::ws::WebSocketServer::new(Default::default());
//!         server.on_upgrade(move |req| {
//!             if req.pathname() != "/ws/bridge" {
//!                 req.destroy();
//!                 return;
//!             }
//!             wss.handle_upgrade(&req, |ws| {
//!                 ws.on_message(|m| println!("{m}"));
//!             });
//!         });
//!         Ok(())
//!     }
//! }
//! ```
//!
//! Node semantics are kept: while at least one upgrade listener is registered, every
//! HTTP/1.1 request carrying `Connection: upgrade` + `Upgrade: <protocol>` is handed to
//! the listeners instead of the router (paths of `#[websocket_gateway]`s are served by
//! the gateways first). Each listener receives an [`UpgradeRequest`]; the first one that
//! accepts (`WebSocketServer::handle_upgrade` / [`UpgradeRequest::switch_protocols`]),
//! responds ([`UpgradeRequest::respond`]) or destroys ([`UpgradeRequest::destroy`]) it
//! wins. When every listener drops the request without deciding, the socket is destroyed.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};

use http::HeaderMap;
use hyper_util::rt::TokioIo;
use tokio::sync::oneshot;

use crate::http::{Request, Response};
use crate::logger::Logger;

/// The raw bidirectional byte stream obtained after a successful protocol switch.
pub type UpgradedIo = TokioIo<hyper::upgrade::Upgraded>;

/// Future returned by [`UpgradeRequest::switch_protocols`].
pub type UpgradeFuture = Pin<Box<dyn Future<Output = std::io::Result<UpgradedIo>> + Send + 'static>>;

type UpgradeListener = Arc<dyn Fn(UpgradeRequest) + Send + Sync + 'static>;

/// `HttpAdapterHost`: injectable (`adapter_host: Arc<HttpAdapterHost>`) in any provider.
#[derive(Clone, Default)]
pub struct HttpAdapterHost {
    adapter: HttpAdapter,
}

impl HttpAdapterHost {
    pub fn new() -> Self {
        Self::default()
    }
    /// `adapterHost.httpAdapter`.
    pub fn http_adapter(&self) -> &HttpAdapter {
        &self.adapter
    }
    /// `adapterHost.listening`: `true` once `app.listen()` bound the socket.
    pub fn listening(&self) -> bool {
        self.adapter.server.listening()
    }
}

/// The HTTP adapter (`AbstractHttpAdapter`).
#[derive(Clone, Default)]
pub struct HttpAdapter {
    server: HttpServer,
}

impl HttpAdapter {
    /// `httpAdapter.getHttpServer()`.
    pub fn get_http_server(&self) -> HttpServer {
        self.server.clone()
    }
    /// `httpAdapter.getType()`.
    pub fn get_type(&self) -> &'static str {
        "corvis"
    }
}

#[derive(Default)]
struct ServerInner {
    upgrade: RwLock<Vec<UpgradeListener>>,
    address: RwLock<Option<SocketAddr>>,
}

/// The application's HTTP server (Node's `http.Server` as seen by `getHttpServer()`).
#[derive(Clone, Default)]
pub struct HttpServer {
    inner: Arc<ServerInner>,
}

impl HttpServer {
    /// `server.on('upgrade', listener)`. Listeners run synchronously, in registration
    /// order, on the connection task; spawn a task for anything slow.
    pub fn on_upgrade(&self, listener: impl Fn(UpgradeRequest) + Send + Sync + 'static) -> &Self {
        self.inner.upgrade.write().unwrap().push(Arc::new(listener));
        self
    }
    /// `server.listenerCount('upgrade')`.
    pub fn upgrade_listener_count(&self) -> usize {
        self.inner.upgrade.read().unwrap().len()
    }
    /// `server.removeAllListeners('upgrade')`.
    pub fn remove_all_upgrade_listeners(&self) {
        self.inner.upgrade.write().unwrap().clear();
    }
    /// `server.address()`: the bound address once listening.
    pub fn address(&self) -> Option<SocketAddr> {
        *self.inner.address.read().unwrap()
    }
    /// `server.listening`.
    pub fn listening(&self) -> bool {
        self.address().is_some()
    }
    pub(crate) fn set_address(&self, addr: Option<SocketAddr>) {
        *self.inner.address.write().unwrap() = addr;
    }
    fn listeners(&self) -> Vec<UpgradeListener> {
        self.inner.upgrade.read().unwrap().clone()
    }
}

/// What the listeners decided for an upgrade request.
pub(crate) enum UpgradeDecision {
    Destroy,
    Respond(Response),
    Switch(HeaderMap),
}

struct UpgradeInner {
    request: Request,
    decision: Mutex<Option<oneshot::Sender<UpgradeDecision>>>,
    on_upgrade: Mutex<Option<hyper::upgrade::OnUpgrade>>,
}

/// An HTTP upgrade request handed to `server.on_upgrade` listeners (`(req, socket, head)`).
/// Cheap to clone; all clones refer to the same socket.
#[derive(Clone)]
pub struct UpgradeRequest {
    inner: Arc<UpgradeInner>,
}

/// Error returned when the upgrade request was already accepted, answered or destroyed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeHandledError;

impl std::fmt::Display for UpgradeHandledError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the upgrade request was already handled (accepted, answered or destroyed)")
    }
}
impl std::error::Error for UpgradeHandledError {}

impl UpgradeRequest {
    /// The handshake request (method, URL, headers, query, remote address...). Its body is empty.
    pub fn request(&self) -> &Request {
        &self.inner.request
    }
    /// `req.method`.
    pub fn method(&self) -> &http::Method {
        self.inner.request.method()
    }
    /// `req.url`: path plus query string, as received.
    pub fn url(&self) -> String {
        self.inner.request.url()
    }
    /// `new URL(req.url, 'http://localhost').pathname`.
    pub fn pathname(&self) -> &str {
        self.inner.request.path()
    }
    /// The raw query string (without `?`), if any.
    pub fn query_string(&self) -> Option<&str> {
        self.inner.request.uri().query()
    }
    /// A query string parameter (`url.searchParams.get(name)`).
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.inner.request.query_param(name)
    }
    /// `req.headers`.
    pub fn headers(&self) -> &HeaderMap {
        self.inner.request.headers()
    }
    /// A header value as a string.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.inner.request.header(name)
    }
    /// The peer address (`req.socket.remoteAddress`).
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.inner.request.remote_addr()
    }
    /// `true` once a listener accepted, answered or destroyed the request.
    pub fn is_handled(&self) -> bool {
        self.inner.decision.lock().unwrap().is_none()
    }

    fn decide(&self, d: UpgradeDecision) -> Result<(), UpgradeHandledError> {
        let tx = self.inner.decision.lock().unwrap().take().ok_or(UpgradeHandledError)?;
        tx.send(d).map_err(|_| UpgradeHandledError)
    }

    /// `socket.destroy()`: closes the TCP connection without writing anything.
    /// Does nothing if the request was already handled.
    pub fn destroy(&self) {
        let _ = self.decide(UpgradeDecision::Destroy);
    }

    /// Writes a plain HTTP response and closes the connection (what `ws` does with
    /// `abortHandshake`, or `socket.end('HTTP/1.1 401 Unauthorized\r\n\r\n')`).
    /// `Connection: close` is added.
    pub fn respond(&self, response: Response) -> Result<(), UpgradeHandledError> {
        self.decide(UpgradeDecision::Respond(response))
    }

    /// Answers `101 Switching Protocols` with `headers` (e.g. `Upgrade`, `Connection`,
    /// `Sec-WebSocket-Accept`) and returns a future resolving to the raw upgraded byte
    /// stream, for implementing any protocol on top of the HTTP server.
    pub fn switch_protocols(&self, headers: HeaderMap) -> Result<UpgradeFuture, UpgradeHandledError> {
        let on_upgrade = self.inner.on_upgrade.lock().unwrap().take().ok_or(UpgradeHandledError)?;
        self.decide(UpgradeDecision::Switch(headers))?;
        Ok(Box::pin(async move { on_upgrade.await.map(TokioIo::new).map_err(std::io::Error::other) }))
    }
}

/// `true` for HTTP/1.x requests carrying `Connection: upgrade` and an `Upgrade` header
/// (what Node's parser reports as `req.upgrade`).
pub(crate) fn is_upgrade_request(req: &hyper::Request<hyper::body::Incoming>) -> bool {
    if req.version() >= http::Version::HTTP_2 || !req.headers().contains_key(http::header::UPGRADE) {
        return false;
    }
    req.headers()
        .get_all(http::header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
}

/// Error used to make hyper drop the connection (`socket.destroy()`).
#[derive(Debug)]
pub(crate) struct SocketDestroyed;
impl std::fmt::Display for SocketDestroyed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("socket destroyed by an upgrade listener")
    }
}
impl std::error::Error for SocketDestroyed {}

/// Runs the upgrade listeners for `req` and turns their decision into a response
/// (or into a destroyed socket).
pub(crate) async fn dispatch_upgrade(
    server: &HttpServer,
    mut req: hyper::Request<hyper::body::Incoming>,
    remote: SocketAddr,
    trust: &crate::http::proxy_addr::TrustProxy,
) -> Result<hyper::Response<crate::server::ResBody>, SocketDestroyed> {
    let on_upgrade = hyper::upgrade::on(&mut req);
    let (parts, _) = req.into_parts();
    let Ok(request) = Request::for_upgrade(parts, remote, trust) else {
        return Err(SocketDestroyed);
    };
    let (tx, rx) = oneshot::channel();
    let upgrade = UpgradeRequest {
        inner: Arc::new(UpgradeInner { request, decision: Mutex::new(Some(tx)), on_upgrade: Mutex::new(Some(on_upgrade)) }),
    };
    for listener in server.listeners() {
        let u = upgrade.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener(u)));
        if result.is_err() {
            Logger::new("HttpServer").error("an 'upgrade' listener panicked");
        }
    }
    drop(upgrade);
    match rx.await {
        Ok(UpgradeDecision::Switch(headers)) => {
            let mut resp = hyper::Response::new(crate::server::empty_body());
            *resp.status_mut() = http::StatusCode::SWITCHING_PROTOCOLS;
            *resp.headers_mut() = headers;
            Ok(resp)
        }
        Ok(UpgradeDecision::Respond(response)) => {
            let mut resp = crate::server::into_hyper(response, false);
            resp.headers_mut().insert(http::header::CONNECTION, http::HeaderValue::from_static("close"));
            Ok(resp)
        }
        Ok(UpgradeDecision::Destroy) | Err(_) => Err(SocketDestroyed),
    }
}

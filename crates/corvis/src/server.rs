//! HTTP server (hyper) integration.

use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited, StreamBody};
use hyper::body::{Frame, Incoming, SizeHint};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::{Instant, Sleep};

use crate::http::{Body, HttpException, Request, Response};
use crate::router::{AppShared, dispatch};

mod h1_framing;

pub(crate) type ResBody = UnsyncBoxBody<Bytes, std::io::Error>;

/// Options of the HTTP server: timeouts, connection and header limits, opt-in h2c.
///
/// The defaults follow Node's `http.Server` (`headersTimeout` 60 s, `requestTimeout` 300 s,
/// `keepAliveTimeout` 5 s) and protect against slow clients (slowloris) that would otherwise keep
/// connections and buffered bodies alive forever.
///
/// ```ignore
/// app.set_server_options(
///     ServerOptions::new().keep_alive_timeout(Duration::from_secs(10)).max_connections(10_000),
/// );
/// ```
#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub(crate) headers_timeout: Option<Duration>,
    pub(crate) request_timeout: Option<Duration>,
    pub(crate) keep_alive_timeout: Option<Duration>,
    pub(crate) max_connections: Option<usize>,
    pub(crate) max_header_size: usize,
    pub(crate) h2c: bool,
    pub(crate) insecure_http_parser: bool,
}

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            headers_timeout: Some(Duration::from_secs(60)),
            request_timeout: Some(Duration::from_secs(300)),
            keep_alive_timeout: Some(Duration::from_secs(5)),
            max_connections: None,
            max_header_size: 32 * 1024,
            h2c: false,
            insecure_http_parser: false,
        }
    }
}

impl ServerOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// Time a client has to send the request head (`server.headersTimeout`, default 60 s): a
    /// connection that sends nothing, or drips the headers, is closed. `None` disables it.
    pub fn headers_timeout(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.headers_timeout = t.into();
        self
    }
    /// Time a client has to send the whole request body, counted from the end of the head
    /// (`server.requestTimeout`, default 300 s). When it expires the request is answered with
    /// `408` and the connection is closed. It also bounds the streamed read of multipart bodies.
    pub fn request_timeout(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.request_timeout = t.into();
        self
    }
    /// Idle time allowed between requests of a keep-alive connection
    /// (`server.keepAliveTimeout`, default 5 s).
    pub fn keep_alive_timeout(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.keep_alive_timeout = t.into();
        self
    }
    /// Maximum number of open connections (`server.maxConnections`); when reached, new
    /// connections wait in the accept queue. Default: unlimited. Upgraded (WebSocket)
    /// connections count until they close, and so do sockets in their short closing drain
    /// (up to 2 s after an early error response); with a cap, give gateways a heartbeat so
    /// dead WebSocket clients do not hold slots.
    pub fn max_connections(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_connections = n.into();
        self
    }
    /// Maximum size of the request head in bytes (HTTP/1, default 32 KiB, minimum 8 KiB).
    /// Bigger heads are answered with `431`.
    pub fn max_header_size(mut self, bytes: usize) -> Self {
        self.max_header_size = bytes.max(8 * 1024);
        self
    }
    /// HTTP/2 over cleartext with prior knowledge (h2c), for a proxy or platform configured to
    /// send it. Disabled by default, like Node's `http` module, which accepts only HTTP/1.x; a
    /// prior-knowledge connection is closed without a response. With it on, one connection carries
    /// up to 200 concurrent requests, so `max_connections` limits connections rather than
    /// requests, and the HTTP/2 header list limit is 16 KiB (`max_header_size` is HTTP/1 only).
    pub fn h2c(mut self, enabled: bool) -> Self {
        self.h2c = enabled;
        self
    }
    /// Accept ambiguous HTTP/1 framing that Corvis rejects by default: bare LF line endings in the
    /// request head and in chunked trailers, whitespace after a chunk size, and `Content-Length`
    /// together with `Transfer-Encoding`. Named after Node's `insecureHTTPParser`. Default `false`.
    /// Only for legacy clients that cannot be fixed; prefer a proxy that normalizes requests.
    ///
    /// It turns off those four checks and nothing else: inputs that hyper already rejects (a bare
    /// CR, a chunk size over 64 bits, a bare LF in the chunk framing, an LF in a chunk extension,
    /// obsolete line folding, a bad `Transfer-Encoding`) stay rejected, and h2c, timeouts and size
    /// limits are not touched. Corvis logs a warning at startup when it is on.
    pub fn insecure_http_parser(mut self, enabled: bool) -> Self {
        self.insecure_http_parser = enabled;
        self
    }
}

fn full(data: impl Into<Bytes>) -> ResBody {
    Full::new(data.into()).map_err(|never: Infallible| match never {}).boxed_unsync()
}

pub(crate) fn empty_body() -> ResBody {
    full(Bytes::new())
}

/// Converts a corvis response into a hyper response.
pub(crate) fn into_hyper(resp: Response, head: bool) -> hyper::Response<ResBody> {
    let status = resp.status.unwrap_or(http::StatusCode::OK);
    let mut builder = hyper::Response::builder().status(status);
    let mut headers = resp.headers;
    for (name, value) in resp.default_headers.iter() {
        if !headers.contains_key(name) {
            headers.insert(name.clone(), value.clone());
        }
    }
    let is_empty = matches!(resp.body, Body::Empty);
    let (content_type, body): (Option<String>, ResBody) = match resp.body {
        Body::Empty => (None, full(Bytes::new())),
        Body::Json(v) => (Some("application/json; charset=utf-8".into()), full(serde_json::to_vec(&v).unwrap_or_default())),
        Body::Text(t) => (Some("text/html; charset=utf-8".into()), full(t)),
        Body::Html(t) => (Some("text/html; charset=utf-8".into()), full(t)),
        Body::Bytes { data, content_type } => (Some(content_type), full(data)),
        Body::Stream { stream, content_type } => {
            let s = stream.map(|r| r.map(Frame::data));
            (Some(content_type), BodyExt::boxed_unsync(StreamBody::new(s)))
        }
    };
    // An empty type (`res.end()` without a `Content-Type`) writes no header, like Node.
    if let Some(ct) = content_type.filter(|ct| !ct.is_empty())
        && !headers.contains_key(http::header::CONTENT_TYPE)
        && let Ok(v) = http::HeaderValue::from_str(&ct)
    {
        headers.insert(http::header::CONTENT_TYPE, v);
    }
    // `X-Powered-By`, ETag, ... are added by `http::express::finalize` (see `handle`).
    let bodyless = status == http::StatusCode::NO_CONTENT || status == http::StatusCode::NOT_MODIFIED || status.is_informational();
    // An empty answer (`res.send()` without a body) sends no length for HEAD.
    if head && !bodyless && !is_empty && !headers.contains_key(http::header::CONTENT_LENGTH) {
        // HEAD keeps the length of the body it would have sent (like Node).
        if let Some(len) = hyper::body::Body::size_hint(&body).exact() {
            headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from(len));
        }
    }
    if let Some(h) = builder.headers_mut() {
        *h = headers;
    }
    let body = if head { full(Bytes::new()) } else { body };
    builder.body(body).unwrap_or_else(|_| hyper::Response::new(full(Bytes::new())))
}

/// Finishes a handler's response: the Express defaults, the conversion to the wire response and,
/// when the application enabled it, compression. The server and the test client both end here.
pub(crate) fn finish_response(
    mut resp: Response,
    info: &crate::http::express::RequestInfo,
    settings: &crate::http::HttpSettings,
    head: bool,
) -> hyper::Response<ResBody> {
    crate::http::express::finalize(&mut resp, info, settings);
    let wire = into_hyper(resp, head);
    #[cfg(feature = "compression")]
    if let Some(options) = &settings.compression {
        return crate::http::compression::apply(wire, info, options);
    }
    wire
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

/// A request body that has not been read yet (multipart bodies: only a file interceptor, after
/// the guards, reads them; see [`crate::multer::read_body`]).
pub(crate) struct PendingBody {
    pub body: Incoming,
    /// `requestTimeout` deadline of the request.
    pub deadline: Option<Instant>,
}

/// Request extension holding the unread body. Shared by all the clones of the request; the
/// first reader takes it.
#[derive(Clone)]
pub(crate) struct PendingBodySlot(pub Arc<Mutex<Option<PendingBody>>>);

impl PendingBodySlot {
    pub(crate) fn take(req: &Request) -> Option<PendingBody> {
        let slot = req.get::<PendingBodySlot>()?;
        let mut guard = slot.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.take()
    }
}

/// `408` of `server.requestTimeout`.
pub(crate) fn request_timeout_error() -> HttpException {
    HttpException::http_error(408, "Request Timeout")
}

/// A body that could not be read (client aborted, malformed chunked encoding, ...).
pub(crate) fn request_aborted_error() -> HttpException {
    HttpException::http_error(400, "request aborted")
}

pub(crate) fn entity_too_large_error() -> HttpException {
    HttpException::http_error(413, "request entity too large")
}

/// `Content-Length` of the request, when present and valid.
pub(crate) fn content_length(headers: &http::HeaderMap) -> Option<u64> {
    headers.get(http::header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

/// How many bytes of a non-multipart body are read: the limit of the body parser that will
/// parse it (JSON / urlencoded), otherwise the transport limit (`set_body_limit`), so that
/// `req.raw_body()` keeps working for other types.
fn read_limit(headers: &http::HeaderMap, transport: usize, parser: &crate::http::body_parser::BodyParserOptions) -> usize {
    if !parser.enabled {
        return transport;
    }
    let identity =
        headers.get(http::header::CONTENT_ENCODING).is_none_or(|v| v.to_str().is_ok_and(|s| s.trim().eq_ignore_ascii_case("identity")));
    if !identity {
        // The parser answers 415 whatever the size: keep the transport limit.
        return transport;
    }
    let Some(ct) = headers.get(http::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) else { return transport };
    let mut params = ct.split(';');
    let essence = params.next().unwrap_or("").trim_matches([' ', '\t']).to_ascii_lowercase();
    let charset = params.find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("charset").then(|| v.trim().trim_matches('"').to_ascii_lowercase())
    });
    let parser_limit = match essence.as_str() {
        "application/json" if charset.as_deref().is_none_or(|c| c.starts_with("utf-")) => parser.json_limit,
        "application/x-www-form-urlencoded" if charset.as_deref().is_none_or(|c| c == "utf-8" || c == "iso-8859-1") => {
            parser.urlencoded_limit
        }
        _ => return transport,
    };
    parser_limit.min(transport)
}

/// Converts a hyper request into a corvis request.
///
/// - `multipart/*` bodies are not read here: the body stays in the request and only a file
///   interceptor reads it, streaming, with its own limits (Express: multer consumes the stream
///   only on routes that use it, after the guards).
/// - Other bodies are read up to the limit of the parser that accepts them (`json` /
///   `urlencoded`) or up to the transport limit (`body_limit`), and within `request_timeout`.
#[allow(clippy::result_large_err)] // internal: the error is the ready-made rejection `Response`
pub(crate) async fn from_hyper(
    req: hyper::Request<Incoming>,
    remote: Option<SocketAddr>,
    limit: usize,
    parser: &crate::http::body_parser::BodyParserOptions,
    request_timeout: Option<Duration>,
) -> Result<Request, Response> {
    let (parts, body) = req.into_parts();
    let deadline = request_timeout.map(|t| Instant::now() + t);
    if crate::multer::is_multipart_content_type(&parts.headers) {
        let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, Bytes::new(), parser);
        r.version = parts.version;
        r.remote_addr = remote;
        let mut ext = parts.extensions;
        ext.insert(PendingBodySlot(Arc::new(Mutex::new(Some(PendingBody { body, deadline })))));
        *r.extensions.write().unwrap() = ext;
        return Ok(r);
    }
    let limit = read_limit(&parts.headers, limit, parser);
    let mut unread = None;
    let (bytes, error) = if content_length(&parts.headers).is_some_and(|n| n > limit as u64) {
        // raw-body: a declared length over the limit is rejected without reading the body
        // (at most a bounded part of it is discarded after the response, see `drain`).
        unread = Some(PendingBody { body, deadline });
        (Bytes::new(), Some(entity_too_large_error()))
    } else {
        let read = Limited::new(body, limit).collect();
        let result = match deadline {
            Some(d) => tokio::time::timeout_at(d, read).await.ok(),
            None => Some(read.await),
        };
        match result {
            Some(Ok(c)) => (c.to_bytes(), None),
            Some(Err(e)) if e.downcast_ref::<LengthLimitError>().is_some() => (Bytes::new(), Some(entity_too_large_error())),
            Some(Err(_)) => (Bytes::new(), Some(request_aborted_error())),
            None => (Bytes::new(), Some(request_timeout_error())),
        }
    };
    // Body-parser errors stay in the request: the router answers them after CORS and through
    // the global filters (Express `next(err)`).
    let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, bytes, parser);
    if let Some(e) = error {
        r.body_error = Some(e);
    }
    r.version = parts.version;
    r.remote_addr = remote;
    let mut ext = parts.extensions;
    if let Some(p) = unread {
        ext.insert(PendingBodySlot(Arc::new(Mutex::new(Some(p)))));
    }
    *r.extensions.write().unwrap() = ext;
    Ok(r)
}

/// Discards what is left of a body nobody read, up to `cap` bytes and until its deadline, so a
/// client that already sent it still reads the response and the connection can be reused
/// (Node drains unread bodies too). Past the cap or the deadline the body is dropped and hyper
/// closes the connection: the rest is never read nor kept in memory.
async fn drain(pending: PendingBody, cap: u64) {
    let PendingBody { mut body, deadline } = pending;
    let deadline = deadline.unwrap_or_else(|| Instant::now() + Duration::from_secs(30));
    let mut read: u64 = 0;
    while read <= cap {
        match tokio::time::timeout_at(deadline, body.frame()).await {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    read += data.len() as u64;
                }
            }
            _ => break,
        }
    }
}

async fn handle(
    shared: Arc<AppShared>,
    req: hyper::Request<Incoming>,
    remote: SocketAddr,
    options: &ServerOptions,
) -> Result<hyper::Response<ResBody>, crate::http_adapter::SocketDestroyed> {
    #[cfg(feature = "websockets")]
    {
        if let Some(gw) = crate::websockets::match_upgrade(&shared, req.headers(), req.uri().path()) {
            return Ok(crate::websockets::upgrade(gw, req, remote));
        }
    }
    // `server.on('upgrade', ...)` listeners take every upgrade request (Node semantics).
    if crate::http_adapter::is_upgrade_request(&req) {
        if shared.http_server.upgrade_listener_count() > 0 {
            return crate::http_adapter::dispatch_upgrade(&shared.http_server, req, remote).await;
        }
        // Like the `ws` adapter, upgrades for paths without a gateway are destroyed.
        #[cfg(feature = "websockets")]
        {
            if !shared.gateways.is_empty() {
                return Err(crate::http_adapter::SocketDestroyed);
            }
        }
    }
    let head = req.method() == http::Method::HEAD;
    let info = crate::http::express::RequestInfo::new(req.method(), req.headers());
    #[cfg(feature = "compression")]
    let info = {
        let mut info = info.with_compression(req.uri().path(), req.headers(), &shared.http_settings);
        info.conn_slots = req.extensions().get::<crate::http::compression::ConnectionSlots>().cloned();
        info
    };
    let resp = match from_hyper(req, Some(remote), shared.body_limit, &shared.http_settings.body_parser, options.request_timeout).await {
        Ok(r) => {
            let pending = r.get::<PendingBodySlot>();
            let resp = dispatch(shared.clone(), r).await;
            // A body nobody read (or left half read by a failed upload) is drained now, not
            // when the last clone of the request goes away.
            if let Some(slot) = pending {
                let unread = slot.0.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(p) = unread {
                    tokio::spawn(drain(p, shared.body_limit as u64));
                }
            }
            resp
        }
        Err(resp) => resp,
    };
    Ok(finish_response(resp, &info, &shared.http_settings, head))
}

// ---------------------------------------------------------------------------
// Connection timeouts
// ---------------------------------------------------------------------------

/// Activity of one connection, shared between its IO and its in-flight requests.
#[derive(Default)]
struct ConnState {
    /// Requests whose response has not been fully written yet.
    inflight: AtomicUsize,
    /// Bumped when a request starts or ends.
    generation: AtomicU64,
    /// Requests whose response is done (fully written or dropped). The framing check compares it
    /// with the requests it forwarded to know when a connection has nothing in flight.
    finished: AtomicU64,
    /// The connection was upgraded (WebSocket, ...): no idle timeouts.
    upgraded: AtomicBool,
    /// An upgrade listener destroyed the socket (or the upgrade was refused): nothing to protect.
    destroyed: AtomicBool,
    /// The server closes the connection to punish the peer (idle timeouts, refused h2c): no drain.
    no_linger: AtomicBool,
    /// The socket and its permit, left here by `TimedIo::drop` for the connection task, which decides
    /// whether to drain before closing (hyper's own timeouts are only visible to that task).
    closing: Mutex<Option<(TcpStream, Option<OwnedSemaphorePermit>)>>,
    /// Task waiting on the IO, woken when the last in-flight request ends.
    waker: Mutex<Option<Waker>>,
}

/// Keeps a request in flight until its response body is dropped (fully written or aborted).
struct InflightGuard(Arc<ConnState>);

impl InflightGuard {
    fn new(state: Arc<ConnState>) -> Self {
        state.inflight.fetch_add(1, Ordering::SeqCst);
        state.generation.fetch_add(1, Ordering::SeqCst);
        InflightGuard(state)
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, Ordering::SeqCst);
        self.0.finished.fetch_add(1, Ordering::SeqCst);
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(w) = self.0.waker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            w.wake();
        }
    }
}

/// A response body that releases its [`InflightGuard`] when hyper drops it.
struct GuardedBody {
    inner: ResBody,
    _guard: InflightGuard,
}

impl hyper::body::Body for GuardedBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[derive(Clone, Copy)]
enum Phase {
    /// Waiting for (the rest of) a request head; counted from the connection start or from the
    /// first byte after an idle keep-alive period.
    Head(Instant),
    /// Idle between requests since the last response ended (or the last byte was written).
    KeepAlive(Instant),
}

/// hyper's `ignore_invalid_headers`; it must stay off while the framing check is on.
const IGNORE_INVALID_HEADERS: bool = false;

const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// The socket of a connection, with the `headersTimeout` / `keepAliveTimeout` of Node: while no
/// request is in flight, a read that waits past the deadline fails and hyper closes the
/// connection. Requests in flight are not limited here (the body read has `requestTimeout`).
///
/// When the connection is dropped by the server, it is closed gracefully (see [`linger_close`]) so
/// that an error response sent before the request was fully read is not lost to a TCP reset.
struct TimedIo {
    /// `None` only while `Drop` hands the socket to the closing task.
    io: Option<TcpStream>,
    /// Held until the socket is really closed, so `max_connections` also counts closing sockets.
    permit: Option<OwnedSemaphorePermit>,
    state: Arc<ConnState>,
    headers_timeout: Option<Duration>,
    keep_alive_timeout: Option<Duration>,
    phase: Phase,
    seen_generation: u64,
    sleep: Option<Pin<Box<Sleep>>>,
    /// h2c disabled: bytes of the HTTP/2 preface matched so far (`None` = not a preface).
    preface: Option<usize>,
    /// The HTTP/1 framing check; `None` with `insecure_http_parser(true)`.
    framing: Option<Box<h1_framing::Framing>>,
    /// Bytes the check released that did not fit in hyper's buffer yet, served before the socket.
    queue: Vec<u8>,
    queue_pos: usize,
    /// The socket reported end of input.
    sock_eof: bool,
}

/// Most input the server reads and discards after an early error response before closing.
const LINGER_MAX_BYTES: usize = 1024 * 1024;
/// Longest the server waits for the peer to finish (or stop) sending after an early error response.
const LINGER_MAX_TIME: Duration = Duration::from_secs(2);

/// Closes a socket the way Node does after answering before the request was fully read: the
/// response is already flushed, so send FIN, then read and discard what the peer still sends until
/// it closes, `LINGER_MAX_BYTES` or `LINGER_MAX_TIME`. Dropping a socket that still has unread
/// input makes Windows (and Linux) send a reset, and the peer then discards the response it has
/// not read yet. Both bounds keep a peer from holding the connection (and its connection permit).
async fn linger_close(mut io: TcpStream) {
    let drain = async {
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut io).await;
        let mut buf = [0u8; 4 * 1024];
        let mut total = 0usize;
        while total < LINGER_MAX_BYTES {
            match io.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => total += n,
            }
        }
    };
    let _ = tokio::time::timeout(LINGER_MAX_TIME, drain).await;
}

impl Drop for TimedIo {
    fn drop(&mut self) {
        if let Some(io) = self.io.take() {
            let permit = self.permit.take();
            *self.state.closing.lock().unwrap_or_else(|e| e.into_inner()) = Some((io, permit));
        }
    }
}

/// `true` when hyper ended the connection because of its own header read timeout.
fn is_hyper_timeout(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut cur = Some(err);
    while let Some(e) = cur {
        if e.downcast_ref::<hyper::Error>().is_some_and(|h| h.is_timeout()) {
            return true;
        }
        cur = e.source();
    }
    false
}

/// Runs a connection to its end, then closes the socket: gracefully (see [`linger_close`]) unless the
/// server is closing it to punish the peer (an idle timeout of this server, hyper's header read
/// timeout, refused h2c), the connection was upgraded, or an upgrade listener destroyed the socket.
async fn finish_connection(state: Arc<ConnState>, conn: impl Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>) {
    let result = conn.await;
    let punish = state.no_linger.load(Ordering::SeqCst)
        || state.destroyed.load(Ordering::SeqCst)
        || state.upgraded.load(Ordering::SeqCst)
        || result.as_ref().err().is_some_and(|e| is_hyper_timeout(e.as_ref()));
    let closing = state.closing.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some((io, permit)) = closing {
        if !punish {
            linger_close(io).await;
        }
        drop(permit);
    }
}

impl TimedIo {
    fn new(io: TcpStream, permit: Option<OwnedSemaphorePermit>, state: Arc<ConnState>, options: &ServerOptions) -> Self {
        TimedIo {
            io: Some(io),
            permit,
            state,
            headers_timeout: options.headers_timeout,
            keep_alive_timeout: options.keep_alive_timeout,
            phase: Phase::Head(Instant::now()),
            seen_generation: 0,
            sleep: None,
            preface: if options.h2c { None } else { Some(0) },
            framing: (!options.insecure_http_parser).then(|| Box::new(h1_framing::Framing::new(options.max_header_size.max(8 * 1024)))),
            queue: Vec::new(),
            queue_pos: 0,
            sock_eof: false,
        }
    }

    /// Notices requests that started or ended since the last poll.
    fn sync(&mut self) {
        let g = self.state.generation.load(Ordering::SeqCst);
        if g != self.seen_generation {
            self.seen_generation = g;
            if self.state.inflight.load(Ordering::SeqCst) == 0 {
                self.phase = Phase::KeepAlive(Instant::now());
            }
        }
    }

    fn sock(&mut self) -> Pin<&mut TcpStream> {
        Pin::new(self.io.as_mut().expect("socket is present until drop"))
    }

    fn on_read(&mut self, data: &[u8]) -> io::Result<()> {
        self.sync();
        if let Phase::KeepAlive(_) = self.phase {
            self.phase = Phase::Head(Instant::now());
        }
        if let Some(matched) = self.preface {
            let n = data.len().min(H2_PREFACE.len() - matched);
            if data[..n] != H2_PREFACE[matched..matched + n] {
                self.preface = None;
            } else if matched + n == H2_PREFACE.len() {
                self.state.no_linger.store(true, Ordering::SeqCst);
                return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP/2 (h2c) is disabled"));
            } else {
                self.preface = Some(matched + n);
            }
        }
        Ok(())
    }

    fn poll_idle(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sync();
        if self.state.upgraded.load(Ordering::SeqCst) {
            return Poll::Pending;
        }
        if self.state.inflight.load(Ordering::SeqCst) > 0 {
            *self.state.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            // The request may have ended in between: look again.
            if self.state.inflight.load(Ordering::SeqCst) > 0 {
                return Poll::Pending;
            }
            self.sync();
        }
        let (since, timeout) = match self.phase {
            Phase::Head(t) => (t, self.headers_timeout),
            Phase::KeepAlive(t) => (t, self.keep_alive_timeout),
        };
        let Some(timeout) = timeout else { return Poll::Pending };
        let deadline = since + timeout;
        let sleep = self.sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => {
                self.state.no_linger.store(true, Ordering::SeqCst);
                Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "connection idle timeout")))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl TimedIo {
    /// Tells whether `done` holds; if not, registers the connection task to be woken when a request
    /// finishes and looks again, so that a request finishing in between is not missed.
    fn wait_for_requests(&mut self, cx: &mut Context<'_>, done: impl Fn(&ConnState) -> bool) -> bool {
        if done(&self.state) {
            return true;
        }
        *self.state.waker.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
        done(&self.state)
    }

    /// `poll_read` with the framing check: bytes reach hyper only when the check let them through.
    fn poll_read_checked(&mut self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        // A read into an empty buffer cannot tell end of input from "no room": do not read.
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            if self.queue_pos < self.queue.len() {
                let n = buf.remaining().min(self.queue.len() - self.queue_pos);
                buf.put_slice(&self.queue[self.queue_pos..self.queue_pos + n]);
                self.queue_pos += n;
                if self.queue_pos == self.queue.len() {
                    self.queue.clear();
                    self.queue_pos = 0;
                    if self.queue.capacity() > 64 * 1024 {
                        self.queue = Vec::new();
                    }
                }
                return Poll::Ready(Ok(()));
            }
            let state = self.state.clone();
            let sock_eof = self.sock_eof;
            let fr = self.framing.as_mut().expect("the framing check is on");
            if fr.is_holding() {
                // After an `Upgrade` request the bytes may not be HTTP: wait for its answer.
                let need = fr.heads();
                // `finished` first, then `upgraded`: a 101 sets `upgraded` before its request finishes, so a request
                // seen as finished here has its `upgraded` flag visible below and cannot be mistaken for a non-101.
                let finished = state.finished.load(Ordering::SeqCst) >= need;
                let released = state.upgraded.load(Ordering::SeqCst);
                if released || finished {
                    if released {
                        fr.release(&mut self.queue);
                    } else {
                        fr.resume(&mut self.queue);
                    }
                    if sock_eof {
                        fr.eof(&mut self.queue);
                    }
                    continue;
                }
                let full = fr.hold_full();
                if self.wait_for_requests(cx, |s| s.upgraded.load(Ordering::SeqCst) || s.finished.load(Ordering::SeqCst) >= need) {
                    continue;
                }
                if full || sock_eof {
                    return self.poll_idle(cx);
                }
            } else if let Some(t) = fr.pending_terminal() {
                if state.finished.load(Ordering::SeqCst) >= t.wait {
                    fr.send_terminal(&mut self.queue);
                    continue;
                }
                if self.wait_for_requests(cx, |s| s.finished.load(Ordering::SeqCst) >= t.wait) {
                    continue;
                }
                return self.poll_idle(cx);
            } else if fr.terminal_sent() || sock_eof {
                return Poll::Ready(Ok(()));
            }
            let before = buf.filled().len();
            match self.sock().poll_read(cx, buf) {
                Poll::Pending => return self.poll_idle(cx),
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => {
                    let n = buf.filled().len() - before;
                    if n == 0 {
                        self.sock_eof = true;
                        if let Some(fr) = self.framing.as_mut() {
                            fr.eof(&mut self.queue);
                        }
                        continue;
                    }
                    self.on_read(&buf.filled()[before..])?;
                    let fr = self.framing.as_mut().expect("the framing check is on");
                    let region = &buf.filled()[before..];
                    let k = fr.passthrough_prefix(region);
                    if k < n {
                        fr.feed(&region[k..], &mut self.queue);
                    }
                    buf.set_filled(before + k);
                    if k > 0 {
                        return Poll::Ready(Ok(()));
                    }
                }
            }
        }
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.framing.is_some() {
            return this.poll_read_checked(cx, buf);
        }
        let before = buf.filled().len();
        match this.sock().poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if buf.filled().len() > before {
                    this.on_read(&buf.filled()[before..])?;
                }
                Poll::Ready(Ok(()))
            }
            Poll::Pending => this.poll_idle(cx),
            other => other,
        }
    }
}

impl AsyncWrite for TimedIo {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let r = this.sock().poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = r
            && n > 0
        {
            this.sync();
            // A response still being flushed is progress, not idleness.
            if let Phase::KeepAlive(_) = this.phase {
                this.phase = Phase::KeepAlive(Instant::now());
            }
        }
        r
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sock().poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sock().poll_shutdown(cx)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        let r = this.sock().poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(n)) = r
            && n > 0
        {
            this.sync();
            if let Phase::KeepAlive(_) = this.phase {
                this.phase = Phase::KeepAlive(Instant::now());
            }
        }
        r
    }
    fn is_write_vectored(&self) -> bool {
        self.io.as_ref().is_some_and(|io| io.is_write_vectored())
    }
}

/// Accepts connections until `shutdown` resolves, then drains them gracefully.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<AppShared>,
    options: ServerOptions,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> std::io::Result<()> {
    let options = Arc::new(options);
    let graceful = hyper_util::server::graceful::GracefulShutdown::new();
    let mut builder = auto::Builder::new(TokioExecutor::new());
    // The framing check reads header lines the way httparse does by default: `ignore_invalid_headers` would let
    // httparse skip lines the check has read (design.md, decision 4, of `e0-reject-ambiguous-http1-framing`).
    debug_assert!(options.insecure_http_parser || !IGNORE_INVALID_HEADERS);
    builder
        .http1()
        .ignore_invalid_headers(IGNORE_INVALID_HEADERS)
        .timer(TokioTimer::new())
        .header_read_timeout(options.headers_timeout)
        .max_buf_size(options.max_header_size.max(8 * 1024));
    builder.http2().timer(TokioTimer::new());
    let permits = options.max_connections.map(|n| Arc::new(tokio::sync::Semaphore::new(n.max(1))));
    tokio::pin!(shutdown);
    loop {
        let permit = match &permits {
            Some(p) => tokio::select! {
                permit = p.clone().acquire_owned() => permit.ok(),
                _ = &mut shutdown => break,
            },
            None => None,
        };
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, remote) = match accepted {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let _ = stream.set_nodelay(true);
                let state = Arc::new(ConnState::default());
                let conn_state = state.clone();
                let io = TimedIo::new(stream, permit, state.clone(), &options);
                let shared = shared.clone();
                let opts = options.clone();
                // Compressions in progress of this connection (one set for all its HTTP/2 streams).
                #[cfg(feature = "compression")]
                let slots = crate::http::compression::ConnectionSlots::default();
                let service = hyper::service::service_fn(move |req: hyper::Request<Incoming>| {
                    #[cfg(feature = "compression")]
                    let req = {
                        let mut req = req;
                        req.extensions_mut().insert(slots.clone());
                        req
                    };
                    let guard = InflightGuard::new(state.clone());
                    let shared = shared.clone();
                    let opts = opts.clone();
                    async move {
                        let resp = match handle(shared, req, remote, &opts).await {
                            Ok(r) => r,
                            Err(e) => {
                                guard.0.destroyed.store(true, Ordering::SeqCst);
                                return Err(e);
                            }
                        };
                        if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
                            guard.0.upgraded.store(true, Ordering::SeqCst);
                        }
                        Ok::<_, crate::http_adapter::SocketDestroyed>(
                            resp.map(|inner| GuardedBody { inner, _guard: guard }.boxed_unsync()),
                        )
                    }
                });
                let conn = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
                let conn = graceful.watch(conn.into_owned());
                tokio::spawn(finish_connection(conn_state, conn));
            }
            _ = &mut shutdown => break,
        }
    }
    drop(listener);
    tokio::select! {
        _ = graceful.shutdown() => {}
        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    #[test]
    fn read_limit_follows_the_parser_that_accepts_the_body() {
        let p = crate::http::body_parser::BodyParserOptions::default();
        let t = 1024 * 1024;
        assert_eq!(read_limit(&headers(&[("content-type", "application/json")]), t, &p), p.json_limit);
        assert_eq!(read_limit(&headers(&[("content-type", "Application/JSON; charset=utf-8")]), t, &p), p.json_limit);
        assert_eq!(read_limit(&headers(&[("content-type", "application/x-www-form-urlencoded")]), t, &p), p.urlencoded_limit);
        // 415 cases keep the transport limit (the parser answers them whatever the size).
        assert_eq!(read_limit(&headers(&[("content-type", "application/json; charset=latin1")]), t, &p), t);
        assert_eq!(read_limit(&headers(&[("content-type", "application/json"), ("content-encoding", "gzip")]), t, &p), t);
        // Other types: raw body up to the transport limit.
        assert_eq!(read_limit(&headers(&[("content-type", "text/plain")]), t, &p), t);
        assert_eq!(read_limit(&headers(&[]), t, &p), t);
        // A transport limit below the parser's wins.
        assert_eq!(read_limit(&headers(&[("content-type", "application/json")]), 10, &p), 10);
    }
}

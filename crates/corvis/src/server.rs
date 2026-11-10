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
#[cfg(feature = "tls")]
pub mod tls;
mod transport;
#[cfg(all(test, feature = "tls"))]
mod transport_tests;

use transport::ServerIo;

pub(crate) type ResBody = UnsyncBoxBody<Bytes, std::io::Error>;

/// Options of the HTTP server: timeouts, connection and header limits, opt-in h2c.
///
/// The defaults follow Node's `http.Server` (`headersTimeout` 60 s, `requestTimeout` 300 s,
/// `keepAliveTimeout` 5 s) and protect against slow clients (slowloris) that would otherwise keep
/// connections and buffered bodies alive forever. [`send_timeout`](ServerOptions::send_timeout) (default 60 s)
/// is not one of Node's defaults: Node's `server.timeout` is off, so a client that stops reading its response
/// keeps its connection there; here it is closed.
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
    pub(crate) send_timeout: Option<Duration>,
    pub(crate) max_connections: Option<usize>,
    pub(crate) max_header_size: usize,
    pub(crate) h2c: bool,
    pub(crate) insecure_http_parser: bool,
    pub(crate) http2_max_concurrent_streams: u32,
    pub(crate) http2_keep_alive_interval: Option<Duration>,
    pub(crate) http2_keep_alive_timeout: Duration,
    pub(crate) http2_max_cancelled_streams: u32,
    pub(crate) max_pending_response_bytes: Option<usize>,
    pub(crate) max_pending_response_wait: Option<Duration>,
}

/// Default of [`ServerOptions::max_pending_response_bytes`]: 8 MiB per connection.
const DEFAULT_MAX_PENDING_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

impl Default for ServerOptions {
    fn default() -> Self {
        ServerOptions {
            headers_timeout: Some(Duration::from_secs(60)),
            request_timeout: Some(Duration::from_secs(300)),
            keep_alive_timeout: Some(Duration::from_secs(5)),
            send_timeout: Some(Duration::from_secs(60)),
            max_connections: None,
            max_header_size: 32 * 1024,
            h2c: false,
            insecure_http_parser: false,
            http2_max_concurrent_streams: 100,
            http2_keep_alive_interval: None,
            http2_keep_alive_timeout: Duration::from_secs(20),
            http2_max_cancelled_streams: 2000,
            max_pending_response_bytes: Some(DEFAULT_MAX_PENDING_RESPONSE_BYTES),
            max_pending_response_wait: Some(Duration::from_secs(5)),
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
    /// Longest the server waits for a client to accept response bytes (default 60 s, like nginx's `send_timeout`).
    /// A connection whose client accepts no byte of a response for that long is closed, over HTTP/1 and HTTP/2:
    /// either the socket stays blocked (the client does not read), or an HTTP/2 stream waits for flow-control
    /// window the client never grants, even while PING, SETTINGS or WINDOW_UPDATE frames keep flowing. The
    /// connection is closed when no response body of it has made progress for `send_timeout`: a stalled stream
    /// next to streams that keep moving does not cost the connection its other streams. The connection is dropped
    /// without the closing drain and without a GOAWAY. `None` disables it, and so does a value too long to add to
    /// an `Instant` (`Duration::MAX`).
    ///
    /// It measures lack of progress, not total time: progress restarts it, so slow but live downloads are not cut;
    /// it never runs while the server has nothing to send (slow handlers, idle Server-Sent Events streams, idle
    /// keep-alive connections).
    ///
    /// It also applies to upgraded connections (WebSockets, any `upgrade` listener), writes only: a write that the
    /// socket accepts no byte of for `send_timeout` fails and the connection is destroyed without a close frame
    /// (a gateway or `WebSocket` sees close code 1006). Bytes the peer accepts of any frame restart it; frames
    /// received never do. The stalled-body watchdog and the idle timers stay off for upgraded connections.
    ///
    /// What it does not bound: how little a client may read to keep a connection. A client needs to take about
    /// 16 KiB (one HTTP/2 frame) from some stream each period over HTTP/2. Over HTTP/1 the server notices progress
    /// when the operating system reports the socket writable: on Linux the server sets `TCP_NOTSENT_LOWAT` to
    /// 64 KiB on its sockets while this timeout is on, so a client that reads steadily is seen when it takes about
    /// 32 KiB per period (the kernel reports the socket writable at half the mark); on Windows about 64 KiB per period were needed on a loopback connection; on other platforms (macOS,
    /// the BSDs) it depends on the autotuned send buffer, which can be several MiB. A client that resets and
    /// reopens HTTP/2 streams needs to read nothing at all. The response memory such clients hold is bounded by
    /// [`max_pending_response_bytes`](Self::max_pending_response_bytes), not by this.
    ///
    /// Node's `server.timeout` (default 0, off) measures inactivity in both directions and also cuts slow
    /// handlers, which is why it is off there; with Node's defaults a client that stops reading is never
    /// disconnected. This default is a deliberate divergence.
    pub fn send_timeout(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.send_timeout = t.into();
        self
    }
    /// Most response bytes one connection may hold for a client that does not read them (default 8 MiB per
    /// connection, `None` turns the limit off). Without it a client could keep `http2_max_concurrent_streams`
    /// large responses in memory by never granting flow-control window, or by resetting and reopening streams.
    ///
    /// What counts: the whole size of a response body held in memory (JSON, text, bytes; also when compression
    /// wraps it) from the moment its handler returns until the body ends or is dropped, and, for streamed bodies
    /// (files, Server-Sent Events), the part of the current data frame not yet handed to the protocol layer.
    ///
    /// Before a request's handler runs the server reserves a share of the limit for it: the limit divided by the
    /// requests the connection may run at once ([`http2_max_concurrent_streams`](Self::http2_max_concurrent_streams)
    /// over HTTP/2, 1 over HTTP/1, so pipelined HTTP/1 requests run one handler at a time). The handler starts only
    /// when the pending bytes plus the reservations leave room, or when the connection holds nothing; until then the
    /// request waits without reaching a handler, and a client that resets its stream meanwhile counts as having
    /// cancelled a request (see [`h2c`](Self::h2c)).
    ///
    /// Over HTTP/2, when a handler returns a response that would take a connection that already holds pending
    /// bytes above the limit, the response is dropped and its stream is reset with `ENHANCE_YOUR_CALM`; the first
    /// reset of a connection is logged. A response that fits its share is never reset, and one response is always
    /// sent whatever its size: a connection holds at most the larger of the limit and its largest response. Large
    /// payloads belong in streamed bodies (static files are).
    ///
    /// A request that waits for room longer than [`max_pending_response_wait`](Self::max_pending_response_wait) is refused.
    /// The reset also reaches a client that reads at full speed when many large responses finish together (a burst
    /// of handlers that return within the same instants): size the limit for the bytes you expect to be in flight, that
    /// is at least `http2_max_concurrent_streams` times your largest common response, or lower the stream cap.
    ///
    /// Not counted: what hyper keeps of a finished HTTP/2 body (at most about 64 KiB per open stream), compression
    /// encoders (see the compression options), request heads and bodies, and memory the application holds outside
    /// the response (an unbounded channel feeding a stream, a cache). Server-wide the bound is
    /// [`max_connections`](Self::max_connections) times this limit.
    ///
    /// Node's `http2` module has a similar credit-based `maxSessionMemory` (10 MB) that refuses new streams
    /// while exceeded; Node's `http` module has no limit.
    pub fn max_pending_response_bytes(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_pending_response_bytes = n.into();
        self
    }
    /// Longest a request waits for room in the connection's response budget before its handler runs (default 5 s,
    /// `None` waits without limit). It only matters with [`max_pending_response_bytes`](Self::max_pending_response_bytes)
    /// on. A request waits while the responses the connection holds (an in-memory response is counted whole until it
    /// is sent) leave no room, so a client that reads one very large response slowly delays its other streams for as
    /// long as that response takes: behind a proxy that multiplexes several users on one HTTP/2 connection (Envoy,
    /// HAProxy), one slow download could otherwise hold the requests of the others for ever.
    ///
    /// It applies to HTTP/2 only. When the time is up the stream is reset with `REFUSED_STREAM`: the handler never ran, so
    /// the client (or the proxy) may retry the request on another connection, as RFC 9113 section 8.7 allows. It does not
    /// count as a cancelled request for the rapid-reset rule. The first expiry on a connection is logged. With `None`, a
    /// waiting request has no bound other than [`send_timeout`](Self::send_timeout) closing a connection whose client
    /// reads nothing: a client that reads a trickle every period keeps its waiting streams waiting as long as it likes.
    ///
    /// Over HTTP/1 the wait is never cut: it only delays the requests that the same client pipelined behind a response it
    /// has not finished reading, and it ends as the client reads or when `send_timeout` closes the connection; closing it
    /// sooner would truncate a legitimate slow download.
    pub fn max_pending_response_wait(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.max_pending_response_wait = t.into();
        self
    }
    /// Maximum number of open connections (`server.maxConnections`); when reached, new
    /// connections wait in the accept queue. Default: unlimited. It counts sockets, not requests:
    /// an HTTP/2 connection is one connection however many streams it carries, so the requests in
    /// flight from HTTP/2 clients are bounded by `max_connections` x
    /// [`http2_max_concurrent_streams`](Self::http2_max_concurrent_streams). Upgraded (WebSocket)
    /// connections count until they close, and so do sockets in their short closing drain
    /// (up to 2 s after an early error response); with a cap, give gateways a heartbeat so
    /// dead WebSocket clients on idle connections do not hold slots (clients that stop reading while the server
    /// sends are dropped by [`send_timeout`](Self::send_timeout)).
    pub fn max_connections(mut self, n: impl Into<Option<usize>>) -> Self {
        self.max_connections = n.into();
        self
    }
    /// Maximum size of the request head in bytes (default 32 KiB, minimum 8 KiB), for HTTP/1 and
    /// HTTP/2. Bigger heads are answered with `431`. HTTP/1 counts the raw head; HTTP/2 counts the
    /// decoded header list as RFC 9113 does (name and value lengths plus 32 bytes per field), so the
    /// same number is comparable, not byte-identical.
    pub fn max_header_size(mut self, bytes: usize) -> Self {
        self.max_header_size = bytes.max(8 * 1024);
        self
    }
    /// HTTP/2 over cleartext with prior knowledge (h2c), for a proxy or platform configured to
    /// send it. Disabled by default, like Node's `http` module, which accepts only HTTP/1.x; a
    /// prior-knowledge connection is closed without a response. With it on, one connection carries
    /// several concurrent requests (see
    /// [`http2_max_concurrent_streams`](Self::http2_max_concurrent_streams), default 100),
    /// `max_header_size` bounds the header list, and
    /// [`http2_keep_alive_interval`](Self::http2_keep_alive_interval) can detect peers that stopped answering.
    ///
    /// Behind a proxy that multiplexes many users on a few connections: clients abort requests all the time, and
    /// the server cannot tell a proxy from an attacker by one reset. A connection is closed only when the requests
    /// it cancelled before their response (a reset stream, also while its body was being read) exceed 500 and
    /// exceed a quarter of the requests it completed, over the last 10 s (the current window plus the previous one,
    /// weighted), or exceed [`http2_max_cancelled_streams`](Self::http2_max_cancelled_streams) (default 2000) whatever
    /// it completed. A proxy tolerates about 20 percent of aborts, up to 200 per second sustained; an attacker pays
    /// four completed requests per reset and is capped by the ceiling. The server first
    /// sends a graceful GOAWAY (so the proxy can retry what was not answered on another connection) and drops the
    /// connection after 1 s.
    pub fn h2c(mut self, enabled: bool) -> Self {
        self.h2c = enabled;
        self
    }
    /// Streams a client may have open at once on one HTTP/2 connection
    /// (`SETTINGS_MAX_CONCURRENT_STREAMS`, default 100, minimum 1). The server never runs more
    /// handlers at once for one connection than this, even when a client resets streams and opens new ones
    /// faster than the handlers stop. A connection that cancels too many requests before their response
    /// over about 10 s is told to go away (GOAWAY, then closed after 1 s); see
    /// [`h2c`](Self::h2c). Together with
    /// [`max_connections`](Self::max_connections) it bounds the in-flight requests of HTTP/2 clients.
    pub fn http2_max_concurrent_streams(mut self, n: u32) -> Self {
        self.http2_max_concurrent_streams = n.max(1);
        self
    }
    /// Interval between HTTP/2 PING frames on each connection, whether or not streams are open.
    /// Default `None`: no PINGs; a zero interval also means none. Set it with SSE or other long-lived
    /// HTTP/2 streams, where a peer that stopped answering (a vanished client or middlebox) would
    /// otherwise never be noticed; see [`http2_keep_alive_timeout`](Self::http2_keep_alive_timeout).
    ///
    /// A live peer that answers PINGs but stops reading is handled by
    /// [`send_timeout`](Self::send_timeout), not by this.
    pub fn http2_keep_alive_interval(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.http2_keep_alive_interval = t.into().filter(|d| !d.is_zero());
        self
    }
    /// Time an HTTP/2 peer has to acknowledge a PING before the connection is closed (default
    /// 20 s, minimum 1 s). Only used when [`http2_keep_alive_interval`](Self::http2_keep_alive_interval)
    /// is set. It does not fire while writing to the client is blocked; [`send_timeout`](Self::send_timeout) covers that.
    pub fn http2_keep_alive_timeout(mut self, t: Duration) -> Self {
        self.http2_keep_alive_timeout = t.max(Duration::from_secs(1));
        self
    }
    /// Cancelled requests over about 10 s that close an HTTP/2 connection whatever it completed (default 2000,
    /// 200 per second sustained, minimum 1). A request counts as cancelled when its stream is reset before the
    /// response (also while its body is read). See [`h2c`](Self::h2c) for the rule that applies below this ceiling.
    pub fn http2_max_cancelled_streams(mut self, n: u32) -> Self {
        self.http2_max_cancelled_streams = n.max(1);
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
    // A `Content-Length` that disagrees with a body of known size would make the peer read the extra bytes as a
    // second response: the real length wins. (HEAD keeps the length it was given, as Node does.)
    if !head
        && let Some(declared) = headers.get(http::header::CONTENT_LENGTH)
        && let Some(len) = hyper::body::Body::size_hint(&body).exact()
        && declared.to_str().ok().and_then(|d| d.trim().parse::<u64>().ok()) != Some(len)
    {
        headers.insert(http::header::CONTENT_LENGTH, http::HeaderValue::from(len));
    }
    if let Some(h) = builder.headers_mut() {
        *h = headers;
    }
    let body = if head { full(Bytes::new()) } else { body };
    builder.body(body).unwrap_or_else(|_| hyper::Response::new(full(Bytes::new())))
}

/// Length of the in-memory body of a response before compression (response extension).
#[derive(Clone, Copy)]
pub(crate) struct PlainLen(pub u64);

/// Finishes a handler's response: the Express defaults, the conversion to the wire response and,
/// when the application enabled it, compression. The server and the test client both end here.
pub(crate) fn finish_response(
    mut resp: Response,
    info: &crate::http::express::RequestInfo,
    settings: &crate::http::HttpSettings,
    head: bool,
) -> hyper::Response<ResBody> {
    crate::http::express::finalize(&mut resp, info, settings);
    #[allow(unused_mut)]
    let mut wire = into_hyper(resp, head);
    // Compression hides the size of an in-memory body: the server still holds it until the body ends.
    if let Some(len) = hyper::body::Body::size_hint(wire.body()).exact() {
        wire.extensions_mut().insert(PlainLen(len));
    }
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
    let deadline = request_timeout.and_then(|t| Instant::now().checked_add(t));
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
    let mut client_aborted = false;
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
            Some(Err(_)) => {
                client_aborted = true;
                (Bytes::new(), Some(request_aborted_error()))
            }
            None => (Bytes::new(), Some(request_timeout_error())),
        }
    };
    // Body-parser errors stay in the request: the router answers them after CORS and through
    // the global filters (Express `next(err)`).
    let mut r = Request::with_parser(parts.method, parts.uri, parts.headers, bytes, parser);
    if let Some(e) = error {
        r.body_error = Some(e);
    }
    r.client_aborted = client_aborted;
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
pub(crate) async fn drain(pending: PendingBody, cap: u64) {
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
    client_aborted: &mut bool,
) -> Result<hyper::Response<ResBody>, crate::http_adapter::SocketDestroyed> {
    // An attached Engine.IO server takes every request under its path first, upgrades included.
    #[cfg(feature = "socket-io")]
    {
        if let Some(engine) = shared.http_server.engine_for(req.uri().path()) {
            return Ok(engine.handle(req, remote, &shared.http_settings.body_parser, options.request_timeout).await);
        }
    }
    #[cfg(feature = "websockets")]
    {
        if let Some(gw) = crate::websockets::match_upgrade(&shared, req.headers(), req.uri().path()) {
            return Ok(crate::websockets::upgrade(gw, req, remote, &shared.http_settings.body_parser));
        }
    }
    // `server.on('upgrade', ...)` listeners take every upgrade request (Node semantics).
    if crate::http_adapter::is_upgrade_request(&req) {
        if shared.http_server.upgrade_listener_count() > 0 {
            return crate::http_adapter::dispatch_upgrade(&shared.http_server, req, remote, &shared.http_settings.body_parser).await;
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
        // An HTTP/2 body that fails to read means the client reset the stream or the connection broke: nobody waits
        // for the answer, so it is not dispatched (and not logged as an error) and counts as a cancelled request.
        Ok(r) if r.client_aborted && r.version == http::Version::HTTP_2 => {
            *client_aborted = true;
            Response::json(request_aborted_error().get_response().clone()).with_status(400)
        }
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
    closing: Mutex<Option<(ServerIo, Option<OwnedSemaphorePermit>)>>,
    /// Task waiting on the IO, woken when the last in-flight request ends.
    waker: Mutex<Option<Waker>>,
    /// Requests cancelled and completed in the current and the previous window (rapid reset detection).
    window: Mutex<Option<CancelWindow>>,
    /// Signalled when too many requests were cancelled before their response (rapid reset): the
    /// connection task closes the connection.
    reset_flood: tokio::sync::Notify,
    /// Set when the flood was detected: no further request of the connection reaches a handler.
    flooded: AtomicBool,
    /// `http2_max_cancelled_streams`: cancellations over the window that close the connection whatever it completed.
    cancel_ceiling: u32,
    /// Origin of the times the response-body clocks record.
    epoch: Epoch,
    /// Clocks of the response bodies of the connection that are alive (send timeout watchdog).
    clocks: Mutex<Vec<Arc<BodyClock>>>,
    /// When hyper last took a frame from any response body of the connection (milliseconds since `epoch` plus
    /// one; `0` = never): the connection's progress.
    progress: AtomicU64,
    /// Signalled when a clock is registered, so a watchdog that had nothing to watch looks again.
    clocks_changed: tokio::sync::Notify,
    /// Response bytes the connection holds (`max_pending_response_bytes`).
    budget: Mutex<Budget>,
    /// Signalled whenever pending bytes or running handlers drop, so waiting requests look again.
    budget_freed: tokio::sync::Notify,
    /// HTTP/1: charges of bodies hyper already took into its write buffer, released when hyper flushes the socket
    /// (it only does so with its write buffer empty).
    deferred: AtomicUsize,
}

/// What a connection holds of the responses of its handlers (design of `e0-cap-pending-response-bytes`).
#[derive(Default)]
struct Budget {
    /// Bytes of response bodies that have neither ended nor been dropped.
    pending: usize,
    /// Handlers admitted that have not returned yet, each with a reserved share.
    running: usize,
    /// The reset for exceeding the cap was logged already.
    warned: bool,
    /// The refusal of a request that waited too long was logged already.
    warned_wait: bool,
    /// Requests waiting for room.
    waiting: usize,
    /// Share and cap of the last admission attempt (all requests of a connection use the same ones).
    share: usize,
    cap: usize,
}

/// What to do with the response a handler returned.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Keep,
    Reset,
}

impl Budget {
    /// Reserves `share` for a handler when `pending + reservations + share` fits `cap`, or when the connection holds
    /// nothing at all.
    fn try_admit(&mut self, share: usize, cap: usize) -> bool {
        (self.share, self.cap) = (share, cap);
        let idle = self.pending == 0 && self.running == 0;
        let fits = self.pending.saturating_add(self.running.saturating_add(1).saturating_mul(share)) <= cap;
        if idle || fits {
            self.running += 1;
        }
        idle || fits
    }

    /// A handler returned a response charging `charge`: its reservation ends and the response is kept (and charged)
    /// unless, over HTTP/2, the connection already holds bytes and the response would take it above `cap`.
    fn finish_handler(&mut self, charge: usize, cap: usize, http2: bool) -> Verdict {
        self.running = self.running.saturating_sub(1);
        if http2 && self.pending > 0 && self.pending.saturating_add(charge) > cap {
            return Verdict::Reset;
        }
        self.pending = self.pending.saturating_add(charge);
        Verdict::Keep
    }

    fn release(&mut self, charge: usize) {
        self.pending = self.pending.saturating_sub(charge);
    }

    /// A request is waiting and would be admitted now: only then is waking the waiters worth the work.
    fn should_wake(&self) -> bool {
        let idle = self.pending == 0 && self.running == 0;
        self.waiting > 0 && (idle || self.pending.saturating_add(self.running.saturating_add(1).saturating_mul(self.share)) <= self.cap)
    }
}

impl ConnState {
    /// HTTP/1: hyper took a body into its write buffer; the memory is held until the buffer is flushed.
    fn defer_release(&self, charge: usize) {
        self.deferred.fetch_add(charge, Ordering::Relaxed);
    }

    /// Hyper flushed the socket, which it only does with its write buffer empty: what it took is out.
    fn flush_deferred(&self) {
        let charge = self.deferred.swap(0, Ordering::Relaxed);
        self.release_budget(charge);
    }

    fn release_budget(&self, charge: usize) {
        if charge > 0 {
            let wake = {
                let mut b = self.budget.lock().unwrap_or_else(|e| e.into_inner());
                b.release(charge);
                b.should_wake()
            };
            if wake {
                self.budget_freed.notify_waiters();
            }
        }
    }

    /// Wakes the waiting requests when one of them could be admitted.
    fn wake_if_room(&self) {
        if self.budget.lock().unwrap_or_else(|e| e.into_inner()).should_wake() {
            self.budget_freed.notify_waiters();
        }
    }

    /// Waits until the budget has room for a handler with this `share`; the reservation lasts until the handler
    /// returns or the returned value is dropped.
    async fn admit(self: &Arc<Self>, share: usize, cap: usize) -> Admission {
        let mut waiting: Option<Waiting> = None;
        loop {
            let notified = self.budget_freed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut b = self.budget.lock().unwrap_or_else(|e| e.into_inner());
                if b.try_admit(share, cap) {
                    return Admission { state: Some(self.clone()) };
                }
                if waiting.is_none() {
                    b.waiting += 1;
                    waiting = Some(Waiting(self.clone()));
                }
            }
            notified.await;
        }
    }

    /// Logs, once per connection, that a request waited too long for room.
    fn warn_wait_once(&self, wait: Duration) {
        let first = !std::mem::replace(&mut self.budget.lock().unwrap_or_else(|e| e.into_inner()).warned_wait, true);
        if first {
            crate::logger::Logger::warn_with(
                format!(
                    "A request waited more than {wait:?} for room in the connection's response budget \
                     (ServerOptions::max_pending_response_bytes) and its HTTP/2 stream was refused (REFUSED_STREAM); its handler never ran. Raise \
                     ServerOptions::max_pending_response_wait or the byte limit if clients read large responses slowly. \
                     Further refusals on this connection are not logged."
                ),
                "HttpServer",
            );
        }
    }
}

/// A request counted as waiting for room, until it is admitted or gives up.
struct Waiting(Arc<ConnState>);

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut b = self.0.budget.lock().unwrap_or_else(|e| e.into_inner());
        b.waiting = b.waiting.saturating_sub(1);
    }
}

/// A reserved share of the budget for a running handler.
struct Admission {
    state: Option<Arc<ConnState>>,
}

impl Admission {
    /// The handler returned: end the reservation and decide about its response.
    fn finish(mut self, charge: usize, cap: usize, http2: bool) -> Verdict {
        let state = self.state.take().expect("an admission finishes once");
        let (verdict, first) = {
            let mut b = state.budget.lock().unwrap_or_else(|e| e.into_inner());
            let verdict = b.finish_handler(charge, cap, http2);
            let first = verdict == Verdict::Reset && !std::mem::replace(&mut b.warned, true);
            (verdict, first)
        };
        state.wake_if_room();
        if first {
            crate::logger::Logger::warn_with(
                format!(
                    "A response was dropped and its HTTP/2 stream reset (ENHANCE_YOUR_CALM): the connection holds more than \
                     {cap} bytes of pending responses (ServerOptions::max_pending_response_bytes). Raise the option or stream \
                     large bodies. Further resets on this connection are not logged."
                ),
                "HttpServer",
            );
        }
        verdict
    }
}

impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let mut b = state.budget.lock().unwrap_or_else(|e| e.into_inner());
            b.running = b.running.saturating_sub(1);
            drop(b);
            state.wake_if_room();
        }
    }
}

struct Epoch(Instant);

impl Default for Epoch {
    fn default() -> Self {
        Epoch(Instant::now())
    }
}

/// When a response body started waiting for hyper to take its next frame, to tell a stalled client from an
/// application that has nothing to send. `0` means the body is not waiting for hyper (its last poll was pending,
/// or it ended); otherwise milliseconds since the connection's epoch plus one.
#[derive(Default)]
struct BodyClock {
    waiting_since: AtomicU64,
}

impl ConnState {
    fn note_progress(&self) {
        let ms = Instant::now().saturating_duration_since(self.epoch.0).as_millis() as u64;
        self.progress.store(ms + 1, Ordering::Relaxed);
    }
    fn last_progress(&self) -> Option<Instant> {
        match self.progress.load(Ordering::Relaxed) {
            0 => None,
            v => Some(self.epoch.0 + Duration::from_millis(v - 1)),
        }
    }
}

impl BodyClock {
    fn wait_for_hyper(&self, epoch: &Epoch) {
        let ms = Instant::now().saturating_duration_since(epoch.0).as_millis() as u64;
        self.waiting_since.store(ms + 1, Ordering::Relaxed);
    }
    fn wait_for_application(&self) {
        self.waiting_since.store(0, Ordering::Relaxed);
    }
    fn since(&self, epoch: &Epoch) -> Option<Instant> {
        match self.waiting_since.load(Ordering::Relaxed) {
            0 => None,
            v => Some(epoch.0 + Duration::from_millis(v - 1)),
        }
    }
}

/// Rapid reset (CVE-2023-44487) detection. A connection is closed when the requests it cancelled before their
/// response, over the last [`CANCEL_WINDOW`] (the current window plus the previous one weighted by how much of it
/// still falls inside), exceed `max(`[`CANCEL_FLOOR`]`, completed / `[`COMPLETED_CREDIT`]`)` (completed requests over the
/// same span count for a quarter, so a proxy tolerates about 20 percent of aborts while an attacker pays four
/// completed requests per reset), or exceed `http2_max_cancelled_streams` whatever was completed.
const CANCEL_FLOOR: f64 = 500.0;
const COMPLETED_CREDIT: f64 = 4.0;
const CANCEL_WINDOW: Duration = Duration::from_secs(10);
/// After the GOAWAY of a flood, the connection is dropped when its streams have not ended by then.
const FLOOD_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct CancelWindow {
    start: Instant,
    cancelled: u32,
    completed: u32,
    prev_cancelled: u32,
    prev_completed: u32,
}

impl ConnState {
    /// Adds one request outcome at `now`; true when the connection has to be closed.
    fn tally(&self, now: Instant, cancelled: bool) -> bool {
        let mut guard = self.window.lock().unwrap_or_else(|e| e.into_inner());
        let w = guard.get_or_insert(CancelWindow { start: now, cancelled: 0, completed: 0, prev_cancelled: 0, prev_completed: 0 });
        let elapsed = now.saturating_duration_since(w.start);
        if elapsed >= CANCEL_WINDOW * 2 {
            *w = CancelWindow { start: now, cancelled: 0, completed: 0, prev_cancelled: 0, prev_completed: 0 };
        } else if elapsed >= CANCEL_WINDOW {
            w.prev_cancelled = w.cancelled;
            w.prev_completed = w.completed;
            w.cancelled = 0;
            w.completed = 0;
            w.start += CANCEL_WINDOW;
        }
        if cancelled {
            w.cancelled = w.cancelled.saturating_add(1);
        } else {
            w.completed = w.completed.saturating_add(1);
        }
        let inside = 1.0 - (now.saturating_duration_since(w.start).as_secs_f64() / CANCEL_WINDOW.as_secs_f64()).min(1.0);
        let cancelled_now = f64::from(w.cancelled) + f64::from(w.prev_cancelled) * inside;
        let completed_now = f64::from(w.completed) + f64::from(w.prev_completed) * inside;
        cancelled && (cancelled_now > CANCEL_FLOOR.max(completed_now / COMPLETED_CREDIT) || cancelled_now > f64::from(self.cancel_ceiling))
    }

    fn record(&self, cancelled: bool) {
        if self.tally(Instant::now(), cancelled) {
            self.flooded.store(true, Ordering::SeqCst);
            self.reset_flood.notify_one();
        }
    }
}

enum Outcome {
    /// The service future was dropped before producing a response.
    Pending,
    Completed,
    /// Refused without running because the connection was flooded: neither cancelled nor completed.
    Refused,
    /// The client reset the stream while its body was being read: nobody waits for the response.
    ClientAborted,
    /// The server dropped the response for exceeding `max_pending_response_bytes`: nothing was sent.
    Dropped,
    /// The response body records the outcome: completed when it ends, cancelled when it is dropped before.
    Body,
}

/// Counts a request as cancelled when its handler future is dropped before producing a response, or when the
/// client reset the stream while its body was read; a response body counts it itself (see [`GuardedBody`]).
struct CancelWatch {
    state: Arc<ConnState>,
    outcome: Outcome,
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        if !matches!(self.outcome, Outcome::Refused | Outcome::Body) {
            self.state.record(!matches!(self.outcome, Outcome::Completed));
        }
    }
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

/// Largest data frame of an HTTP/2 response body (the default `SETTINGS_MAX_FRAME_SIZE`).
const H2_SLICE: usize = 16 * 1024;
/// What hyper may buffer per HTTP/2 stream (`max_send_buf_size`; its default is 400 KiB).
const H2_SEND_BUF: usize = 32 * 1024;
/// h2 can still hold this much of a stream after its body ended: the send buffer, the slice that was handed over
/// when there was room, and the one data frame its codec keeps after taking it out of the stream's queue (it is not
/// counted in the buffer). Slices that may be part of it are copied, so they do not keep the whole response alive.
const H2_TAIL: usize = H2_SEND_BUF + 2 * H2_SLICE;

/// Cuts the first [`H2_SLICE`] bytes off `data` for hyper. A slice that may outlive the body (fewer than
/// [`H2_TAIL`] bytes of the frame remain after it) is copied into an allocation of its own.
fn cut_slice(data: &mut Bytes) -> Bytes {
    let part = data.split_to(H2_SLICE.min(data.len()));
    if data.is_empty() {
        // An empty split still points into the allocation of the frame.
        *data = Bytes::new();
    }
    if data.len() < H2_TAIL { Bytes::copy_from_slice(&part) } else { part }
}

/// A response body that releases its [`InflightGuard`] when hyper drops it, and keeps a [`BodyClock`] so that the
/// watchdog of the connection can tell when the client does not let it move (`send_timeout`).
struct GuardedBody {
    inner: ResBody,
    clock: Arc<BodyClock>,
    /// HTTP/2: data frames are handed to hyper in slices of [`H2_SLICE`], so the body does not finish (and leave the
    /// watchdog's sight) while a window of a few bytes holds back megabytes already given to h2.
    slice: bool,
    /// What is left of a data frame that was cut.
    rest: Bytes,
    /// Hyper has polled the body at least once.
    polled: bool,
    guard: InflightGuard,
    /// Bytes this body adds to the connection's pending response bytes right now (`max_pending_response_bytes`).
    charge: usize,
    /// In-memory body: its whole size stays charged until it ends or is dropped. Otherwise the charge follows `rest`.
    fixed: bool,
    /// `max_pending_response_bytes` is on.
    track: bool,
    /// The body reported its end (or had nothing to send).
    ended: bool,
    /// A data frame was handed to hyper.
    sent_data: bool,
    /// The body records the outcome of its request for the rapid-reset rule: completed when it ended, cancelled when
    /// it is dropped before.
    record: bool,
}

impl GuardedBody {
    #[cfg(test)]
    fn new(inner: ResBody, guard: InflightGuard, http2: bool) -> Self {
        Self::with_charge(inner, guard, http2, None, false)
    }

    fn with_charge(inner: ResBody, guard: InflightGuard, http2: bool, charge: Option<usize>, record: bool) -> Self {
        let clock = Arc::new(BodyClock::default());
        clock.wait_for_hyper(&guard.0.epoch);
        guard.0.clocks.lock().unwrap_or_else(|e| e.into_inner()).push(clock.clone());
        guard.0.clocks_changed.notify_one();
        GuardedBody {
            inner,
            clock,
            slice: http2,
            rest: Bytes::new(),
            polled: false,
            guard,
            charge: charge.unwrap_or(0),
            fixed: charge.is_some_and(|c| c > 0),
            track: charge.is_some(),
            ended: false,
            sent_data: false,
            record,
        }
    }

    /// Follows the part of a cut frame not yet handed to hyper (bodies of unknown size).
    fn charge_rest(&mut self) {
        if !self.track || self.fixed {
            return;
        }
        let now = self.rest.len();
        if now > self.charge {
            self.guard.0.budget.lock().unwrap_or_else(|e| e.into_inner()).pending += now - self.charge;
        } else if now < self.charge {
            self.guard.0.release_budget(self.charge - now);
        }
        self.charge = now;
    }
}

impl Drop for GuardedBody {
    fn drop(&mut self) {
        let ended = self.ended || hyper::body::Body::is_end_stream(&*self);
        let charge = std::mem::take(&mut self.charge);
        if self.slice || !self.track {
            self.guard.0.release_budget(charge);
        } else if charge > 0 {
            // HTTP/1 hyper queued the whole frame: the memory is held until the socket accepts it.
            self.guard.0.defer_release(charge);
        }
        if self.record {
            self.guard.0.record(!ended);
        }
        let mut clocks = self.guard.0.clocks.lock().unwrap_or_else(|e| e.into_inner());
        clocks.retain(|c| !Arc::ptr_eq(c, &self.clock));
    }
}

impl hyper::body::Body for GuardedBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        // Hyper asks for the next frame only once it took the previous one: the connection moves. The first poll is
        // not that: it takes nothing.
        if std::mem::replace(&mut this.polled, true) && this.clock.since(&this.guard.0.epoch).is_some() {
            this.guard.0.note_progress();
        }
        if !this.rest.is_empty() {
            let part = cut_slice(&mut this.rest);
            this.sent_data = true;
            this.charge_rest();
            this.clock.wait_for_hyper(&this.guard.0.epoch);
            return Poll::Ready(Some(Ok(Frame::data(part))));
        }
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Pending => {
                this.clock.wait_for_application();
                Poll::Pending
            }
            Poll::Ready(Some(Ok(frame))) => {
                this.clock.wait_for_hyper(&this.guard.0.epoch);
                this.sent_data |= frame.is_data();
                if this.slice {
                    match frame.into_data() {
                        Ok(mut data) if data.len() > H2_SLICE => {
                            let first = cut_slice(&mut data);
                            this.rest = data;
                            this.charge_rest();
                            return Poll::Ready(Some(Ok(Frame::data(first))));
                        }
                        Ok(data) => return Poll::Ready(Some(Ok(Frame::data(data)))),
                        Err(frame) => return Poll::Ready(Some(Ok(frame))),
                    }
                }
                Poll::Ready(Some(Ok(frame)))
            }
            other => {
                this.clock.wait_for_application();
                if matches!(other, Poll::Ready(None)) {
                    this.ended = true;
                }
                other
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        let drained = self.rest.is_empty() && self.inner.is_end_stream();
        // Over HTTP/2 hyper asks this right after taking a frame, before h2 has room for it: a response that is a
        // single small frame would count as finished while no byte left. It ends when `poll_frame` says so (at the
        // cost of an empty DATA frame with END_STREAM).
        if self.slice && self.sent_data { drained && self.ended } else { drained }
    }
    fn size_hint(&self) -> SizeHint {
        let mut hint = self.inner.size_hint();
        let rest = self.rest.len() as u64;
        if rest > 0 {
            hint.set_lower(hint.lower().saturating_add(rest));
            if let Some(upper) = hint.upper() {
                hint.set_upper(upper.saturating_add(rest));
            }
        }
        hint
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

/// Which protocol a connection speaks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Proto {
    /// A plain listener: HTTP/1, or HTTP/2 when `h2c` is on, told apart by the first bytes.
    Auto,
    /// TLS with ALPN `http/1.1` (or none): HTTP/1 only, the HTTP/2 preface is refused.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    H1Only,
    /// TLS with ALPN `h2`: HTTP/2 only.
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    H2Only,
}

/// hyper's `ignore_invalid_headers`; it must stay off while the framing check is on.
const IGNORE_INVALID_HEADERS: bool = false;

const H2_PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

/// Frame boundaries of one direction of an HTTP/2 connection: which kinds of frame the bytes that pass belong to.
#[derive(Default)]
struct H2Frames {
    head: [u8; 9],
    head_len: usize,
    payload_left: usize,
    kind: u8,
}

const H2_DATA: u16 = 1 << 0x0;
const H2_HEADERS: u16 = 1 << 0x1;
const H2_CONTINUATION: u16 = 1 << 0x9;

impl H2Frames {
    /// Walks `data`; the bit of every frame kind (`1 << type`) that has a byte in it. A frame header counts for its
    /// frame once complete.
    fn scan(&mut self, mut data: &[u8]) -> u16 {
        let mut kinds = 0u16;
        while !data.is_empty() {
            if self.payload_left > 0 {
                let n = data.len().min(self.payload_left);
                self.payload_left -= n;
                data = &data[n..];
                kinds |= 1 << self.kind.min(15);
                continue;
            }
            let n = data.len().min(9 - self.head_len);
            self.head[self.head_len..self.head_len + n].copy_from_slice(&data[..n]);
            self.head_len += n;
            data = &data[n..];
            if self.head_len == 9 {
                self.head_len = 0;
                self.payload_left = u32::from_be_bytes([0, self.head[0], self.head[1], self.head[2]]) as usize;
                self.kind = self.head[3];
                kinds |= 1 << self.kind.min(15);
            }
        }
        kinds
    }
}

/// The socket of a connection, with the `headersTimeout` / `keepAliveTimeout` of Node: while no
/// request is in flight, a read that waits past the deadline fails and hyper closes the
/// connection. Requests in flight are not limited here (the body read has `requestTimeout`).
///
/// When the connection is dropped by the server, it is closed gracefully (see [`linger_close`]) so
/// that an error response sent before the request was fully read is not lost to a TCP reset.
struct TimedIo {
    /// `None` only while `Drop` hands the socket to the closing task.
    io: Option<ServerIo>,
    /// Held until the socket is really closed, so `max_connections` also counts closing sockets.
    permit: Option<OwnedSemaphorePermit>,
    state: Arc<ConnState>,
    headers_timeout: Option<Duration>,
    keep_alive_timeout: Option<Duration>,
    phase: Phase,
    seen_generation: u64,
    sleep: Option<Pin<Box<Sleep>>>,
    /// Bytes of the HTTP/2 preface matched so far (`None` = done: not a preface, or an h2c connection).
    preface: Option<usize>,
    /// h2c enabled: the preface was seen and the frames that follow are tracked, so only a HEADERS frame (a request)
    /// ends the idle keep-alive phase.
    h2: Option<H2Frames>,
    /// The frames the server writes, tracked once the connection is HTTP/2: only DATA, HEADERS and CONTINUATION
    /// refresh the keep-alive phase (a PONG or a SETTINGS acknowledgement does not make a connection busy).
    h2_out: Option<H2Frames>,
    h2c: bool,
    /// TLS with ALPN `h2`: the connection must start with the HTTP/2 preface.
    h2_only: bool,
    send_timeout: Option<Duration>,
    /// Since when `poll_write` or `poll_flush` have been pending without the socket accepting a byte.
    write_stalled_since: Option<Instant>,
    write_sleep: Option<Pin<Box<Sleep>>>,
    /// The last write was pending (a flush that completes does not mean the socket accepts bytes).
    write_blocked: bool,
    /// Upgraded connections stop reading while a write is blocked (see `read_paused`): the task parked in
    /// `poll_read`, and the task whose write is pending.
    read_waker: Option<Waker>,
    write_waker: Option<Waker>,
    /// How many bytes an upgraded connection may read while a write is blocked.
    read_credit: ReadCredit,
    /// The deadline of the parked reader, separate from `write_sleep` (a `Sleep` keeps one waker: sharing it
    /// would take the wake-up away from the task whose write is pending).
    read_sleep: Option<Pin<Box<Sleep>>>,
    /// The send timeout fired: an upgraded connection fails its reads too.
    send_failed: bool,
    /// Bytes TCP had accepted when the send deadline last restarted (TLS: the progress shows below rustls).
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    seen_accepted: u64,
    /// The HTTP/1 framing check; `None` with `insecure_http_parser(true)`.
    framing: Option<Box<h1_framing::Framing>>,
    /// Bytes the check released that did not fit in hyper's buffer yet, served before the socket.
    queue: Vec<u8>,
    queue_pos: usize,
    /// The socket reported end of input.
    sock_eof: bool,
}

/// Most credit an upgraded connection keeps: enough for a client that writes a moderately large message before it
/// reads, small enough to bound what a ping flood can make the protocol layer queue as replies.
const READ_CREDIT_MAX: usize = 64 * 1024;

/// The read credit of an upgraded connection: the bytes it may read while a write is blocked. Every byte the socket
/// accepts earns a byte (up to [`READ_CREDIT_MAX`]) and every byte read spends one, so what a peer can make the
/// server read during blocked writes never exceeds what the socket took plus the maximum, however small the slices
/// the socket accepts are. That bounds the replies (pongs) queued behind a peer that does not read.
#[derive(Debug, Clone, Copy)]
struct ReadCredit(usize);

impl ReadCredit {
    fn new() -> Self {
        ReadCredit(READ_CREDIT_MAX)
    }
    fn earn(&mut self, written: usize) {
        self.0 = self.0.saturating_add(written).min(READ_CREDIT_MAX);
    }
    fn spend(&mut self, read: usize) {
        self.0 = self.0.saturating_sub(read);
    }
    fn exhausted(&self) -> bool {
        self.0 == 0
    }
    #[cfg_attr(not(feature = "tls"), allow(dead_code))]
    fn available(&self) -> usize {
        self.0
    }
}

impl Default for ReadCredit {
    fn default() -> Self {
        Self::new()
    }
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
async fn linger_close(mut io: ServerIo) {
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

/// Resolves when the connection has stalled: some response body waits for hyper to take its next frame and no body
/// of the connection has had a frame taken for `timeout` (the client does not make room: a socket that does not
/// read, HTTP/2 windows that stay closed). A stalled stream next to streams that move does not count, and bodies
/// that wait for the application never do. Pending forever without a timeout, with one too long to add to an
/// `Instant`, and on upgraded connections.
async fn watchdog(state: &ConnState, timeout: Option<Duration>) {
    let Some(timeout) = timeout.filter(|t| Instant::now().checked_add(*t).is_some()) else { return std::future::pending().await };
    loop {
        let notified = state.clocks_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let now = Instant::now();
        let (mut any, mut earliest) = (false, None::<Instant>);
        for clock in state.clocks.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            any = true;
            if let Some(t) = clock.since(&state.epoch) {
                earliest = Some(earliest.map_or(t, |e| e.min(t)));
            }
        }
        if state.upgraded.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        // The stall counts from the later of the oldest wait and the last frame any body gave: a body that starts
        // waiting after a long quiet period (idle SSE) gets a full period of its own.
        let stalled_since = earliest.map(|t| state.last_progress().map_or(t, |p| p.max(t)));
        if let Some(t) = stalled_since
            && t + timeout <= now
        {
            state.no_linger.store(true, Ordering::SeqCst);
            return;
        }
        if any {
            // Progress and new waits only move the deadline later, so waking at the current one is never late.
            let wake = stalled_since.unwrap_or(now) + timeout;
            tokio::select! {
                _ = tokio::time::sleep_until(wake) => {}
                _ = notified.as_mut() => {}
            }
        } else {
            notified.await;
        }
    }
}

/// Runs a connection to its end, then closes the socket: gracefully (see [`linger_close`]) unless the
/// server is closing it to punish the peer (an idle timeout of this server, hyper's header read
/// timeout, refused h2c), the connection was upgraded, or an upgrade listener destroyed the socket.
async fn finish_connection<C>(
    state: Arc<ConnState>,
    conn: C,
    mut stop: tokio::sync::watch::Receiver<()>,
    graceful_shutdown: fn(Pin<&mut C>),
    send_timeout: Option<Duration>,
) where
    C: Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
{
    let mut conn = Box::pin(conn);
    let mut stopping = false;
    let mut flood = false;
    let mut flood_deadline: Option<Pin<Box<tokio::time::Sleep>>> = None;
    let mut watch = Box::pin(watchdog(&state, send_timeout));
    let result = loop {
        tokio::select! {
            r = conn.as_mut() => break r,
            // A response the client does not let move: drop the connection without a drain.
            _ = watch.as_mut() => break Ok(()),
            // The server is shutting down: finish the streams in flight and stop.
            _ = stop.changed(), if !stopping => {
                stopping = true;
                graceful_shutdown(conn.as_mut());
            }
            // Rapid reset: a graceful GOAWAY refuses new streams (hyper cannot send ENHANCE_YOUR_CALM), and the
            // connection is dropped after a short grace period whatever the streams are doing.
            _ = state.reset_flood.notified(), if !flood => {
                flood = true;
                state.no_linger.store(true, Ordering::SeqCst);
                if !stopping {
                    graceful_shutdown(conn.as_mut());
                }
                flood_deadline = Some(Box::pin(tokio::time::sleep(FLOOD_GRACE)));
            }
            _ = async { flood_deadline.as_mut().unwrap().await }, if flood_deadline.is_some() => break Ok(()),
        }
    };
    // Dropping the connection hands its socket to `state.closing` (see `TimedIo::drop`); the receiver goes with it
    // so the server's shutdown does not wait for the drain below.
    drop(watch);
    drop(conn);
    drop(stop);
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
    fn new(io: ServerIo, permit: Option<OwnedSemaphorePermit>, state: Arc<ConnState>, options: &ServerOptions, proto: Proto) -> Self {
        let framing = match proto {
            Proto::H2Only => None,
            _ => (!options.insecure_http_parser).then(|| Box::new(h1_framing::Framing::new(options.max_header_size.max(8 * 1024)))),
        };
        TimedIo {
            io: Some(io),
            permit,
            state,
            headers_timeout: options.headers_timeout,
            keep_alive_timeout: options.keep_alive_timeout,
            phase: Phase::Head(Instant::now()),
            seen_generation: 0,
            sleep: None,
            preface: Some(0),
            h2: None,
            h2_out: None,
            // HTTP/2 negotiated with ALPN starts with the preface too, and is always allowed; HTTP/1 over TLS never is.
            h2c: match proto {
                Proto::Auto => options.h2c,
                Proto::H1Only => false,
                Proto::H2Only => true,
            },
            h2_only: proto == Proto::H2Only,
            send_timeout: options.send_timeout,
            write_stalled_since: None,
            write_sleep: None,
            write_blocked: false,
            read_waker: None,
            write_waker: None,
            read_credit: ReadCredit::new(),
            read_sleep: None,
            send_failed: false,
            framing,
            seen_accepted: 0,
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

    fn sock(&mut self) -> Pin<&mut ServerIo> {
        Pin::new(self.io.as_mut().expect("socket is present until drop"))
    }

    fn on_read(&mut self, data: &[u8]) -> io::Result<()> {
        self.sync();
        // An HTTP/2 connection leaves the keep-alive phase only when a request starts; other frames after the last
        // response (WINDOW_UPDATE, PING, SETTINGS) must not swap `keep_alive_timeout` for `headers_timeout`.
        let mut request = self.h2.is_none();
        if let Some(matched) = self.preface {
            let n = data.len().min(H2_PREFACE.len() - matched);
            if data[..n] != H2_PREFACE[matched..matched + n] {
                if self.h2_only {
                    // ALPN negotiated `h2`: anything but the HTTP/2 preface is a protocol violation, and must not be
                    // parsed as HTTP/1 (the framing check is off on this path).
                    self.state.no_linger.store(true, Ordering::SeqCst);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP/2 connection preface expected"));
                }
                self.preface = None;
            } else if matched + n == H2_PREFACE.len() {
                if !self.h2c {
                    self.state.no_linger.store(true, Ordering::SeqCst);
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "HTTP/2 (h2c) is disabled"));
                }
                self.preface = None;
                self.h2_out = Some(H2Frames::default());
                let mut frames = H2Frames::default();
                request = frames.scan(&data[n..]) & H2_HEADERS != 0;
                self.h2 = Some(frames);
            } else {
                self.preface = Some(matched + n);
            }
        } else if let Some(frames) = self.h2.as_mut() {
            request = frames.scan(data) & H2_HEADERS != 0;
        }
        if request && let Phase::KeepAlive(_) = self.phase {
            self.phase = Phase::Head(Instant::now());
        }
        Ok(())
    }

    /// A write or flush is pending: fails with `TimedOut` once the socket has accepted nothing for `send_timeout`
    /// (the client does not read), after which the connection is closed without the drain. Pending otherwise, with
    /// the task woken at the deadline.
    fn poll_write_deadline(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let Some(timeout) = self.send_timeout else { return Poll::Pending };
        self.sync_wire_progress();
        let since = *self.write_stalled_since.get_or_insert_with(Instant::now);
        let Some(deadline) = since.checked_add(timeout) else { return Poll::Pending };
        let sleep = self.write_sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(self.send_timed_out())),
            Poll::Pending => Poll::Pending,
        }
    }

    /// Under TLS a flush can stay pending while TCP keeps taking bytes: whatever TCP accepted since the last look is
    /// progress, so the send deadline restarts from now.
    fn sync_wire_progress(&mut self) {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            let accepted = t.wire.accepted();
            if accepted != self.seen_accepted {
                self.seen_accepted = accepted;
                self.write_stalled_since = None;
            }
        }
    }

    fn is_tls(&self) -> bool {
        #[cfg(feature = "tls")]
        {
            matches!(self.io, Some(ServerIo::Tls(_)))
        }
        #[cfg(not(feature = "tls"))]
        {
            false
        }
    }

    /// Whether the last write is blocked. Under TLS that is the last TCP write, or ciphertext rustls still holds.
    fn write_is_blocked(&self) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            return t.wire.blocked() || t.wants_write();
        }
        self.write_blocked
    }

    fn read_credit_exhausted(&self) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            return t.wire.credit_exhausted();
        }
        self.read_credit.exhausted()
    }

    /// Pushes the ciphertext rustls holds to the socket. `true` when nothing is left pending (or the flush failed,
    /// which the next read reports); `false` when the socket does not take it yet. The task whose write is pending
    /// keeps the socket's write waker.
    fn drain_tls(&mut self, cx: &mut Context<'_>) -> bool {
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_mut()
            && t.wants_write()
        {
            let waker = self.write_waker.clone().unwrap_or_else(|| cx.waker().clone());
            let mut wcx = Context::from_waker(&waker);
            return !Pin::new(&mut t.stream).poll_flush(&mut wcx).is_pending();
        }
        let _ = cx;
        true
    }

    /// The send timeout fired: mark the socket for an abortive close and wake both halves of an upgraded
    /// connection, so the one that is not polling the deadline sees the failure too.
    fn send_timed_out(&mut self) -> io::Error {
        self.state.no_linger.store(true, Ordering::SeqCst);
        self.send_failed = true;
        for w in [self.read_waker.take(), self.write_waker.take()].into_iter().flatten() {
            w.wake();
        }
        io::Error::new(io::ErrorKind::TimedOut, "send timeout")
    }

    /// The socket accepted bytes: the client reads.
    fn write_progress(&mut self, written: &[io::IoSlice<'_>], n: usize) {
        self.write_stalled_since = None;
        self.write_blocked = false;
        // Under TLS the credit is earned by the bytes TCP accepts (the gate), not by plaintext rustls buffers.
        if !self.is_tls() {
            self.read_credit.earn(n);
        }
        if let Some(w) = self.read_waker.take() {
            w.wake();
        }
        self.sync();
        // A response still being flushed is progress, not idleness. Over HTTP/2 only the frames of a response count:
        // PONGs, SETTINGS acknowledgements and the like would keep an idle connection open for ever.
        let response = match self.h2_out.as_mut() {
            None => true,
            Some(frames) => {
                let mut left = n;
                let mut kinds = 0;
                for part in written {
                    if left == 0 {
                        break;
                    }
                    let take = left.min(part.len());
                    kinds |= frames.scan(&part[..take]);
                    left -= take;
                }
                kinds & (H2_DATA | H2_HEADERS | H2_CONTINUATION) != 0
            }
        };
        if response && let Phase::KeepAlive(_) = self.phase {
            self.phase = Phase::KeepAlive(Instant::now());
        }
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
        // A timeout too long to add to an `Instant` never fires.
        let Some(deadline) = timeout.and_then(|t| since.checked_add(t)) else { return Poll::Pending };
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

impl TimedIo {
    /// Read backpressure of upgraded connections: while a write is blocked the peer is not read once
    /// the read credit ([`ReadCredit`]) is spent, so frames that need a reply (WebSocket pings answered with
    /// pongs) cannot pile up in the protocol's write buffer behind a peer that does not read. `Some` is the result
    /// of the read poll while paused, `None` means read normally.
    ///
    /// Nothing here registers a waker in a place the writer also uses: the socket is probed with a zero-byte
    /// `try_write` (it reports the readiness the driver already knows, without a waker), and the parked reader has
    /// its own timer. The reader is woken by `write_progress`, by the timer, or by the writer's own wake-up when the
    /// socket becomes writable (a protocol layer that wakes both halves, like tokio-tungstenite, wakes it too).
    fn read_paused(&mut self, cx: &mut Context<'_>) -> Option<Poll<io::Result<()>>> {
        if !self.state.upgraded.load(Ordering::SeqCst) || !self.write_is_blocked() || !self.read_credit_exhausted() {
            return None;
        }
        // The blocked write may have been issued by the read path itself (a pong flushed while reading), in which
        // case nobody else retries it: if the socket is writable again, or reports an error, let the read go on
        // (which retries the write, or returns the error) and wake the task whose write is pending. The credit is
        // not refilled: only bytes the socket accepts earn it.
        //
        // Under TLS a writable socket is not enough: ciphertext rustls still holds is pushed first, and the pause
        // lifts only when none is left (or the socket took part of it, which earns credit).
        if self.io.as_ref().expect("socket is present until drop").socket_writable() && self.drain_tls(cx) {
            self.write_blocked = false;
            #[cfg(feature = "tls")]
            if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
                t.wire.unblock();
            }
            if let Some(w) = self.write_waker.take() {
                w.wake();
            }
            return None;
        }
        self.sync_wire_progress();
        self.read_waker = Some(cx.waker().clone());
        #[cfg(feature = "tls")]
        if let Some(ServerIo::Tls(t)) = self.io.as_ref() {
            t.wire.park_reader(cx.waker());
        }
        let Some(timeout) = self.send_timeout else { return Some(Poll::Pending) };
        let since = *self.write_stalled_since.get_or_insert_with(Instant::now);
        let Some(deadline) = since.checked_add(timeout) else { return Some(Poll::Pending) };
        let sleep = self.read_sleep.get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if sleep.deadline() != deadline {
            sleep.as_mut().reset(deadline);
        }
        match sleep.as_mut().poll(cx) {
            Poll::Ready(()) => Some(Poll::Ready(Err(self.send_timed_out()))),
            Poll::Pending => Some(Poll::Pending),
        }
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.send_failed && this.state.upgraded.load(Ordering::SeqCst) {
            return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "send timeout")));
        }
        if let Some(r) = this.read_paused(cx) {
            return r;
        }
        let upgraded = this.state.upgraded.load(Ordering::SeqCst);
        if !upgraded {
            return this.poll_read_unpaused(cx, buf);
        }
        if this.is_tls() {
            // The credit is spent below TLS, on the ciphertext the gate reads (see `transport::tls_io::Gate`). A read
            // the gate parks looks again at the pause, which starts the deadline of the parked reader.
            for _ in 0..2 {
                let r = this.poll_read_unpaused(cx, buf);
                if r.is_ready() {
                    return r;
                }
                this.read_waker = Some(cx.waker().clone());
                if let Some(paused) = this.read_paused(cx) {
                    return paused;
                }
            }
            return Poll::Pending;
        }
        // While a write is blocked, never read more than the credit left (it is not exhausted here, or the read
        // would have been paused): small slices accepted by the socket must not let whole chunks through.
        let limit = if this.write_blocked { this.read_credit.0.min(buf.remaining()) } else { buf.remaining() };
        let (r, read) = if limit < buf.remaining() {
            let mut limited = ReadBuf::new(buf.initialize_unfilled_to(limit));
            let r = this.poll_read_unpaused(cx, &mut limited);
            let n = limited.filled().len();
            buf.advance(n);
            (r, n)
        } else {
            let before = buf.filled().len();
            let r = this.poll_read_unpaused(cx, buf);
            (r, buf.filled().len() - before)
        };
        if matches!(r, Poll::Ready(Ok(()))) {
            this.read_credit.spend(read);
        }
        if r.is_pending() {
            // So that a send timeout noticed by the writer wakes this reader too.
            this.read_waker = Some(cx.waker().clone());
        }
        r
    }
}

impl TimedIo {
    fn poll_read_unpaused(&mut self, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self;
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
        match this.sock().poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    this.write_progress(&[io::IoSlice::new(buf)], n);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Pending => {
                this.write_blocked = true;
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx).map(|r| r.map(|()| 0))
            }
            other => other,
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match this.sock().poll_flush(cx) {
            Poll::Ready(Ok(())) => {
                if !this.write_blocked {
                    this.write_stalled_since = None;
                }
                // Hyper flushes only with its write buffer empty: the HTTP/1 responses it took are out of memory.
                this.state.flush_deferred();
                Poll::Ready(Ok(()))
            }
            Poll::Pending => {
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx)
            }
            other => other,
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.sock().poll_shutdown(cx)
    }
    fn poll_write_vectored(mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[io::IoSlice<'_>]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match this.sock().poll_write_vectored(cx, bufs) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    this.write_progress(bufs, n);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Pending => {
                this.write_blocked = true;
                this.write_waker = Some(cx.waker().clone());
                this.poll_write_deadline(cx).map(|r| r.map(|()| 0))
            }
            other => other,
        }
    }
    fn is_write_vectored(&self) -> bool {
        self.io.as_ref().is_some_and(|io| io.is_write_vectored())
    }
}

/// Unsent bytes a connection may hold in the kernel before its socket stops being reported writable
/// (`TCP_NOTSENT_LOWAT`), when `send_timeout` is on.
///
/// Linux reports a TCP socket writable only when the free part of the send buffer reaches half of what is queued, and
/// autotuning grows that buffer to several MiB: a client that reads slowly but steadily frees space the server cannot
/// see for many seconds, and the send timeout would cut it. With a low-water mark the socket is writable as soon as
/// the unsent part falls under half of it (`tcp_poll` asks `tcp_stream_memory_free` with a wake-up margin of one
/// shift), whatever the size of the buffer, so progress shows in steps of about half the mark.
/// HTTP/2 uses the same socket: its flow control bounds what h2 writes, but a client with large windows can still
/// fill the kernel buffers, so it gets the same limit. Where the socket option is not available (see
/// [`limit_unsent_bytes`]) the granularity is whatever the platform's buffers give.
#[cfg(any(target_os = "linux", target_os = "android"))]
const NOTSENT_LOWAT: u32 = 64 * 1024;

/// Sets `TCP_NOTSENT_LOWAT` on an accepted socket (Linux and Android). Failures are ignored: the option only makes
/// the send timeout notice progress sooner. Not available through `socket2` on macOS and the BSDs, nor on Windows,
/// where the platform's own buffers decide how soon progress is seen.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn limit_unsent_bytes(stream: &TcpStream) {
    let _ = socket2::SockRef::from(stream).set_tcp_notsent_lowat(NOTSENT_LOWAT);
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn limit_unsent_bytes(_stream: &TcpStream) {}

/// Whether hyper can add `t` to an `Instant` (it panics otherwise): a longer timeout is the same as none.
fn representable(t: Duration) -> bool {
    std::time::Instant::now().checked_add(t).is_some()
}

/// The connection builder of every listener: HTTP/1 and HTTP/2 settings from the options. Any listener that
/// can speak HTTP/2 (h2c, and TLS with ALPN `h2`) must be built with it so the limits apply to all of them.
pub(crate) fn connection_builder(options: &ServerOptions) -> auto::Builder<TokioExecutor> {
    let mut builder = auto::Builder::new(TokioExecutor::new());
    // The framing check reads header lines the way httparse does by default: `ignore_invalid_headers` would let
    // httparse skip lines the check has read (design.md, decision 4, of `e0-reject-ambiguous-http1-framing`).
    debug_assert!(options.insecure_http_parser || !IGNORE_INVALID_HEADERS);
    builder
        .http1()
        .ignore_invalid_headers(IGNORE_INVALID_HEADERS)
        .timer(TokioTimer::new())
        .header_read_timeout(options.headers_timeout.filter(|t| representable(*t)))
        .max_buf_size(options.max_header_size.max(8 * 1024));
    builder
        .http2()
        .timer(TokioTimer::new())
        .max_header_list_size(u32::try_from(options.max_header_size.max(8 * 1024)).unwrap_or(u32::MAX))
        .max_concurrent_streams(options.http2_max_concurrent_streams.max(1))
        .max_send_buf_size(H2_SEND_BUF)
        .keep_alive_interval(options.http2_keep_alive_interval.filter(|t| representable(*t)))
        .keep_alive_timeout(if representable(options.http2_keep_alive_timeout) {
            options.http2_keep_alive_timeout
        } else {
            Duration::from_secs(100 * 365 * 24 * 3600)
        });
    builder
}

/// Why a request ends without a response: the socket is destroyed (upgrade listeners, flood, a wait for room that
/// took too long over HTTP/1), or the server resets the HTTP/2 stream (response above `max_pending_response_bytes`,
/// wait for room that took too long) and hyper uses the reason in the source.
#[derive(Debug)]
enum ServiceError {
    Destroyed,
    Reset(h2::Error),
}

impl From<crate::http_adapter::SocketDestroyed> for ServiceError {
    fn from(_: crate::http_adapter::SocketDestroyed) -> Self {
        ServiceError::Destroyed
    }
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Destroyed => f.write_str("socket destroyed"),
            ServiceError::Reset(_) => f.write_str("stream reset: the connection holds too many pending response bytes"),
        }
    }
}

impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ServiceError::Destroyed => None,
            ServiceError::Reset(e) => Some(e),
        }
    }
}

/// Where a listener's connections come from: plain TCP, or TCP with a TLS handshake first.
pub(crate) enum Transport {
    Plain,
    #[cfg(feature = "tls")]
    Tls(tls::TlsSettings),
}

impl Transport {
    pub(crate) fn is_secure(&self) -> bool {
        match self {
            Transport::Plain => false,
            #[cfg(feature = "tls")]
            Transport::Tls(_) => true,
        }
    }
}

/// What a connection task needs, gathered by the accept loop.
struct ConnCtx {
    shared: Arc<AppShared>,
    opts: Arc<ServerOptions>,
    builder: auto::Builder<TokioExecutor>,
    stream_cap: usize,
    remote: SocketAddr,
    state: Arc<ConnState>,
    stop: tokio::sync::watch::Receiver<()>,
}

/// Serves one connection to its end (see [`finish_connection`]).
async fn run_connection(ctx: ConnCtx, stream: ServerIo, permit: Option<OwnedSemaphorePermit>, proto: Proto) {
    let ConnCtx { shared, opts, builder, stream_cap, remote, state, stop } = ctx;
    let send_timeout = opts.send_timeout;
    let conn_state = state.clone();
    let io = TimedIo::new(stream, permit, state.clone(), &opts, proto);
    let handlers = Arc::new(tokio::sync::Semaphore::new(stream_cap));
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
        let http2 = req.version() == http::Version::HTTP_2;
        let guard = InflightGuard::new(state.clone());
        let watch = CancelWatch { state: state.clone(), outcome: Outcome::Pending };
        let handlers = handlers.clone();
        let shared = shared.clone();
        let opts = opts.clone();
        async move {
            // Streams reset and replaced faster than their handlers stop would otherwise run more
            // handlers than the stream cap; the permit lives until this future is dropped.
            let mut watch = watch;
            // After a rapid-reset verdict the connection only waits for its GOAWAY: new streams are
            // reset without reaching a handler (hyper resets the stream on a service error).
            if guard.0.flooded.load(Ordering::SeqCst) {
                watch.outcome = Outcome::Refused;
                return Err(ServiceError::Destroyed);
            }
            let _permit = handlers.acquire_owned().await;
            // Room in the connection's response budget before the handler runs (a client that resets
            // its stream meanwhile drops this future: a cancelled request that never reached a handler).
            let admission = match opts.max_pending_response_bytes {
                Some(cap) => {
                    let limit = if http2 { stream_cap } else { 1 };
                    let admit = guard.0.admit(cap / limit, cap);
                    // Over HTTP/1 the wait only delays the client's own pipelined requests and ends when it
                    // reads (or `send_timeout` closes the connection): closing it would cut a slow download.
                    let admitted = match opts.max_pending_response_wait.filter(|_| http2) {
                        Some(t) => tokio::time::timeout(t, admit).await.ok(),
                        None => Some(admit.await),
                    };
                    let Some(admission) = admitted else {
                        // The handler never ran: neither cancelled nor completed.
                        watch.outcome = Outcome::Refused;
                        guard.0.warn_wait_once(opts.max_pending_response_wait.unwrap_or_default());
                        return Err(ServiceError::Reset(h2::Error::from(h2::Reason::REFUSED_STREAM)));
                    };
                    Some((admission, cap))
                }
                None => None,
            };
            if guard.0.flooded.load(Ordering::SeqCst) {
                watch.outcome = Outcome::Refused;
                return Err(ServiceError::Destroyed);
            }
            let mut aborted = false;
            let resp = handle(shared, req, remote, &opts, &mut aborted).await;
            watch.outcome = if aborted { Outcome::ClientAborted } else { Outcome::Completed };
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    guard.0.destroyed.store(true, Ordering::SeqCst);
                    return Err(e.into());
                }
            };
            if resp.status() == http::StatusCode::SWITCHING_PROTOCOLS {
                guard.0.upgraded.store(true, Ordering::SeqCst);
            }
            // What the response holds until its body ends: an in-memory body in full (also when
            // compression hides its size), a streamed one only the unsent part of a cut frame.
            let charge = match admission {
                Some((admission, cap)) => {
                    let size = hyper::body::Body::size_hint(resp.body())
                        .exact()
                        .or_else(|| resp.extensions().get::<PlainLen>().map(|p| p.0))
                        .map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
                    if admission.finish(size, cap, http2) == Verdict::Reset {
                        // Dropping the response frees its allocation and its compression slot now.
                        drop(resp);
                        watch.outcome = Outcome::Dropped;
                        return Err(ServiceError::Reset(h2::Error::from(h2::Reason::ENHANCE_YOUR_CALM)));
                    }
                    Some(size)
                }
                None => None,
            };
            // From here the body decides how the request ends (completed or cancelled).
            let record = !aborted;
            if record {
                watch.outcome = Outcome::Body;
            }
            Ok::<_, ServiceError>(resp.map(|inner| GuardedBody::with_charge(inner, guard, http2, charge, record).boxed_unsync()))
        }
    });
    if proto == Proto::H2Only {
        // ALPN said `h2`: the HTTP/2-only path. `serve_connection_with_upgrades` sniffs the first bytes whatever the
        // builder's version says, which would let a client speak HTTP/1 here without the framing check.
        let conn = builder.serve_connection(TokioIo::new(io), service).into_owned();
        finish_connection(conn_state, conn, stop, |c| c.graceful_shutdown(), send_timeout).await;
        return;
    }
    let conn = builder.serve_connection_with_upgrades(TokioIo::new(io), service);
    let conn = conn.into_owned();
    finish_connection(conn_state, conn, stop, |c| c.graceful_shutdown(), send_timeout).await;
}

/// The TLS handshake of one connection, in the connection's own task: bounded by the handshake timeout, abandoned
/// when the server stops, and never answered with HTTP data when it fails. The permit stays with the task, so a
/// connection in its handshake counts against `max_connections`.
#[cfg(feature = "tls")]
async fn handshake(
    settings: tls::TlsSettings,
    tcp: TcpStream,
    state: Arc<ConnState>,
    stop: &mut tokio::sync::watch::Receiver<()>,
) -> Option<(ServerIo, Proto)> {
    use transport::tls_io::{Gate, TlsIo, Wire};
    let wire = Wire::new();
    let gate = Gate::new(tcp, wire.clone(), state);
    let accept = settings.acceptor.accept(gate);
    let stream = tokio::select! {
        r = tokio::time::timeout(settings.handshake_timeout, accept) => r.ok()?.ok()?,
        _ = stop.changed() => return None,
    };
    let tls = TlsIo { stream, wire };
    let proto = match tls.alpn() {
        Some(b"h2") => Proto::H2Only,
        Some(b"http/1.1") | None => Proto::H1Only,
        Some(_) => return None,
    };
    Some((ServerIo::Tls(Box::new(tls)), proto))
}

/// Accepts connections until `shutdown` resolves, then drains them gracefully.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<AppShared>,
    options: ServerOptions,
    transport: Transport,
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> std::io::Result<()> {
    let options = Arc::new(options);
    let (stop_tx, _) = tokio::sync::watch::channel(());
    let builder = connection_builder(&options);
    let stream_cap = options.http2_max_concurrent_streams.max(1) as usize;
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
                if options.send_timeout.is_some() {
                    limit_unsent_bytes(&stream);
                }
                let state = Arc::new(ConnState { cancel_ceiling: options.http2_max_cancelled_streams, ..ConnState::default() });
                let ctx = ConnCtx {
                    shared: shared.clone(),
                    opts: options.clone(),
                    builder: builder.clone(),
                    stream_cap,
                    remote,
                    state,
                    stop: stop_tx.subscribe(),
                };
                match &transport {
                    Transport::Plain => {
                        tokio::spawn(run_connection(ctx, ServerIo::Plain(stream), permit, Proto::Auto));
                    }
                    #[cfg(feature = "tls")]
                    Transport::Tls(settings) => {
                        let settings = settings.clone();
                        tokio::spawn(async move {
                            let mut ctx = ctx;
                            let Some((io, proto)) = handshake(settings, stream, ctx.state.clone(), &mut ctx.stop).await else { return };
                            ctx.builder = match proto {
                                Proto::H2Only => ctx.builder.http2_only(),
                                _ => ctx.builder.http1_only(),
                            };
                            run_connection(ctx, io, permit, proto).await;
                        });
                    }
                }
            }
            _ = &mut shutdown => break,
        }
    }
    drop(listener);
    // Pending long-polls would hold the drain below: answer them now.
    #[cfg(feature = "socket-io")]
    shared.http_server.close_engines();
    tokio::select! {
        _ = async {
            let _ = stop_tx.send(());
            stop_tx.closed().await;
        } => {}
        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A peer that floods pings and reads slowly: the socket takes `slice` bytes per writable event. Whatever the
    /// slice, the bytes read while writes are blocked stay within what the socket took plus the maximum credit.
    #[test]
    fn the_read_credit_follows_the_bytes_the_socket_accepts() {
        for slice in [1usize, 512, 4 * 1024, 16 * 1024, 32 * 1024, 64 * 1024] {
            let mut credit = ReadCredit::new();
            let (mut written, mut read) = (0usize, 0usize);
            for _ in 0..10_000 {
                // The socket accepts a slice (a writable event), then the reader takes chunks until the credit is
                // spent (paused: a blocked write with no credit left).
                credit.earn(slice);
                written += slice;
                while !credit.exhausted() {
                    // A read never takes more than the credit left.
                    let chunk = credit.0.min(8 * 1024);
                    credit.spend(chunk);
                    read += chunk;
                }
            }
            assert!(read <= written + READ_CREDIT_MAX, "slice {slice}: read {read}, written {written}");
        }
    }

    /// Feeds `events` (seconds from a start, cancelled or not) and returns the time of the first close verdict.
    fn first_close(events: impl IntoIterator<Item = (f64, bool)>) -> Option<f64> {
        let state = ConnState { cancel_ceiling: 2000, ..ConnState::default() };
        let start = Instant::now();
        events.into_iter().find(|&(t, cancelled)| state.tally(start + Duration::from_secs_f64(t), cancelled)).map(|(t, _)| t)
    }

    #[test]
    fn cancel_floor_closes_a_pure_flood() {
        let flood = (0..2000).map(|i| (i as f64 * 0.001, true));
        assert_eq!(first_close(flood), Some(0.5));
    }

    #[test]
    fn about_a_thousand_cancels_do_not_fit_across_a_window_edge() {
        // 500 at the end of the first window, 500 at the start of the next: the weighted count reaches the floor.
        let mut events: Vec<(f64, bool)> = (0..500).map(|i| (9.9 + i as f64 * 0.0001, true)).collect();
        events.extend((0..500).map(|i| (10.0 + i as f64 * 0.0001, true)));
        let closed = first_close(events).expect("closed");
        assert!(closed < 10.06, "{closed}");
    }

    #[test]
    fn one_cheap_completed_request_per_reset_does_not_hide_a_flood() {
        let events = (0..20_000).map(|i| (i as f64 * 0.0005, i % 2 == 0));
        let closed = first_close(events).expect("closed");
        assert!(closed < 1.5, "{closed}");
    }

    #[test]
    fn the_ceiling_closes_a_connection_that_completes_plenty() {
        // 5,000 requests/s with 10% cancelled (500 per second): within the proportional rule, over the ceiling.
        let events = (0..50_000).map(|i| (i as f64 / 5000.0, i % 10 == 0));
        let closed = first_close(events).expect("closed");
        assert!((3.5..4.5).contains(&closed), "{closed}");
    }

    #[test]
    fn a_proxy_with_aborts_within_the_credit_is_not_closed() {
        // 5,000 requests/s with 10% cancelled would trip the ceiling; 150 per second (3%) does not.
        let events = (0..150_000).map(|i| (i as f64 / 5000.0, i % 33 == 0));
        assert_eq!(first_close(events), None);
    }

    #[test]
    fn a_proxy_that_completes_more_than_it_cancels_is_not_closed() {
        // 5,000 requests/s for 30 s with 2% cancelled (100 per second, 3,000 in total).
        let events = (0..150_000).map(|i| (i as f64 / 5000.0, i % 50 == 0));
        assert_eq!(first_close(events), None);
    }

    #[test]
    fn an_old_burst_is_forgotten() {
        let mut events: Vec<(f64, bool)> = (0..400).map(|i| (i as f64 * 0.001, true)).collect();
        events.extend((0..400).map(|i| (25.0 + i as f64 * 0.001, true)));
        assert_eq!(first_close(events), None);
    }

    #[test]
    fn send_timeout_defaults_to_sixty_seconds_and_can_be_turned_off() {
        assert_eq!(ServerOptions::default().send_timeout, Some(Duration::from_secs(60)));
        assert_eq!(ServerOptions::new().send_timeout(None).send_timeout, None);
        assert_eq!(ServerOptions::new().send_timeout(Duration::from_secs(5)).send_timeout, Some(Duration::from_secs(5)));
    }

    struct Script {
        parts: std::collections::VecDeque<Option<Result<Frame<Bytes>, io::Error>>>,
        pending_first: bool,
    }

    impl hyper::body::Body for Script {
        type Data = Bytes;
        type Error = io::Error;
        fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
            if std::mem::take(&mut self.pending_first) {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Poll::Ready(self.parts.pop_front().flatten())
        }
    }

    fn body_of(parts: Vec<Option<Result<Frame<Bytes>, io::Error>>>, pending_first: bool, http2: bool) -> (GuardedBody, Arc<ConnState>) {
        let state = Arc::new(ConnState::default());
        let inner = Script { parts: parts.into(), pending_first }.boxed_unsync();
        (GuardedBody::new(inner, InflightGuard::new(state.clone()), http2), state)
    }

    fn poll_once(body: &mut GuardedBody) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        hyper::body::Body::poll_frame(Pin::new(body), &mut cx)
    }

    fn waiting(state: &ConnState) -> Vec<bool> {
        state.clocks.lock().unwrap().iter().map(|c| c.since(&state.epoch).is_some()).collect()
    }

    #[test]
    fn a_body_waits_for_hyper_until_it_waits_for_the_application() {
        let (mut body, state) = body_of(vec![Some(Ok(Frame::data(Bytes::from_static(b"a"))))], true, false);
        // Created, never polled: hyper has to take its first frame.
        assert_eq!(waiting(&state), vec![true]);
        assert!(poll_once(&mut body).is_pending());
        assert_eq!(waiting(&state), vec![false], "pending: it waits for the application");
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(waiting(&state), vec![true], "a frame was produced: hyper has to take it");
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        assert_eq!(waiting(&state), vec![false], "ended");
        drop(body);
        assert!(state.clocks.lock().unwrap().is_empty(), "the clock goes with the body");
        assert_eq!(state.inflight.load(Ordering::SeqCst), 0);
    }

    fn many_frames(n: usize) -> Vec<Option<Result<Frame<Bytes>, io::Error>>> {
        (0..n).map(|_| Some(Ok(Frame::data(Bytes::from_static(b"x"))))).collect()
    }

    /// One stalled body next to one that keeps moving is not a stalled connection; when neither moves it is.
    #[tokio::test(start_paused = true)]
    async fn the_watchdog_closes_the_connection_only_when_no_body_moves() {
        let state = Arc::new(ConnState::default());
        let make = |parts| {
            GuardedBody::new(
                Script { parts: std::collections::VecDeque::from(parts), pending_first: false }.boxed_unsync(),
                InflightGuard::new(state.clone()),
                false,
            )
        };
        let (mut stalled, mut moving) = (make(many_frames(3)), make(many_frames(50)));
        let mut watch = Box::pin(watchdog(&state, Some(Duration::from_secs(1))));
        // Both took their first frame; hyper takes nothing more from `stalled`.
        assert!(matches!(poll_once(&mut stalled), Poll::Ready(Some(_))));
        assert!(matches!(poll_once(&mut moving), Poll::Ready(Some(_))));
        for _ in 0..8 {
            tokio::time::advance(Duration::from_millis(400)).await;
            assert!(matches!(poll_once(&mut moving), Poll::Ready(Some(_))));
            assert!(futures::poll!(watch.as_mut()).is_pending(), "a body moved: the connection is not stalled");
        }
        tokio::time::advance(Duration::from_millis(1100)).await;
        assert!(futures::poll!(watch.as_mut()).is_ready(), "nothing moved for a send timeout");
        assert!(state.no_linger.load(Ordering::SeqCst));
    }

    #[tokio::test(start_paused = true)]
    async fn the_watchdog_ignores_bodies_that_wait_for_the_application_and_huge_timeouts() {
        let state = Arc::new(ConnState::default());
        let (mut idle, _) = body_of(vec![Some(Ok(Frame::data(Bytes::from_static(b"a"))))], true, false);
        let _ = &state;
        assert!(poll_once(&mut idle).is_pending());
        let idle_state = idle.guard.0.clone();
        let mut watch = Box::pin(watchdog(&idle_state, Some(Duration::from_secs(1))));
        tokio::time::advance(Duration::from_secs(30)).await;
        assert!(futures::poll!(watch.as_mut()).is_pending(), "waiting for the application is not a stall");
        let mut forever = Box::pin(watchdog(&state, Some(Duration::MAX)));
        assert!(futures::poll!(forever.as_mut()).is_pending());
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[tokio::test]
    async fn accepted_sockets_get_a_low_water_mark_for_unsent_data() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        limit_unsent_bytes(&accepted);
        assert_eq!(socket2::SockRef::from(&accepted).tcp_notsent_lowat().unwrap(), NOTSENT_LOWAT);
        drop(client);
    }

    #[test]
    fn a_timeout_too_long_for_an_instant_is_off() {
        assert!(Instant::now().checked_add(Duration::MAX).is_none());
        let options = ServerOptions::new().send_timeout(Duration::MAX).headers_timeout(Duration::MAX).keep_alive_timeout(Duration::MAX);
        assert_eq!(options.send_timeout, Some(Duration::MAX));
    }

    #[test]
    fn http2_data_is_handed_over_in_slices_of_sixteen_kib() {
        let big = Bytes::from(vec![7u8; 40 * 1024]);
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, true);
        let mut sizes = Vec::new();
        while let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) {
            sizes.push(frame.into_data().unwrap().len());
            if sizes.len() < 3 {
                assert!(!hyper::body::Body::is_end_stream(&body), "a body with data left must not report its end");
            }
        }
        assert_eq!(sizes, vec![16 * 1024, 16 * 1024, 8 * 1024]);
    }

    fn charged_body(
        parts: Vec<Option<Result<Frame<Bytes>, io::Error>>>,
        http2: bool,
        charge: Option<usize>,
        state: &Arc<ConnState>,
    ) -> GuardedBody {
        let inner = Script { parts: parts.into(), pending_first: false }.boxed_unsync();
        if let Some(c) = charge {
            state.budget.lock().unwrap().pending += c;
        }
        GuardedBody::with_charge(inner, InflightGuard::new(state.clone()), http2, charge, true)
    }

    fn pending(state: &ConnState) -> usize {
        state.budget.lock().unwrap().pending
    }

    #[test]
    fn max_pending_response_bytes_defaults_to_eight_mib_and_can_be_turned_off() {
        assert_eq!(ServerOptions::default().max_pending_response_bytes, Some(8 * 1024 * 1024));
        assert_eq!(ServerOptions::new().max_pending_response_bytes(None).max_pending_response_bytes, None);
        assert_eq!(ServerOptions::new().max_pending_response_bytes(1024).max_pending_response_bytes, Some(1024));
    }

    #[test]
    fn a_handler_is_admitted_from_idle_whatever_the_share() {
        let mut b = Budget::default();
        assert!(b.try_admit(10 * 1024 * 1024, 1024));
        assert_eq!(b.running, 1);
    }

    #[test]
    fn admission_reserves_a_share_per_running_handler() {
        let mut b = Budget::default();
        // cap 100, share 25: four handlers fit, the fifth waits.
        for _ in 0..4 {
            assert!(b.try_admit(25, 100));
        }
        assert!(!b.try_admit(25, 100));
        assert_eq!(b.running, 4);
        // One returns a small response: its reservation is replaced by the charge.
        assert_eq!(b.finish_handler(10, 100, true), Verdict::Keep);
        assert_eq!((b.running, b.pending), (3, 10));
        assert!(!b.try_admit(25, 100), "10 pending + 3 reserved + its own 25 does not fit");
        b.release(10);
        assert!(b.try_admit(25, 100), "3 reserved + its own 25 fits again");
    }

    #[test]
    fn pending_bytes_alone_block_admission_and_a_release_frees_it() {
        let mut b = Budget::default();
        assert!(b.try_admit(10, 100));
        assert_eq!(b.finish_handler(95, 100, true), Verdict::Keep);
        assert!(!b.try_admit(10, 100), "95 pending leave no room for a share of 10");
        b.release(95);
        assert!(b.try_admit(10, 100));
    }

    #[test]
    fn a_response_is_kept_when_nothing_is_pending_and_reset_only_over_http2_above_the_cap() {
        let mut b = Budget::default();
        assert!(b.try_admit(10, 100));
        assert_eq!(b.finish_handler(1000, 100, true), Verdict::Keep, "pending was 0: one response is always sent");
        assert_eq!(b.pending, 1000);
        let mut b = Budget { pending: 90, running: 1, ..Budget::default() };
        assert_eq!(b.finish_handler(20, 100, true), Verdict::Reset);
        assert_eq!((b.pending, b.running), (90, 0), "a reset response is not charged");
        let mut b = Budget { pending: 90, running: 1, ..Budget::default() };
        assert_eq!(b.finish_handler(20, 100, false), Verdict::Keep, "HTTP/1 never resets");
    }

    #[test]
    fn responses_within_their_share_are_never_reset() {
        // cap 1000 over 10 handlers: share 100. Ten handlers each return exactly their share.
        let mut b = Budget::default();
        for _ in 0..10 {
            assert!(b.try_admit(100, 1000));
        }
        for _ in 0..10 {
            assert_eq!(b.finish_handler(100, 1000, true), Verdict::Keep);
        }
        assert_eq!(b.pending, 1000);
    }

    #[test]
    fn a_dropped_admission_gives_its_reservation_back() {
        let state = Arc::new(ConnState::default());
        let admission = futures::executor::block_on(state.admit(10, 100));
        assert_eq!(state.budget.lock().unwrap().running, 1);
        drop(admission);
        assert_eq!(state.budget.lock().unwrap().running, 0);
    }

    #[test]
    fn an_in_memory_body_stays_charged_until_it_ends_or_is_dropped() {
        let state = Arc::new(ConnState::default());
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 100 * 1024]))))], true, Some(100 * 1024), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 100 * 1024, "slices went out; the allocation is still alive");
        drop(body);
        assert_eq!(pending(&state), 0);
    }

    /// HTTP/1: hyper takes a whole frame into its write buffer, so the body ending does not free the memory; the
    /// flush of the socket (hyper flushes only with an empty write buffer) does.
    #[test]
    fn an_http1_body_stays_charged_until_hyper_flushes_the_socket() {
        let state = Arc::new(ConnState::default());
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 1000]))))], false, Some(1000), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        drop(body);
        assert_eq!(pending(&state), 1000, "the data is still in hyper's write buffer");
        state.flush_deferred();
        assert_eq!(pending(&state), 0);
        state.flush_deferred();
        assert_eq!(pending(&state), 0, "a flush with nothing deferred changes nothing");
    }

    /// Bytes hyper drops after taking them (a body longer than its `Content-Length`) never reach the socket: the
    /// charge must not depend on counting them, or the connection waits for ever.
    #[test]
    fn an_http1_charge_is_released_even_when_hyper_wrote_fewer_bytes_than_it_took() {
        let state = Arc::new(ConnState::default());
        for _ in 0..1000 {
            let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from(vec![1u8; 64 * 1024]))))], false, Some(64 * 1024), &state);
            assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
            drop(body);
            // Only ten bytes of it were written; hyper still flushes with its buffer empty.
            state.flush_deferred();
            assert_eq!(pending(&state), 0);
        }
    }

    #[test]
    fn waiters_are_woken_only_when_one_of_them_could_be_admitted() {
        let mut b = Budget { pending: 95, running: 0, waiting: 0, share: 10, cap: 100, ..Budget::default() };
        assert!(!b.should_wake(), "nobody waits");
        b.waiting = 3;
        assert!(!b.should_wake(), "95 pending leave no room for a share of 10");
        b.release(10);
        assert!(b.should_wake());
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_that_waits_too_long_gives_up_without_a_reservation() {
        let state = Arc::new(ConnState::default());
        let held = state.admit(60, 100).await;
        state.budget.lock().unwrap().pending = 100;
        let waited = tokio::time::timeout(Duration::from_secs(5), state.admit(10, 100)).await;
        assert!(waited.is_err(), "no room: the wait times out");
        {
            let b = state.budget.lock().unwrap();
            assert_eq!((b.running, b.waiting), (1, 0), "the abandoned wait holds nothing");
        }
        drop(held);
    }

    /// An HTTP/2 body of two bytes in one frame, as handlers of small JSON responses produce.
    fn full_body(state: &Arc<ConnState>) -> GuardedBody {
        state.budget.lock().unwrap().pending += 2;
        GuardedBody::with_charge(full(Bytes::from_static(b"{}")), InflightGuard::new(state.clone()), true, Some(2), true)
    }

    #[test]
    fn an_http2_response_that_fits_one_frame_is_not_finished_until_its_body_says_so() {
        let state = Arc::new(ConnState::default());
        let mut body = full_body(&state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(!hyper::body::Body::is_end_stream(&body), "the frame is with hyper but may not have left");
        // Dropped now (a client reset while h2 had no room): cancelled, not completed.
        drop(body);
        let w = state.window.lock().unwrap().unwrap();
        assert_eq!((w.cancelled, w.completed), (1, 0));
        let mut body = full_body(&state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        assert!(hyper::body::Body::is_end_stream(&body));
        drop(body);
        let w = state.window.lock().unwrap().unwrap();
        assert_eq!((w.cancelled, w.completed), (1, 1));
    }

    #[test]
    fn an_empty_http2_body_is_finished_from_the_start() {
        let body = GuardedBody::with_charge(empty_body(), InflightGuard::new(Arc::new(ConnState::default())), true, Some(0), true);
        assert!(hyper::body::Body::is_end_stream(&body));
    }

    #[test]
    fn a_streamed_body_charges_only_what_it_has_not_handed_over() {
        let state = Arc::new(ConnState::default());
        let frame = Frame::data(Bytes::from(vec![1u8; 64 * 1024]));
        let mut body = charged_body(vec![Some(Ok(frame))], true, Some(0), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 48 * 1024);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 32 * 1024);
        drop(body);
        assert_eq!(pending(&state), 0, "dropping releases the rest");
    }

    #[test]
    fn an_uncharged_body_leaves_the_budget_alone() {
        let state = Arc::new(ConnState::default());
        let frame = Frame::data(Bytes::from(vec![1u8; 64 * 1024]));
        let mut body = charged_body(vec![Some(Ok(frame))], true, None, &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert_eq!(pending(&state), 0);
    }

    /// Whether `part` lies inside the allocation of `outer`.
    fn inside(outer: &[u8], part: &Bytes) -> bool {
        let (o, p) = (outer.as_ptr() as usize, part.as_ptr() as usize);
        p >= o && p + part.len() <= o + outer.len()
    }

    #[test]
    fn the_last_slices_of_an_http2_body_are_copies() {
        let big = Bytes::from(vec![7u8; 1024 * 1024]);
        let original = big.clone();
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, true);
        let mut slices = Vec::new();
        while let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) {
            slices.push(frame.into_data().unwrap());
        }
        assert_eq!(slices.len(), 64);
        let shared: Vec<bool> = slices.iter().map(|s| inside(&original, s)).collect();
        assert!(shared[..60].iter().all(|s| *s), "the earlier slices keep pointing into the response");
        assert!(shared[60..].iter().all(|s| !*s), "the last 64 KiB (send buffer, one slice, one frame in the codec) are copies");
        assert!(slices.iter().all(|s| s.iter().all(|b| *b == 7)));
    }

    #[test]
    fn a_body_records_completed_when_it_ends_and_cancelled_when_dropped_before() {
        let state = Arc::new(ConnState { cancel_ceiling: 2000, ..ConnState::default() });
        let recorded = |state: &ConnState| {
            let w = state.window.lock().unwrap();
            w.map_or((0, 0), |w| (w.cancelled, w.completed))
        };
        let mut body = charged_body(vec![Some(Ok(Frame::data(Bytes::from_static(b"abc"))))], false, Some(3), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        assert!(matches!(poll_once(&mut body), Poll::Ready(None)));
        drop(body);
        assert_eq!(recorded(&state), (0, 1));
        let big = Bytes::from(vec![7u8; 100 * 1024]);
        let mut body = charged_body(vec![Some(Ok(Frame::data(big)))], true, Some(100 * 1024), &state);
        assert!(matches!(poll_once(&mut body), Poll::Ready(Some(Ok(_)))));
        drop(body);
        assert_eq!(recorded(&state), (1, 1), "dropped with data left: cancelled");
        let body = GuardedBody::with_charge(empty_body(), InflightGuard::new(state.clone()), false, Some(0), true);
        drop(body);
        assert_eq!(recorded(&state), (1, 2), "a body with nothing to send is completed");
    }

    #[test]
    fn http1_data_is_not_sliced() {
        let big = Bytes::from(vec![7u8; 40 * 1024]);
        let (mut body, _state) = body_of(vec![Some(Ok(Frame::data(big)))], false, false);
        let Poll::Ready(Some(Ok(frame))) = poll_once(&mut body) else { panic!("a frame") };
        assert_eq!(frame.into_data().unwrap().len(), 40 * 1024);
    }

    #[test]
    fn the_frame_scanner_tells_the_kinds_of_frame_that_pass() {
        fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
            let mut v = vec![0, (payload.len() >> 8) as u8, payload.len() as u8, kind, 0, 0, 0, 0, 1];
            v.extend_from_slice(payload);
            v
        }
        let mut f = H2Frames::default();
        assert_eq!(f.scan(&frame(0x8, &[0, 0, 0, 1])), 1 << 8, "WINDOW_UPDATE");
        assert_eq!(f.scan(&frame(0x6, &[0; 8])), 1 << 6, "PING");
        assert_eq!(f.scan(&frame(0x4, &[])), 1 << 4, "SETTINGS");
        assert_eq!(f.scan(&frame(0x0, &[1; 300])), H2_DATA, "DATA");
        assert_eq!(f.scan(&frame(0x1, &[0x82])), H2_HEADERS, "HEADERS");
        // A frame cut at every byte boundary: every byte of its payload still belongs to it.
        let mut stream = frame(0x8, &[0, 0, 0, 1]);
        stream.extend(frame(0x1, &[0x82, 0x86]));
        stream.extend(frame(0x4, &[]));
        let mut f = H2Frames::default();
        let kinds: Vec<u16> = stream.chunks(1).map(|c| f.scan(c)).collect();
        assert_eq!(kinds.iter().filter(|k| **k & H2_HEADERS != 0).count(), 3, "the byte that completes the header and the payload");
        assert_eq!(kinds.iter().filter(|k| **k & (H2_DATA | H2_HEADERS | H2_CONTINUATION) != 0).count(), 3);
        assert!(kinds[..13].iter().all(|k| *k & H2_HEADERS == 0), "bytes of the WINDOW_UPDATE are not a response");
    }

    fn headers(pairs: &[(&str, &str)]) -> http::HeaderMap {
        let mut h = http::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(http::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
        }
        h
    }

    /// A wrong `Content-Length` set by a handler must not frame a second response on a keep-alive connection.
    #[test]
    fn content_length_always_matches_a_body_of_known_size() {
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "5");
        let out = into_hyper(resp, false);
        assert_eq!(out.headers().get("content-length").unwrap(), "20");
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "20");
        assert_eq!(into_hyper(resp, false).headers().get("content-length").unwrap(), "20");
        let resp = Response::bytes(vec![b'x'; 20], "image/png").with_header("Content-Length", "junk");
        assert_eq!(into_hyper(resp, false).headers().get("content-length").unwrap(), "20");
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

//! The options of the HTTP server (`ServerOptions`).

use std::time::Duration;

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
    pub(crate) shutdown_timeout: Option<Duration>,
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
            shutdown_timeout: Some(Duration::from_secs(10)),
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
    /// Longest the server waits, after its shutdown signal, for the requests in flight to finish (default 10 s;
    /// `None` waits without limit, and so does a value too long to add to an `Instant`).
    ///
    /// On the signal the server stops accepting, closes idle connections and lets the requests in flight finish. When
    /// the time is up, whatever the server started and is still running is cancelled and awaited before the shutdown
    /// goes on: connections (their handlers are dropped at the `.await` they were waiting on, so a database transaction
    /// they had open is never committed), TLS handshakes and the reads that discard unread request bodies. One warning
    /// line says how many connections were cut. Only then do the application's shutdown hooks
    /// (`on_module_destroy`, `before_application_shutdown`, `on_application_shutdown`) run, so no handler of the server
    /// uses a provider those hooks already closed.
    ///
    /// Not covered: tasks the application spawns itself (`tokio::spawn` inside a handler), upgraded connections
    /// (WebSockets), and code that blocks a worker thread without awaiting (`spawn_blocking` work runs to its end).
    /// A handler that never reaches an `.await` cannot be cancelled: the shutdown waits for it beyond this deadline.
    /// With `None`, a client that keeps a response open (a stream, a slow read) decides how long the shutdown takes.
    /// A process manager usually kills a process some seconds after its stop signal: keep this below that.
    pub fn shutdown_timeout(mut self, t: impl Into<Option<Duration>>) -> Self {
        self.shutdown_timeout = t.into();
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

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::time::Instant;

    #[test]
    fn send_timeout_defaults_to_sixty_seconds_and_can_be_turned_off() {
        assert_eq!(ServerOptions::default().send_timeout, Some(Duration::from_secs(60)));
        assert_eq!(ServerOptions::new().send_timeout(None).send_timeout, None);
        assert_eq!(ServerOptions::new().send_timeout(Duration::from_secs(5)).send_timeout, Some(Duration::from_secs(5)));
    }

    #[test]
    fn a_timeout_too_long_for_an_instant_is_off() {
        assert!(Instant::now().checked_add(Duration::MAX).is_none());
        let options = ServerOptions::new().send_timeout(Duration::MAX).headers_timeout(Duration::MAX).keep_alive_timeout(Duration::MAX);
        assert_eq!(options.send_timeout, Some(Duration::MAX));
    }

    #[test]
    fn max_pending_response_bytes_defaults_to_eight_mib_and_can_be_turned_off() {
        assert_eq!(ServerOptions::default().max_pending_response_bytes, Some(8 * 1024 * 1024));
        assert_eq!(ServerOptions::new().max_pending_response_bytes(None).max_pending_response_bytes, None);
        assert_eq!(ServerOptions::new().max_pending_response_bytes(1024).max_pending_response_bytes, Some(1024));
    }

    #[test]
    fn shutdown_timeout_defaults_to_ten_seconds_and_can_be_turned_off() {
        assert_eq!(ServerOptions::new().shutdown_timeout, Some(Duration::from_secs(10)));
        assert_eq!(ServerOptions::new().shutdown_timeout(None).shutdown_timeout, None);
        assert_eq!(ServerOptions::new().shutdown_timeout(Duration::from_secs(1)).shutdown_timeout, Some(Duration::from_secs(1)));
    }
}

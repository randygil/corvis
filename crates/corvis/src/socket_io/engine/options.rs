//! Options of an Engine.IO server, with Node's `engine.io` defaults and a bounded default for every limit.

use std::time::Duration;

use crate::router::cors::CorsOptions;

/// The default path of the Socket.IO server (`/socket.io/`).
pub const DEFAULT_PATH: &str = "/socket.io/";
/// `pingInterval` of Node's `engine.io`.
pub const DEFAULT_PING_INTERVAL: Duration = Duration::from_millis(25_000);
/// `pingTimeout` of Node's `engine.io`.
pub const DEFAULT_PING_TIMEOUT: Duration = Duration::from_millis(20_000);
/// `upgradeTimeout` of Node's `engine.io`.
pub const DEFAULT_UPGRADE_TIMEOUT: Duration = Duration::from_millis(10_000);
/// `maxHttpBufferSize` of Node's `engine.io` (1e6 bytes).
pub const DEFAULT_MAX_HTTP_BUFFER_SIZE: usize = 1_000_000;
/// Packets accepted in one polling `POST`. Node has no limit.
pub const DEFAULT_MAX_PAYLOAD_PACKETS: usize = 10_000;
/// Open sessions per server. Node has no limit.
pub const DEFAULT_MAX_SESSIONS: usize = 10_000;
/// Open sessions per client address. Node has no limit.
pub const DEFAULT_MAX_SESSIONS_PER_IP: usize = 256;
/// Received bytes waiting for the application, across every session of a server. Node has no limit.
pub const DEFAULT_MAX_TOTAL_UNREAD_BYTES: usize = 256 * 1024 * 1024;
/// The longest a first request timeout (see [`EngineOptions::first_request_timeout`]) may be by default.
pub const DEFAULT_FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// The largest value accepted for a duration option: a day. Larger values only ever meant "never" and overflow
/// the clock arithmetic of the timers.
pub const MAX_DURATION: Duration = Duration::from_secs(24 * 60 * 60);

/// What an [`allow_request`](EngineOptions::allow_request) hook decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// Let the handshake proceed.
    Allow,
    /// Refuse it: `403` with `{"code":4,"message":"Forbidden"}`.
    Reject,
}

type AllowFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Admission> + Send>>;

/// The `allow_request` hook, type erased.
#[derive(Clone)]
pub(crate) struct AllowRequestHook(pub(crate) std::sync::Arc<dyn Fn(&crate::http::Request) -> AllowFuture + Send + Sync>);

impl std::fmt::Debug for AllowRequestHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AllowRequestHook")
    }
}

/// Why a set of [`EngineOptions`] was refused when attaching the server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineOptionsError {
    /// A numeric option that must be positive was zero.
    #[error("Engine.IO option `{option}` must not be zero")]
    Zero { option: &'static str },
    /// A value outside its allowed range.
    #[error("Engine.IO option `{option}` {reason}")]
    Range { option: &'static str, reason: String },
    /// The path does not start with `/`.
    #[error("Engine.IO option `path` must start with `/`, got {path:?}")]
    Path { path: String },
    /// Another attached Engine.IO server already takes this path (or one that contains it).
    #[error("Engine.IO option `path` {path:?} overlaps the path {other:?} of another attached server")]
    Overlap { path: String, other: String },
}

/// Options of an Engine.IO server, modelled after the options of Node's `engine.io`.
///
/// Every limit has a bounded default, and zero is refused. Where Node has no bound, the table says so: these are
/// the places where Corvis deliberately differs, each listed in the harness's `expected-differences.json`.
///
/// | Option | Default | Node `engine.io` 6.6.x |
/// |---|---|---|
/// | [`path`](Self::path) | `/socket.io/` | `/engine.io/` (Socket.IO sets `/socket.io/`) |
/// | [`ping_interval`](Self::ping_interval) | 25 s | same |
/// | [`ping_timeout`](Self::ping_timeout) | 20 s | same |
/// | [`upgrade_timeout`](Self::upgrade_timeout) | 10 s | same |
/// | [`max_http_buffer_size`](Self::max_http_buffer_size) | 1,000,000 bytes | same |
/// | [`max_payload_packets`](Self::max_payload_packets) | 10,000 | none |
/// | [`max_buffered_amount`](Self::max_buffered_amount) | 8 MiB | none |
/// | [`max_unread_bytes`](Self::max_unread_bytes) | 16 MiB | none |
/// | [`max_sessions`](Self::max_sessions) | 10,000 | none |
/// | [`max_sessions_per_ip`](Self::max_sessions_per_ip) | 256 | none |
/// | [`max_total_unread_bytes`](Self::max_total_unread_bytes) | 256 MiB | none |
/// | [`first_request_timeout`](Self::first_request_timeout) | 10 s (at most `ping_interval + ping_timeout`) | none |
/// | [`allow_request`](Self::allow_request) | off | off |
/// | [`close_timeout`](Self::close_timeout) | 30 s | 30 s |
/// | [`cors`](Self::cors) | off | off |
/// | [`allowed_origins`](Self::allowed_origins) | off | none |
///
/// Other differences: a polling response carries at most `max_http_buffer_size` bytes of packets (Node sends the
/// whole buffer); session ids have 120 random bits (Node: 96 plus a counter); polling responses are not compressed
/// (Node compresses above 1 KiB when the client accepts it); Engine.IO v3, cookies and WebTransport are not supported.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub(crate) path: String,
    pub(crate) ping_interval: Duration,
    pub(crate) ping_timeout: Duration,
    pub(crate) upgrade_timeout: Duration,
    pub(crate) max_http_buffer_size: usize,
    pub(crate) max_payload_packets: usize,
    pub(crate) max_buffered_amount: usize,
    pub(crate) max_unread_bytes: usize,
    pub(crate) max_sessions: usize,
    pub(crate) max_sessions_per_ip: usize,
    pub(crate) max_total_unread_bytes: usize,
    pub(crate) first_request_timeout: Option<Duration>,
    pub(crate) allow_request: Option<AllowRequestHook>,
    pub(crate) close_timeout: Duration,
    pub(crate) cors: Option<CorsOptions>,
    pub(crate) allowed_origins: Option<Vec<String>>,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions {
            path: DEFAULT_PATH.to_string(),
            ping_interval: DEFAULT_PING_INTERVAL,
            ping_timeout: DEFAULT_PING_TIMEOUT,
            upgrade_timeout: DEFAULT_UPGRADE_TIMEOUT,
            max_http_buffer_size: DEFAULT_MAX_HTTP_BUFFER_SIZE,
            max_payload_packets: DEFAULT_MAX_PAYLOAD_PACKETS,
            max_buffered_amount: crate::ws::DEFAULT_MAX_BUFFERED_AMOUNT,
            max_unread_bytes: crate::ws::DEFAULT_MAX_UNREAD_BYTES,
            max_sessions: DEFAULT_MAX_SESSIONS,
            max_sessions_per_ip: DEFAULT_MAX_SESSIONS_PER_IP,
            max_total_unread_bytes: DEFAULT_MAX_TOTAL_UNREAD_BYTES,
            first_request_timeout: None,
            allow_request: None,
            close_timeout: crate::ws::DEFAULT_CLOSE_TIMEOUT,
            cors: None,
            allowed_origins: None,
        }
    }
}

impl EngineOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The path the server takes over, with the prefix rule of Node's `attach`: a request whose path equals it or
    /// starts with it is served by Engine.IO. A trailing slash is added when missing, as Node does.
    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// `pingInterval`: how long the server waits after the last pong before the next ping.
    pub fn ping_interval(mut self, interval: Duration) -> Self {
        self.ping_interval = interval;
        self
    }

    /// `pingTimeout`: how long the server waits for the pong before closing the session with `ping timeout`.
    pub fn ping_timeout(mut self, timeout: Duration) -> Self {
        self.ping_timeout = timeout;
        self
    }

    /// `upgradeTimeout`: how long a WebSocket candidate has to complete the upgrade.
    pub fn upgrade_timeout(mut self, timeout: Duration) -> Self {
        self.upgrade_timeout = timeout;
        self
    }

    /// `maxHttpBufferSize`: the largest polling `POST` body and WebSocket message accepted, also advertised to the
    /// client as `maxPayload`, and the most a polling response carries.
    pub fn max_http_buffer_size(mut self, bytes: usize) -> Self {
        self.max_http_buffer_size = bytes;
        self
    }

    /// The most packets one polling `POST` may carry. A body with more is refused as a whole and closes the
    /// session with `parse error`. Node has no limit.
    pub fn max_payload_packets(mut self, packets: usize) -> Self {
        self.max_payload_packets = packets;
        self
    }

    /// The most bytes queued for one session and not yet written. A send above it fails and closes the session
    /// with `transport error`. Node has no limit.
    pub fn max_buffered_amount(mut self, bytes: usize) -> Self {
        self.max_buffered_amount = bytes;
        self
    }

    /// The most received bytes of one session waiting for the application. Above it the WebSocket stops being
    /// read and a polling `POST` is not answered. Node has no limit.
    pub fn max_unread_bytes(mut self, bytes: usize) -> Self {
        self.max_unread_bytes = bytes;
        self
    }

    /// The most open sessions of the server. A handshake above it is answered `503`. Node has no limit.
    pub fn max_sessions(mut self, sessions: usize) -> Self {
        self.max_sessions = sessions;
        self
    }

    /// The most open sessions per client address (the request's `ip()`, so `set_trust_proxy` is honored; IPv6
    /// addresses count per `/64`). A handshake above it is answered `503` like the global cap. Node has no limit.
    ///
    /// Clients behind one NAT or proxy share an address: raise this for a deployment where many users come from
    /// few addresses (a company gateway, a CDN that is not a trusted proxy), or turn on `set_trust_proxy` so the
    /// real client address is used.
    pub fn max_sessions_per_ip(mut self, sessions: usize) -> Self {
        self.max_sessions_per_ip = sessions;
        self
    }

    /// The most received bytes waiting for the application across every session of the server; while it is
    /// exceeded, every WebSocket stops being read and every polling `POST` is held, as for `max_unread_bytes`.
    /// Must be at least `max_unread_bytes`. Node has no limit.
    pub fn max_total_unread_bytes(mut self, bytes: usize) -> Self {
        self.max_total_unread_bytes = bytes;
        self
    }

    /// How long a polling session may go without any request after its handshake before it is closed and its slot
    /// released. WebSocket-first sessions are not affected. Defaults to 10 s, or `ping_interval + ping_timeout`
    /// when that is shorter; an explicit value must be above zero and at most `ping_interval + ping_timeout`.
    /// Node has no such timer (the session ends at its first ping timeout instead).
    pub fn first_request_timeout(mut self, timeout: Duration) -> Self {
        self.first_request_timeout = Some(timeout);
        self
    }

    /// Node's `allowRequest`: a hook that runs before a session is created, for polling handshakes and
    /// WebSocket-first connections, with the handshake request (`ip()` follows `set_trust_proxy`). Returning
    /// [`Admission::Reject`] answers `403` with `{"code":4,"message":"Forbidden"}`. Requests of existing sessions do
    /// not go through it (Node runs it for those too). A panic in the hook rejects the request. This is where an
    /// application plugs a rate limiter, since requests under the Engine.IO path never reach guards or throttlers.
    ///
    /// ```ignore
    /// EngineOptions::new().allow_request(|req| {
    ///     let ip = req.ip();
    ///     async move { if limiter.check(&ip) { Admission::Allow } else { Admission::Reject } }
    /// })
    /// ```
    pub fn allow_request<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(&crate::http::Request) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Admission> + Send + 'static,
    {
        self.allow_request = Some(AllowRequestHook(std::sync::Arc::new(move |req| Box::pin(hook(req)))));
        self
    }

    /// The first-request timeout in force.
    pub(crate) fn effective_first_request_timeout(&self) -> Duration {
        self.first_request_timeout
            .unwrap_or_else(|| DEFAULT_FIRST_REQUEST_TIMEOUT.min(self.ping_interval.saturating_add(self.ping_timeout)))
    }

    /// How long a closing session waits for its transport (a poll that never comes, a WebSocket closing handshake).
    pub fn close_timeout(mut self, timeout: Duration) -> Self {
        self.close_timeout = timeout;
        self
    }

    /// The `cors` option of Node's `engine.io`: CORS headers and preflight answers for the polling transport.
    /// Requests under the Engine.IO path never reach the router, so the application's `enable_cors` does not
    /// apply to them. Off by default.
    pub fn cors(mut self, cors: CorsOptions) -> Self {
        self.cors = Some(cors);
        self
    }

    /// Refuses handshakes (polling and WebSocket) whose `Origin` is not in the list (compared ignoring ASCII
    /// case) with `403` and code 4. Requests without `Origin` are accepted.
    pub fn allowed_origins<I: IntoIterator<Item = S>, S: Into<String>>(mut self, origins: I) -> Self {
        self.allowed_origins = Some(origins.into_iter().map(Into::into).collect());
        self
    }

    /// Checks the options; the server does this when it is created.
    pub fn validate(&self) -> Result<(), EngineOptionsError> {
        let durations = [
            ("ping_interval", self.ping_interval),
            ("ping_timeout", self.ping_timeout),
            ("upgrade_timeout", self.upgrade_timeout),
            ("close_timeout", self.close_timeout),
        ];
        for (option, value) in durations {
            if value.is_zero() {
                return Err(EngineOptionsError::Zero { option });
            }
            if value > MAX_DURATION {
                return Err(EngineOptionsError::Range { option, reason: format!("must be at most one day, got {value:?}") });
            }
        }
        if let Some(timeout) = self.first_request_timeout {
            if timeout.is_zero() {
                return Err(EngineOptionsError::Zero { option: "first_request_timeout" });
            }
            if timeout > self.ping_interval + self.ping_timeout {
                return Err(EngineOptionsError::Range {
                    option: "first_request_timeout",
                    reason: format!(
                        "must be at most ping_interval + ping_timeout ({:?}), got {timeout:?}",
                        self.ping_interval + self.ping_timeout
                    ),
                });
            }
        }
        let sizes = [
            ("max_http_buffer_size", self.max_http_buffer_size),
            ("max_payload_packets", self.max_payload_packets),
            ("max_buffered_amount", self.max_buffered_amount),
            ("max_unread_bytes", self.max_unread_bytes),
            ("max_sessions", self.max_sessions),
            ("max_sessions_per_ip", self.max_sessions_per_ip),
            ("max_total_unread_bytes", self.max_total_unread_bytes),
        ];
        for (option, value) in sizes {
            if value == 0 {
                return Err(EngineOptionsError::Zero { option });
            }
        }
        if self.max_total_unread_bytes < self.max_unread_bytes {
            return Err(EngineOptionsError::Range {
                option: "max_total_unread_bytes",
                reason: format!("must be at least max_unread_bytes ({}), got {}", self.max_unread_bytes, self.max_total_unread_bytes),
            });
        }
        if !self.path.starts_with('/') {
            return Err(EngineOptionsError::Path { path: self.path.clone() });
        }
        Ok(())
    }

    /// The path with Node's normalization: trailing slashes collapsed to exactly one.
    pub(crate) fn normalized_path(&self) -> String {
        let mut path = self.path.trim_end_matches('/').to_string();
        path.push('/');
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_node_s_and_bounded() {
        let o = EngineOptions::default();
        assert_eq!(o.path, "/socket.io/");
        assert_eq!(o.ping_interval, Duration::from_millis(25_000));
        assert_eq!(o.ping_timeout, Duration::from_millis(20_000));
        assert_eq!(o.upgrade_timeout, Duration::from_millis(10_000));
        assert_eq!(o.max_http_buffer_size, 1_000_000);
        assert_eq!(o.max_payload_packets, 10_000);
        assert_eq!(o.max_buffered_amount, 8 * 1024 * 1024);
        assert_eq!(o.max_unread_bytes, 16 * 1024 * 1024);
        assert_eq!(o.max_sessions, 10_000);
        assert_eq!(o.close_timeout, Duration::from_secs(30));
        assert!(o.cors.is_none() && o.allowed_origins.is_none());
        assert!(o.validate().is_ok());
    }

    #[test]
    fn every_zero_value_is_refused_by_name() {
        let cases: Vec<(&str, EngineOptions)> = vec![
            ("ping_interval", EngineOptions::new().ping_interval(Duration::ZERO)),
            ("ping_timeout", EngineOptions::new().ping_timeout(Duration::ZERO)),
            ("upgrade_timeout", EngineOptions::new().upgrade_timeout(Duration::ZERO)),
            ("close_timeout", EngineOptions::new().close_timeout(Duration::ZERO)),
            ("max_http_buffer_size", EngineOptions::new().max_http_buffer_size(0)),
            ("max_payload_packets", EngineOptions::new().max_payload_packets(0)),
            ("max_buffered_amount", EngineOptions::new().max_buffered_amount(0)),
            ("max_unread_bytes", EngineOptions::new().max_unread_bytes(0)),
            ("max_sessions", EngineOptions::new().max_sessions(0)),
        ];
        for (name, options) in cases {
            let err = options.validate().unwrap_err();
            assert_eq!(err, EngineOptionsError::Zero { option: name });
            assert!(err.to_string().contains(name));
        }
    }

    #[test]
    fn a_path_must_start_with_a_slash() {
        let err = EngineOptions::new().path("socket.io").validate().unwrap_err();
        assert!(matches!(err, EngineOptionsError::Path { .. }));
        assert!(err.to_string().contains("path"));
    }

    #[test]
    fn the_path_gets_exactly_one_trailing_slash() {
        assert_eq!(EngineOptions::new().path("/a").normalized_path(), "/a/");
        assert_eq!(EngineOptions::new().path("/a///").normalized_path(), "/a/");
        assert_eq!(EngineOptions::new().normalized_path(), "/socket.io/");
    }

    #[test]
    fn the_new_bounds_have_their_defaults() {
        let o = EngineOptions::default();
        assert_eq!(o.max_sessions_per_ip, 256);
        assert_eq!(o.max_total_unread_bytes, 256 * 1024 * 1024);
        assert_eq!(o.effective_first_request_timeout(), Duration::from_secs(10));
        // With short pings the default follows them.
        let fast = EngineOptions::new().ping_interval(Duration::from_millis(300)).ping_timeout(Duration::from_millis(200));
        assert!(fast.validate().is_ok());
        assert_eq!(fast.effective_first_request_timeout(), Duration::from_millis(500));
        assert!(o.allow_request.is_none());
    }

    #[test]
    fn the_new_options_refuse_zero_and_out_of_range_values_by_name() {
        for (name, options) in [
            ("max_sessions_per_ip", EngineOptions::new().max_sessions_per_ip(0)),
            ("max_total_unread_bytes", EngineOptions::new().max_total_unread_bytes(0)),
            ("first_request_timeout", EngineOptions::new().first_request_timeout(Duration::ZERO)),
        ] {
            assert_eq!(options.validate().unwrap_err(), EngineOptionsError::Zero { option: name });
        }
        let too_long = EngineOptions::new().first_request_timeout(Duration::from_secs(46));
        let err = too_long.validate().unwrap_err();
        assert!(matches!(err, EngineOptionsError::Range { option: "first_request_timeout", .. }), "{err}");
        let small_total = EngineOptions::new().max_unread_bytes(1000).max_total_unread_bytes(999);
        assert!(matches!(small_total.validate().unwrap_err(), EngineOptionsError::Range { option: "max_total_unread_bytes", .. }));
        assert!(EngineOptions::new().max_unread_bytes(1000).max_total_unread_bytes(1000).validate().is_ok());
    }

    #[test]
    fn durations_above_a_day_are_refused_by_name() {
        let day = Duration::from_secs(24 * 3600);
        assert!(EngineOptions::new().ping_interval(day).validate().is_ok());
        for (name, options) in [
            ("ping_interval", EngineOptions::new().ping_interval(day + Duration::from_secs(1))),
            ("ping_timeout", EngineOptions::new().ping_timeout(Duration::MAX)),
            ("upgrade_timeout", EngineOptions::new().upgrade_timeout(Duration::from_secs(u64::MAX))),
            ("close_timeout", EngineOptions::new().close_timeout(Duration::MAX)),
        ] {
            let err = options.validate().unwrap_err();
            assert!(matches!(err, EngineOptionsError::Range { option, .. } if option == name), "{name}: {err}");
        }
    }
}

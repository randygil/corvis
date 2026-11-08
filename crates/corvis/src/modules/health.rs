//! Health checks (`TerminusModule`, `HealthCheckService` and the health indicators).
//!
//! Every indicator runs with a time limit (5 seconds, or `HealthIndicator::timeout`); one that exceeds it is
//! reported `down` and the others are still reported. `HttpHealthIndicator::ping_check` makes a real HTTP request,
//! `tcp_check` only opens a TCP connection, and `MemoryHealthIndicator::check_rss` is `down` where it cannot measure.
//!
//! ```ignore
//! #[controller("health")]
//! pub struct HealthController { health: Arc<HealthCheckService> }
//!
//! #[routes]
//! impl HealthController {
//!     #[get]
//!     async fn check(&self) -> HttpResult<Value> {
//!         self.health.check(vec![
//!             HealthIndicator::new("database", || async { Ok(json!({})) }),
//!             MemoryHealthIndicator::check_rss("memory_rss", 300 * 1024 * 1024),
//!         ]).await
//!     }
//! }
//! ```

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::FutureExt;
use futures::future::BoxFuture;
use http_body_util::Empty;
use hyper_util::rt::TokioIo;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::di::{Module, ModuleDefinition, Provider};
use crate::http::HttpException;

type Check = Arc<dyn Fn() -> BoxFuture<'static, Result<Value, Value>> + Send + Sync>;

/// Time limit applied to an indicator that was not given its own with [`HealthIndicator::timeout`].
pub const DEFAULT_INDICATOR_TIMEOUT: Duration = Duration::from_secs(5);

/// A named health indicator. `Ok(details)` = up, `Err(details)` = down.
#[derive(Clone)]
pub struct HealthIndicator {
    key: String,
    check: Check,
    timeout: Duration,
}

impl HealthIndicator {
    pub fn new<F, Fut>(key: impl Into<String>, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value, Value>> + Send + 'static,
    {
        let f = Arc::new(f);
        HealthIndicator {
            key: key.into(),
            check: Arc::new(move || {
                let f = f.clone();
                async move { f().await }.boxed()
            }),
            timeout: DEFAULT_INDICATOR_TIMEOUT,
        }
    }

    /// Sets the time limit of this indicator (default [`DEFAULT_INDICATOR_TIMEOUT`], 5 seconds). An indicator that
    /// does not finish in time is reported `down` with `"timeout of <ms>ms exceeded"`.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// The one place where an indicator runs: applies its time limit.
async fn run_indicator(ind: &HealthIndicator) -> Result<Value, Value> {
    match tokio::time::timeout(ind.timeout, (ind.check)()).await {
        Ok(res) => res,
        Err(_) => Err(down(format!("timeout of {}ms exceeded", ind.timeout.as_millis()))),
    }
}

const DEFAULT_MAX_REDIRECTS: usize = 5;

/// Options of [`HttpHealthIndicator::ping_check_with`].
///
/// By default up to 5 redirects are followed, and only within the same origin (an `http` to `https` upgrade of the
/// same host is also allowed). A redirect from `https` to `http` is never followed.
#[derive(Debug, Clone)]
pub struct PingOptions {
    max_redirects: usize,
    follow_cross_origin: bool,
}

impl Default for PingOptions {
    fn default() -> Self {
        PingOptions { max_redirects: DEFAULT_MAX_REDIRECTS, follow_cross_origin: false }
    }
}

impl PingOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Most redirects to follow; `0` follows none, so a redirect response is reported `down` like any non-2xx one.
    pub fn max_redirects(mut self, n: usize) -> Self {
        self.max_redirects = n;
        self
    }

    /// Follow redirects to another origin (scheme, host or port). Off by default. Even when on, a redirect to a
    /// loopback, link-local or private address is refused unless the configured URL already pointed to one, and an
    /// `https` to `http` redirect is refused.
    pub fn follow_cross_origin(mut self, follow: bool) -> Self {
        self.follow_cross_origin = follow;
        self
    }
}

/// Built-in indicators for HTTP endpoints and TCP services.
pub struct HttpHealthIndicator;

impl HttpHealthIndicator {
    /// Sends an HTTP `GET` to `url` (`http://`, or `https://` with the `health-tls` feature) and is `up` only when
    /// the final response is 2xx. Up to 5 same-origin redirects are followed (see [`PingOptions`] to change that).
    /// When a response arrived but is not 2xx, the details include `statusCode` and `statusText`.
    ///
    /// A value without `http://` or `https://` (such as `host:port`) is reported `down`: use
    /// [`tcp_check`](Self::tcp_check) for plain TCP addresses.
    ///
    /// The `down` details can contain the host and port of the target and operating system error text, so a public
    /// health endpoint exposes them.
    pub fn ping_check(key: impl Into<String>, url: impl Into<String>) -> HealthIndicator {
        Self::ping_check_with(key, url, PingOptions::default())
    }

    /// [`ping_check`](Self::ping_check) with explicit redirect options.
    pub fn ping_check_with(key: impl Into<String>, url: impl Into<String>, options: PingOptions) -> HealthIndicator {
        let url = url.into();
        HealthIndicator::new(key, move || {
            let url = url.clone();
            let options = options.clone();
            async move { http_ping(&url, &options).await }
        })
    }

    /// Opens a TCP connection to `addr` (`host:port`); `up` when it connects. It does not send any request, so it
    /// suits services without HTTP (Redis, SMTP).
    pub fn tcp_check(key: impl Into<String>, addr: impl Into<String>) -> HealthIndicator {
        let addr = addr.into();
        HealthIndicator::new(key, move || {
            let addr = addr.clone();
            async move {
                match tokio::net::TcpStream::connect(&addr).await {
                    Ok(_) => Ok(json!({})),
                    Err(e) => Err(json!({ "message": e.to_string() })),
                }
            }
        })
    }
}

fn down(message: impl Into<String>) -> Value {
    json!({ "message": message.into() })
}

fn response_failure(status: http::StatusCode, message: Option<&str>) -> Value {
    let message = match message {
        Some(m) => m.to_string(),
        None => format!("Request failed with status code {}", status.as_u16()),
    };
    json!({
        "message": message,
        "statusCode": status.as_u16(),
        "statusText": status.canonical_reason().unwrap_or(""),
    })
}

fn is_internal(host: &url::Host<&str>) -> bool {
    match host {
        url::Host::Domain(d) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            d == "localhost" || d.ends_with(".localhost")
        }
        url::Host::Ipv4(a) => is_internal_v4(a),
        url::Host::Ipv6(a) => {
            let seg = a.segments();
            // IPv4-mapped (`::ffff:a.b.c.d`), IPv4-compatible (`::a.b.c.d`) and NAT64 (`64:ff9b::/96`) addresses
            // reach the embedded IPv4 address on dual-stack hosts, so they are judged by it.
            let embedded = if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let o = a.octets();
                Some(std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15]))
            } else {
                a.to_ipv4()
            };
            if let Some(v4) = embedded.filter(|v4| !v4.is_unspecified() || a.is_unspecified()) {
                return is_internal_v4(&v4);
            }
            a.is_loopback() || a.is_unspecified() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Loopback, private, link-local and the other ranges that are not public unicast destinations.
fn is_internal_v4(a: &std::net::Ipv4Addr) -> bool {
    let [o0, o1, o2, _] = a.octets();
    a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_broadcast()
        || o0 == 0 // 0.0.0.0/8 ("this network")
        || (o0 == 100 && (o1 & 0xc0) == 64) // 100.64.0.0/10 (shared address space, cloud metadata)
        || (o0 == 192 && o1 == 0 && o2 == 0) // 192.0.0.0/24 (protocol assignments)
        || (o0 == 198 && (o1 & 0xfe) == 18) // 198.18.0.0/15 (benchmarking)
}

/// A host for an error message, cut to a readable length: after a redirect it comes from the target server.
fn shown_host(host: &url::Host<&str>) -> String {
    let h = host.to_string();
    if h.chars().count() > 64 { format!("{}…", h.chars().take(64).collect::<String>()) } else { h }
}

fn same_origin_or_upgrade(from: &url::Url, to: &url::Url) -> bool {
    if from.host_str() != to.host_str() {
        return false;
    }
    let same_port = from.port_or_known_default() == to.port_or_known_default();
    match (from.scheme(), to.scheme()) {
        (a, b) if a == b => same_port,
        ("http", "https") => to.port().is_none(),
        _ => false,
    }
}

/// Why a redirect from `from` to `next` must not be followed, if it must not. `origin` is the configured URL.
fn redirect_refusal(origin: &url::Url, from: &url::Url, next: &url::Url, options: &PingOptions) -> Option<&'static str> {
    if !matches!(next.scheme(), "http" | "https") || next.host().is_none() {
        return Some("redirect to an unsupported URL was not followed");
    }
    if from.scheme() == "https" && next.scheme() == "http" {
        return Some("redirect from https to http was not followed");
    }
    if !options.follow_cross_origin && !same_origin_or_upgrade(from, next) {
        return Some("redirect to another origin was not followed (see PingOptions::follow_cross_origin)");
    }
    let next_internal = next.host().is_some_and(|h| is_internal(&h));
    let origin_internal = origin.host().is_some_and(|h| is_internal(&h));
    if next_internal && !origin_internal {
        return Some("redirect to an internal address was not followed");
    }
    None
}

async fn http_ping(start: &str, options: &PingOptions) -> Result<Value, Value> {
    // The configured URL can carry credentials (`user:pass@`), so it is never echoed in a message.
    let origin = match url::Url::parse(start) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => return Err(down("invalid URL: expected http:// or https:// (use tcp_check for host:port)")),
    };
    let mut url = origin.clone();
    let mut redirects = 0;
    loop {
        let (status, location) = request(&url).await?;
        if status.is_success() {
            return Ok(json!({}));
        }
        let redirect = matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308);
        let Some(location) = location.filter(|_| redirect && options.max_redirects > 0) else {
            return Err(response_failure(status, None));
        };
        if redirects >= options.max_redirects {
            return Err(down("maximum number of redirects exceeded"));
        }
        redirects += 1;
        // The `Location` value is attacker-controlled data: it never goes into a message.
        let Ok(next) = url.join(&location) else {
            return Err(response_failure(status, Some("redirect with an invalid location was not followed")));
        };
        if let Some(reason) = redirect_refusal(&origin, &url, &next, options) {
            return Err(response_failure(status, Some(reason)));
        }
        url = next;
    }
}

/// One `GET` request without redirects. Returns the status and the `Location` header, if any.
async fn request(url: &url::Url) -> Result<(http::StatusCode, Option<String>), Value> {
    let host = url.host().ok_or_else(|| down("invalid URL: no host"))?;
    let https = url.scheme() == "https";
    let port = url.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let tcp = match &host {
        url::Host::Domain(d) => tokio::net::TcpStream::connect((*d, port)).await,
        url::Host::Ipv4(a) => tokio::net::TcpStream::connect((*a, port)).await,
        url::Host::Ipv6(a) => tokio::net::TcpStream::connect((*a, port)).await,
    }
    .map_err(|e| down(format!("connect to {}:{port} failed: {e}", shown_host(&host))))?;
    if https {
        #[cfg(feature = "health-tls")]
        {
            let io = tls::connect(tcp, &host.to_string()).await.map_err(down)?;
            return send(io, url).await;
        }
        #[cfg(not(feature = "health-tls"))]
        return Err(down("https URLs need the corvis \"health-tls\" feature; no request was sent"));
    }
    send(tcp, url).await
}

async fn send<S>(io: S, url: &url::Url) -> Result<(http::StatusCode, Option<String>), Value>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(io)).await.map_err(|e| down(e.to_string()))?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    let mut target = url.path().to_string();
    if let Some(q) = url.query() {
        target.push('?');
        target.push_str(q);
    }
    let host_header = match url.port() {
        Some(p) => format!("{}:{p}", url.host_str().unwrap_or("")),
        None => url.host_str().unwrap_or("").to_string(),
    };
    let req = http::Request::builder()
        .method("GET")
        .uri(target)
        .header("host", host_header)
        .header("user-agent", concat!("corvis-health/", env!("CARGO_PKG_VERSION")))
        .header("accept", "application/json, text/plain, */*")
        .header("connection", "close")
        .body(Empty::<Bytes>::new())
        .map_err(|e| down(e.to_string()))?;
    let res = sender.send_request(req).await;
    driver.abort();
    let res = res.map_err(|e| down(e.to_string()))?;
    let location = res.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
    Ok((res.status(), location))
}

#[cfg(feature = "health-tls")]
mod tls {
    use std::sync::Arc;

    use tokio::net::TcpStream;
    use tokio_rustls::TlsConnector;
    use tokio_rustls::client::TlsStream;
    use tokio_rustls::rustls::crypto::{CryptoProvider, ring};
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    pub(super) async fn connect(tcp: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, String> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        // `ClientConfig::builder()` panics when no process-wide provider is installed and both `ring` and `aws-lc-rs`
        // are compiled in (any app that also uses reqwest, hyper-rustls or the AWS SDK). Use the installed provider,
        // or ring explicitly, and never panic: a panic here would turn the health endpoint into a 500.
        let provider = CryptoProvider::get_default().cloned().unwrap_or_else(|| Arc::new(ring::default_provider()));
        let config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS configuration failed: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = ServerName::try_from(host.trim_matches(['[', ']']).to_string()).map_err(|e| e.to_string())?;
        TlsConnector::from(Arc::new(config)).connect(name, tcp).await.map_err(|e| format!("TLS handshake failed: {e}"))
    }
}

/// Built-in memory indicator (`MemoryHealthIndicator`).
pub struct MemoryHealthIndicator;

impl MemoryHealthIndicator {
    /// `up` while the resident set size of the process stays at or under `threshold_bytes`. Measured on Linux,
    /// Windows and macOS; where it cannot be measured the indicator is `down`, never a silent `up`.
    pub fn check_rss(key: impl Into<String>, threshold_bytes: u64) -> HealthIndicator {
        HealthIndicator::new(key, move || async move {
            rss_verdict(memory_stats::memory_stats().map(|m| m.physical_mem as u64), threshold_bytes)
        })
    }
}

fn rss_verdict(rss: Option<u64>, threshold_bytes: u64) -> Result<Value, Value> {
    match rss {
        Some(rss) if rss > threshold_bytes => Err(down("Used rss exceeded the set threshold")),
        Some(_) => Ok(json!({})),
        None => Err(down("rss could not be measured on this platform")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    fn refusal(origin: &str, from: &str, next: &str, opts: &PingOptions) -> Option<&'static str> {
        redirect_refusal(&u(origin), &u(from), &u(next), opts)
    }

    #[test]
    fn https_to_http_is_never_followed() {
        let open = PingOptions::new().follow_cross_origin(true);
        let r = refusal("https://a.example/", "https://a.example/", "http://a.example/x", &open);
        assert_eq!(r, Some("redirect from https to http was not followed"));
        let r = refusal("https://a.example/", "https://a.example/", "http://b.example/x", &PingOptions::new());
        assert_eq!(r, Some("redirect from https to http was not followed"));
    }

    #[test]
    fn cross_origin_is_refused_by_default_and_allowed_on_request() {
        let d = PingOptions::new();
        let o = "https://a.example/";
        assert!(refusal(o, o, "https://b.example/", &d).is_some());
        assert!(refusal(o, o, "https://a.example:8443/", &d).is_some());
        assert!(refusal(o, o, "https://a.example/other", &d).is_none());
        assert!(refusal("http://a.example/", "http://a.example/", "https://a.example/", &d).is_none());
        let open = PingOptions::new().follow_cross_origin(true);
        assert!(refusal(o, o, "https://b.example/", &open).is_none());
    }

    #[test]
    fn internal_targets_are_refused_unless_the_origin_was_internal() {
        let open = PingOptions::new().follow_cross_origin(true);
        let o = "http://a.example/";
        for t in [
            "http://127.0.0.1/",
            "http://localhost/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:169.254.169.254]/latest/meta-data",
            "http://[::127.0.0.1]/",
            "http://[64:ff9b::a9fe:a9fe]/",
            "http://100.100.100.200/",
            "http://192.0.0.192/",
            "http://198.18.0.1/",
            "http://0.1.2.3/",
            "http://255.255.255.255/",
        ] {
            assert_eq!(refusal(o, o, t, &open), Some("redirect to an internal address was not followed"), "{t}");
        }
        let internal = "http://10.1.1.1/";
        assert!(refusal(internal, internal, "http://127.0.0.1/", &open).is_none());
        assert!(refusal(o, o, "http://8.8.8.8/", &open).is_none());
        assert!(refusal(o, o, "http://[::ffff:8.8.8.8]/", &open).is_none());
        assert!(refusal(o, o, "http://[2606:4700::1111]/", &open).is_none());
    }

    #[test]
    fn long_hosts_are_cut_in_messages() {
        let long = "a".repeat(60_000);
        let shown = shown_host(&url::Host::Domain(&long));
        assert!(shown.chars().count() <= 65, "{}", shown.len());
    }

    #[test]
    fn unmeasurable_rss_is_down() {
        let res = rss_verdict(None, u64::MAX);
        assert_eq!(res.unwrap_err()["message"], "rss could not be measured on this platform");
    }
}

/// `HealthCheckService`.
#[derive(Default)]
pub struct HealthCheckService;

impl HealthCheckService {
    /// Runs all indicators concurrently, each within its time limit. Returns the Terminus-style body, or a 503 with
    /// the same body.
    pub async fn check(&self, indicators: Vec<HealthIndicator>) -> Result<Value, HttpException> {
        let results = futures::future::join_all(indicators.iter().map(run_indicator)).await;
        let mut info = Map::new();
        let mut error = Map::new();
        let mut details = Map::new();
        for (ind, res) in indicators.iter().zip(results) {
            let (status, extra) = match res {
                Ok(v) => ("up", v),
                Err(v) => ("down", v),
            };
            let mut entry = Map::new();
            entry.insert("status".into(), json!(status));
            if let Value::Object(o) = extra {
                entry.extend(o);
            }
            let entry = Value::Object(entry);
            details.insert(ind.key.clone(), entry.clone());
            if status == "up" {
                info.insert(ind.key.clone(), entry);
            } else {
                error.insert(ind.key.clone(), entry);
            }
        }
        let ok = error.is_empty();
        let body = json!({
            "status": if ok { "ok" } else { "error" },
            "info": info,
            "error": error,
            "details": details,
        });
        if ok { Ok(body) } else { Err(HttpException::with_error(body, "Service Unavailable", http::StatusCode::SERVICE_UNAVAILABLE)) }
    }
}

/// `TerminusModule` – provides `HealthCheckService`.
#[derive(Default)]
pub struct TerminusModule;
crate::default_injectable!(TerminusModule);

impl Module for TerminusModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<TerminusModule>("TerminusModule")
            .module_provider(Provider::class::<TerminusModule>())
            .providers(vec![Provider::value(HealthCheckService)])
            .exports(vec![crate::di::ExportItem::Token(crate::di::Token::of::<HealthCheckService>())])
    }
}

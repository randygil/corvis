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

const MAX_REDIRECTS: usize = 5;

/// Built-in indicators for HTTP endpoints and TCP services.
pub struct HttpHealthIndicator;

impl HttpHealthIndicator {
    /// Sends an HTTP `GET` to `url` (`http://`, or `https://` with the `health-tls` feature) and is `up` only when
    /// the final response is 2xx. Up to 5 redirects are followed. When a response arrived but is not 2xx, the
    /// details include `statusCode` and `statusText`.
    ///
    /// A value without `http://` or `https://` (such as `host:port`) is reported `down`: use
    /// [`tcp_check`](Self::tcp_check) for plain TCP addresses.
    pub fn ping_check(key: impl Into<String>, url: impl Into<String>) -> HealthIndicator {
        let url = url.into();
        HealthIndicator::new(key, move || {
            let url = url.clone();
            async move { http_ping(&url).await }
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

async fn http_ping(start: &str) -> Result<Value, Value> {
    let mut url = match url::Url::parse(start) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => u,
        _ => {
            return Err(down(format!("invalid URL {start:?}: expected http:// or https://; use tcp_check for plain TCP addresses")));
        }
    };
    let mut redirects = 0;
    loop {
        let (status, location) = request(&url).await?;
        if status.is_success() {
            return Ok(json!({}));
        }
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308)
            && let Some(location) = location
        {
            if redirects >= MAX_REDIRECTS {
                return Err(down(format!("Maximum number of redirects exceeded ({MAX_REDIRECTS})")));
            }
            redirects += 1;
            url = url.join(&location).map_err(|e| down(format!("invalid redirect location {location:?}: {e}")))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(down(format!("redirect to unsupported scheme {:?}", url.scheme())));
            }
            continue;
        }
        return Err(json!({
            "message": format!("Request failed with status code {}", status.as_u16()),
            "statusCode": status.as_u16(),
            "statusText": status.canonical_reason().unwrap_or(""),
        }));
    }
}

/// One `GET` request without redirects. Returns the status and the `Location` header, if any.
async fn request(url: &url::Url) -> Result<(http::StatusCode, Option<String>), Value> {
    let host = url.host().ok_or_else(|| down(format!("invalid URL {url}: no host")))?;
    let https = url.scheme() == "https";
    let port = url.port_or_known_default().unwrap_or(if https { 443 } else { 80 });
    let tcp = match &host {
        url::Host::Domain(d) => tokio::net::TcpStream::connect((*d, port)).await,
        url::Host::Ipv4(a) => tokio::net::TcpStream::connect((*a, port)).await,
        url::Host::Ipv6(a) => tokio::net::TcpStream::connect((*a, port)).await,
    }
    .map_err(|e| down(format!("connect to {host}:{port} failed: {e}")))?;
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
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    pub(super) async fn connect(tcp: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, String> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
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

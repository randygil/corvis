//! Health checks (`TerminusModule`, `HealthCheckService` and the health indicators).
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

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Map, Value, json};

use crate::di::{Module, ModuleDefinition, Provider};
use crate::http::HttpException;

type Check = Arc<dyn Fn() -> BoxFuture<'static, Result<Value, Value>> + Send + Sync>;

/// A named health indicator. `Ok(details)` = up, `Err(details)` = down.
#[derive(Clone)]
pub struct HealthIndicator {
    key: String,
    check: Check,
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
        }
    }
}

/// Built-in indicator for HTTP endpoints (`HttpHealthIndicator.pingCheck`).
pub struct HttpHealthIndicator;

impl HttpHealthIndicator {
    /// TCP connect check to `host:port` (timeout 3s).
    pub fn ping_check(key: impl Into<String>, addr: impl Into<String>) -> HealthIndicator {
        let addr = addr.into();
        HealthIndicator::new(key, move || {
            let addr = addr.clone();
            async move {
                match tokio::time::timeout(std::time::Duration::from_secs(3), tokio::net::TcpStream::connect(&addr)).await {
                    Ok(Ok(_)) => Ok(json!({})),
                    Ok(Err(e)) => Err(json!({ "message": e.to_string() })),
                    Err(_) => Err(json!({ "message": "timeout of 3000ms exceeded" })),
                }
            }
        })
    }
}

/// Built-in memory indicator (`MemoryHealthIndicator`). Uses `/proc/self/statm` when available.
pub struct MemoryHealthIndicator;

impl MemoryHealthIndicator {
    pub fn check_rss(key: impl Into<String>, threshold_bytes: u64) -> HealthIndicator {
        HealthIndicator::new(key, move || async move {
            match rss_bytes() {
                Some(rss) if rss > threshold_bytes => Err(json!({ "message": "Used rss exceeded the set threshold".to_string(), "rss": rss })),
                Some(rss) => Ok(json!({ "rss": rss })),
                None => Ok(json!({})),
            }
        })
    }
}

fn rss_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

/// `HealthCheckService`.
#[derive(Default)]
pub struct HealthCheckService;

impl HealthCheckService {
    /// Runs all indicators. Returns the Terminus-style body, or a 503 with the same body.
    pub async fn check(&self, indicators: Vec<HealthIndicator>) -> Result<Value, HttpException> {
        let results = futures::future::join_all(indicators.iter().map(|i| (i.check)())).await;
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

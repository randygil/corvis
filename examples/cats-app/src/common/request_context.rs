use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use corvis::prelude::*;

/// What a request knows about itself, with one instance per HTTP request (`#[injectable(scope = "request")]`).
///
/// It injects the request being handled and derives the request id from the `X-Request-Id` header, or generates one.
/// Handlers and interceptors take it with `Scoped<RequestContext>`; every one of them in the same request receives the
/// same instance.
#[injectable(scope = "request")]
pub struct RequestContext {
    req: Request,
    id: OnceLock<String>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

impl RequestContext {
    /// The id the client sent in `X-Request-Id`, or a generated one.
    pub fn request_id(&self) -> &str {
        self.id.get_or_init(|| match self.req.header("x-request-id") {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => {
                let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
                format!("req-{nanos:x}-{:x}", NEXT.fetch_add(1, Ordering::Relaxed))
            }
        })
    }
}

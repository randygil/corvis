//! CORS (`app.enableCors(options)`), with the semantics of the `cors` package used by
//! the Express platform:
//!
//! - every `OPTIONS` request is a preflight (answered with `optionsSuccessStatus` and
//!   `Content-Length: 0`) unless `preflight_continue` is set;
//! - `origin`: `'*'` (default), `true` (reflect), `false` (disabled), a fixed string,
//!   a list of strings/regexes, or a function of the request origin;
//! - `Vary: Origin` whenever the origin is not `'*'` (even if the request origin is not
//!   allowed or missing), merged with an existing `Vary` like the `vary` package;
//! - without `allowed_headers` the requested headers are reflected
//!   (`Vary: Access-Control-Request-Headers`);
//! - headers are applied before the handler runs in Express, so a header set by the
//!   handler wins over the CORS one.

use std::sync::Arc;

use http::header::{HeaderName, HeaderValue};

use crate::http::{Request, Response};

/// One entry of an origin list: an exact origin or a regular expression.
#[derive(Debug, Clone)]
pub enum OriginMatcher {
    Exact(String),
    Regex(regex::Regex),
}

impl OriginMatcher {
    fn matches(&self, origin: Option<&str>) -> bool {
        match (self, origin) {
            (OriginMatcher::Exact(s), Some(o)) => s == o,
            (OriginMatcher::Regex(r), Some(o)) => r.is_match(o),
            // `regex.test(undefined)` tests the string "undefined".
            (OriginMatcher::Regex(r), None) => r.is_match("undefined"),
            (OriginMatcher::Exact(_), None) => false,
        }
    }
}

impl From<&str> for OriginMatcher {
    fn from(s: &str) -> Self {
        OriginMatcher::Exact(s.to_string())
    }
}
impl From<String> for OriginMatcher {
    fn from(s: String) -> Self {
        OriginMatcher::Exact(s)
    }
}
impl From<regex::Regex> for OriginMatcher {
    fn from(r: regex::Regex) -> Self {
        OriginMatcher::Regex(r)
    }
}

/// Function form of `origin`: receives the request `Origin` and returns the static
/// configuration to use (`(origin, cb) => cb(null, value)`).
pub type OriginFn = Arc<dyn Fn(Option<&str>) -> CorsOrigin + Send + Sync>;

/// The `origin` option.
#[derive(Clone, Default)]
pub enum CorsOrigin {
    /// `'*'` (the default).
    #[default]
    Any,
    /// `true`: reflect the request origin.
    Reflect,
    /// `false`: CORS disabled (no headers; `OPTIONS` reaches the router).
    Disabled,
    /// A fixed string, always sent as `Access-Control-Allow-Origin`.
    Fixed(String),
    /// Strings and/or regexes: the request origin is reflected when it matches one.
    List(Vec<OriginMatcher>),
    /// Computed per request. Returning `Disabled` skips CORS for that request.
    Dynamic(OriginFn),
}

impl std::fmt::Debug for CorsOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CorsOrigin::Any => write!(f, "Any"),
            CorsOrigin::Reflect => write!(f, "Reflect"),
            CorsOrigin::Disabled => write!(f, "Disabled"),
            CorsOrigin::Fixed(s) => write!(f, "Fixed({s:?})"),
            CorsOrigin::List(l) => write!(f, "List({l:?})"),
            CorsOrigin::Dynamic(_) => write!(f, "Dynamic(..)"),
        }
    }
}

/// Options for `app.enable_cors(...)`.
#[derive(Debug, Clone)]
pub struct CorsOptions {
    pub origin: CorsOrigin,
    pub methods: Vec<String>,
    /// Empty = reflect `Access-Control-Request-Headers`.
    pub allowed_headers: Vec<String>,
    pub exposed_headers: Vec<String>,
    pub credentials: bool,
    pub max_age: Option<u64>,
    /// Pass the preflight to the next handler instead of answering it.
    pub preflight_continue: bool,
    pub options_success_status: u16,
}

impl Default for CorsOptions {
    fn default() -> Self {
        CorsOptions {
            origin: CorsOrigin::Any,
            methods: ["GET", "HEAD", "PUT", "PATCH", "POST", "DELETE"].iter().map(|s| s.to_string()).collect(),
            allowed_headers: Vec::new(),
            exposed_headers: Vec::new(),
            credentials: false,
            max_age: None,
            preflight_continue: false,
            options_success_status: 204,
        }
    }
}

impl CorsOptions {
    pub fn new() -> Self {
        Self::default()
    }
    /// `origin: 'http://a.com'` (fixed). Calling it again builds a list of origins.
    pub fn origin(mut self, origin: impl Into<String>) -> Self {
        let o = origin.into();
        self.origin = match std::mem::take(&mut self.origin) {
            CorsOrigin::Fixed(prev) => CorsOrigin::List(vec![prev.into(), o.into()]),
            CorsOrigin::List(mut l) => {
                l.push(o.into());
                CorsOrigin::List(l)
            }
            _ if o == "*" => CorsOrigin::Any,
            _ => CorsOrigin::Fixed(o),
        };
        self
    }
    /// `origin: ['http://a.com', 'http://b.com']`.
    pub fn origins<I: IntoIterator<Item = S>, S: Into<String>>(mut self, origins: I) -> Self {
        let mut list = match std::mem::take(&mut self.origin) {
            CorsOrigin::List(l) => l,
            CorsOrigin::Fixed(prev) => vec![prev.into()],
            _ => Vec::new(),
        };
        list.extend(origins.into_iter().map(|s| OriginMatcher::Exact(s.into())));
        self.origin = CorsOrigin::List(list);
        self
    }
    /// `origin: /\.example\.com$/` (added to the origin list).
    pub fn origin_regex(mut self, regex: regex::Regex) -> Self {
        let mut list = match std::mem::take(&mut self.origin) {
            CorsOrigin::List(l) => l,
            CorsOrigin::Fixed(prev) => vec![prev.into()],
            _ => Vec::new(),
        };
        list.push(OriginMatcher::Regex(regex));
        self.origin = CorsOrigin::List(list);
        self
    }
    /// `origin: true`: reflect the request origin.
    pub fn reflect_origin(mut self) -> Self {
        self.origin = CorsOrigin::Reflect;
        self
    }
    /// `origin: (origin, cb) => ...`.
    pub fn origin_fn(mut self, f: impl Fn(Option<&str>) -> CorsOrigin + Send + Sync + 'static) -> Self {
        self.origin = CorsOrigin::Dynamic(Arc::new(f));
        self
    }
    /// Any `origin` value.
    pub fn origin_value(mut self, origin: CorsOrigin) -> Self {
        self.origin = origin;
        self
    }
    pub fn methods<I: IntoIterator<Item = S>, S: Into<String>>(mut self, methods: I) -> Self {
        self.methods = methods.into_iter().map(Into::into).collect();
        self
    }
    pub fn allowed_headers<I: IntoIterator<Item = S>, S: Into<String>>(mut self, h: I) -> Self {
        self.allowed_headers = h.into_iter().map(Into::into).collect();
        self
    }
    pub fn exposed_headers<I: IntoIterator<Item = S>, S: Into<String>>(mut self, h: I) -> Self {
        self.exposed_headers = h.into_iter().map(Into::into).collect();
        self
    }
    pub fn credentials(mut self, v: bool) -> Self {
        self.credentials = v;
        self
    }
    pub fn max_age(mut self, seconds: u64) -> Self {
        self.max_age = Some(seconds);
        self
    }
    pub fn preflight_continue(mut self, v: bool) -> Self {
        self.preflight_continue = v;
        self
    }
    pub fn options_success_status(mut self, status: u16) -> Self {
        self.options_success_status = status;
        self
    }

    /// The static origin configuration for this request (`None` = CORS skipped).
    fn resolve(&self, req: &Request) -> Option<CorsOrigin> {
        match &self.origin {
            CorsOrigin::Disabled => None,
            CorsOrigin::Dynamic(f) => match f(req.header("origin")) {
                CorsOrigin::Disabled | CorsOrigin::Dynamic(_) => None,
                other => Some(other),
            },
            other => Some(other.clone()),
        }
    }

    /// `configureOrigin`: `(Access-Control-Allow-Origin, vary Origin)`.
    fn configure_origin(origin: &CorsOrigin, req: &Request) -> (Option<String>, bool) {
        let request_origin = req.header("origin");
        match origin {
            CorsOrigin::Any => (Some("*".into()), false),
            CorsOrigin::Fixed(s) if s == "*" => (Some("*".into()), false),
            CorsOrigin::Fixed(s) => (Some(s.clone()), true),
            CorsOrigin::Reflect => (request_origin.map(String::from), true),
            CorsOrigin::List(list) => {
                let allowed = list.iter().any(|m| m.matches(request_origin));
                (if allowed { request_origin.map(String::from) } else { None }, true)
            }
            CorsOrigin::Disabled | CorsOrigin::Dynamic(_) => (None, false),
        }
    }

    /// The CORS headers for `req` (`None` when CORS is skipped for it). `preflight`
    /// selects the preflight header set.
    fn headers(&self, req: &Request, preflight: bool) -> Option<Vec<(&'static str, String)>> {
        let origin = self.resolve(req)?;
        let mut out = Vec::new();
        let (allow, vary) = Self::configure_origin(&origin, req);
        if let Some(a) = allow.filter(|a| !a.is_empty()) {
            out.push(("access-control-allow-origin", a));
        }
        if vary {
            out.push(("vary", "Origin".into()));
        }
        if self.credentials {
            out.push(("access-control-allow-credentials", "true".into()));
        }
        if preflight {
            let methods = self.methods.join(",");
            if !methods.is_empty() {
                out.push(("access-control-allow-methods", methods));
            }
            let headers = if self.allowed_headers.is_empty() {
                out.push(("vary", "Access-Control-Request-Headers".into()));
                req.header("access-control-request-headers").unwrap_or("").to_string()
            } else {
                self.allowed_headers.join(",")
            };
            if !headers.is_empty() {
                out.push(("access-control-allow-headers", headers));
            }
            if let Some(m) = self.max_age {
                out.push(("access-control-max-age", m.to_string()));
            }
        }
        let exposed = self.exposed_headers.join(",");
        if !exposed.is_empty() {
            out.push(("access-control-expose-headers", exposed));
        }
        Some(out)
    }

    /// `true` when the request must be answered as a preflight right away.
    pub(crate) fn is_preflight(&self, req: &Request) -> bool {
        req.method() == http::Method::OPTIONS && !self.preflight_continue && self.resolve(req).is_some()
    }

    pub(crate) fn preflight(&self, req: &Request) -> Response {
        let mut res = Response::new().with_status(self.options_success_status);
        for (k, v) in self.headers(req, true).unwrap_or_default() {
            set_cors_header(&mut res, k, &v, true);
        }
        res.set_header("content-length", "0");
        res
    }

    /// Adds the CORS headers to a response produced by the router. Headers already set
    /// by the handler are kept (in Express they would overwrite the CORS ones).
    pub(crate) fn apply(&self, req: &Request, res: &mut Response) {
        let preflight = req.method() == http::Method::OPTIONS;
        for (k, v) in self.headers(req, preflight).unwrap_or_default() {
            set_cors_header(res, k, &v, false);
        }
    }
}

fn set_cors_header(res: &mut Response, name: &'static str, value: &str, overwrite: bool) {
    let n = HeaderName::from_static(name);
    if name == "vary" {
        let current = res.headers().get(&n).and_then(|v| v.to_str().ok()).map(str::to_string);
        if let Some(v) = vary_append(current.as_deref().unwrap_or(""), value)
            && let Ok(v) = HeaderValue::from_str(&v)
        {
            res.headers_mut().insert(n, v);
        }
        return;
    }
    if !overwrite && res.headers().contains_key(&n) {
        return;
    }
    if let Ok(v) = HeaderValue::from_str(value) {
        res.headers_mut().insert(n, v);
    }
}

/// The `vary` package: appends `field` to a `Vary` header value (case-insensitive
/// de-duplication, `*` absorbs everything).
pub fn vary_append(header: &str, field: &str) -> Option<String> {
    if header.trim() == "*" {
        return Some("*".into());
    }
    let mut vals: Vec<String> = header.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let lower: Vec<String> = vals.iter().map(|v| v.to_ascii_lowercase()).collect();
    for f in field.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if f == "*" {
            return Some("*".into());
        }
        if !lower.contains(&f.to_ascii_lowercase()) && !vals.iter().any(|v| v.eq_ignore_ascii_case(f)) {
            vals.push(f.to_string());
        }
    }
    Some(vals.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vary() {
        assert_eq!(vary_append("", "Origin").as_deref(), Some("Origin"));
        assert_eq!(vary_append("Accept-Encoding", "Origin").as_deref(), Some("Accept-Encoding, Origin"));
        assert_eq!(vary_append("origin", "Origin").as_deref(), Some("origin"));
        assert_eq!(vary_append("*", "Origin").as_deref(), Some("*"));
    }

    #[test]
    fn origin_builders() {
        assert!(matches!(CorsOptions::new().origin, CorsOrigin::Any));
        assert!(matches!(CorsOptions::new().origin("http://a").origin, CorsOrigin::Fixed(_)));
        assert!(matches!(CorsOptions::new().origin("http://a").origin("http://b").origin, CorsOrigin::List(ref l) if l.len() == 2));
        assert!(matches!(CorsOptions::new().origins(["http://a"]).origin, CorsOrigin::List(ref l) if l.len() == 1));
    }
}

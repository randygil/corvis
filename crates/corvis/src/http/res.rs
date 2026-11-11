//! `@Res()` / `@Res({ passthrough: true })`: the Express response object (`express` 5).
//!
//! ```ignore
//! #[routes]
//! impl AuthController {
//!     // The handler owns the response: the return value is ignored and the request
//!     // is answered when the handler calls `send`/`json`/`end`/`redirect`/`send_status`.
//!     #[get("magic")]
//!     async fn magic(&self, #[query("token")] token: Option<String>, #[res] res: Res) {
//!         res.set_header("Set-Cookie", "sid=1; Path=/");
//!         res.redirect_with(302, "/");
//!     }
//!
//!     // Passthrough: status, headers and cookies set on `res` are kept and the return
//!     // value is still serialized by the framework (interceptors, `#[http_code]`, ...).
//!     #[post("login")]
//!     #[http_code(204)]
//!     async fn login(&self, #[res(passthrough)] res: Res) -> Result<(), HttpException> {
//!         res.cookie("sid", "abc", CookieOptions::new().http_only(true));
//!         Ok(())
//!     }
//! }
//! ```
//!
//! Semantics follow Express: before the interceptors run the response already
//! has the route status (`#[http_code]`, `201` for POST, `200` otherwise) and the
//! `#[header]` values, which the handler may change. Bodies written with
//! [`send`](Res::send)/[`json`](Res::json)/[`send_status`](Res::send_status) get the
//! `res.send()` treatment (charset, `ETag`, `Content-Length`, `304`), while
//! [`end`](Res::end) and [`redirect`](Res::redirect) write the bytes untouched. When the
//! handler throws, the exception filter answers but the headers set on `res` stay (a
//! filter's own headers win), like with the shared Express response.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http::header::{self, HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use serde_json::Value;

use super::express;
use super::response::{Body, Response};

/// How a route uses `@Res()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResMode {
    /// `@Res()`: the handler sends the response itself.
    Handled,
    /// `@Res({ passthrough: true })`.
    Passthrough,
}

struct ResState {
    status: StatusCode,
    headers: HeaderMap,
    /// `Some` once the response was ended.
    body: Option<Body>,
    /// The body went through `res.send()` (ETag / charset / freshness apply).
    sent: bool,
    finished: bool,
}

struct ResInner {
    state: Mutex<ResState>,
    done: tokio::sync::Notify,
    method: Method,
    accept: Option<String>,
    mode: ResMode,
}

/// The Express response (`@Res() res: Response`). Cheap to clone; clones share the state.
#[derive(Clone)]
pub struct Res {
    inner: Arc<ResInner>,
}

impl std::fmt::Debug for Res {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.inner.state.lock().unwrap();
        f.debug_struct("Res").field("status", &s.status).field("headers", &s.headers).field("finished", &s.finished).finish()
    }
}

/// What [`Res::send`] accepts (the argument types of Express `res.send()`).
#[derive(Debug, Clone)]
pub enum SendBody {
    /// `res.send()` / `res.send(null)`.
    Empty,
    /// A string: `text/html` unless a `Content-Type` is set.
    Text(String),
    /// A `Buffer`: `application/octet-stream` unless a `Content-Type` is set.
    Bytes(Bytes),
    /// Objects, numbers and booleans go through `res.json()`.
    Json(Value),
}

impl From<&str> for SendBody {
    fn from(v: &str) -> Self {
        SendBody::Text(v.to_string())
    }
}
impl From<String> for SendBody {
    fn from(v: String) -> Self {
        SendBody::Text(v)
    }
}
impl From<&String> for SendBody {
    fn from(v: &String) -> Self {
        SendBody::Text(v.clone())
    }
}
impl From<Vec<u8>> for SendBody {
    fn from(v: Vec<u8>) -> Self {
        SendBody::Bytes(Bytes::from(v))
    }
}
impl From<&[u8]> for SendBody {
    fn from(v: &[u8]) -> Self {
        SendBody::Bytes(Bytes::copy_from_slice(v))
    }
}
impl From<Bytes> for SendBody {
    fn from(v: Bytes) -> Self {
        SendBody::Bytes(v)
    }
}
impl From<Value> for SendBody {
    fn from(v: Value) -> Self {
        match v {
            Value::Null => SendBody::Empty,
            Value::String(s) => SendBody::Text(s),
            other => SendBody::Json(other),
        }
    }
}
impl From<()> for SendBody {
    fn from(_: ()) -> Self {
        SendBody::Empty
    }
}

/// `res.cookie()` / `res.clearCookie()` options (the `cookie` package + Express defaults).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CookieOptions {
    /// Milliseconds (Express): sets `Max-Age` (seconds) and `Expires`.
    pub max_age: Option<i64>,
    pub expires: Option<chrono::DateTime<chrono::Utc>>,
    pub domain: Option<String>,
    /// Defaults to `/`.
    pub path: Option<String>,
    pub http_only: bool,
    pub secure: bool,
    pub partitioned: bool,
    /// `low` | `medium` | `high`.
    pub priority: Option<String>,
    /// `strict` | `lax` | `none` (`true` in Express is `strict`).
    pub same_site: Option<String>,
    /// Write the value as is instead of `encodeURIComponent(value)`.
    pub raw: bool,
}

impl CookieOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn max_age(mut self, ms: i64) -> Self {
        self.max_age = Some(ms);
        self
    }
    pub fn expires(mut self, at: chrono::DateTime<chrono::Utc>) -> Self {
        self.expires = Some(at);
        self
    }
    pub fn domain(mut self, d: impl Into<String>) -> Self {
        self.domain = Some(d.into());
        self
    }
    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.path = Some(p.into());
        self
    }
    pub fn http_only(mut self, v: bool) -> Self {
        self.http_only = v;
        self
    }
    pub fn secure(mut self, v: bool) -> Self {
        self.secure = v;
        self
    }
    pub fn partitioned(mut self, v: bool) -> Self {
        self.partitioned = v;
        self
    }
    pub fn priority(mut self, p: impl Into<String>) -> Self {
        self.priority = Some(p.into());
        self
    }
    pub fn same_site(mut self, s: impl Into<String>) -> Self {
        self.same_site = Some(s.into());
        self
    }
    pub fn raw(mut self, v: bool) -> Self {
        self.raw = v;
        self
    }
}

/// `true` when the cookie name `name` starts with `prefix` (`__Host-`, `__Secure-`), ignoring ASCII case as browsers
/// do when they apply the rules of the prefix.
#[cfg(any(feature = "jwt", feature = "auth-session"))]
pub(crate) fn has_cookie_prefix(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len() && name.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// cookie 0.7 `cookieNameRegExp`: `[\x21-\x3A\x3C\x3E-\x7E]+` (no `;`, `=`, spaces or controls): what
/// [`serialize_cookie`] accepts, the parity of `res.cookie`.
pub(crate) fn valid_cookie_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| matches!(b, 0x21..=0x3a | 0x3c | 0x3e..=0x7e))
}

/// cookie 0.7 `cookieValueRegExp`: `("?)[\x21\x23-\x2B\x2D-\x3A\x3C-\x5B\x5D-\x7E]*\1`.
fn valid_cookie_value(value: &str) -> bool {
    let inner = match value.strip_prefix('"') {
        Some(rest) => match rest.strip_suffix('"') {
            Some(inner) => inner,
            None => return false,
        },
        None => value,
    };
    inner.bytes().all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
}

/// cookie 0.7 `domainValueRegExp`: an optional leading dot and dot-separated labels of 1–63
/// letters, digits or `-` that neither start nor end with `-` (case-insensitive).
pub(crate) fn valid_cookie_domain(domain: &str) -> bool {
    let d = domain.strip_prefix('.').unwrap_or(domain);
    !d.is_empty()
        && d.split('.').all(|l| {
            (1..=63).contains(&l.len())
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !l.starts_with('-')
                && !l.ends_with('-')
        })
}

/// cookie 0.7 `pathValueRegExp`: `[\x20-\x3A\x3D-\x7E]*` (no `;`, `<`, controls or non-ASCII).
pub(crate) fn valid_cookie_path(path: &str) -> bool {
    path.bytes().all(|b| matches!(b, 0x20..=0x3a | 0x3d..=0x7e))
}

/// `cookie.serialize(name, value, options)` (cookie 0.7) with Express' `res.cookie` defaults.
/// Panics like the package throws (`TypeError`) on an invalid name, value or option; `path`
/// and `domain` are validated too (CVE-2024-47764), so they cannot inject attributes.
pub fn serialize_cookie(name: &str, value: &str, opts: &CookieOptions) -> String {
    if !valid_cookie_name(name) {
        panic!("TypeError: argument name is invalid");
    }
    let value = if opts.raw { value.to_string() } else { encode_uri_component(value) };
    if !valid_cookie_value(&value) {
        panic!("TypeError: argument val is invalid");
    }
    let mut out = format!("{name}={value}");
    let mut expires = opts.expires;
    if let Some(ms) = opts.max_age {
        expires = Some(chrono::Utc::now() + chrono::Duration::milliseconds(ms));
        out.push_str(&format!("; Max-Age={}", ms.div_euclid(1000)));
    }
    if let Some(d) = opts.domain.as_deref().filter(|d| !d.is_empty()) {
        if !valid_cookie_domain(d) {
            panic!("TypeError: option domain is invalid");
        }
        out.push_str(&format!("; Domain={d}"));
    }
    let path = opts.path.clone().unwrap_or_else(|| "/".into());
    if !path.is_empty() {
        if !valid_cookie_path(&path) {
            panic!("TypeError: option path is invalid");
        }
        out.push_str(&format!("; Path={path}"));
    }
    if let Some(e) = expires {
        out.push_str(&format!("; Expires={}", e.format("%a, %d %b %Y %H:%M:%S GMT")));
    }
    if opts.http_only {
        out.push_str("; HttpOnly");
    }
    if opts.secure {
        out.push_str("; Secure");
    }
    if opts.partitioned {
        out.push_str("; Partitioned");
    }
    if let Some(p) = &opts.priority {
        out.push_str(match p.to_ascii_lowercase().as_str() {
            "low" => "; Priority=Low",
            "medium" => "; Priority=Medium",
            "high" => "; Priority=High",
            _ => panic!("TypeError: option priority is invalid"),
        });
    }
    if let Some(s) = &opts.same_site {
        out.push_str(match s.to_ascii_lowercase().as_str() {
            "true" | "strict" => "; SameSite=Strict",
            "lax" => "; SameSite=Lax",
            "none" => "; SameSite=None",
            _ => panic!("TypeError: option sameSite is invalid"),
        });
    }
    out
}

fn percent_encode_char(c: char, out: &mut String) {
    let mut buf = [0u8; 4];
    for b in c.encode_utf8(&mut buf).bytes() {
        out.push_str(&format!("%{b:02X}"));
    }
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            percent_encode_char(c, &mut out);
        }
    }
    out
}

/// JS `encodeURI`.
fn encode_uri(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || "-_.!~*'();/?:@&=+$,#".contains(c) {
            out.push(c);
        } else {
            percent_encode_char(c, &mut out);
        }
    }
    out
}

/// The `encodeurl` package (what `res.location()` / `res.redirect()` use): encodes what is
/// not a valid URL character, leaving existing `%XX` escapes alone.
pub fn encode_url(url: &str) -> String {
    let allowed = |c: char| matches!(c as u32, 0x21 | 0x23..=0x3B | 0x3D | 0x3F..=0x5F | 0x61..=0x7A | 0x7C | 0x7E);
    let chars: Vec<char> = url.chars().collect();
    let mut out = String::with_capacity(url.len());
    let mut run = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // One unit of ENCODE_CHARS_REGEXP: a disallowed char or a malformed `%` escape.
        let unit_len = if c == '%' {
            let h1 = chars.get(i + 1).copied();
            let h2 = chars.get(i + 2).copied();
            match (h1, h2) {
                (None, _) => 1,
                (Some(a), _) if !a.is_ascii_hexdigit() => 2,
                (Some(_), None) => 0,
                (Some(_), Some(b)) if !b.is_ascii_hexdigit() => 3,
                _ => 0,
            }
        } else if allowed(c) {
            0
        } else {
            1
        };
        if unit_len == 0 {
            if !run.is_empty() {
                out.push_str(&encode_uri(&run));
                run.clear();
            }
            out.push(c);
            i += 1;
        } else {
            run.extend(&chars[i..i + unit_len]);
            i += unit_len;
        }
    }
    if !run.is_empty() {
        out.push_str(&encode_uri(&run));
    }
    out
}

/// `statuses.message[code]` (statuses 2.0).
pub fn status_message(code: u16) -> Option<&'static str> {
    Some(match code {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a Teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        509 => "Bandwidth Limit Exceeded",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return None,
    })
}

/// `req.accepts(types)` (accepts 2 + negotiator 1): the best of `types` (MIME types) for
/// the `Accept` header, the first one when there is no header, `None` when none is acceptable.
pub fn accepts<'a>(accept: Option<&str>, types: &[&'a str]) -> Option<&'a str> {
    let Some(accept) = accept else { return types.first().copied() };
    struct Spec {
        ty: String,
        sub: String,
        params: Vec<(String, String)>,
        q: f64,
        i: usize,
    }
    let parse = |s: &str| -> (String, String, Vec<(String, String)>, f64) {
        let mut parts = s.split(';');
        let full = parts.next().unwrap_or("").trim();
        let (ty, sub) = full.split_once('/').unwrap_or((full, ""));
        let mut q = 1.0;
        let mut params = Vec::new();
        for p in parts {
            let Some((k, v)) = p.split_once('=') else { continue };
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().trim_matches('"').to_string();
            if k == "q" {
                q = v.parse().unwrap_or(1.0);
            } else {
                params.push((k, v));
            }
        }
        (ty.to_string(), sub.to_string(), params, q)
    };
    let accepted: Vec<Spec> = accept
        .split(',')
        .enumerate()
        .filter(|(_, s)| !s.trim().is_empty())
        .map(|(i, s)| {
            let (ty, sub, params, q) = parse(s);
            Spec { ty, sub, params, q, i }
        })
        .collect();
    // (q, s, o, i) per provided type.
    let mut prios: Vec<(f64, i32, i64, usize)> = Vec::new();
    for (index, t) in types.iter().enumerate() {
        let (pty, psub, pparams, _) = parse(t);
        let mut best: (f64, i32, i64) = (0.0, 0, -1);
        for spec in &accepted {
            let mut s = 0;
            if spec.ty.eq_ignore_ascii_case(&pty) {
                s |= 4;
            } else if spec.ty != "*" {
                continue;
            }
            if spec.sub.eq_ignore_ascii_case(&psub) {
                s |= 2;
            } else if spec.sub != "*" {
                continue;
            }
            if !spec.params.is_empty() {
                let ok = spec.params.iter().all(|(k, v)| {
                    let pv = pparams.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.as_str()).unwrap_or("");
                    v == "*" || v.eq_ignore_ascii_case(pv)
                });
                if !ok {
                    continue;
                }
                s |= 1;
            }
            let cand = (spec.q, s, spec.i as i64);
            let better = (best.1 - cand.1) < 0 || (best.1 == cand.1 && (best.0 < cand.0 || (best.0 == cand.0 && best.2 < cand.2)));
            if better {
                best = cand;
            }
        }
        prios.push((best.0, best.1, best.2, index));
    }
    prios.retain(|p| p.0 > 0.0);
    prios.sort_by(|a, b| {
        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3))
    });
    prios.first().map(|p| types[p.3])
}

fn header_name(name: &str) -> HeaderName {
    HeaderName::from_bytes(name.as_bytes()).unwrap_or_else(|_| panic!("TypeError: Invalid header name: \"{name}\""))
}

/// Node's `checkInvalidHeaderChar`: tab, visible ASCII and U+0080..U+00FF (written as UTF-8).
fn header_value(name: &str, value: &str) -> HeaderValue {
    let valid = value.chars().all(|c| matches!(c as u32, 0x09 | 0x20..=0x7e | 0x80..=0xff));
    valid
        .then(|| HeaderValue::from_bytes(value.as_bytes()).ok())
        .flatten()
        .unwrap_or_else(|| panic!("TypeError: Invalid character in header content [\"{name}\"]"))
}

fn header_string(v: &HeaderValue) -> String {
    String::from_utf8_lossy(v.as_bytes()).into_owned()
}

impl Res {
    /// A response for a request with `method` and `Accept` header, starting with `status`.
    pub fn new(method: Method, accept: Option<String>, status: StatusCode, mode: ResMode) -> Self {
        Res {
            inner: Arc::new(ResInner {
                state: Mutex::new(ResState { status, headers: HeaderMap::new(), body: None, sent: false, finished: false }),
                done: tokio::sync::Notify::new(),
                method,
                accept,
                mode,
            }),
        }
    }

    /// `@Res()` or `@Res({ passthrough: true })`.
    pub fn mode(&self) -> ResMode {
        self.inner.mode
    }

    // ---- status ----

    /// `res.status(code)`. Panics (Express throws) outside `100..=999`.
    pub fn status(&self, code: u16) -> &Self {
        let status = StatusCode::from_u16(code)
            .unwrap_or_else(|_| panic!("RangeError: Invalid status code: {code}. Status code must be greater than 99 and less than 1000."));
        self.inner.state.lock().unwrap().status = status;
        self
    }
    /// `res.statusCode`.
    pub fn status_code(&self) -> u16 {
        self.inner.state.lock().unwrap().status.as_u16()
    }
    /// `res.sendStatus(code)`: the status and its message as `text/plain`.
    pub fn send_status(&self, code: u16) {
        let body = status_message(code).map(str::to_string).unwrap_or_else(|| code.to_string());
        self.status(code);
        self.type_("txt");
        self.send(body);
    }

    // ---- headers ----

    /// `res.setHeader(name, value)` (Node): no charset handling.
    pub fn set_header(&self, name: &str, value: impl AsRef<str>) -> &Self {
        let (n, v) = (header_name(name), header_value(name, value.as_ref()));
        self.inner.state.lock().unwrap().headers.insert(n, v);
        self
    }
    /// Sets an already validated header without the panics of [`set_header`](Self::set_header).
    pub(crate) fn set_header_value(&self, name: HeaderName, value: HeaderValue) {
        self.inner.state.lock().unwrap().headers.insert(name, value);
    }
    /// Adds an already validated header value without the panics of [`append`](Self::append).
    pub(crate) fn append_header_value(&self, name: HeaderName, value: HeaderValue) {
        self.inner.state.lock().unwrap().headers.append(name, value);
    }
    /// `res.setHeader(name, [values])`.
    pub fn set_header_values<I, S>(&self, name: &str, values: I) -> &Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let n = header_name(name);
        let vals: Vec<HeaderValue> = values.into_iter().map(|v| header_value(name, v.as_ref())).collect();
        let mut s = self.inner.state.lock().unwrap();
        s.headers.remove(&n);
        for v in vals {
            s.headers.append(n.clone(), v);
        }
        self
    }
    /// `res.set(name, value)` / `res.header(name, value)`: like `setHeader`, but a
    /// `Content-Type` gets its default charset (`mime.contentType`).
    pub fn set(&self, name: &str, value: impl AsRef<str>) -> &Self {
        if name.eq_ignore_ascii_case("content-type") {
            self.set_header(name, express::content_type(value.as_ref()))
        } else {
            self.set_header(name, value)
        }
    }
    /// Alias of [`set`](Self::set).
    pub fn header(&self, name: &str, value: impl AsRef<str>) -> &Self {
        self.set(name, value)
    }
    /// `res.get(name)` / `res.getHeader(name)` (several values are joined with `, `;
    /// see [`get_header_values`](Self::get_header_values)).
    pub fn get(&self, name: &str) -> Option<String> {
        let vals = self.get_header_values(name);
        if vals.is_empty() { None } else { Some(vals.join(", ")) }
    }
    /// Alias of [`get`](Self::get).
    pub fn get_header(&self, name: &str) -> Option<String> {
        self.get(name)
    }
    /// Every value of a header (`Set-Cookie` is usually a list).
    pub fn get_header_values(&self, name: &str) -> Vec<String> {
        let s = self.inner.state.lock().unwrap();
        s.headers.get_all(name).iter().map(header_string).collect()
    }
    /// `res.getHeaders()`.
    pub fn get_headers(&self) -> HeaderMap {
        self.inner.state.lock().unwrap().headers.clone()
    }
    /// `res.getHeaderNames()` (lower case).
    pub fn get_header_names(&self) -> Vec<String> {
        let s = self.inner.state.lock().unwrap();
        s.headers.keys().map(|k| k.as_str().to_string()).collect()
    }
    /// `res.hasHeader(name)`.
    pub fn has_header(&self, name: &str) -> bool {
        self.inner.state.lock().unwrap().headers.contains_key(name)
    }
    /// `res.removeHeader(name)`.
    pub fn remove_header(&self, name: &str) -> &Self {
        self.inner.state.lock().unwrap().headers.remove(name);
        self
    }
    /// `res.append(name, value)`: adds a value to the header.
    pub fn append(&self, name: &str, value: impl AsRef<str>) -> &Self {
        let n = header_name(name);
        let v = if name.eq_ignore_ascii_case("content-type") {
            header_value(name, &express::content_type(value.as_ref()))
        } else {
            header_value(name, value.as_ref())
        };
        self.inner.state.lock().unwrap().headers.append(n, v);
        self
    }
    /// `res.type(type)` / `res.contentType(type)`: a MIME type or an extension (`'json'`, `'.html'`).
    pub fn type_(&self, ty: &str) -> &Self {
        let ct = if ty.contains('/') {
            ty.to_string()
        } else {
            let ext = ty.trim_start_matches('.');
            let mime = crate::common::mime::from_path(std::path::Path::new(&format!("f.{ext}")));
            if mime == "application/octet-stream" && !ext.eq_ignore_ascii_case("bin") {
                "application/octet-stream".to_string()
            } else {
                mime.to_string()
            }
        };
        self.set("Content-Type", ct)
    }
    /// Alias of [`type_`](Self::type_).
    pub fn content_type(&self, ty: &str) -> &Self {
        self.type_(ty)
    }
    /// `res.vary(field)`.
    pub fn vary(&self, field: &str) -> &Self {
        let current = self.get("vary").unwrap_or_default();
        if let Some(v) = crate::router::cors::vary_append(&current, field) {
            self.set_header("Vary", v);
        }
        self
    }
    /// `res.links({ rel: url })`.
    pub fn links(&self, links: &[(&str, &str)]) -> &Self {
        let mut link = self.get("link").unwrap_or_default();
        if !link.is_empty() {
            link.push_str(", ");
        }
        let parts: Vec<String> = links.iter().map(|(rel, url)| format!("<{url}>; rel=\"{rel}\"")).collect();
        link.push_str(&parts.join(", "));
        self.set("Link", link)
    }
    /// `res.attachment([filename])`: `Content-Disposition` (and the type of the extension).
    pub fn attachment(&self, filename: Option<&str>) -> &Self {
        match filename {
            Some(name) => {
                if let Some(ext) = std::path::Path::new(name).extension().and_then(|e| e.to_str()) {
                    self.type_(ext);
                }
                self.set("Content-Disposition", content_disposition(name))
            }
            None => self.set("Content-Disposition", "attachment"),
        }
    }
    /// `res.location(url)`.
    pub fn location(&self, url: &str) -> &Self {
        self.set("Location", encode_url(url))
    }

    // ---- cookies ----

    /// `res.cookie(name, value, options)`.
    pub fn cookie(&self, name: &str, value: &str, options: CookieOptions) -> &Self {
        self.append("Set-Cookie", serialize_cookie(name, value, &options))
    }
    /// `res.cookie(name, object)`: the value is `j:` + JSON.
    pub fn cookie_json(&self, name: &str, value: &Value, options: CookieOptions) -> &Self {
        self.cookie(name, &format!("j:{value}"), options)
    }
    /// `res.clearCookie(name, options)`: empty value, `Expires` in 1970, no `Max-Age`.
    pub fn clear_cookie(&self, name: &str, options: CookieOptions) -> &Self {
        let opts = CookieOptions {
            max_age: None,
            expires: chrono::DateTime::from_timestamp_millis(1),
            path: options.path.clone().or_else(|| Some("/".into())),
            ..options
        };
        self.cookie(name, "", opts)
    }

    // ---- body ----

    fn finish(&self, body: Body, sent: bool) {
        {
            let mut s = self.inner.state.lock().unwrap();
            if s.finished {
                drop(s);
                crate::logger::Logger::new("ExceptionsHandler")
                    .error("Error [ERR_HTTP_HEADERS_SENT]: Cannot set headers after they are sent to the client");
                return;
            }
            s.body = Some(body);
            s.sent = sent;
            s.finished = true;
        }
        self.inner.done.notify_waiters();
        self.inner.done.notify_one();
    }

    /// `res.send(body)`: strings default to `text/html`, buffers to `application/octet-stream`,
    /// JSON values go through `res.json()`. Gets `ETag`, charset and `304` handling.
    pub fn send(&self, body: impl Into<SendBody>) {
        match body.into() {
            SendBody::Json(v) => self.json(v),
            SendBody::Empty => self.finish(Body::Text(String::new()), true),
            SendBody::Text(t) => {
                if !self.has_header("content-type") {
                    self.type_("html");
                }
                self.finish(Body::Text(t), true)
            }
            SendBody::Bytes(b) => {
                if !self.has_header("content-type") {
                    self.type_("bin");
                }
                let ct = self.get("content-type").unwrap_or_default();
                self.finish(Body::Bytes { data: b, content_type: ct }, true)
            }
        }
    }
    /// `res.json(value)`.
    pub fn json(&self, value: impl serde::Serialize) {
        let v = serde_json::to_value(value).unwrap_or(Value::Null);
        if !self.has_header("content-type") {
            self.set("Content-Type", "application/json");
        }
        self.finish(Body::Json(v), true)
    }
    /// `res.end([data])`: writes the bytes as they are (no `ETag`, no charset change).
    pub fn end(&self, data: impl Into<Bytes>) {
        let data = data.into();
        // Node's `res.end(data)` before any write: `Content-Length` (unless chunked or bodyless).
        let status = self.status_code();
        if !self.has_header("content-length")
            && !self.has_header("transfer-encoding")
            && !(status == 204 || status == 304 || (100..200).contains(&status))
        {
            self.set_header("Content-Length", data.len().to_string());
        }
        let ct = self.get("content-type").unwrap_or_default();
        self.finish(Body::Bytes { data, content_type: ct }, false)
    }
    /// `res.end()`.
    pub fn end_empty(&self) {
        self.end(Bytes::new())
    }
    /// `res.redirect(url)` (302).
    pub fn redirect(&self, url: &str) {
        self.redirect_with(302, url)
    }
    /// `res.redirect(status, url)`: `Location` plus a short body negotiated with `Accept`
    /// (`text/plain` or `text/html`, empty otherwise), and `Vary: Accept`.
    pub fn redirect_with(&self, status: u16, url: &str) {
        self.location(url);
        let address = self.get("location").unwrap_or_default();
        let message = status_message(status).unwrap_or("undefined");
        self.vary("Accept");
        let body = match accepts(self.inner.accept.as_deref(), &["text/plain", "text/html"]) {
            Some("text/plain") => {
                self.set("Content-Type", "text/plain");
                format!("{message}. Redirecting to {address}")
            }
            Some(_) => {
                self.set("Content-Type", "text/html");
                format!("<p>{message}. Redirecting to {}</p>", escape_html(&address))
            }
            None => String::new(),
        };
        self.status(status);
        self.set("Content-Length", body.len().to_string());
        if self.inner.method == Method::HEAD {
            self.end_empty();
        } else {
            self.end(body);
        }
    }

    /// `res.headersSent` (here: the response was ended).
    pub fn headers_sent(&self) -> bool {
        self.inner.state.lock().unwrap().finished
    }
    /// `res.writableEnded`.
    pub fn writable_ended(&self) -> bool {
        self.headers_sent()
    }

    /// Waits until the handler ends the response (Express keeps the request open until then).
    pub(crate) async fn wait_end(&self) {
        loop {
            let notified = self.inner.done.notified();
            if self.headers_sent() {
                return;
            }
            notified.await;
        }
    }

    /// The ended response (status, headers and body). `None` if not ended.
    pub(crate) fn take_response(&self) -> Option<Response> {
        let mut s = self.inner.state.lock().unwrap();
        let body = s.body.take()?;
        let mut r = Response { status: Some(s.status), headers: s.headers.clone(), body, ..Default::default() };
        r.express_send = s.sent;
        Some(r)
    }

    /// Passthrough: the response produced by the framework gets the status and the headers set on
    /// `res` (headers already on `resp` win; `Set-Cookie` values are added).
    pub(crate) fn merge_into(&self, resp: &mut Response, keep_status: bool) {
        let s = self.inner.state.lock().unwrap();
        if !keep_status || resp.status.is_none() {
            resp.status = Some(s.status);
        }
        let mut names: Vec<HeaderName> = Vec::new();
        for k in s.headers.keys() {
            if !names.contains(k) {
                names.push(k.clone());
            }
        }
        for name in names {
            if name == header::SET_COOKIE {
                let existing: Vec<HeaderValue> = resp.headers.get_all(&name).iter().cloned().collect();
                resp.headers.remove(&name);
                for v in s.headers.get_all(&name).iter().cloned().chain(existing) {
                    resp.headers.append(name.clone(), v);
                }
            } else if !resp.headers.contains_key(&name) {
                for v in s.headers.get_all(&name) {
                    resp.headers.append(name.clone(), v.clone());
                }
            }
        }
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// The `content-disposition` package (`contentDisposition(filename)`): `attachment` with the
/// basename quoted, plus `filename*` (RFC 5987) for non-latin1 names or names with `%XX`.
pub fn content_disposition(filename: &str) -> String {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    // TEXT_REGEXP: printable latin1.
    let is_quoted = name.chars().all(|c| matches!(c as u32, 0x20..=0x7e | 0x80..=0xff));
    let fallback: String = name.chars().map(|c| if matches!(c as u32, 0x20..=0x7e | 0xa0..=0xff) { c } else { '?' }).collect();
    let has_fallback = fallback != name;
    let b = name.as_bytes();
    let hex_escape = (0..b.len().saturating_sub(2)).any(|i| b[i] == b'%' && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit());
    let mut out = String::from("attachment");
    if is_quoted || has_fallback {
        out.push_str(&format!("; filename={}", quote(if has_fallback { &fallback } else { name })));
    }
    if has_fallback || !is_quoted || hex_escape {
        let enc: String = encode_uri_component(name)
            .chars()
            .map(|c| match c {
                '\'' | '(' | ')' | '*' => format!("%{:02X}", c as u32),
                c => c.to_string(),
            })
            .collect();
        out.push_str(&format!("; filename*=UTF-8''{enc}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodeurl_matches_the_package() {
        assert_eq!(encode_url("/?auth=expired"), "/?auth=expired");
        assert_eq!(encode_url("/foo bar"), "/foo%20bar");
        assert_eq!(encode_url("/a%20b%zz%"), "/a%20b%25zz%25");
        assert_eq!(encode_url("/ñ<\">`{}"), "/%C3%B1%3C%22%3E%60%7B%7D");
        assert_eq!(encode_url("/a\\b^[c]|"), "/a\\b^[c]|");
        assert_eq!(encode_url("%["), "%25%5B");
    }

    #[test]
    fn cookies_like_express() {
        let c = serialize_cookie("sid", "a b;c", &CookieOptions::new().http_only(true).same_site("lax").secure(true));
        assert_eq!(c, "sid=a%20b%3Bc; Path=/; HttpOnly; Secure; SameSite=Lax");
        let c = serialize_cookie("x", "", &CookieOptions { expires: chrono::DateTime::from_timestamp_millis(1), ..Default::default() });
        assert_eq!(c, "x=; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
        let c = serialize_cookie("m", "1", &CookieOptions::new().max_age(90_500).domain("a.com").priority("High"));
        assert!(c.starts_with("m=1; Max-Age=90; Domain=a.com; Path=/; Expires="), "{c}");
        assert!(c.ends_with(" GMT; Priority=High"), "{c}");
    }

    #[test]
    fn negotiation() {
        let t = ["text/plain", "text/html"];
        assert_eq!(accepts(None, &t), Some("text/plain"));
        assert_eq!(accepts(Some("*/*"), &t), Some("text/plain"));
        assert_eq!(accepts(Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"), &t), Some("text/html"));
        assert_eq!(accepts(Some("application/json"), &t), None);
        assert_eq!(accepts(Some("text/*;q=0.5, text/html"), &t), Some("text/html"));
        assert_eq!(accepts(Some("text/plain;q=0, */*"), &t), Some("text/html"));
    }
}

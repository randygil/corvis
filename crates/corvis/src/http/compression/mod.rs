//! Response compression compatible with the `compression` middleware (1.8.2).
//!
//! Available with the `compression` cargo feature (off by default) and applied only after
//! [`CorvisApplication::enable_compression`](crate::CorvisApplication::enable_compression). It runs on
//! the final wire response, after the Express response defaults, in the server and in the test client
//! (see `server::finish_response`), and follows the reference's rules in the reference's order:
//!
//! 1. the filter ([`should_compress`] by default: compressible `Content-Type` per mime-db);
//! 2. `Cache-Control: no-transform`;
//! 3. `Accept-Encoding` is added to `Vary` (whether or not the response ends up compressed);
//! 4. the size threshold (default 1024 bytes; a body of unknown length is compressed);
//! 5. an existing `Content-Encoding` other than `identity`;
//! 6. `HEAD`;
//! 7. responses with `Content-Range` and bodyless statuses (a deliberate difference);
//! 8. the encoding negotiated from `Accept-Encoding` (`br`, `gzip`, `deflate`; no header, no compression).
//!
//! `text/event-stream` is never compressed either (a second deliberate difference, in the filter).

mod body;
mod compressible;
mod negotiate;

use std::sync::Arc;

use http::{HeaderMap, HeaderValue, Method, StatusCode, header};

use self::body::{Coding, CompressedBody};
use self::negotiate::{js_number, negotiate};
use crate::http::express::RequestInfo;
use crate::server::ResBody;

/// Default size threshold in bytes (`compression`'s `threshold`, 1 KiB).
pub const DEFAULT_THRESHOLD: usize = 1024;
/// Default gzip / deflate level (zlib's default).
pub const DEFAULT_LEVEL: u32 = 6;
/// Default brotli quality (the reference's).
pub const DEFAULT_BROTLI_QUALITY: u32 = 4;

/// What a [`CompressionOptions::filter`] sees: the request method, path and headers and the
/// status and headers of the response about to be sent.
#[derive(Debug)]
pub struct CompressionContext<'a> {
    pub method: &'a Method,
    /// The path of the request URI, without the query string.
    pub path: &'a str,
    pub request_headers: &'a HeaderMap,
    pub status: StatusCode,
    pub response_headers: &'a HeaderMap,
}

type Filter = Arc<dyn Fn(&CompressionContext<'_>) -> bool + Send + Sync>;

/// Options of the response compression (`compression({ threshold, filter, level, brotli })`).
///
/// ```
/// # #[cfg(feature = "compression")] {
/// use corvis::http::compression::{CompressionOptions, should_compress};
///
/// let options = CompressionOptions::new()
///     .threshold(2048)
///     .level(6)
///     .brotli_quality(4)
///     .filter(|ctx| !ctx.request_headers.contains_key("x-no-compression") && should_compress(ctx));
/// # let _ = options;
/// # }
/// ```
#[derive(Clone)]
pub struct CompressionOptions {
    threshold: usize,
    level: u32,
    brotli_quality: u32,
    filter: Option<Filter>,
}

impl Default for CompressionOptions {
    fn default() -> Self {
        CompressionOptions { threshold: DEFAULT_THRESHOLD, level: DEFAULT_LEVEL, brotli_quality: DEFAULT_BROTLI_QUALITY, filter: None }
    }
}

impl std::fmt::Debug for CompressionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompressionOptions")
            .field("threshold", &self.threshold)
            .field("level", &self.level)
            .field("brotli_quality", &self.brotli_quality)
            .field("filter", &self.filter.as_ref().map(|_| "custom"))
            .finish()
    }
}

impl CompressionOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Responses smaller than this many bytes are not compressed (default 1024).
    pub fn threshold(mut self, bytes: usize) -> Self {
        self.threshold = bytes;
        self
    }

    /// gzip and deflate level, clamped to 0..=9 (default 6).
    pub fn level(mut self, level: u32) -> Self {
        self.level = level.min(9);
        self
    }

    /// brotli quality, clamped to 0..=11 (default 4).
    pub fn brotli_quality(mut self, quality: u32) -> Self {
        self.brotli_quality = quality.min(11);
        self
    }

    /// Replaces [`should_compress`], which a custom filter can call to fall back to it.
    pub fn filter(mut self, filter: impl Fn(&CompressionContext<'_>) -> bool + Send + Sync + 'static) -> Self {
        self.filter = Some(Arc::new(filter));
        self
    }

    pub fn threshold_bytes(&self) -> usize {
        self.threshold
    }

    pub fn gzip_level(&self) -> u32 {
        self.level
    }

    pub fn quality(&self) -> u32 {
        self.brotli_quality
    }

    pub(crate) fn has_custom_filter(&self) -> bool {
        self.filter.is_some()
    }
}

/// `compressible(type)`: the `compressible` flag of mime-db 1.54.0 for the media type (parameters
/// removed, lowercase), else `true` for `text/*` and `+json`, `+text`, `+xml` suffixes, else unknown.
pub fn compressible(content_type: &str) -> Option<bool> {
    // `^\s*([^;\s]*)(?:;|\s|$)`
    let mime = content_type.trim_start().split([';', ' ', '\t', '\n', '\r', '\u{b}', '\u{c}']).next().unwrap_or("").to_ascii_lowercase();
    if let Ok(i) = compressible::COMPRESSIBLE.binary_search_by(|(t, _)| (*t).cmp(mime.as_str())) {
        return Some(compressible::COMPRESSIBLE[i].1);
    }
    let suffix = ["+json", "+text", "+xml"].iter().any(|s| mime.ends_with(s));
    (mime.starts_with("text/") || suffix).then_some(true)
}

/// The default filter: a response with a compressible `Content-Type` (see [`compressible`]) is
/// compressed, except `text/event-stream`, which is never compressed. Responses without
/// `Content-Type` are not compressed.
pub fn should_compress(ctx: &CompressionContext<'_>) -> bool {
    let Some(value) = ctx.response_headers.get(header::CONTENT_TYPE) else { return false };
    let content_type = String::from_utf8_lossy(value.as_bytes());
    if !compressible(&content_type).unwrap_or(false) {
        return false;
    }
    let essence = content_type.trim_start().split([';', ' ', '\t']).next().unwrap_or("");
    !essence.eq_ignore_ascii_case("text/event-stream")
}

/// `Cache-Control` has the `no-transform` directive.
fn has_no_transform(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::CACHE_CONTROL)
        .iter()
        .flat_map(|v| {
            String::from_utf8_lossy(v.as_bytes()).split(',').map(|s| s.trim().eq_ignore_ascii_case("no-transform")).collect::<Vec<_>>()
        })
        .any(|found| found)
}

/// The `vary` package appending one field: keeps the original text, adds `, field` unless a value
/// is already there, and answers `*` when the header or the field is `*`.
pub(crate) fn vary_append(existing: &str, field: &str) -> String {
    if existing == "*" {
        return "*".into();
    }
    let parse = |s: &str| -> Vec<String> { s.split(',').map(|v| v.trim_matches(' ').to_string()).filter(|v| !v.is_empty()).collect() };
    let values = parse(&existing.to_ascii_lowercase());
    if field == "*" || values.iter().any(|v| v == "*") {
        return "*".into();
    }
    if values.contains(&field.to_ascii_lowercase()) {
        return existing.to_string();
    }
    if existing.is_empty() { field.to_string() } else { format!("{existing}, {field}") }
}

/// What to do with a response.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decision {
    pub add_vary: bool,
    pub encoding: Option<&'static str>,
}

/// The facts [`decide`] needs about the response.
pub(crate) struct ResponseFacts<'a> {
    pub status: StatusCode,
    pub headers: &'a HeaderMap,
    /// The exact size of the body when it is known.
    pub body_len: Option<u64>,
}

/// The reference's checks, in the reference's order (see the module docs).
pub(crate) fn decide(req: &RequestInfo, resp: &ResponseFacts<'_>, opts: &CompressionOptions) -> Decision {
    let no = Decision { add_vary: false, encoding: None };
    let ctx = CompressionContext {
        method: &req.method,
        path: req.path.as_str(),
        request_headers: req.filter_headers(),
        status: resp.status,
        response_headers: resp.headers,
    };
    let passes = match &opts.filter {
        Some(f) => f(&ctx),
        None => should_compress(&ctx),
    };
    if !passes || has_no_transform(resp.headers) {
        return no;
    }
    let vary_only = Decision { add_vary: true, encoding: None };
    let below = match resp.headers.get(header::CONTENT_LENGTH) {
        Some(v) => js_number(&String::from_utf8_lossy(v.as_bytes())) < opts.threshold as f64,
        None => resp.body_len.is_some_and(|n| (n as f64) < opts.threshold as f64),
    };
    if below {
        return vary_only;
    }
    let encoded =
        resp.headers.get(header::CONTENT_ENCODING).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).filter(|v| !v.is_empty());
    if encoded.is_some_and(|e| e != "identity") || req.method == Method::HEAD {
        return vary_only;
    }
    let bodyless = resp.status.is_informational() || resp.status == StatusCode::NO_CONTENT || resp.status == StatusCode::NOT_MODIFIED;
    if bodyless || resp.headers.contains_key(header::CONTENT_RANGE) {
        return vary_only;
    }
    let accept = req.accept_encoding();
    let encoding = accept.as_deref().filter(|a| !a.is_empty()).and_then(|a| negotiate(Some(a))).filter(|e| *e != "identity");
    Decision { add_vary: true, encoding }
}

/// Compresses a wire response when the rules say so. Called by `server::finish_response`.
pub(crate) fn apply(resp: hyper::Response<ResBody>, req: &RequestInfo, opts: &CompressionOptions) -> hyper::Response<ResBody> {
    use hyper::body::Body;
    let (mut parts, body) = resp.into_parts();
    let body_len = body.size_hint().exact();
    let decision = decide(req, &ResponseFacts { status: parts.status, headers: &parts.headers, body_len }, opts);
    if decision.add_vary {
        let existing = parts
            .headers
            .get_all(header::VARY)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        let value = vary_append(&existing, "Accept-Encoding");
        if let Ok(v) = HeaderValue::from_str(&value) {
            parts.headers.insert(header::VARY, v);
        }
    }
    let Some(coding) = decision.encoding.and_then(Coding::from_name) else {
        return hyper::Response::from_parts(parts, body);
    };
    // The window only needs the length of the plain body: `Content-Length` if present, else the exact size.
    let plain_len =
        parts.headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).or(body_len);
    parts.headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static(coding.name()));
    parts.headers.remove(header::CONTENT_LENGTH);
    let body = CompressedBody::new(body, coding, opts.level, opts.brotli_quality, plain_len);
    hyper::Response::from_parts(parts, http_body_util::BodyExt::boxed_unsync(body))
}

#[cfg(test)]
mod tests;

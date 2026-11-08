//! Outgoing responses and the [`IntoResponse`] conversion used for handler return values.

use std::pin::Pin;

use bytes::Bytes;
use futures::Stream;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, StatusCode};
use serde::Serialize;
use serde_json::Value;

use super::exception::{HttpException, IntoStatus};

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static>>;

/// Largest chunk read from disk at a time by [`file_stream`].
const FILE_CHUNK: usize = 64 * 1024;

/// `fs.createReadStream(path, { start, end: start + len - 1 })`: `len` bytes of the file from
/// `start`, read in chunks of up to 64 KiB, so serving a file (or a range of it) never holds it
/// in memory. A file that shrinks meanwhile ends the stream with `UnexpectedEof`.
pub(crate) async fn file_stream(path: &std::path::Path, start: u64, len: u64) -> std::io::Result<ByteStream> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::open(path).await?;
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start)).await?;
    }
    let stream = futures::stream::try_unfold(file.take(len), |mut reader| async move {
        let want = reader.limit().min(FILE_CHUNK as u64) as usize;
        if want == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; want];
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        buf.truncate(n);
        Ok(Some((Bytes::from(buf), reader)))
    });
    Ok(Box::pin(stream))
}

/// Response body.
pub enum Body {
    Empty,
    Json(Value),
    Text(String),
    Html(String),
    Bytes { data: Bytes, content_type: String },
    Stream { stream: ByteStream, content_type: String },
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Body::Empty => write!(f, "Empty"),
            Body::Json(v) => write!(f, "Json({v})"),
            Body::Text(t) => write!(f, "Text({t:?})"),
            Body::Html(t) => write!(f, "Html({t:?})"),
            Body::Bytes { data, .. } => write!(f, "Bytes({} bytes)", data.len()),
            Body::Stream { .. } => write!(f, "Stream"),
        }
    }
}

/// The result of a route handler after interceptors (`Ok`) or an exception (`Err`).
pub type HandlerResult = Result<Response, HttpException>;

/// An HTTP response. `status` is `None` until resolved, so the framework can apply
/// the default status (`201` for POST, `200` otherwise, or `#[http_code]`).
#[derive(Debug)]
pub struct Response {
    pub(crate) status: Option<StatusCode>,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Body,
    /// Headers written only if `headers` does not contain them when the response is
    /// sent (e.g. `StreamableFile`'s `Content-Disposition`, which `@Header()` overrides).
    pub(crate) default_headers: HeaderMap,
    /// The body was written with Express `res.send()` (`@Res()`): raw bytes also get the
    /// `ETag` / freshness treatment of `http::express::finalize`.
    pub(crate) express_send: bool,
}

impl Default for Response {
    fn default() -> Self {
        Response { status: None, headers: HeaderMap::new(), body: Body::Empty, default_headers: HeaderMap::new(), express_send: false }
    }
}

impl Response {
    pub fn new() -> Self {
        Self::default()
    }
    /// JSON body, literal even when the value is `null` (the text `null` with a JSON type); a
    /// handler result with no value answers an empty body instead (`None`, `Value::Null`).
    pub fn json(value: impl Serialize) -> Self {
        Response { body: Body::Json(serde_json::to_value(value).unwrap_or(Value::Null)), ..Default::default() }
    }
    pub fn text(text: impl Into<String>) -> Self {
        Response { body: Body::Text(text.into()), ..Default::default() }
    }
    pub fn html(html: impl Into<String>) -> Self {
        Response { body: Body::Html(html.into()), ..Default::default() }
    }
    pub fn bytes(data: impl Into<Bytes>, content_type: impl Into<String>) -> Self {
        Response { body: Body::Bytes { data: data.into(), content_type: content_type.into() }, ..Default::default() }
    }
    pub fn stream(stream: ByteStream, content_type: impl Into<String>) -> Self {
        Response { body: Body::Stream { stream, content_type: content_type.into() }, ..Default::default() }
    }
    /// `res.redirect(url, status)`: `Location` goes through `encodeurl` like in Express
    /// (spaces, `<`, `"`, CR/LF and non-ASCII are percent-encoded; `%XX` escapes are kept).
    pub fn redirect(url: impl AsRef<str>, status: impl IntoStatus) -> Self {
        let mut r = Response::new().with_status(status);
        if let Ok(v) = HeaderValue::from_str(&super::res::encode_url(url.as_ref())) {
            r.headers.insert(http::header::LOCATION, v);
        }
        r
    }

    pub fn with_status(mut self, status: impl IntoStatus) -> Self {
        self.status = Some(status.into_status());
        self
    }
    pub fn set_status(&mut self, status: impl IntoStatus) {
        self.status = Some(status.into_status());
    }
    pub fn status(&self) -> Option<StatusCode> {
        self.status
    }
    pub fn status_code(&self) -> u16 {
        self.status.map(|s| s.as_u16()).unwrap_or(200)
    }
    pub fn with_header(mut self, name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.set_header(name, value);
        self
    }
    pub fn set_header(&mut self, name: impl AsRef<str>, value: impl AsRef<str>) {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_ref().as_bytes()), HeaderValue::from_str(value.as_ref())) {
            self.headers.insert(n, v);
        }
    }
    pub fn append_header(&mut self, name: impl AsRef<str>, value: impl AsRef<str>) {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_ref().as_bytes()), HeaderValue::from_str(value.as_ref())) {
            self.headers.append(n, v);
        }
    }
    /// Sets a header that is only written if no header with that name is set by the
    /// time the response is sent (`@Header()`, interceptors and filters win).
    pub fn set_default_header(&mut self, name: impl AsRef<str>, value: impl AsRef<str>) {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_ref().as_bytes()), HeaderValue::from_str(value.as_ref())) {
            self.default_headers.insert(n, v);
        }
    }
    /// The headers registered with [`set_default_header`](Self::set_default_header).
    pub fn default_headers(&self) -> &HeaderMap {
        &self.default_headers
    }
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
    /// `res.cookie(name, value)`. Panics (`TypeError`) on an invalid cookie, see
    /// [`Cookie::try_to_string`].
    pub fn with_cookie(mut self, cookie: Cookie) -> Self {
        self.append_header("set-cookie", cookie.to_string());
        self
    }
    pub fn set_cookie(&mut self, cookie: Cookie) {
        self.append_header("set-cookie", cookie.to_string());
    }
    pub fn body(&self) -> &Body {
        &self.body
    }
    pub fn body_mut(&mut self) -> &mut Body {
        &mut self.body
    }
    pub fn set_body(&mut self, body: Body) {
        self.body = body;
    }
    /// The JSON body, if any.
    pub fn json_body(&self) -> Option<&Value> {
        match &self.body {
            Body::Json(v) => Some(v),
            _ => None,
        }
    }
    /// Maps the response data – the RxJS `map()` of an interceptor.
    /// Text bodies are given as JSON strings; empty bodies as `null`. A `null` result of `f` is
    /// stored as an empty body (like an RxJS `map` that returns `null`), so an identity closure
    /// keeps an empty answer empty.
    pub fn map_data(mut self, f: impl FnOnce(Value) -> Value) -> Self {
        let current = std::mem::replace(&mut self.body, Body::Empty);
        let value = match current {
            Body::Json(v) => v,
            Body::Text(t) | Body::Html(t) => Value::String(t),
            Body::Empty => Value::Null,
            other => {
                self.body = other;
                return self;
            }
        };
        self.body = match f(value) {
            Value::Null => Body::Empty,
            v => Body::Json(v),
        };
        self
    }
}

/// A cookie for `Set-Cookie`.
#[derive(Debug, Clone)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub path: Option<String>,
    pub domain: Option<String>,
    /// Lifetime in milliseconds (see `Cookie::max_age`).
    pub max_age: Option<i64>,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: Option<String>,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Cookie {
            name: name.into(),
            value: value.into(),
            path: Some("/".into()),
            domain: None,
            max_age: None,
            http_only: false,
            secure: false,
            same_site: None,
        }
    }
    pub fn path(mut self, p: impl Into<String>) -> Self {
        self.path = Some(p.into());
        self
    }
    pub fn domain(mut self, d: impl Into<String>) -> Self {
        self.domain = Some(d.into());
        self
    }
    /// Lifetime in **milliseconds**, like Express `res.cookie` and `CookieOptions::max_age`.
    /// `Max-Age` is written as whole seconds, rounded down.
    pub fn max_age(mut self, ms: i64) -> Self {
        self.max_age = Some(ms);
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
    pub fn same_site(mut self, v: impl Into<String>) -> Self {
        self.same_site = Some(v.into());
        self
    }
}

impl Cookie {
    /// The `Set-Cookie` value. The name, `path`, `domain` and `same_site` are validated like
    /// `cookie` 0.7 (`serialize`), so a value taken from the request cannot add attributes;
    /// `Err` carries the `TypeError` message of the package. The value is always percent-encoded.
    pub fn try_to_string(&self) -> Result<String, String> {
        use super::res::{valid_cookie_domain, valid_cookie_name, valid_cookie_path};
        if !valid_cookie_name(&self.name) {
            return Err("TypeError: argument name is invalid".into());
        }
        let enc = percent_encoding::utf8_percent_encode(&self.value, percent_encoding::NON_ALPHANUMERIC);
        let mut out = format!("{}={}", self.name, enc);
        if let Some(p) = &self.path {
            if !valid_cookie_path(p) {
                return Err("TypeError: option path is invalid".into());
            }
            out.push_str(&format!("; Path={p}"));
        }
        if let Some(d) = &self.domain {
            if !d.is_empty() && !valid_cookie_domain(d) {
                return Err("TypeError: option domain is invalid".into());
            }
            out.push_str(&format!("; Domain={d}"));
        }
        if let Some(m) = self.max_age {
            out.push_str(&format!("; Max-Age={}", m.div_euclid(1000)));
        }
        if self.http_only {
            out.push_str("; HttpOnly");
        }
        if self.secure {
            out.push_str("; Secure");
        }
        if let Some(s) = &self.same_site {
            if !["strict", "lax", "none"].iter().any(|v| s.eq_ignore_ascii_case(v)) {
                return Err("TypeError: option sameSite is invalid".into());
            }
            out.push_str(&format!("; SameSite={s}"));
        }
        Ok(out)
    }
}

/// [`Cookie::try_to_string`]. Panics on an invalid cookie, like `cookie.serialize` throws (and
/// `res.cookie()` panics): inside a handler that ends in a generic `500`.
impl std::fmt::Display for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.try_to_string() {
            Ok(s) => f.write_str(&s),
            Err(e) => panic!("{e}"),
        }
    }
}

/// Conversion of handler return values into a response.
///
/// Any `Serialize` type is also accepted as a handler return value (it is sent as JSON),
/// so returning an object serializes it automatically.
pub trait IntoResponse {
    fn into_response(self) -> HandlerResult;
}

impl IntoResponse for Response {
    fn into_response(self) -> HandlerResult {
        Ok(self)
    }
}
impl IntoResponse for () {
    fn into_response(self) -> HandlerResult {
        Ok(Response::new())
    }
}
impl IntoResponse for String {
    fn into_response(self) -> HandlerResult {
        Ok(Response::text(self))
    }
}
impl IntoResponse for &'static str {
    fn into_response(self) -> HandlerResult {
        Ok(Response::text(self))
    }
}
impl IntoResponse for Value {
    fn into_response(self) -> HandlerResult {
        // `Value::Null` is "no value" and answers an empty body; use `Json(Value::Null)` or
        // `Response::json(Value::Null)` to send the JSON text `null`.
        if self.is_null() {
            return Ok(Response::new());
        }
        Ok(Response { body: Body::Json(self), ..Default::default() })
    }
}
impl IntoResponse for Bytes {
    fn into_response(self) -> HandlerResult {
        Ok(Response::bytes(self, "application/octet-stream"))
    }
}
impl IntoResponse for Vec<u8> {
    fn into_response(self) -> HandlerResult {
        Ok(Response::bytes(self, "application/octet-stream"))
    }
}
impl IntoResponse for HttpException {
    fn into_response(self) -> HandlerResult {
        Err(self)
    }
}
impl<T: IntoResponse, E: Into<HttpException>> IntoResponse for Result<T, E> {
    fn into_response(self) -> HandlerResult {
        match self {
            Ok(v) => v.into_response(),
            Err(e) => Err(e.into()),
        }
    }
}
impl<T: IntoResponse> IntoResponse for Option<T> {
    fn into_response(self) -> HandlerResult {
        match self {
            Some(v) => v.into_response(),
            None => Ok(Response::new()),
        }
    }
}
impl<T: IntoResponse> IntoResponse for (StatusCode, T) {
    fn into_response(self) -> HandlerResult {
        let mut r = self.1.into_response()?;
        r.status = Some(self.0);
        Ok(r)
    }
}
impl<T: IntoResponse> IntoResponse for (u16, T) {
    fn into_response(self) -> HandlerResult {
        let mut r = self.1.into_response()?;
        r.status = Some(self.0.into_status());
        Ok(r)
    }
}

/// Explicit JSON response wrapper. It serializes the value as is, so `Json(None::<T>)` answers the
/// text `null` with a JSON type, unlike returning `None` directly (an empty body).
pub struct Json<T>(pub T);
impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> HandlerResult {
        let v = serde_json::to_value(self.0)?;
        Ok(Response { body: Body::Json(v), ..Default::default() })
    }
}

/// HTML response wrapper.
pub struct Html<T>(pub T);
impl<T: Into<String>> IntoResponse for Html<T> {
    fn into_response(self) -> HandlerResult {
        Ok(Response::html(self.0))
    }
}

/// Redirect response (`@Redirect()` equivalent when returned from a handler).
pub struct Redirect {
    pub url: String,
    pub status_code: u16,
}
impl Redirect {
    pub fn to(url: impl Into<String>) -> Self {
        Redirect { url: url.into(), status_code: 302 }
    }
    pub fn permanent(url: impl Into<String>) -> Self {
        Redirect { url: url.into(), status_code: 301 }
    }
}
impl IntoResponse for Redirect {
    fn into_response(self) -> HandlerResult {
        Ok(Response::redirect(self.url, self.status_code))
    }
}

/// `StreamableFileOptions`: `{ type, disposition, length }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamableFileOptions {
    /// `Content-Type` (default `application/octet-stream`).
    pub r#type: Option<String>,
    /// `Content-Disposition`.
    pub disposition: Option<String>,
    /// `Content-Length` (filled automatically for in-memory data).
    pub length: Option<u64>,
}

impl StreamableFileOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn r#type(mut self, t: impl Into<String>) -> Self {
        self.r#type = Some(t.into());
        self
    }
    pub fn disposition(mut self, d: impl Into<String>) -> Self {
        self.disposition = Some(d.into());
        self
    }
    pub fn length(mut self, l: u64) -> Self {
        self.length = Some(l);
        self
    }
}

enum FileSource {
    Bytes(Bytes),
    Stream(ByteStream),
}

/// A streamable file body, equivalent to `new StreamableFile(buffer | stream, { type, disposition, length })`.
///
/// Like the Express adapter, its headers are only written when the response does not
/// already have them (`@Header('Content-Type', ...)` wins), and the body is written raw
/// (no ETag, no charset rewriting). In-memory data gets `Content-Length` automatically;
/// a stream without `length` is sent chunked.
pub struct StreamableFile {
    source: FileSource,
    options: StreamableFileOptions,
}

impl StreamableFile {
    /// `new StreamableFile(buffer)`.
    pub fn new(data: impl Into<Bytes>) -> Self {
        let data = data.into();
        let options = StreamableFileOptions { length: Some(data.len() as u64), ..Default::default() };
        StreamableFile { source: FileSource::Bytes(data), options }
    }
    /// `new StreamableFile(buffer, options)`. `length` defaults to the buffer length.
    pub fn with_options(data: impl Into<Bytes>, options: StreamableFileOptions) -> Self {
        let data = data.into();
        let mut options = options;
        options.length.get_or_insert(data.len() as u64);
        StreamableFile { source: FileSource::Bytes(data), options }
    }
    /// `new StreamableFile(readable, options)`.
    pub fn from_stream(stream: ByteStream, options: StreamableFileOptions) -> Self {
        StreamableFile { source: FileSource::Stream(stream), options }
    }
    /// Sets `type` (`Content-Type`).
    pub fn content_type(mut self, ct: impl Into<String>) -> Self {
        self.options.r#type = Some(ct.into());
        self
    }
    /// Sets `disposition` (`Content-Disposition`), e.g. `inline; filename="a.pdf"`.
    pub fn disposition(mut self, d: impl Into<String>) -> Self {
        self.options.disposition = Some(d.into());
        self
    }
    /// Sets `length` (`Content-Length`).
    pub fn length(mut self, len: u64) -> Self {
        self.options.length = Some(len);
        self
    }
    /// Sets `Content-Disposition` with the `content-disposition` package, like
    /// `res.attachment(name)`: `attachment; filename="..."` with `"` and `\` escaped, the
    /// basename only, and `filename*` for non-latin1 names.
    pub fn filename(mut self, name: impl AsRef<str>) -> Self {
        self.options.disposition = Some(super::res::content_disposition(name.as_ref()));
        self
    }
    pub fn options(&self) -> &StreamableFileOptions {
        &self.options
    }
    /// `getHeaders()`: `(type, disposition, length)` with `type` defaulting to
    /// `application/octet-stream`.
    pub fn get_headers(&self) -> (String, Option<String>, Option<u64>) {
        (
            self.options.r#type.clone().unwrap_or_else(|| "application/octet-stream".into()),
            self.options.disposition.clone(),
            self.options.length,
        )
    }
    /// `new StreamableFile(createReadStream(path))`: the file is streamed from disk in chunks;
    /// `type` is inferred from its extension and `length` from its size. Fails if `path` is
    /// not a regular file.
    pub async fn from_path(path: impl AsRef<std::path::Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let meta = tokio::fs::metadata(path).await?;
        if !meta.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a regular file"));
        }
        let stream = file_stream(path, 0, meta.len()).await?;
        let options = StreamableFileOptions::new().r#type(crate::common::mime::from_path(path)).length(meta.len());
        Ok(StreamableFile::from_stream(stream, options))
    }
}
impl IntoResponse for StreamableFile {
    fn into_response(self) -> HandlerResult {
        let (content_type, disposition, length) = self.get_headers();
        let mut r = match self.source {
            FileSource::Bytes(data) => Response::bytes(data, content_type),
            FileSource::Stream(stream) => Response::stream(stream, content_type),
        };
        if let Some(d) = disposition {
            r.set_default_header("content-disposition", d);
        }
        if let Some(l) = length {
            r.set_default_header("content-length", l.to_string());
        }
        Ok(r)
    }
}

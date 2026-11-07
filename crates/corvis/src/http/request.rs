//! The incoming HTTP request (Express' `req`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use http::{HeaderMap, Method, Uri, Version};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::exception::{BadRequestException, HttpException};

/// An HTTP request. Cheap to clone; clones share the same `extensions`
/// (so a guard can attach the authenticated user and the handler can read it).
#[derive(Clone)]
pub struct Request {
    pub(crate) method: Method,
    pub(crate) uri: Uri,
    pub(crate) version: Version,
    pub(crate) headers: HeaderMap,
    pub(crate) raw_body: Bytes,
    pub(crate) body: Value,
    /// Error of the body parser, answered by the router (`next(err)` in Express).
    pub(crate) body_error: Option<HttpException>,
    pub(crate) params: HashMap<String, String>,
    pub(crate) query: Map<String, Value>,
    pub(crate) remote_addr: Option<SocketAddr>,
    pub(crate) extensions: Arc<RwLock<http::Extensions>>,
    pub(crate) route_path: Option<String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).field("uri", &self.uri).finish()
    }
}

impl Request {
    /// An empty `GET /` request (used for error contexts when the real one is gone).
    pub(crate) fn empty() -> Self {
        Request::new(Method::GET, Uri::from_static("/"), HeaderMap::new(), Bytes::new()).expect("empty request")
    }

    /// Builds a request from its parts. The body is parsed like `body-parser` (JSON and
    /// urlencoded forms, see [`crate::http::body_parser`]); a malformed body is an error.
    pub fn new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Result<Self, HttpException> {
        let mut req = Self::with_parser(method, uri, headers, body, &super::body_parser::BodyParserOptions::default());
        match req.body_error.take() {
            Some(e) => Err(e),
            None => Ok(req),
        }
    }

    /// Builds a request keeping the body-parser error (if any) inside it: the router raises it
    /// after CORS and through the global filters, like an Express middleware calling `next(err)`.
    pub(crate) fn with_parser(
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: Bytes,
        parser: &super::body_parser::BodyParserOptions,
    ) -> Self {
        let (parsed, body_error) = match super::body_parser::parse_body(&headers, &body, parser) {
            Ok(v) => (v, None),
            Err(e) => (Value::Null, Some(e)),
        };
        let query = parse_query_with(uri.query().unwrap_or(""), parser.query_parser);
        Request {
            method,
            uri,
            version: Version::HTTP_11,
            headers,
            raw_body: body,
            body: parsed,
            body_error,
            params: HashMap::new(),
            query,
            remote_addr: None,
            extensions: Arc::new(RwLock::new(http::Extensions::new())),
            route_path: None,
        }
    }

    pub fn method(&self) -> &Method {
        &self.method
    }
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
    /// The path without query string (`req.path`).
    pub fn path(&self) -> &str {
        self.uri.path()
    }
    /// The original URL including query string (`req.originalUrl`).
    pub fn url(&self) -> String {
        self.uri.path_and_query().map(|p| p.to_string()).unwrap_or_else(|| self.uri.path().to_string())
    }
    pub fn version(&self) -> Version {
        self.version
    }
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }
    /// A single header as string (`req.get('authorization')`).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
    /// The raw request body.
    pub fn raw_body(&self) -> &Bytes {
        &self.raw_body
    }
    /// The parsed request body (`req.body`).
    pub fn body(&self) -> &Value {
        &self.body
    }
    pub fn set_body(&mut self, body: Value) {
        self.body = body;
    }
    /// Deserializes the body into `T`.
    pub fn body_as<T: DeserializeOwned>(&self) -> Result<T, HttpException> {
        serde_json::from_value(self.body.clone()).map_err(|e| BadRequestException::new(e.to_string()))
    }
    /// Route params (`req.params`).
    pub fn params(&self) -> &HashMap<String, String> {
        &self.params
    }
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params.get(name).map(|s| s.as_str())
    }
    /// Query string (`req.query`).
    pub fn query(&self) -> &Map<String, Value> {
        &self.query
    }
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).and_then(|v| v.as_str())
    }
    /// Deserializes the query string into `T` (numbers and booleans are parsed from strings).
    pub fn query_as<T: DeserializeOwned>(&self) -> Result<T, HttpException> {
        crate::common::lenient::from_value(Value::Object(self.query.clone()))
            .map_err(|e| BadRequestException::new(e.to_string()))
    }
    /// Client IP (`req.ip`), honoring `X-Forwarded-For`.
    pub fn ip(&self) -> String {
        if let Some(fwd) = self.header("x-forwarded-for") {
            if let Some(first) = fwd.split(',').next() {
                return first.trim().to_string();
            }
        }
        self.remote_addr.map(|a| a.ip().to_string()).unwrap_or_else(|| "127.0.0.1".to_string())
    }
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }
    /// `req.hostname`.
    pub fn hostname(&self) -> Option<&str> {
        self.header("host").map(|h| h.split(':').next().unwrap_or(h))
    }
    /// The matched route pattern (e.g. `/cats/:id`), once routed.
    pub fn route_path(&self) -> Option<&str> {
        self.route_path.as_deref()
    }
    /// Cookies sent by the client.
    pub fn cookies(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for v in self.headers.get_all(http::header::COOKIE) {
            if let Ok(s) = v.to_str() {
                for pair in s.split(';') {
                    if let Some((k, v)) = pair.split_once('=') {
                        let v = percent_encoding::percent_decode_str(v.trim()).decode_utf8_lossy().to_string();
                        out.insert(k.trim().to_string(), v);
                    }
                }
            }
        }
        out
    }
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies().remove(name)
    }

    /// Attaches a value to the request (like `req.user = user`). Shared by all clones.
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.extensions.write().unwrap().insert(value);
    }
    /// Reads a value previously attached with [`Request::insert`].
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.extensions.read().unwrap().get::<T>().cloned()
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.extensions.write().unwrap().remove::<T>()
    }

    /// `req.user` convention used by passport-like guards: stores a JSON value.
    pub fn set_user(&self, user: impl serde::Serialize) {
        if let Ok(v) = serde_json::to_value(user) {
            self.insert(RequestUser(v));
        }
    }
    pub fn user(&self) -> Option<Value> {
        self.get::<RequestUser>().map(|u| u.0)
    }
    pub fn user_as<T: DeserializeOwned>(&self) -> Option<T> {
        self.user().and_then(|u| serde_json::from_value(u).ok())
    }
}

#[derive(Clone)]
struct RequestUser(Value);

/// `req.query` as Express 5 computes it (`query parser` setting).
pub(crate) fn parse_query_with(q: &str, mode: super::body_parser::QueryParser) -> Map<String, Value> {
    use super::body_parser::QueryParser;
    match mode {
        QueryParser::Disabled => Map::new(),
        QueryParser::Extended => match super::body_parser::qs::parse(q, &super::body_parser::qs::Options::express_extended()) {
            Ok(Value::Object(m)) => m,
            _ => Map::new(),
        },
        QueryParser::Simple => {
            // node:querystring.parse: keys are literal (`a[]` stays `a[]`), repeated keys
            // become arrays, at most 1000 keys, object keys in JavaScript order.
            let mut obj = super::json_parse::JsObject::new();
            let mut seen: Vec<(String, Value)> = Vec::new();
            for (k, v) in form_urlencoded::parse(q.as_bytes()) {
                let v = Value::String(v.into_owned());
                match seen.iter_mut().find(|(key, _)| *key == k) {
                    Some((_, Value::Array(arr))) => arr.push(v),
                    Some((_, existing)) => {
                        let prev = existing.take();
                        *existing = Value::Array(vec![prev, v]);
                    }
                    None => {
                        if seen.len() >= 1000 {
                            break;
                        }
                        seen.push((k.into_owned(), v));
                    }
                }
            }
            for (k, v) in seen {
                obj.set(k, v);
            }
            obj.into_map()
        }
    }
}


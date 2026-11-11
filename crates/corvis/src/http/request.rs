//! The incoming HTTP request (Express' `req`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

use bytes::Bytes;
use http::{HeaderMap, Method, Uri, Version};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::exception::{BadRequestException, HttpException};

/// An HTTP request. Cloning copies the parsed query, the headers and the body (the cost grows with
/// their size), but clones share the same `extensions` (so a guard can attach the authenticated user
/// and the handler can read it). Code that needs the same request in many places should share it
/// through an `Arc`, as WebSocket gateways do with the handshake.
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
    /// The body could not be read because the peer reset the stream or broke the connection.
    pub(crate) client_aborted: bool,
    pub(crate) params: HashMap<String, String>,
    pub(crate) query: Map<String, Value>,
    /// The `query parser` setting `query` was built with (`redacted_url` splits the query string the same way).
    pub(crate) query_parser: super::body_parser::QueryParser,
    /// The query parameters hidden in the log line of every request (`None` when no strategy registered one).
    pub(crate) app_redaction: Option<Arc<super::redact::AppRedaction>>,
    pub(crate) remote_addr: Option<SocketAddr>,
    /// `trust proxy` setting used by `ip()`, `ips()` and `hostname()`.
    pub(crate) trust_proxy: super::proxy_addr::TrustProxy,
    pub(crate) extensions: Arc<RwLock<http::Extensions>>,
    pub(crate) route_path: Option<String>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The redacted form: a `{:?}` in an error report or a tracing span must not print a credential that a
        // strategy reads from the query string.
        f.debug_struct("Request").field("method", &self.method).field("uri", &self.redacted_url()).finish()
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

    /// The handshake request of an upgrade (WebSocket gateways and `upgrade` listeners): the hyper
    /// parts with an empty body and the peer address, built with the application's request
    /// settings like an HTTP request: the query parser fills `query()` and `trust proxy` makes
    /// `ip()`, `ips()` and `hostname()` resolve the same way.
    pub(crate) fn for_upgrade(
        parts: http::request::Parts,
        remote: SocketAddr,
        parser: &super::body_parser::BodyParserOptions,
    ) -> Result<Self, HttpException> {
        let mut req = Self::with_parser(parts.method, parts.uri, parts.headers, Bytes::new(), parser);
        if let Some(e) = req.body_error.take() {
            return Err(e);
        }
        req.version = parts.version;
        req.remote_addr = Some(remote);
        Ok(req)
    }

    /// A request of an existing Engine.IO session (a poll or a `POST`): the method, URI and headers moved from the
    /// hyper parts and the peer, with an empty query and body. The engine only checks it (origin, CORS, method) and
    /// never hands it to application code, which sees the session's handshake: so the application's query parser does
    /// not run and nothing is copied.
    #[cfg(feature = "socket-io")]
    pub(crate) fn for_engine_poll(parts: http::request::Parts, remote: SocketAddr) -> Self {
        Request {
            method: parts.method,
            uri: parts.uri,
            version: parts.version,
            headers: parts.headers,
            raw_body: Bytes::new(),
            body: Value::Null,
            body_error: None,
            client_aborted: false,
            params: HashMap::new(),
            query: Map::new(),
            query_parser: super::body_parser::QueryParser::default(),
            app_redaction: None,
            remote_addr: Some(remote),
            trust_proxy: super::proxy_addr::TrustProxy::default(),
            extensions: Arc::new(RwLock::new(http::Extensions::new())),
            route_path: None,
        }
    }

    /// The same request with its header names and values and its URI in allocations of their own. A request parsed from
    /// the wire holds them as slices of the connection's read buffer (up to the server's `max_header_size`), so a
    /// request kept for long (an Engine.IO session's handshake, and the sockets and hooks that share it) would keep that
    /// whole buffer alive. Clones share the `extensions` as usual.
    #[cfg(feature = "socket-io")]
    pub(crate) fn detached(&self) -> Request {
        let mut headers = HeaderMap::with_capacity(self.headers.len());
        for (name, value) in self.headers.iter() {
            // `from_bytes` copies; a standard name is a constant and allocates nothing.
            let name = http::HeaderName::from_bytes(name.as_str().as_bytes()).unwrap_or_else(|_| name.clone());
            let mut copy = http::HeaderValue::from_bytes(value.as_bytes()).unwrap_or_else(|_| value.clone());
            copy.set_sensitive(value.is_sensitive());
            headers.append(name, copy);
        }
        let uri = Uri::try_from(self.uri.to_string()).unwrap_or_else(|_| self.uri.clone());
        Request { headers, uri, ..self.clone() }
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
            client_aborted: false,
            params: HashMap::new(),
            query,
            query_parser: parser.query_parser,
            app_redaction: parser.redacted_params.clone(),
            remote_addr: None,
            trust_proxy: parser.trust_proxy.clone(),
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
        crate::common::lenient::from_value(Value::Object(self.query.clone())).map_err(|e| BadRequestException::new(e.to_string()))
    }
    /// Client IP (`req.ip`). By default the socket address (`X-Forwarded-For` is ignored); with a
    /// `trust proxy` setting ([`crate::CorvisApplication::set_trust_proxy`]) the closest
    /// `X-Forwarded-For` address that is not a trusted proxy, like `proxy-addr`.
    pub fn ip(&self) -> String {
        super::proxy_addr::client_addr(self.socket_ip(), &self.headers, &self.trust_proxy)
    }
    /// `req.ips`: the trusted `X-Forwarded-For` chain, from the client to the closest proxy
    /// (empty unless `trust proxy` is set).
    pub fn ips(&self) -> Vec<String> {
        let mut addrs = self.proxy_addrs();
        addrs.remove(0);
        addrs.reverse();
        addrs
    }
    fn socket_ip(&self) -> String {
        self.remote_addr.map(|a| a.ip().to_string()).unwrap_or_else(|| "127.0.0.1".to_string())
    }
    /// `proxyaddr.all(req, trust)`: the socket address, then the trusted hops up to the client.
    fn proxy_addrs(&self) -> Vec<String> {
        super::proxy_addr::all_addrs(self.socket_ip(), &self.headers, &self.trust_proxy)
    }
    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr
    }
    /// `req.hostname`: the `Host` header without the port (IPv6 literals keep their brackets:
    /// `[::1]`), or the first `X-Forwarded-Host` when `trust proxy` trusts the socket address.
    /// HTTP/2 requests without `Host` use the `:authority`.
    pub fn hostname(&self) -> Option<&str> {
        let forwarded = self.header("x-forwarded-host").filter(|h| !h.is_empty() && self.trust_proxy.trusts(&self.socket_ip(), 0));
        let host = match forwarded {
            Some(h) => h.split(',').next().unwrap_or(h).trim_end(),
            None => match self.header("host") {
                Some(h) => h,
                None => return self.uri.authority().map(|a| a.host()).filter(|h| !h.is_empty()),
            },
        };
        if host.is_empty() {
            return None;
        }
        let offset = if host.starts_with('[') { host.find(']').map_or(0, |i| i + 1) } else { 0 };
        Some(match host[offset..].find(':') {
            Some(i) => &host[..offset + i],
            None => host,
        })
    }
    /// The matched route pattern (e.g. `/cats/:id`), once routed.
    pub fn route_path(&self) -> Option<&str> {
        self.route_path.as_deref()
    }
    /// Cookies sent by the client, parsed like `cookie-parser` (`cookie.parse`): the first
    /// occurrence of a name wins, surrounding double quotes are removed and a value is
    /// URI-decoded only when it contains `%` (an invalid escape keeps the raw value).
    pub fn cookies(&self) -> HashMap<String, String> {
        parse_cookies(&self.cookie_header())
    }
    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies().remove(name)
    }

    /// The `Cookie` headers joined as `cookies()` reads them.
    fn cookie_header(&self) -> String {
        let header: Vec<String> =
            self.headers.get_all(http::header::COOKIE).iter().map(|v| super::body_parser::latin1(v.as_bytes())).collect();
        header.join("; ")
    }

    /// Every value sent under `name` (`cookies()` keeps the first one only), in header order, at most `limit` of
    /// them. A value whose raw form is longer than `max_raw` bytes is not decoded and counts as an empty string.
    #[cfg(any(feature = "auth-session", feature = "jwt"))]
    pub(crate) fn cookie_values(&self, name: &str, limit: usize, max_raw: usize) -> Vec<String> {
        let mut out = Vec::new();
        scan_cookies(&self.cookie_header(), |key, raw| {
            if key == name {
                out.push(if raw.len() > max_raw { String::new() } else { decode_cookie_value(raw) });
            }
            out.len() < limit
        });
        out
    }

    /// The session of this request, `None` when no `SessionModule` runs for it (see [`crate::auth::session`]).
    #[cfg(feature = "auth-session")]
    pub fn session(&self) -> Option<crate::auth::session::Session> {
        self.get()
    }

    /// The value of the signed cookie `name` (see [`CookieKeys`](super::signed_cookie::CookieKeys)): what
    /// cookie-parser's `req.signedCookies[name]` holds for a string value. `None` when the cookie is absent, unsigned
    /// or does not verify with any of the keys.
    #[cfg(feature = "auth-session")]
    pub fn signed_cookie(&self, name: &str, keys: &super::signed_cookie::CookieKeys) -> Option<String> {
        self.cookie(name).and_then(|v| keys.unsign(&v))
    }

    /// Attaches a value to the request (like `req.user = user`). Shared by all clones. A panic elsewhere that poisoned
    /// the extensions' lock does not stop it, as with the rest of `Request`.
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.extensions.write().unwrap_or_else(|e| e.into_inner()).insert(value);
    }
    /// Reads a value previously attached with [`Request::insert`].
    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.extensions.read().unwrap_or_else(|e| e.into_inner()).get::<T>().cloned()
    }
    pub fn remove<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.extensions.write().unwrap_or_else(|e| e.into_inner()).remove::<T>()
    }

    /// The typed user of the request, for guards (a handler takes a [`User<T>`](crate::auth::User) parameter): a clone
    /// of what an [`AuthGuard`](crate::auth::AuthGuard) stored, or of what the session middleware restored (or
    /// `Authenticator::log_in` stored), when it is a `T`. It reads that slot only, never another request extension of
    /// the same type, so a guard that inserts a `T` does not make a user. On a route without an `AuthGuard` it is the
    /// session's user, which nothing about the request itself has checked; see the authentication threat model.
    ///
    /// ```ignore
    /// let roles = request.typed_user::<Account>().map(|a| a.roles).unwrap_or_default();
    /// ```
    pub fn typed_user<T: Clone + 'static>(&self) -> Option<T> {
        crate::auth::AuthUserSlot::read::<T>(self)
    }

    /// `req.user` convention used by passport-like guards: stores a JSON value. It also drops the typed user (what an
    /// [`AuthGuard`](crate::auth::AuthGuard) stored, or what the session middleware restored), so `request.user()` and
    /// `User<T>` never disagree: after this call `User<T>` answers 401 and [`typed_user`](Self::typed_user) gives
    /// `None`, because they read only the typed user's own slot. A value that does not serialize leaves both users as
    /// they were.
    pub fn set_user(&self, user: impl serde::Serialize) {
        if let Ok(v) = serde_json::to_value(user) {
            self.remove::<crate::auth::AuthUserSlot>();
            self.insert(RequestUser(v));
        }
    }
    /// Stores an already serialized user (see [`Request::set_user`]).
    pub(crate) fn set_user_value(&self, user: Value) {
        self.insert(RequestUser(user));
    }
    /// Removes the user stored with [`Request::set_user`].
    pub(crate) fn clear_user(&self) {
        self.remove::<RequestUser>();
        self.remove::<crate::auth::AuthUserSlot>();
    }
    pub fn user(&self) -> Option<Value> {
        self.get::<RequestUser>().map(|u| u.0)
    }
    /// The info the last authentication attempt of an [`AuthGuard`](crate::auth::AuthGuard) produced
    /// (`req.authInfo`): the info of a successful strategy, or the challenge of a failed one (the list of the
    /// challenges, `null` for a failure without one, when several strategies failed).
    pub fn auth_info(&self) -> Option<Value> {
        self.get::<crate::auth::AuthInfo>().map(|i| i.0)
    }
    pub fn user_as<T: DeserializeOwned>(&self) -> Option<T> {
        self.user().and_then(|u| serde_json::from_value(u).ok())
    }
}

#[derive(Clone)]
struct RequestUser(Value);

/// `cookie.parse(str)` (cookie 0.7).
fn parse_cookies(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    scan_cookies(s, |key, raw| {
        if !out.contains_key(key) {
            out.insert(key.to_string(), decode_cookie_value(raw));
        }
        true
    });
    out
}

/// A cookie value as `cookie.parse` returns it: URI-decoded only when it contains `%`, the raw value when the escape
/// is invalid.
fn decode_cookie_value(raw: &str) -> String {
    if raw.contains('%') { super::body_parser::qs::decode_uri_component(raw).unwrap_or_else(|| raw.to_string()) } else { raw.to_string() }
}

/// Walks the pairs of a `Cookie` header like `cookie.parse` (cookie 0.7) and calls `f(name, raw_value)` for each one,
/// repeats included, with the surrounding quotes of the value removed. `f` returns `false` to stop.
fn scan_cookies(s: &str, mut f: impl FnMut(&str, &str) -> bool) {
    let b = s.as_bytes();
    let len = b.len();
    if len < 2 {
        return;
    }
    let is_ws = |c: u8| c == b' ' || c == b'\t';
    // `startIndex` / `endIndex`: skip spaces and tabs inside `[start, end)`.
    let trim = |mut start: usize, mut end: usize| {
        while start < end && is_ws(b[start]) {
            start += 1;
        }
        while end > start && is_ws(b[end - 1]) {
            end -= 1;
        }
        (start, end)
    };
    let find = |c: u8, from: usize| b[from..].iter().position(|&x| x == c).map(|p| p + from);
    let mut index = 0;
    while index < len {
        let Some(eq) = find(b'=', index) else { break };
        let end = match find(b';', index) {
            None => len,
            // A pair without `=`: resume after the last `;` before this `=`.
            Some(semi) if eq > semi => {
                index = b[..eq].iter().rposition(|&x| x == b';').map_or(eq, |p| p + 1);
                continue;
            }
            Some(semi) => semi,
        };
        let (ks, ke) = trim(index, eq);
        let (mut vs, mut ve) = trim(eq + 1, end);
        if ve > vs && b[vs] == b'"' && b[ve - 1] == b'"' {
            vs += 1;
            ve = ve.saturating_sub(1).max(vs);
        }
        if !f(&s[ks..ke], &s[vs..ve]) {
            return;
        }
        index = end + 1;
    }
}

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
            // become arrays, at most 1000 pairs (`maxKeys`, empty ones included), object keys in
            // JavaScript order.
            let q = match q.match_indices('&').nth(999) {
                Some((i, _)) => &q[..i],
                None => q,
            };
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

#[cfg(test)]
mod upgrade_tests {
    use super::*;
    use crate::http::body_parser::{BodyParserOptions, QueryParser};
    use crate::http::proxy_addr::TrustProxy;

    fn parts(xff: &str) -> http::request::Parts {
        http::Request::builder().uri("/ws?a=1&r[k]=2").header("x-forwarded-for", xff).body(()).unwrap().into_parts().0
    }

    fn options(query_parser: QueryParser, trust_proxy: TrustProxy) -> BodyParserOptions {
        BodyParserOptions { query_parser, trust_proxy, ..BodyParserOptions::default() }
    }

    #[test]
    fn for_upgrade_resolves_the_address_with_the_given_setting() {
        let remote: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let trusted = Request::for_upgrade(parts("203.0.113.7"), remote, &options(QueryParser::Simple, TrustProxy::from(1usize))).unwrap();
        assert_eq!(trusted.ip(), "203.0.113.7");
        assert_eq!(trusted.remote_addr(), Some(remote));
        assert_eq!(trusted.query_param("a"), Some("1"));

        let default = Request::for_upgrade(parts("203.0.113.7"), remote, &BodyParserOptions::default()).unwrap();
        assert_eq!(default.ip(), "127.0.0.1");

        let untrusted =
            Request::for_upgrade(parts("203.0.113.7"), remote, &options(QueryParser::Simple, TrustProxy::parse("10.0.0.0/8").unwrap()))
                .unwrap();
        assert_eq!(untrusted.ip(), "127.0.0.1");
    }

    #[test]
    fn for_upgrade_parses_the_query_with_the_given_parser() {
        let remote: SocketAddr = "127.0.0.1:4000".parse().unwrap();
        let simple = Request::for_upgrade(parts("1.1.1.1"), remote, &BodyParserOptions::default()).unwrap();
        assert_eq!(simple.query().get("r[k]"), Some(&serde_json::json!("2")));
        let extended = Request::for_upgrade(parts("1.1.1.1"), remote, &options(QueryParser::Extended, TrustProxy::default())).unwrap();
        assert_eq!(extended.query().get("r"), Some(&serde_json::json!({ "k": "2" })));
        let disabled = Request::for_upgrade(parts("1.1.1.1"), remote, &options(QueryParser::Disabled, TrustProxy::default())).unwrap();
        assert!(disabled.query().is_empty());
        assert_eq!(disabled.uri().query(), Some("a=1&r[k]=2"));
    }
}

#[cfg(test)]
mod cookie_tests {
    use super::*;

    fn request(headers: &[&str]) -> Request {
        let mut map = HeaderMap::new();
        for h in headers {
            map.append(http::header::COOKIE, http::HeaderValue::from_str(h).unwrap());
        }
        Request::new(Method::GET, Uri::from_static("/"), map, Bytes::new()).unwrap()
    }

    #[test]
    fn the_first_cookie_of_a_name_wins() {
        let req = request(&["a=1; b=2; a=3", "a=4"]);
        assert_eq!(req.cookie("a").as_deref(), Some("1"));
        assert_eq!(req.cookie("b").as_deref(), Some("2"));
    }

    #[cfg(any(feature = "auth-session", feature = "jwt"))]
    #[test]
    fn every_value_of_a_name_is_listed_across_headers_and_bounded() {
        let req = request(&["a=1; b=2; a=3", "a=%34"]);
        assert_eq!(req.cookie_values("a", 10, 100), ["1", "3", "4"]);
        assert_eq!(req.cookie_values("a", 2, 100), ["1", "3"], "at most `limit` values");
        assert_eq!(req.cookie_values("b", 2, 100), ["2"]);
        assert!(req.cookie_values("c", 2, 100).is_empty());
        // A raw value past the bound is not decoded: it counts as an empty string.
        let req = request(&["a=%61%61%61%61%61%61"]);
        assert_eq!(req.cookie_values("a", 2, 5), [""]);
        assert_eq!(req.cookie_values("a", 2, 100), ["aaaaaa"]);
    }
}

#[cfg(test)]
mod extension_tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Account {
        id: u32,
    }

    /// A value inserted by another guard is a plain extension, never the typed user.
    #[test]
    fn typed_user_never_reads_a_foreign_extension() {
        let req = Request::empty();
        req.insert(Account { id: 7 });
        assert_eq!(req.typed_user::<Account>(), None);
        req.insert(crate::auth::AuthUserSlot(std::sync::Arc::new(Account { id: 1 })));
        assert_eq!(req.typed_user::<Account>(), Some(Account { id: 1 }));
        assert_eq!(req.typed_user::<String>(), None, "another type is no user");
        req.set_user(serde_json::json!({ "id": 2 }));
        assert_eq!(req.typed_user::<Account>(), None, "set_user drops the typed user");
    }

    /// `insert`, `get` and `remove` work on extensions whose lock a panic poisoned.
    #[test]
    fn extensions_survive_a_poisoned_lock() {
        let req = Request::empty();
        req.insert(1u8);
        let shared = req.extensions.clone();
        let _ = std::thread::spawn(move || {
            let _guard = shared.write().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(req.extensions.is_poisoned());
        assert_eq!(req.get::<u8>(), Some(1));
        req.insert(2u16);
        assert_eq!(req.remove::<u16>(), Some(2));
        assert_eq!(req.get::<u16>(), None);
    }
}

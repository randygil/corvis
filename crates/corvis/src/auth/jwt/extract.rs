//! Where the JWT strategy reads the token from: the extractors of `passport-jwt` (`ExtractJwt`).

use std::sync::Arc;

use http::HeaderMap;

use crate::auth::auth_header;
use crate::http::Request;
use crate::http::body_parser::latin1;

type CustomFn = dyn Fn(&Request) -> Option<String> + Send + Sync;

/// A way to find the token in a request. Build one with [`ExtractJwt`]; the strategy has no default, tokens are read
/// only from the places configured.
#[derive(Clone)]
pub struct JwtExtractor(Kind);

#[derive(Clone)]
enum Kind {
    AuthHeader { scheme: String },
    Header { name: String },
    Query { name: String },
    Body { name: String },
    Cookie { name: String },
    First(Vec<JwtExtractor>),
    Custom(Arc<CustomFn>),
}

/// Prints the kind and the name of the place, never a request value.
impl std::fmt::Debug for JwtExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Kind::AuthHeader { scheme } => write!(f, "JwtExtractor::AuthHeader({scheme})"),
            Kind::Header { name } => write!(f, "JwtExtractor::Header({name})"),
            Kind::Query { name } => write!(f, "JwtExtractor::Query({name})"),
            Kind::Body { name } => write!(f, "JwtExtractor::Body({name})"),
            Kind::Cookie { name } => write!(f, "JwtExtractor::Cookie({name})"),
            Kind::First(list) => f.debug_tuple("JwtExtractor::First").field(list).finish(),
            Kind::Custom(_) => f.write_str("JwtExtractor::Custom(..)"),
        }
    }
}

/// The extractors of `passport-jwt` 4.0.1 (`ExtractJwt`), plus [`from_cookie`](ExtractJwt::from_cookie) and
/// [`custom`](ExtractJwt::custom). An empty string, or a value that is not a string, is no token.
///
/// ```ignore
/// JwtStrategyOptions::new()
///     .jwt_from_request(ExtractJwt::from_extractors([
///         ExtractJwt::from_auth_header_as_bearer_token(),
///         ExtractJwt::from_cookie("session"),
///     ]))
/// ```
pub struct ExtractJwt;

impl ExtractJwt {
    /// `fromAuthHeaderAsBearerToken()`: `Authorization: Bearer <token>`.
    pub fn from_auth_header_as_bearer_token() -> JwtExtractor {
        Self::from_auth_header_with_scheme("Bearer")
    }

    /// `fromAuthHeaderWithScheme(scheme)`. The header is parsed as `passport-jwt` does: its first value, read as latin1,
    /// is matched with `(\S+)\s+(\S+)` using JavaScript's whitespace, and the second group is the token when the
    /// first equals `scheme` ignoring case. A tab can separate them and words after the token are ignored.
    pub fn from_auth_header_with_scheme(scheme: impl Into<String>) -> JwtExtractor {
        JwtExtractor(Kind::AuthHeader { scheme: scheme.into() })
    }

    /// `fromHeader(name)`: repeated headers are joined as Node joins them (`", "`). The name is case-insensitive
    /// (`passport-jwt` only finds names written in lowercase).
    pub fn from_header(name: impl AsRef<str>) -> JwtExtractor {
        JwtExtractor(Kind::Header { name: name.as_ref().to_ascii_lowercase() })
    }

    /// `fromUrlQueryParameter(name)`. Tokens in URLs end up in access logs and browser history: the strategy warns at
    /// startup when this extractor is configured.
    pub fn from_url_query_parameter(name: impl Into<String>) -> JwtExtractor {
        JwtExtractor(Kind::Query { name: name.into() })
    }

    /// `fromBodyField(name)`: a string field of the parsed body.
    pub fn from_body_field(name: impl Into<String>) -> JwtExtractor {
        JwtExtractor(Kind::Body { name: name.into() })
    }

    /// The cookie `name`, parsed like `cookie-parser` (the equivalent of `req => req.cookies?.[name]`), with one
    /// difference on purpose: a name that appears more than once in the request yields no token (the uniform 401)
    /// instead of the first value, since a cookie planted from a sibling subdomain would otherwise win. The strategy
    /// warns at startup when the name has no `__Host-` prefix.
    pub fn from_cookie(name: impl Into<String>) -> JwtExtractor {
        JwtExtractor(Kind::Cookie { name: name.into() })
    }

    /// `fromExtractors([..])`: the first extractor that finds a token wins.
    pub fn from_extractors(extractors: impl IntoIterator<Item = JwtExtractor>) -> JwtExtractor {
        JwtExtractor(Kind::First(extractors.into_iter().collect()))
    }

    /// Any function of the request.
    pub fn custom(f: impl Fn(&Request) -> Option<String> + Send + Sync + 'static) -> JwtExtractor {
        JwtExtractor(Kind::Custom(Arc::new(f)))
    }
}

impl JwtExtractor {
    /// The token of `req`, `None` when there is none (an empty string counts as none).
    pub fn extract(&self, req: &Request) -> Option<String> {
        let found = match &self.0 {
            Kind::AuthHeader { scheme } => auth_header(req.headers(), scheme),
            Kind::Header { name } => header_value(req.headers(), name),
            Kind::Query { name } => req.query().get(name).and_then(|v| v.as_str()).map(str::to_string),
            Kind::Body { name } => req.body().get(name).and_then(|v| v.as_str()).map(str::to_string),
            // A name sent twice means a cookie planted from somewhere else (a sibling subdomain can set cookies for the
            // whole domain, in front of the victim's own): no token, as the session cookie does.
            Kind::Cookie { name } => {
                let mut values = req.cookie_values(name, 2, usize::MAX);
                if values.len() == 1 { values.pop() } else { None }
            }
            Kind::First(list) => list.iter().find_map(|e| e.extract(req)),
            Kind::Custom(f) => f(req),
        };
        found.filter(|t| !t.is_empty())
    }

    /// `true` when this extractor, or one it combines, reads the query string.
    pub(crate) fn reads_query(&self) -> bool {
        match &self.0 {
            Kind::Query { .. } => true,
            Kind::First(list) => list.iter().any(JwtExtractor::reads_query),
            _ => false,
        }
    }

    /// The names of the cookies this extractor, or one it combines, reads (a custom function is opaque).
    pub(crate) fn cookie_names(&self) -> Vec<String> {
        match &self.0 {
            Kind::Cookie { name } => vec![name.clone()],
            Kind::First(list) => list.iter().flat_map(JwtExtractor::cookie_names).collect(),
            _ => Vec::new(),
        }
    }

    /// The names of the query parameters this extractor, or one it combines, reads (a custom function is opaque).
    pub(crate) fn query_names(&self) -> Vec<String> {
        match &self.0 {
            Kind::Query { name } => vec![name.clone()],
            Kind::First(list) => list.iter().flat_map(JwtExtractor::query_names).collect(),
            _ => Vec::new(),
        }
    }
}

/// `true` when the latin1 bytes `found` are `scheme`, ignoring ASCII case.
fn is_scheme(found: &[u8], scheme: &str) -> bool {
    scheme.chars().count() == found.len() && scheme.chars().zip(found).all(|(c, &b)| c.eq_ignore_ascii_case(&char::from(b)))
}

fn auth_header(headers: &HeaderMap, scheme: &str) -> Option<String> {
    let credentials = auth_header::parse(auth_header::first_value(headers)?, None)?;
    if !is_scheme(credentials.scheme, scheme) {
        return None;
    }
    let token = credentials.token(None).ok()?;
    (!token.is_empty()).then(|| latin1(token))
}

/// Headers whose repeated values Node drops after the first (`IncomingMessage` `_addHeaderLine`).
const FIRST_VALUE_ONLY: &[&str] = &[
    "age",
    "authorization",
    "content-length",
    "content-type",
    "etag",
    "expires",
    "from",
    "host",
    "if-modified-since",
    "if-unmodified-since",
    "last-modified",
    "location",
    "max-forwards",
    "proxy-authorization",
    "referer",
    "retry-after",
    "server",
    "user-agent",
];

/// The value of the header `name` (lowercase) as Node's `req.headers[name]` shows it: latin1, repeated values joined
/// with `", "` (`"; "` for `cookie`), or only the first for the headers Node keeps once.
fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers.get_all(name).iter().map(|v| latin1(v.as_bytes()));
    if FIRST_VALUE_ONLY.contains(&name) {
        return values.next();
    }
    let first = values.next()?;
    let separator = if name == "cookie" { "; " } else { ", " };
    Some(values.fold(first, |acc, v| format!("{acc}{separator}{v}")))
}

#[cfg(test)]
mod tests {
    use http::{HeaderName, HeaderValue};
    use proptest::prelude::*;

    use super::*;

    fn request(headers: &[(&str, &[u8])]) -> Request {
        let mut req = Request::empty();
        for (name, value) in headers {
            req.headers_mut().append(HeaderName::from_bytes(name.as_bytes()).unwrap(), HeaderValue::from_bytes(value).unwrap());
        }
        req
    }

    fn bearer(value: &[u8]) -> Option<String> {
        ExtractJwt::from_auth_header_as_bearer_token().extract(&request(&[("authorization", value)]))
    }

    #[test]
    fn the_authorization_header_is_parsed_like_passport_jwt() {
        assert_eq!(bearer(b"Bearer abc"), Some("abc".into()));
        assert_eq!(bearer(b"bearer abc"), Some("abc".into()));
        assert_eq!(bearer(b"BEARER abc"), Some("abc".into()));
        assert_eq!(bearer(b"Bearer\tabc"), Some("abc".into()));
        assert_eq!(bearer(b"Bearer \t  abc"), Some("abc".into()));
        assert_eq!(bearer(b"Bearer\xa0abc"), Some("abc".into()), "the no-break space is JavaScript whitespace");
        assert_eq!(bearer(b"Bearer abc extra"), Some("abc".into()));
        assert_eq!(bearer(b"  Bearer abc"), Some("abc".into()));
        assert_eq!(bearer(b"Bearer abc "), Some("abc".into()));
        assert_eq!(bearer(b"Basic abc"), None);
        assert_eq!(bearer(b"Bearer"), None);
        assert_eq!(bearer(b"Bearer "), None);
        assert_eq!(bearer(b""), None);
        assert_eq!(bearer(b"abc"), None);
        assert_eq!(bearer(b"Bearer\x85abc"), None, "0x85 is not whitespace in JavaScript");
        assert_eq!(bearer(b"Bearerabc"), None);
        assert_eq!(bearer(b"Bearer: abc"), None);
        let with_scheme = ExtractJwt::from_auth_header_with_scheme("JWT");
        assert_eq!(with_scheme.extract(&request(&[("authorization", b"jwt abc")])), Some("abc".into()));
        assert_eq!(with_scheme.extract(&request(&[("authorization", b"Bearer abc")])), None);
    }

    #[test]
    fn only_the_first_authorization_value_counts() {
        let req = request(&[("authorization", b"Basic x"), ("authorization", b"Bearer abc")]);
        assert_eq!(ExtractJwt::from_auth_header_as_bearer_token().extract(&req), None);
        let req = request(&[("authorization", b"Bearer abc"), ("authorization", b"Bearer other")]);
        assert_eq!(ExtractJwt::from_auth_header_as_bearer_token().extract(&req), Some("abc".into()));
    }

    #[test]
    fn extract_bearer_token_is_unchanged() {
        let req = request(&[("authorization", b"Bearer\tabc")]);
        assert_eq!(crate::jwt::extract_bearer_token(&req), None);
        let req = request(&[("authorization", b"Bearer abc extra")]);
        assert_eq!(crate::jwt::extract_bearer_token(&req), Some("abc extra".into()));
        let req = request(&[("authorization", b"bearer  abc")]);
        assert_eq!(crate::jwt::extract_bearer_token(&req), Some("abc".into()));
    }

    #[test]
    fn from_header_joins_repeated_values_and_ignores_the_case_of_the_name() {
        let req = request(&[("x-token", b"abc"), ("x-token", b"abc")]);
        assert_eq!(ExtractJwt::from_header("x-token").extract(&req), Some("abc, abc".into()));
        assert_eq!(ExtractJwt::from_header("X-Token").extract(&req), Some("abc, abc".into()));
        let one = request(&[("x-token", b"abc")]);
        assert_eq!(ExtractJwt::from_header("X-Token").extract(&one), Some("abc".into()));
        assert_eq!(ExtractJwt::from_header("x-other").extract(&one), None);
        let empty = request(&[("x-token", b"")]);
        assert_eq!(ExtractJwt::from_header("x-token").extract(&empty), None);
        let ua = request(&[("user-agent", b"a"), ("user-agent", b"b")]);
        assert_eq!(ExtractJwt::from_header("user-agent").extract(&ua), Some("a".into()));
    }

    #[test]
    fn query_body_and_cookie_extractors_read_strings_only() {
        let mut req = request(&[("cookie", b"a=1; jwt=abc; b=2")]);
        assert_eq!(ExtractJwt::from_cookie("jwt").extract(&req), Some("abc".into()));
        assert_eq!(ExtractJwt::from_cookie("missing").extract(&req), None);
        let empty = request(&[("cookie", b"jwt=; b=2")]);
        assert_eq!(ExtractJwt::from_cookie("jwt").extract(&empty), None);

        req.set_body(serde_json::json!({ "token": "abc", "num": 5, "obj": { "a": 1 }, "empty": "" }));
        assert_eq!(ExtractJwt::from_body_field("token").extract(&req), Some("abc".into()));
        for name in ["num", "obj", "empty", "missing"] {
            assert_eq!(ExtractJwt::from_body_field(name).extract(&req), None, "{name}");
        }
        req.set_body(serde_json::json!("not an object"));
        assert_eq!(ExtractJwt::from_body_field("token").extract(&req), None);

        let query =
            Request::new(http::Method::GET, "/x?token=abc&n=5&list=a&list=b&e=".parse().unwrap(), HeaderMap::new(), bytes::Bytes::new())
                .unwrap();
        assert_eq!(ExtractJwt::from_url_query_parameter("token").extract(&query), Some("abc".into()));
        for name in ["n_missing", "list", "e"] {
            assert_eq!(ExtractJwt::from_url_query_parameter(name).extract(&query), None, "{name}");
        }
    }

    #[test]
    fn the_first_non_empty_extractor_wins() {
        let both = request(&[("authorization", b"Bearer fromheader"), ("cookie", b"jwt=fromcookie")]);
        let cookie_first = ExtractJwt::from_extractors([ExtractJwt::from_cookie("jwt"), ExtractJwt::from_auth_header_as_bearer_token()]);
        assert_eq!(cookie_first.extract(&both), Some("fromcookie".into()));
        let only_header = request(&[("authorization", b"Bearer fromheader")]);
        assert_eq!(cookie_first.extract(&only_header), Some("fromheader".into()));
        let custom = ExtractJwt::from_extractors([ExtractJwt::custom(|_| Some(String::new())), ExtractJwt::from_cookie("jwt")]);
        assert_eq!(custom.extract(&both), Some("fromcookie".into()), "an empty token is skipped");
        assert_eq!(ExtractJwt::from_extractors([]).extract(&both), None);
    }

    #[test]
    fn reads_query_looks_inside_combinations() {
        assert!(ExtractJwt::from_url_query_parameter("t").reads_query());
        assert!(ExtractJwt::from_extractors([ExtractJwt::from_cookie("c"), ExtractJwt::from_url_query_parameter("t")]).reads_query());
        assert!(!ExtractJwt::from_auth_header_as_bearer_token().reads_query());
    }

    #[test]
    fn debug_output_does_not_print_request_values() {
        let text = format!("{:?}", ExtractJwt::from_extractors([ExtractJwt::custom(|_| Some("secret".into()))]));
        assert!(!text.contains("secret"));
    }

    proptest! {
        /// Random bytes in every place a token can come from never panic the extractors.
        #[test]
        fn random_values_never_panic(value in proptest::collection::vec(any::<u8>(), 0..200)) {
            let mut req = Request::empty();
            if let Ok(v) = HeaderValue::from_bytes(&value) {
                for name in ["authorization", "x-token", "cookie"] {
                    req.headers_mut().append(HeaderName::from_static(name), v.clone());
                }
            }
            let text = String::from_utf8_lossy(&value).to_string();
            req.set_body(serde_json::json!({ "token": text }));
            let extractors = [
                ExtractJwt::from_auth_header_as_bearer_token(),
                ExtractJwt::from_header("x-token"),
                ExtractJwt::from_cookie("jwt"),
                ExtractJwt::from_body_field("token"),
                ExtractJwt::from_url_query_parameter("token"),
            ];
            for e in &extractors {
                let _ = e.extract(&req);
            }
        }
    }
}

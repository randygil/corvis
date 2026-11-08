//! Response defaults of the Express platform, applied to
//! every HTTP response right before it is written:
//!
//! - `X-Powered-By` (`app.disable('x-powered-by')` turns it off);
//! - `ETag` for bodies sent with `res.send()` / `res.json()` (JSON, text and HTML
//!   bodies), weak by default (`app.set('etag', 'strong' | false)`);
//! - conditional GET/HEAD (`If-None-Match` / `If-Modified-Since`) answered with `304`;
//! - `charset=utf-8` forced on explicit `Content-Type`s of text/JSON bodies, and the
//!   `Content-Type`/`Content-Length` stripping of `204`/`304` and the empty body of `205`.
//!
//! Raw bodies (`Response::bytes`, `StreamableFile`, streams) are written untouched,
//! like `res.end(buffer)` / piped streams in Express: no ETag, no charset change.

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};

use super::response::{Body, Response};

/// `app.set('etag', ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EtagMode {
    /// `W/"<len>-<sha1>"` (Express default, `'weak'` / `true`).
    #[default]
    Weak,
    /// `"<len>-<sha1>"` (`'strong'`).
    Strong,
    /// No ETag (`false`).
    Disabled,
}

/// HTTP response settings of the application (`app.set` / `app.enable` / `app.disable`).
#[derive(Debug, Clone)]
pub struct HttpSettings {
    /// Value of `X-Powered-By`; `None` disables the header.
    pub x_powered_by: Option<String>,
    pub etag: EtagMode,
    /// `express.json()` / `express.urlencoded()` options (`app.useBodyParser`).
    pub body_parser: super::body_parser::BodyParserOptions,
}

impl Default for HttpSettings {
    fn default() -> Self {
        HttpSettings { x_powered_by: Some("corvis".into()), etag: EtagMode::Weak, body_parser: Default::default() }
    }
}

/// The parts of the request that the response finalization needs.
#[derive(Debug, Clone)]
pub(crate) struct RequestInfo {
    pub method: Method,
    pub headers: HeaderMap,
}

impl RequestInfo {
    pub fn new(method: &Method, headers: &HeaderMap) -> Self {
        let mut h = HeaderMap::new();
        for name in [header::IF_NONE_MATCH, header::IF_MODIFIED_SINCE, header::CACHE_CONTROL] {
            for v in headers.get_all(&name) {
                h.append(name.clone(), v.clone());
            }
        }
        RequestInfo { method: method.clone(), headers: h }
    }
}

/// The `etag` package: `"<hex length>-<base64(sha1) first 27 chars>"`.
pub fn entity_tag(body: &[u8], weak: bool) -> String {
    use base64::Engine;
    use sha1::{Digest, Sha1};
    let tag = if body.is_empty() {
        "\"0-2jmj7l5rSw0yVb/vlWAYkK/YBwk\"".to_string()
    } else {
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha1::digest(body));
        format!("\"{:x}-{}\"", body.len(), &hash[..27])
    };
    if weak { format!("W/{tag}") } else { tag }
}

/// The `fresh` package: is the cached representation described by the request's
/// conditional headers still fresh given the response `ETag` / `Last-Modified`?
pub fn fresh(req: &HeaderMap, res: &HeaderMap) -> bool {
    let get = |h: &HeaderMap, n: header::HeaderName| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let modified_since = get(req, header::IF_MODIFIED_SINCE).filter(|s| !s.is_empty());
    let none_match = get(req, header::IF_NONE_MATCH).filter(|s| !s.is_empty());
    if modified_since.is_none() && none_match.is_none() {
        return false;
    }
    if let Some(cc) = get(req, header::CACHE_CONTROL) {
        // /(?:^|,)\s*?no-cache\s*?(?:,|$)/
        if cc.split(',').any(|p| p.trim_matches([' ', '\t']) == "no-cache") {
            return false;
        }
    }
    if let Some(none_match) = none_match {
        if none_match == "*" {
            return true;
        }
        let Some(etag) = get(res, header::ETAG) else { return false };
        return parse_token_list(&none_match)
            .iter()
            .any(|m| *m == etag || format!("W/{etag}") == *m || format!("W/{m}") == etag);
    }
    if let Some(since) = modified_since {
        let last = get(res, header::LAST_MODIFIED).and_then(|l| parse_http_date(&l));
        let since = parse_http_date(&since);
        return matches!((last, since), (Some(l), Some(s)) if l <= s);
    }
    true
}

fn parse_token_list(s: &str) -> Vec<String> {
    let bytes = s.as_bytes();
    let (mut start, mut end) = (0usize, 0usize);
    let mut list = Vec::new();
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b' ' => {
                if start == end {
                    start = i + 1;
                    end = i + 1;
                }
            }
            b',' => {
                list.push(s[start..end].to_string());
                start = i + 1;
                end = i + 1;
            }
            _ => end = i + 1,
        }
    }
    list.push(s[start..end].to_string());
    list
}

fn parse_http_date(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(s.trim()).ok().map(|d| d.timestamp_millis()).or_else(|| {
        // IMF-fixdate with a literal "GMT" zone, e.g. "Sun, 06 Nov 1994 08:49:37 GMT".
        chrono::NaiveDateTime::parse_from_str(s.trim(), "%a, %d %b %Y %H:%M:%S GMT").ok().map(|d| d.and_utc().timestamp_millis())
    })
}

/// Charset of a MIME type per `mime-db` (`mime.charset(type)`): text types and a few
/// application types are UTF-8.
pub fn default_charset(mime: &str) -> Option<&'static str> {
    let t = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    const UTF8: &[&str] = &[
        "application/3gpdash-qoe-report+xml",
        "application/beep+xml",
        "application/cda+xml",
        "application/elm+json",
        "application/emergencycalldata.cap+xml",
        "application/fhir+json",
        "application/fhir+xml",
        "application/hl7v2+xml",
        "application/im-iscomposing+xml",
        "application/javascript",
        "application/json",
        "application/manifest+json",
        "application/msc-ivr+xml",
        "application/msc-mixer+xml",
        "application/pidf+xml",
        "application/pidf-diff+xml",
        "application/poc-settings+xml",
        "application/vnd.omads-email+xml",
        "application/vnd.omads-file+xml",
        "application/vnd.omads-folder+xml",
        "application/vnd.syncml+xml",
        "application/vnd.syncml.dm+wbxml",
        "application/vnd.syncml.dm+xml",
        "application/vnd.syncml.dmddf+xml",
        "application/vnd.syncml.dmtnds+xml",
        "application/vnd.wap.wbxml",
    ];
    match t.as_str() {
        "application/news-checkgroups" | "application/news-groupinfo" => Some("us-ascii"),
        "application/prs.cyn" => Some("7-bit"),
        "application/vnd.fdsn.stationxml+xml" => Some("xml-based"),
        _ if UTF8.contains(&t.as_str()) => Some("utf-8"),
        _ if t.starts_with("text/") => Some("utf-8"),
        _ => None,
    }
}

/// Express `res.set('Content-Type', value)` (`mime.contentType`): a bare extension
/// (`'json'`, `'html'`) is resolved to its MIME type and a default charset is appended
/// when the value has none.
pub fn content_type(value: &str) -> String {
    let mut v = value.to_string();
    if !v.contains('/') {
        let ext = v.trim_start_matches('.').to_ascii_lowercase();
        let path = std::path::PathBuf::from(format!("file.{ext}"));
        v = crate::common::mime::from_path(&path).split(';').next().unwrap_or("").to_string();
    }
    if !v.to_ascii_lowercase().contains("charset")
        && let Some(cs) = default_charset(&v) {
            v = format!("{v}; charset={cs}");
        }
    v
}

/// Express `utils.setCharset(type, charset)` (`content-type` parse + format): the type
/// is lower-cased, parameters sorted and `charset` replaced.
pub fn set_charset(content_type: &str, charset: &str) -> String {
    let mut parts = content_type.split(';');
    let ty = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if !ty.contains('/') {
        return content_type.to_string();
    }
    let mut params: Vec<(String, String)> = Vec::new();
    for p in parts {
        let Some((k, v)) = p.split_once('=') else { continue };
        let k = k.trim().to_ascii_lowercase();
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v).replace("\\\"", "\"");
        if k != "charset" {
            params.push((k, v));
        }
    }
    params.push(("charset".into(), charset.to_string()));
    params.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = ty;
    for (k, v) in params {
        let token = !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b));
        if token {
            out.push_str(&format!("; {k}={v}"));
        } else {
            out.push_str(&format!("; {k}=\"{}\"", v.replace('"', "\\\"")));
        }
    }
    out
}

/// Applies the Express response defaults to `resp` (see the module docs).
pub(crate) fn finalize(resp: &mut Response, req: &RequestInfo, settings: &HttpSettings) {
    if let Some(p) = &settings.x_powered_by
        && !resp.headers.contains_key("x-powered-by")
            && let Ok(v) = HeaderValue::from_str(p) {
                resp.headers.insert(header::HeaderName::from_static("x-powered-by"), v);
            }
    // Only bodies written through `res.send()` / `res.json()` get the send() semantics;
    // empty, raw and streamed bodies are written as they are (`res.end(...)`, pipes).
    let sent: Option<(Bytes, &'static str)> = match &resp.body {
        Body::Json(v) => Some((Bytes::from(serde_json::to_vec(v).unwrap_or_default()), "application/json; charset=utf-8")),
        Body::Text(t) | Body::Html(t) => Some((Bytes::from(t.clone()), "text/html; charset=utf-8")),
        // `res.send(buffer)` through `@Res()`.
        Body::Bytes { data, .. } if resp.express_send => Some((data.clone(), "application/octet-stream")),
        _ => None,
    };
    let Some((data, default_ct)) = sent else { return };
    {
        match resp.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
            // `res.send(buffer)` keeps its type as is; strings are written in utf-8.
            Some(_) if matches!(resp.body, Body::Bytes { .. }) => {}
            Some(ct) => {
                let ct = set_charset(ct, "utf-8");
                if let Ok(v) = HeaderValue::from_str(&ct) {
                    resp.headers.insert(header::CONTENT_TYPE, v);
                }
            }
            None => {
                resp.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(default_ct));
            }
        }
        let etag = match settings.etag {
            EtagMode::Weak => Some(true),
            EtagMode::Strong => Some(false),
            EtagMode::Disabled => None,
        };
        if let Some(weak) = etag
            && !resp.headers.contains_key(header::ETAG)
                && let Ok(v) = HeaderValue::from_str(&entity_tag(&data, weak)) {
                    resp.headers.insert(header::ETAG, v);
                }
        resp.headers.insert(header::CONTENT_LENGTH, HeaderValue::from(data.len()));
        let ct = resp.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or(default_ct).to_string();
        resp.body = Body::Bytes { data, content_type: ct };
    }
    let status = resp.status.unwrap_or(StatusCode::OK);
    let conditional = req.method == Method::GET || req.method == Method::HEAD;
    if conditional && (status.is_success() || status == StatusCode::NOT_MODIFIED) && fresh(&req.headers, &resp.headers) {
        resp.status = Some(StatusCode::NOT_MODIFIED);
    }
    let status = resp.status.unwrap_or(StatusCode::OK);
    if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_MODIFIED {
        resp.headers.remove(header::CONTENT_TYPE);
        resp.headers.remove(header::CONTENT_LENGTH);
        resp.headers.remove(header::TRANSFER_ENCODING);
        resp.body = Body::Empty;
    } else if status == StatusCode::RESET_CONTENT {
        resp.headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("0"));
        resp.headers.remove(header::TRANSFER_ENCODING);
        resp.body = Body::Empty;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_matches_the_etag_package() {
        // Values produced by express 5 / etag 1.8.1 for the same bodies.
        assert_eq!(
            entity_tag(br#"{"message":"Cannot GET /x","error":"Not Found","statusCode":404}"#, true),
            r#"W/"40-Lts3F9bcD4+FSck1x7cIWF0Qf44""#
        );
        assert_eq!(entity_tag(b"", true), r#"W/"0-2jmj7l5rSw0yVb/vlWAYkK/YBwk""#);
        assert_eq!(entity_tag(b"", false), r#""0-2jmj7l5rSw0yVb/vlWAYkK/YBwk""#);
    }

    #[test]
    fn freshness() {
        let mut res = HeaderMap::new();
        res.insert(header::ETAG, HeaderValue::from_static("W/\"3-abc\""));
        let req = |pairs: &[(&'static str, &'static str)]| {
            let mut h = HeaderMap::new();
            for (k, v) in pairs {
                h.insert(*k, HeaderValue::from_static(v));
            }
            h
        };
        assert!(!fresh(&req(&[]), &res));
        assert!(fresh(&req(&[("if-none-match", "W/\"3-abc\"")]), &res));
        assert!(fresh(&req(&[("if-none-match", "\"x\", W/\"3-abc\"")]), &res));
        assert!(fresh(&req(&[("if-none-match", "*")]), &res));
        assert!(!fresh(&req(&[("if-none-match", "\"other\"")]), &res));
        assert!(!fresh(&req(&[("if-none-match", "W/\"3-abc\""), ("cache-control", "no-cache")]), &res));
        let mut lm = HeaderMap::new();
        lm.insert(header::LAST_MODIFIED, HeaderValue::from_static("Sun, 06 Nov 1994 08:49:37 GMT"));
        assert!(fresh(&req(&[("if-modified-since", "Sun, 06 Nov 1994 08:49:37 GMT")]), &lm));
        assert!(!fresh(&req(&[("if-modified-since", "Sat, 05 Nov 1994 08:49:37 GMT")]), &lm));
    }

    #[test]
    fn charsets() {
        assert_eq!(content_type("text/plain"), "text/plain; charset=utf-8");
        assert_eq!(content_type("application/json"), "application/json; charset=utf-8");
        assert_eq!(content_type("application/pdf"), "application/pdf");
        assert_eq!(content_type("text/plain; charset=latin1"), "text/plain; charset=latin1");
        assert_eq!(content_type("json"), "application/json; charset=utf-8");
        assert_eq!(set_charset("text/plain; charset=windows-1252", "utf-8"), "text/plain; charset=utf-8");
        assert_eq!(set_charset("Text/CSV; header=present", "utf-8"), "text/csv; charset=utf-8; header=present");
        assert_eq!(set_charset("application/vnd.api+json", "utf-8"), "application/vnd.api+json; charset=utf-8");
    }
}

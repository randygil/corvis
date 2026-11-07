//! `express.static(root, options)` (`serve-static` 2 + `send` 1): the middleware Express apps
//! mount with `app.use('/uploads', express.static(join(process.cwd(), 'uploads')))`.
//!
//! ```ignore
//! app.use_middleware(corvis::serve_static("uploads").mount("/uploads"));
//! ```
//!
//! Same semantics as Express:
//! - Only `GET`/`HEAD`; other methods fall through to the next handler (`fallthrough`).
//! - 4xx errors before a file is found (bad encoding, `..` traversal, dotfiles, missing files)
//!   fall through, so the app answers its usual 404.
//! - A directory without trailing slash redirects (`301`, `Location: /dir/`, the same HTML
//!   body); with trailing slash it serves `index.html` or falls through.
//! - Files get `Accept-Ranges`, `Cache-Control: public, max-age=N`, `Last-Modified`, a weak
//!   `ETag` (`W/"<size hex>-<mtime hex>"`), the `mime-types` content type, conditional GET
//!   (`304`, `412`) and single byte ranges (`206`, `416`).

use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use bytes::Bytes;
use http::StatusCode;

use super::{HandlerResult, Request, Response};
use crate::common::middleware::{CorvisMiddleware, Next};

/// `dotfiles` option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dotfiles {
    Allow,
    Deny,
    #[default]
    Ignore,
}

/// `express.static(root)`. Mount it with [`ServeStatic::mount`] (`app.use(path, ...)`).
#[derive(Debug, Clone)]
pub struct ServeStatic {
    root: PathBuf,
    mount: String,
    fallthrough: bool,
    redirect: bool,
    max_age_ms: u64,
    immutable: bool,
    index: Vec<String>,
    dotfiles: Dotfiles,
    etag: bool,
    last_modified: bool,
    accept_ranges: bool,
    cache_control: bool,
    extensions: Vec<String>,
}

/// `express.static(root)`.
pub fn serve_static(root: impl AsRef<Path>) -> ServeStatic {
    ServeStatic::new(root)
}

impl ServeStatic {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
        ServeStatic {
            root,
            mount: String::new(),
            fallthrough: true,
            redirect: true,
            max_age_ms: 0,
            immutable: false,
            index: vec!["index.html".into()],
            dotfiles: Dotfiles::Ignore,
            etag: true,
            last_modified: true,
            accept_ranges: true,
            cache_control: true,
            extensions: Vec::new(),
        }
    }
    /// `app.use(path, express.static(...))`: only requests under `path` (case-insensitive,
    /// whole segments) are served, with `path` stripped.
    pub fn mount(mut self, path: impl Into<String>) -> Self {
        self.mount = path.into().trim_end_matches('/').to_string();
        self
    }
    pub fn fallthrough(mut self, v: bool) -> Self {
        self.fallthrough = v;
        self
    }
    pub fn redirect(mut self, v: bool) -> Self {
        self.redirect = v;
        self
    }
    /// `maxAge` in milliseconds (capped at one year).
    pub fn max_age(mut self, ms: u64) -> Self {
        self.max_age_ms = ms.min(365 * 24 * 60 * 60 * 1000);
        self
    }
    pub fn immutable(mut self, v: bool) -> Self {
        self.immutable = v;
        self
    }
    /// `index: false` → `index(&[])`.
    pub fn index(mut self, files: &[&str]) -> Self {
        self.index = files.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn dotfiles(mut self, d: Dotfiles) -> Self {
        self.dotfiles = d;
        self
    }
    pub fn etag(mut self, v: bool) -> Self {
        self.etag = v;
        self
    }
    pub fn last_modified(mut self, v: bool) -> Self {
        self.last_modified = v;
        self
    }
    pub fn accept_ranges(mut self, v: bool) -> Self {
        self.accept_ranges = v;
        self
    }
    pub fn cache_control(mut self, v: bool) -> Self {
        self.cache_control = v;
        self
    }
    /// `extensions: ['html']`: fallback extensions for extensionless paths.
    pub fn extensions(mut self, exts: &[&str]) -> Self {
        self.extensions = exts.iter().map(|s| s.to_string()).collect();
        self
    }

    /// The path inside the mount (`parseurl(req).pathname` after Express strips the mount).
    fn inner_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        if self.mount.is_empty() {
            return Some(path);
        }
        let head = path.get(..self.mount.len())?;
        if !head.eq_ignore_ascii_case(&self.mount) {
            return None;
        }
        let rest = &path[self.mount.len()..];
        if rest.is_empty() {
            Some("/")
        } else if rest.starts_with('/') {
            Some(rest)
        } else {
            None
        }
    }
}

/// What `send` decided.
enum Outcome {
    /// Fall through to the next handler (`next()`).
    Next,
    /// A response written by the middleware.
    Respond(Response),
}

fn html_document(title: &str, body: &str) -> String {
    format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n</head>\n<body>\n<pre>{body}</pre>\n</body>\n</html>\n"
    )
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

/// `encodeurl`: percent-encodes what is not allowed in a URL, keeping existing escapes.
fn encode_url(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let keep = c.is_ascii_alphanumeric() || b"!#$&'()*+,-./:;=?@[]_~".contains(&c);
        if c == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            out.push('%');
        } else if keep {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
        i += 1;
    }
    out
}

fn collapse_leading_slashes(s: &str) -> String {
    let n = s.bytes().take_while(|&c| c == b'/').count();
    if n > 1 { format!("/{}", &s[n..]) } else { s.to_string() }
}

fn http_date(t: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = t.into();
    dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn parse_http_date(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(s.trim()).ok().map(|d| d.timestamp())
}

fn content_type(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        // mime-db 1.54
        "js" | "mjs" => "text/javascript; charset=utf-8".into(),
        _ => crate::common::mime::from_path(path).to_string(),
    }
}

/// `range-parser` for a single combined range: `None` = ignore the header, `Some(Err)` = 416.
fn parse_range(size: u64, header: &str) -> Option<Result<(u64, u64), ()>> {
    let spec = header.trim_start().strip_prefix("bytes=")?;
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        let (a, b) = part.split_once('-')?;
        let (start, end) = if a.is_empty() {
            let n: u64 = b.parse().ok()?;
            (size.saturating_sub(n), size.saturating_sub(1))
        } else {
            let s: u64 = a.parse().ok()?;
            let e: u64 = if b.is_empty() { size.saturating_sub(1) } else { b.parse::<u64>().ok()?.min(size.saturating_sub(1)) };
            (s, e)
        };
        if start > end || start >= size {
            continue;
        }
        ranges.push((start, end));
    }
    if ranges.is_empty() {
        return Some(Err(()));
    }
    ranges.sort();
    let mut combined: Vec<(u64, u64)> = Vec::new();
    for r in ranges {
        match combined.last_mut() {
            Some(last) if r.0 <= last.1 + 1 => last.1 = last.1.max(r.1),
            _ => combined.push(r),
        }
    }
    if combined.len() == 1 { Some(Ok(combined[0])) } else { None }
}

impl ServeStatic {
    fn error_response(status: u16) -> Response {
        let msg = StatusCode::from_u16(status).ok().and_then(|s| s.canonical_reason()).unwrap_or("Error").to_string();
        let doc = html_document("Error", &escape_html(&msg));
        Response::bytes(doc.into_bytes(), "text/html; charset=UTF-8")
            .with_status(status)
            .with_header("Content-Security-Policy", "default-src 'none'")
            .with_header("X-Content-Type-Options", "nosniff")
    }

    /// `next()` for 4xx errors while no file was found yet (fallthrough), else the error page.
    fn fail(&self, status: u16, file_found: bool) -> Outcome {
        if self.fallthrough && !file_found && status < 500 {
            Outcome::Next
        } else {
            Outcome::Respond(Self::error_response(status))
        }
    }

    async fn serve(&self, req: &Request, inner: &str) -> Outcome {
        if req.method() != http::Method::GET && req.method() != http::Method::HEAD {
            if self.fallthrough {
                return Outcome::Next;
            }
            return Outcome::Respond(
                Response::bytes(Vec::new(), "").with_status(405).with_header("Allow", "GET, HEAD").with_header("Content-Length", "0"),
            );
        }
        let original = req.path().to_string();
        // Make sure the redirect occurs at the mount.
        let path = if inner == "/" && !original.ends_with('/') { "" } else { inner };
        let has_trailing_slash = path.ends_with('/');

        let Ok(decoded) = percent_encoding::percent_decode_str(path).decode_utf8() else { return self.fail(400, false) };
        let decoded = decoded.into_owned();
        if decoded.contains('\0') {
            return self.fail(400, false);
        }
        // `normalize('.' + sep + path)` and the UP_PATH check (both separators on Windows).
        let rel = decoded.replace('\\', "/");
        let mut parts: Vec<&str> = Vec::new();
        for seg in rel.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.last().is_some_and(|p| *p != "..") {
                        parts.pop();
                    } else {
                        parts.push("..");
                    }
                }
                s => parts.push(s),
            }
        }
        if parts.contains(&"..") {
            return self.fail(403, false);
        }
        if parts.iter().any(|p| p.len() > 1 && p.starts_with('.')) {
            match self.dotfiles {
                Dotfiles::Allow => {}
                Dotfiles::Deny => return self.fail(403, false),
                Dotfiles::Ignore => return self.fail(404, false),
            }
        }
        let mut file = self.root.clone();
        for p in &parts {
            file.push(p);
        }
        if parts.iter().any(|p| Path::new(p).components().any(|c| !matches!(c, Component::Normal(_)))) {
            // Drive prefixes or roots smuggled in a segment (`C:`): never leave the root.
            return self.fail(403, false);
        }

        if !self.index.is_empty() && has_trailing_slash {
            for idx in &self.index {
                let p = file.join(idx);
                if let Ok(meta) = tokio::fs::metadata(&p).await {
                    if meta.is_file() {
                        return Outcome::Respond(self.send(req, &p, &meta).await);
                    }
                }
            }
            return self.fail(404, false);
        }

        let meta = match tokio::fs::metadata(&file).await {
            Ok(m) => m,
            Err(_) => {
                if file.extension().is_none() && !has_trailing_slash {
                    for ext in &self.extensions {
                        let p = PathBuf::from(format!("{}.{ext}", file.display()));
                        if let Ok(m) = tokio::fs::metadata(&p).await {
                            if m.is_file() {
                                return Outcome::Respond(self.send(req, &p, &m).await);
                            }
                        }
                    }
                }
                return self.fail(404, false);
            }
        };
        if meta.is_dir() {
            if !self.redirect || has_trailing_slash {
                return self.fail(404, false);
            }
            let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
            let loc = encode_url(&format!("{}{query}", collapse_leading_slashes(&format!("{original}/"))));
            let doc = html_document("Redirecting", &format!("Redirecting to {}", escape_html(&loc)));
            return Outcome::Respond(
                Response::bytes(doc.into_bytes(), "text/html; charset=UTF-8")
                    .with_status(301)
                    .with_header("Content-Security-Policy", "default-src 'none'")
                    .with_header("X-Content-Type-Options", "nosniff")
                    .with_header("Location", loc),
            );
        }
        if has_trailing_slash {
            return self.fail(404, false);
        }
        Outcome::Respond(self.send(req, &file, &meta).await)
    }

    async fn send(&self, req: &Request, path: &Path, meta: &std::fs::Metadata) -> Response {
        let size = meta.len();
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        let mtime_ms = mtime.duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let mut headers: Vec<(&'static str, String)> = Vec::new();
        if self.accept_ranges {
            headers.push(("Accept-Ranges", "bytes".into()));
        }
        if self.cache_control {
            let mut cc = format!("public, max-age={}", self.max_age_ms / 1000);
            if self.immutable {
                cc.push_str(", immutable");
            }
            headers.push(("Cache-Control", cc));
        }
        let last_modified = http_date(mtime);
        if self.last_modified {
            headers.push(("Last-Modified", last_modified.clone()));
        }
        let etag = format!("W/\"{size:x}-{mtime_ms:x}\"");
        if self.etag {
            headers.push(("ETag", etag.clone()));
        }
        let ctype = content_type(path);
        let with_headers = |mut r: Response, hs: &[(&'static str, String)]| {
            for (k, v) in hs {
                r.set_header(k, v);
            }
            r
        };

        // Conditional GET.
        let h = |n: &str| req.header(n).map(String::from);
        let conditional = h("if-match").is_some() || h("if-unmodified-since").is_some() || h("if-none-match").is_some() || h("if-modified-since").is_some();
        if conditional {
            let precondition_failed = if let Some(m) = h("if-match") {
                !self.etag
                    || (m.trim() != "*"
                        && m.split(',').map(str::trim).all(|t| t != etag && format!("W/{t}") != etag && format!("W/{etag}") != t))
            } else if let Some(since) = h("if-unmodified-since").and_then(|s| parse_http_date(&s)) {
                parse_http_date(&last_modified).is_none_or(|lm| lm > since)
            } else {
                false
            };
            if precondition_failed {
                return Self::error_response(412);
            }
            if self.is_fresh(req, &etag, &last_modified) {
                return with_headers(Response::bytes(Vec::new(), "").with_status(304), &headers);
            }
        }

        let mut status = 200u16;
        let (mut start, mut len) = (0u64, size);
        if self.accept_ranges {
            if let Some(range) = h("range").filter(|r| r.trim_start().starts_with("bytes=")) {
                let fresh = match h("if-range") {
                    None => true,
                    Some(ir) if ir.contains('"') => self.etag && ir.contains(&etag),
                    Some(ir) => match (parse_http_date(&last_modified), parse_http_date(&ir)) {
                        (Some(a), Some(b)) => a <= b,
                        _ => false,
                    },
                };
                if fresh {
                    match parse_range(size, &range) {
                        Some(Err(())) => {
                            return Self::error_response(416).with_header("Content-Range", format!("bytes */{size}"));
                        }
                        Some(Ok((s, e))) => {
                            status = 206;
                            headers.push(("Content-Range", format!("bytes {s}-{e}/{size}")));
                            start = s;
                            len = e - s + 1;
                        }
                        None => {}
                    }
                }
            }
        }
        headers.push(("Content-Length", len.to_string()));
        let data = if req.method() == http::Method::HEAD {
            Bytes::new()
        } else {
            match tokio::fs::read(path).await {
                Ok(all) => {
                    let all = Bytes::from(all);
                    let end = (start + len).min(all.len() as u64);
                    all.slice(start.min(end) as usize..end as usize)
                }
                Err(_) => return Self::error_response(500),
            }
        };
        let mut r = with_headers(Response::bytes(data, ctype), &headers).with_status(status);
        if req.method() == http::Method::HEAD {
            r.set_header("Content-Length", len.to_string());
        }
        r
    }

    /// The `fresh` package.
    fn is_fresh(&self, req: &Request, etag: &str, last_modified: &str) -> bool {
        let none_match = req.header("if-none-match");
        let modified_since = req.header("if-modified-since");
        if none_match.is_none() && modified_since.is_none() {
            return false;
        }
        if req.header("cache-control").is_some_and(|c| c.contains("no-cache")) {
            return false;
        }
        if let Some(nm) = none_match.filter(|n| *n != "*") {
            if !self.etag {
                return false;
            }
            let matches = nm.split(',').map(str::trim).any(|t| t == etag || t == format!("W/{etag}") || format!("W/{t}") == etag);
            if !matches {
                return false;
            }
        }
        if let Some(ms) = modified_since {
            if !self.last_modified {
                return false;
            }
            match (parse_http_date(last_modified), parse_http_date(ms)) {
                (Some(lm), Some(since)) => {
                    if lm > since {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }
}

#[async_trait]
impl CorvisMiddleware for ServeStatic {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let Some(inner) = self.inner_path(req.path()).map(String::from) else { return next.run(req).await };
        match self.serve(&req, &inner).await {
            Outcome::Next => next.run(req).await,
            Outcome::Respond(r) => Ok(r),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_match_the_node_packages() {
        assert_eq!(collapse_leading_slashes("///a/"), "/a/");
        assert_eq!(encode_url("/a b/%20ñ"), "/a%20b/%20%C3%B1");
        assert_eq!(parse_range(10, "bytes=0-4"), Some(Ok((0, 4))));
        assert_eq!(parse_range(10, "bytes=-3"), Some(Ok((7, 9))));
        assert_eq!(parse_range(10, "bytes=20-30"), Some(Err(())));
        assert_eq!(parse_range(10, "bytes=0-1,5-6"), None);
        assert_eq!(parse_range(10, "bytes=0-4,3-8"), Some(Ok((0, 8))));
        assert!(html_document("Redirecting", "Redirecting to /x/").starts_with("<!DOCTYPE html>\n<html lang=\"en\">"));
    }
}

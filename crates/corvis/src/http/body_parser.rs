//! Request body parsing with the semantics of `body-parser` 2.x as an Express platform
//! registers it (`express.json()` and `express.urlencoded({ extended: true })`):
//!
//! - Only requests that have a body (`Transfer-Encoding` or a numeric `Content-Length`, like
//!   `type-is.hasBody`) and whose `Content-Type` is exactly `application/json` /
//!   `application/x-www-form-urlencoded` (parameters allowed) are parsed. Anything else leaves
//!   the body `undefined` (`Value::Null`).
//! - JSON: an empty body is `{}`; strict mode (only `{`/`[` at the top level); `JSON.parse` with
//!   V8's exact error messages ([`super::json_parse`]); the charset must be `utf-*`.
//! - urlencoded: `qs.parse` with body-parser's options (depth 32 with `strictDepth`, array limit
//!   `max(100, params)`, 1000 parameters); the charset must be `utf-8` or `iso-8859-1`.
//! - Limit of 100kb per parser (`app.useBodyParser(type, { limit })`).
//! - Nesting depth limited to [`BodyParserOptions::max_depth`] (256 by default): deeper JSON is a
//!   400. body-parser has no such limit, but a deep `serde_json::Value` overflows the stack when
//!   it is dropped, cloned or serialized (recursively), aborting the process.
//!
//! Errors are answered like on the Express platform: a JSON `SyntaxError` becomes
//! `BadRequestException(err.message)`; other `http-errors` (413, 415, 400) are answered by the
//! `BaseExceptionFilter` as `{ "statusCode": n, "message": "..." }`.

use bytes::Bytes;
use http::HeaderMap;
use serde_json::{Map, Value};

use super::exception::{BadRequestException, HttpException};
use super::json_parse;

/// `body-parser`'s default limit (`100kb`).
pub const DEFAULT_BODY_LIMIT: usize = 100 * 1024;

/// Options of the body parsers registered by the platform (`app.useBodyParser`) and of the
/// other request parsers (`query parser`, `trust proxy`).
#[derive(Debug, Clone)]
pub struct BodyParserOptions {
    /// `false` = [`CorvisApplication::disable_body_parser`](crate::CorvisApplication::disable_body_parser): bodies are never parsed.
    pub enabled: bool,
    /// Limit of `express.json()` in bytes.
    pub json_limit: usize,
    /// Limit of `express.urlencoded()` in bytes.
    pub urlencoded_limit: usize,
    /// Express' `query parser` setting used for `req.query`.
    pub query_parser: QueryParser,
    /// Maximum nesting depth of a JSON body (default [`json_parse::DEFAULT_MAX_DEPTH`], 256);
    /// deeper bodies are answered with 400. It also caps the depth of urlencoded bodies (32,
    /// like body-parser).
    pub max_depth: usize,
    /// Express' `trust proxy` setting used by `req.ip()`, `req.ips()` and `req.hostname()`
    /// (default: trust no proxy, the socket address is the client).
    pub trust_proxy: super::proxy_addr::TrustProxy,
}

/// `app.set('query parser', ...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueryParser {
    /// `node:querystring` (Express 5 default): `a[]=1` keeps the key `a[]`.
    #[default]
    Simple,
    /// `qs.parse(str, { allowPrototypes: true })` (Express 4 default).
    Extended,
    /// `false`: `req.query` is `{}`.
    Disabled,
}

impl Default for BodyParserOptions {
    fn default() -> Self {
        BodyParserOptions {
            enabled: true,
            json_limit: DEFAULT_BODY_LIMIT,
            urlencoded_limit: DEFAULT_BODY_LIMIT,
            query_parser: QueryParser::Simple,
            max_depth: json_parse::DEFAULT_MAX_DEPTH,
            trust_proxy: super::proxy_addr::TrustProxy::default(),
        }
    }
}

/// An `http-errors` error that is not an `HttpException`: `BaseExceptionFilter`
/// answers `{ statusCode, message }`.
fn http_error(status: u16, message: impl Into<String>) -> HttpException {
    HttpException::http_error(status, message)
}

/// Parses `body` as `body-parser` would. `Ok(Value::Null)` means `req.body === undefined`.
pub(crate) fn parse_body(headers: &HeaderMap, body: &Bytes, options: &BodyParserOptions) -> Result<Value, HttpException> {
    if !options.enabled || !has_body(headers, body) {
        return Ok(Value::Null);
    }
    // Node reads header values as latin1: a non-ASCII byte must not make the header vanish.
    let Some(ct) = headers.get(http::header::CONTENT_TYPE).map(|v| latin1(v.as_bytes())) else {
        return Ok(Value::Null);
    };
    let Some(media) = MediaType::parse(&ct) else {
        return Ok(Value::Null);
    };
    let kind = match media.essence.as_str() {
        "application/json" => Kind::Json,
        "application/x-www-form-urlencoded" => Kind::Urlencoded,
        _ => return Ok(Value::Null),
    };
    let charset = media.charset.clone().unwrap_or_else(|| "utf-8".into());
    let valid_charset = match kind {
        Kind::Json => charset.starts_with("utf-"),
        Kind::Urlencoded => charset == "utf-8" || charset == "iso-8859-1",
    };
    if !valid_charset {
        return Err(http_error(415, format!("unsupported charset \"{}\"", charset.to_uppercase())));
    }
    // `(headers['content-encoding'] || 'identity').toLowerCase()`: any value other than
    // `identity` (non-ASCII ones included) is a 415, never a body parsed in clear.
    let encoding = headers
        .get(http::header::CONTENT_ENCODING)
        .map(|v| latin1(v.as_bytes()).to_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "identity".into());
    if encoding != "identity" {
        return Err(http_error(415, format!("unsupported content encoding \"{encoding}\"")));
    }
    let limit = match kind {
        Kind::Json => options.json_limit,
        Kind::Urlencoded => options.urlencoded_limit,
    };
    if body.len() > limit {
        return Err(http_error(413, "request entity too large"));
    }
    let text = decode(body, &charset).ok_or_else(|| http_error(415, format!("unsupported charset \"{}\"", charset.to_uppercase())))?;
    match kind {
        Kind::Json => parse_json(&text, options.max_depth),
        Kind::Urlencoded => parse_urlencoded(&text, &charset, options.max_depth),
    }
}

/// A header value as Node exposes it (each byte is one latin1 character).
pub(crate) fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

enum Kind {
    Json,
    Urlencoded,
}

/// `type-is.hasBody(req)`.
fn has_body(headers: &HeaderMap, body: &Bytes) -> bool {
    if headers.contains_key(http::header::TRANSFER_ENCODING) {
        return true;
    }
    match headers.get(http::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()) {
        // `!isNaN(req.headers['content-length'])`
        Some(v) => js_number(v).is_some(),
        // HTTP/2 requests may carry a body without Content-Length.
        None => !body.is_empty(),
    }
}

/// `Number(s)` for the strings a header can hold (`None` = `NaN`).
fn js_number(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if t.is_empty() {
        return Some(0.0);
    }
    if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u64::from_str_radix(h, 16).ok().map(|n| n as f64);
    }
    if t.chars().any(|c| c.is_ascii_alphabetic() && c != 'e' && c != 'E') && t != "Infinity" && t != "+Infinity" && t != "-Infinity" {
        return None;
    }
    if t.ends_with("Infinity") {
        return Some(f64::INFINITY);
    }
    t.parse::<f64>().ok()
}

/// A media type parsed like the `content-type` package (`None` when it throws).
struct MediaType {
    essence: String,
    charset: Option<String>,
}

fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b))
}

impl MediaType {
    fn parse(header: &str) -> Option<MediaType> {
        let (ty, mut rest) = match header.find(';') {
            Some(i) => (&header[..i], &header[i..]),
            None => (header, ""),
        };
        let ty = ty.trim_matches(|c| c == ' ' || c == '\t');
        let (t, s) = ty.split_once('/')?;
        if !is_token(t) || !is_token(s) {
            return None;
        }
        let mut charset = None;
        while !rest.is_empty() {
            // `; *key *= *value *`
            rest = rest.strip_prefix(';')?.trim_start_matches(' ');
            let eq = rest.find('=')?;
            let key = rest[..eq].trim_end_matches(' ');
            if !is_token(key) {
                return None;
            }
            rest = rest[eq + 1..].trim_start_matches(' ');
            let value;
            if let Some(q) = rest.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = q.char_indices();
                let mut end = None;
                while let Some((i, c)) = chars.next() {
                    match c {
                        '"' => {
                            end = Some(i + 1);
                            break;
                        }
                        '\\' => out.push(chars.next()?.1),
                        c => out.push(c),
                    }
                }
                let end = end?;
                value = out;
                rest = &q[end..];
            } else {
                let end = rest.find([';', ' ']).unwrap_or(rest.len());
                if !is_token(&rest[..end]) {
                    return None;
                }
                value = rest[..end].to_string();
                rest = &rest[end..];
            }
            rest = rest.trim_start_matches(' ');
            if !rest.is_empty() && !rest.starts_with(';') {
                return None;
            }
            if key.eq_ignore_ascii_case("charset") {
                charset = Some(value.to_ascii_lowercase());
            }
        }
        Some(MediaType { essence: format!("{t}/{s}").to_ascii_lowercase(), charset })
    }
}

/// `iconv.decode(body, charset)` for the charsets the parsers accept (the BOM is stripped).
fn decode(body: &[u8], charset: &str) -> Option<Vec<u16>> {
    let units: Vec<u16> = match charset {
        "utf-8" | "utf8" => String::from_utf8_lossy(body).encode_utf16().collect(),
        "utf-16le" | "utf-16" => body.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect(),
        "utf-16be" => body.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect(),
        "iso-8859-1" | "latin1" => body.iter().map(|&b| b as u16).collect(),
        _ => return None,
    };
    Some(if units.first() == Some(&0xFEFF) { units[1..].to_vec() } else { units })
}

fn parse_json(text: &[u16], max_depth: usize) -> Result<Value, HttpException> {
    if text.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let first = text.iter().copied().find(|&c| !matches!(c, 0x20 | 0x09 | 0x0A | 0x0D));
    if first != Some(b'{' as u16) && first != Some(b'[' as u16) {
        return Err(BadRequestException::new(json_parse::strict_violation(text, first).message));
    }
    json_parse::parse_utf16_with_depth(text, max_depth).map_err(|e| BadRequestException::new(e.message))
}

fn parse_urlencoded(text: &[u16], charset: &str, max_depth: usize) -> Result<Value, HttpException> {
    let text = json_parse::utf16_to_string(text);
    if text.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let params = text.matches('&').count() + 1;
    if params > 1000 {
        return Err(http_error(413, "too many parameters"));
    }
    // qs nests one level per `[...]` segment below `depth`, plus the root key and the leaf.
    let depth = 32.min(max_depth.saturating_sub(2));
    let options =
        qs::Options { array_limit: params.max(100), depth, parameter_limit: 1000, latin1: charset == "iso-8859-1", strict_depth: true };
    qs::parse(&text, &options).map_err(|_| http_error(400, "The input exceeded the depth"))
}

/// `qs.parse` (qs 6.15) with the options `body-parser` passes: `allowPrototypes: true`,
/// `strictDepth: true`, `allowDots: false`, `comma: false`, `duplicates: 'combine'`.
pub(crate) mod qs {
    use serde_json::{Map, Value};

    use super::super::json_parse::is_array_index;

    pub struct Options {
        pub array_limit: usize,
        pub depth: usize,
        pub parameter_limit: usize,
        pub latin1: bool,
        /// `strictDepth`: going deeper than `depth` is an error instead of a literal key.
        pub strict_depth: bool,
    }

    impl Options {
        /// The options of Express' `extended` query parser (qs defaults + `allowPrototypes`).
        pub fn express_extended() -> Self {
            Options { array_limit: 20, depth: 5, parameter_limit: 1000, latin1: false, strict_depth: false }
        }
    }

    /// `RangeError` raised by `strictDepth`.
    #[derive(Debug)]
    pub struct DepthExceeded;

    #[derive(Debug, Clone)]
    enum Qv {
        Str(String),
        /// Sparse JS array (`None` = hole).
        Arr(Vec<Option<Qv>>),
        Obj(QObj),
    }

    #[derive(Debug, Clone, Default)]
    struct QObj {
        entries: Vec<(String, Qv)>,
        /// `markOverflow(obj, maxIndex)`.
        overflow: Option<usize>,
    }

    impl QObj {
        fn get_mut(&mut self, key: &str) -> Option<&mut Qv> {
            self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
        }
        fn has(&self, key: &str) -> bool {
            self.entries.iter().any(|(k, _)| k == key)
        }
        fn set(&mut self, key: String, value: Qv) {
            match self.get_mut(&key) {
                Some(slot) => *slot = value,
                None => self.entries.push((key, value)),
            }
        }
        /// `Object.keys(obj)` order.
        fn into_js_order(self) -> Vec<(String, Qv)> {
            let (mut idx, rest): (Vec<_>, Vec<_>) = self.entries.into_iter().partition(|(k, _)| is_array_index(k));
            idx.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(0));
            idx.into_iter().chain(rest).collect()
        }
    }

    fn is_overflow(v: &Qv) -> bool {
        matches!(v, Qv::Obj(o) if o.overflow.is_some())
    }

    /// JS truthiness of a parsed value (only `''` is falsy).
    fn truthy(v: &Qv) -> bool {
        !matches!(v, Qv::Str(s) if s.is_empty())
    }

    fn array_to_object(items: Vec<Option<Qv>>) -> QObj {
        let mut o = QObj::default();
        for (i, v) in items.into_iter().enumerate() {
            if let Some(v) = v {
                o.set(i.to_string(), v);
            }
        }
        o
    }

    /// `[].concat(a, b)`.
    fn concat(a: Qv, b: Qv) -> Vec<Option<Qv>> {
        let mut out = Vec::new();
        for v in [a, b] {
            match v {
                Qv::Arr(items) => out.extend(items),
                other => out.push(Some(other)),
            }
        }
        out
    }

    /// `utils.combine(a, b, arrayLimit)`.
    fn combine(a: Qv, b: Qv, limit: usize) -> Qv {
        if let Qv::Obj(mut o) = a {
            if let Some(max) = o.overflow {
                o.set((max + 1).to_string(), b);
                o.overflow = Some(max + 1);
                return Qv::Obj(o);
            }
            return combine_plain(Qv::Obj(o), b, limit);
        }
        combine_plain(a, b, limit)
    }

    fn combine_plain(a: Qv, b: Qv, limit: usize) -> Qv {
        let result = concat(a, b);
        if result.len() > limit {
            let max = result.len() - 1;
            let mut o = array_to_object(result);
            o.overflow = Some(max);
            return Qv::Obj(o);
        }
        Qv::Arr(result)
    }

    /// The byte encoded by `%XX` at `b[i..]`. Both `X` must be hexadecimal digits, like in
    /// JavaScript (`u8::from_str_radix` alone would also take `%+A`).
    pub(crate) fn percent_byte(b: &[u8], i: usize) -> Option<u8> {
        let h = b.get(i + 1..i + 3)?;
        if b.get(i) != Some(&b'%') || !h.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()
    }

    /// `decodeURIComponent(s)`; `None` when it throws `URIError`.
    pub(crate) fn decode_uri_component(s: &str) -> Option<String> {
        let b = s.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(b.len());
        let hex = |i: usize| percent_byte(b, i);
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'%' {
                out.push(b[i]);
                i += 1;
                continue;
            }
            let lead = hex(i)?;
            i += 3;
            if lead < 0x80 {
                out.push(lead);
                continue;
            }
            let n = match lead {
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => return None,
            };
            let mut seq = vec![lead];
            for _ in 1..n {
                if b.get(i) != Some(&b'%') {
                    return None;
                }
                seq.push(hex(i)?);
                i += 3;
            }
            std::str::from_utf8(&seq).ok()?;
            out.extend(seq);
        }
        String::from_utf8(out).ok()
    }

    /// `utils.decode(str, decoder, charset)`.
    fn decode(s: &str, latin1: bool) -> String {
        let plus = s.replace('+', " ");
        if latin1 {
            let b = plus.as_bytes();
            let mut out = String::new();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'%'
                    && let Some(v) = percent_byte(b, i)
                {
                    out.push(v as char);
                    i += 3;
                    continue;
                }
                let ch = plus[i..].chars().next().unwrap_or('\u{FFFD}');
                out.push(ch);
                i += ch.len_utf8();
            }
            return out;
        }
        decode_uri_component(&plus).unwrap_or(plus)
    }

    fn replace_ci(s: &str, from: &str, to: &str) -> String {
        let lower = s.to_ascii_lowercase();
        let mut out = String::with_capacity(s.len());
        let mut last = 0;
        for (i, _) in lower.match_indices(&from.to_ascii_lowercase()) {
            out.push_str(&s[last..i]);
            out.push_str(to);
            last = i + from.len();
        }
        out.push_str(&s[last..]);
        out
    }

    fn parse_values(s: &str, o: &Options) -> QObj {
        let mut obj = QObj::default();
        let clean = replace_ci(&replace_ci(s, "%5B", "["), "%5D", "]");
        for part in clean.split('&').take(o.parameter_limit) {
            let pos = match part.find("]=") {
                Some(p) => Some(p + 1),
                None => part.find('='),
            };
            let (key, val) = match pos {
                None => (decode(part, o.latin1), String::new()),
                Some(p) => (decode(&part[..p], o.latin1), decode(&part[p + 1..], o.latin1)),
            };
            let val = Qv::Str(val);
            if obj.has(&key) {
                let slot = obj.get_mut(&key).expect("existing key");
                let prev = std::mem::replace(slot, Qv::Str(String::new()));
                *slot = combine(prev, val, o.array_limit);
            } else {
                obj.set(key, val);
            }
        }
        obj
    }

    fn split_key(key: &str, o: &Options) -> Result<Vec<String>, DepthExceeded> {
        let mut segments = Vec::new();
        let first = key.find('[');
        let parent = match first {
            Some(i) => &key[..i],
            None => key,
        };
        if !parent.is_empty() {
            segments.push(parent.to_string());
        }
        let b = key.as_bytes();
        let mut open = first;
        let mut collected = 0;
        while let Some(start) = open {
            if collected >= o.depth {
                break;
            }
            let mut level = 1;
            let mut i = start + 1;
            let mut close = None;
            while i < b.len() && close.is_none() {
                match b[i] {
                    b'[' => level += 1,
                    b']' => {
                        level -= 1;
                        if level == 0 {
                            close = Some(i);
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let Some(close) = close else {
                segments.push(format!("[{}]", &key[start..]));
                return Ok(segments);
            };
            segments.push(key[start..=close].to_string());
            collected += 1;
            open = key[close + 1..].find('[').map(|p| p + close + 1);
        }
        if let Some(start) = open {
            if o.strict_depth {
                return Err(DepthExceeded);
            }
            segments.push(format!("[{}]", &key[start..]));
        }
        Ok(segments)
    }

    fn parse_object(chain: &[String], val: Qv, o: &Options) -> Option<Qv> {
        let mut leaf = Some(val);
        for root in chain.iter().rev() {
            let current = leaf.take()?;
            let obj = if root == "[]" {
                if is_overflow(&current) { current } else { combine_plain(Qv::Arr(Vec::new()), current, o.array_limit) }
            } else {
                let clean =
                    if root.starts_with('[') && root.ends_with(']') && root.len() >= 2 { &root[1..root.len() - 1] } else { root.as_str() };
                let bracketed = clean.len() != root.len();
                let index = if bracketed && is_canonical_index(clean) { clean.parse::<usize>().ok() } else { None };
                match index {
                    Some(i) if i < o.array_limit => {
                        let mut arr: Vec<Option<Qv>> = vec![None; i + 1];
                        arr[i] = Some(current);
                        Qv::Arr(arr)
                    }
                    Some(i) => {
                        let mut obj = QObj::default();
                        obj.set(i.to_string(), current);
                        obj.overflow = Some(i);
                        Qv::Obj(obj)
                    }
                    None => {
                        let mut obj = QObj::default();
                        if clean != "__proto__" {
                            obj.set(clean.to_string(), current);
                        }
                        Qv::Obj(obj)
                    }
                }
            };
            leaf = Some(obj);
        }
        leaf
    }

    fn is_canonical_index(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()) && (s == "0" || !s.starts_with('0')) && s.len() <= 15
    }

    fn merge(target: Qv, source: Option<Qv>, o: &Options) -> Qv {
        let Some(source) = source.filter(truthy) else { return target };
        if let Qv::Str(_) = source {
            return match target {
                Qv::Arr(mut items) => {
                    let next = items.len();
                    if next >= o.array_limit {
                        items.push(Some(source));
                        let mut obj = array_to_object(items);
                        obj.overflow = Some(next);
                        Qv::Obj(obj)
                    } else {
                        items.push(Some(source));
                        Qv::Arr(items)
                    }
                }
                Qv::Obj(mut obj) => match obj.overflow {
                    Some(max) => {
                        obj.set((max + 1).to_string(), source);
                        obj.overflow = Some(max + 1);
                        Qv::Obj(obj)
                    }
                    // strictMerge
                    None => Qv::Arr(vec![Some(Qv::Obj(obj)), Some(source)]),
                },
                t @ Qv::Str(_) => Qv::Arr(vec![Some(t), Some(source)]),
            };
        }
        if let Qv::Str(_) = target {
            if let Qv::Obj(src) = &source
                && let Some(max) = src.overflow
            {
                let mut result = QObj::default();
                result.set("0".into(), target);
                for (k, v) in src.entries.clone() {
                    let n = k.parse::<usize>().unwrap_or(0);
                    result.set((n + 1).to_string(), v);
                }
                result.overflow = Some(max + 1);
                return Qv::Obj(result);
            }
            let combined = concat(target, source);
            if combined.len() > o.array_limit {
                let max = combined.len() - 1;
                let mut obj = array_to_object(combined);
                obj.overflow = Some(max);
                return Qv::Obj(obj);
            }
            return Qv::Arr(combined);
        }
        match (target, source) {
            (Qv::Arr(mut t), Qv::Arr(s)) => {
                for (i, item) in s.into_iter().enumerate() {
                    let Some(item) = item else { continue };
                    if matches!(t.get(i), Some(Some(_))) {
                        let existing = t[i].take().expect("present");
                        let both_objects = !matches!(existing, Qv::Str(_)) && !matches!(item, Qv::Str(_));
                        if both_objects {
                            t[i] = Some(merge(existing, Some(item), o));
                        } else {
                            t[i] = Some(existing);
                            t.push(Some(item));
                        }
                    } else {
                        if t.len() <= i {
                            t.resize(i + 1, None);
                        }
                        t[i] = Some(item);
                    }
                }
                if t.len() > o.array_limit {
                    let max = t.len() - 1;
                    let mut obj = array_to_object(t);
                    obj.overflow = Some(max);
                    return Qv::Obj(obj);
                }
                Qv::Arr(t)
            }
            (target, source) => {
                let mut acc = match target {
                    Qv::Arr(items) => array_to_object(items),
                    Qv::Obj(obj) => obj,
                    Qv::Str(_) => unreachable!(),
                };
                let (src_entries, src_overflow) = match source {
                    Qv::Obj(obj) => {
                        let overflow = obj.overflow;
                        (obj.into_js_order(), overflow)
                    }
                    Qv::Arr(items) => (items.into_iter().enumerate().filter_map(|(i, v)| v.map(|v| (i.to_string(), v))).collect(), None),
                    Qv::Str(_) => unreachable!(),
                };
                for (key, value) in src_entries {
                    if acc.has(&key) {
                        let slot = acc.get_mut(&key).expect("present");
                        let prev = std::mem::replace(slot, Qv::Str(String::new()));
                        *slot = merge(prev, Some(value), o);
                    } else {
                        acc.set(key.clone(), value);
                    }
                    if let (Some(max), None) = (src_overflow, acc.overflow) {
                        acc.overflow = Some(max);
                    }
                    if let Some(max) = acc.overflow
                        && is_canonical_index(&key)
                        && let Ok(n) = key.parse::<usize>()
                        && n > max
                    {
                        acc.overflow = Some(n);
                    }
                }
                Qv::Obj(acc)
            }
        }
    }

    /// `utils.compact` + conversion to JSON (object keys in JS order).
    fn to_json(v: Qv) -> Value {
        match v {
            Qv::Str(s) => Value::String(s),
            Qv::Arr(items) => Value::Array(items.into_iter().flatten().map(to_json).collect()),
            Qv::Obj(obj) => {
                let mut map = Map::new();
                for (k, v) in obj.into_js_order() {
                    map.insert(k, to_json(v));
                }
                Value::Object(map)
            }
        }
    }

    /// `qs.parse(str, options)`.
    pub fn parse(s: &str, o: &Options) -> Result<Value, DepthExceeded> {
        if s.is_empty() {
            return Ok(Value::Object(Map::new()));
        }
        let temp = parse_values(s, o);
        let mut obj = Qv::Obj(QObj::default());
        for (key, val) in temp.into_js_order() {
            if key.is_empty() {
                continue;
            }
            let chain = split_key(&key, o)?;
            let new_obj = parse_object(&chain, val, o);
            obj = merge(obj, new_obj, o);
        }
        Ok(to_json(obj))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn p(s: &str) -> String {
            parse(s, &Options { array_limit: 100, depth: 32, parameter_limit: 1000, latin1: false, strict_depth: true })
                .unwrap()
                .to_string()
        }

        #[test]
        fn qs_parity() {
            assert_eq!(p("a=1&b=2"), r#"{"a":"1","b":"2"}"#);
            assert_eq!(p("a=1&a=2"), r#"{"a":["1","2"]}"#);
            assert_eq!(p("a[]=1&a[]=2"), r#"{"a":["1","2"]}"#);
            assert_eq!(p("a[b][c]=d"), r#"{"a":{"b":{"c":"d"}}}"#);
            assert_eq!(p("a[1]=b&a[0]=c"), r#"{"a":["c","b"]}"#);
            assert_eq!(p("a[3]=x"), r#"{"a":["x"]}"#);
            assert_eq!(p("a[101]=x"), r#"{"a":{"101":"x"}}"#);
            assert_eq!(p("a[0]=b&a[c]=d"), r#"{"a":{"0":"b","c":"d"}}"#);
            assert_eq!(p("a=b&a[c]=d"), r#"{"a":["b",{"c":"d"}]}"#);
            assert_eq!(p("x=%E2%9C%93&y=a+b&z=%ZZ"), r#"{"x":"✓","y":"a b","z":"%ZZ"}"#);
            assert_eq!(p("a%5Bb%5D=c"), r#"{"a":{"b":"c"}}"#);
            assert_eq!(p("a&b="), r#"{"a":"","b":""}"#);
            assert_eq!(p("__proto__[x]=1&b=2"), r#"{"b":"2"}"#);
            assert_eq!(p("b=1&2=x&1=y"), r#"{"1":"y","2":"x","b":"1"}"#);
            let deep = format!("a{}=1", "[b]".repeat(33));
            assert!(
                parse(&deep, &Options { array_limit: 100, depth: 32, parameter_limit: 1000, latin1: false, strict_depth: true }).is_err()
            );
        }

        #[test]
        fn percent_escapes_need_hex_digits() {
            // `%+A` is not an escape (`decodeURIComponent` throws, qs keeps the raw text).
            assert_eq!(p("a=%+A&b=%2B&c=%-1"), r#"{"a":"% A","b":"+","c":"%-1"}"#);
            assert_eq!(decode_uri_component("%+A"), None);
            assert_eq!(decode_uri_component("%0A%e2%9c%93"), Some("\n✓".into()));
            let latin1 = Options { array_limit: 100, depth: 32, parameter_limit: 1000, latin1: true, strict_depth: true };
            assert_eq!(parse("a=%+A&b=%E9", &latin1).unwrap().to_string(), "{\"a\":\"% A\",\"b\":\"é\"}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ct: Option<&str>, body: &str) -> Result<Value, (u16, Value)> {
        let mut h = HeaderMap::new();
        if let Some(ct) = ct {
            h.insert(http::header::CONTENT_TYPE, ct.parse().unwrap());
        }
        run_with(h, body, &BodyParserOptions::default())
    }

    fn run_with(mut h: HeaderMap, body: &str, options: &BodyParserOptions) -> Result<Value, (u16, Value)> {
        h.insert(http::header::CONTENT_LENGTH, body.len().into());
        parse_body(&h, &Bytes::from(body.to_string()), options).map_err(|e| (e.status().as_u16(), e.get_response().clone()))
    }

    fn raw_headers(pairs: &[(http::HeaderName, &[u8])]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), http::HeaderValue::from_bytes(v).unwrap());
        }
        h
    }

    #[test]
    fn json_depth() {
        let nested = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert_eq!(run(Some("application/json"), &nested(256)).unwrap().to_string(), nested(256));
        let (s, b) = run(Some("application/json"), &nested(257)).unwrap_err();
        assert_eq!(s, 400);
        assert_eq!(b["message"], "Maximum nesting depth of 256 exceeded in JSON at position 256 (line 1 column 257)");
        let (s, _) = run(Some("application/json"), &nested(50_000)).unwrap_err();
        assert_eq!(s, 400);
        let options = BodyParserOptions { max_depth: 3, ..Default::default() };
        let ct = || raw_headers(&[(http::header::CONTENT_TYPE, b"application/json")]);
        assert!(run_with(ct(), r#"{"a":[{}]}"#, &options).is_ok());
        assert_eq!(run_with(ct(), r#"{"a":[{"b":[]}]}"#, &options).unwrap_err().0, 400);
        // The depth also caps urlencoded bodies (root key + segments + leaf).
        let form = || raw_headers(&[(http::header::CONTENT_TYPE, b"application/x-www-form-urlencoded")]);
        assert!(run_with(form(), "a[b]=1", &options).is_ok());
        assert_eq!(run_with(form(), "a[b][c]=1", &options).unwrap_err().0, 400);
    }

    #[test]
    fn non_ascii_headers() {
        // A non-ASCII Content-Encoding is not "absent": 415 like body-parser (latin1 + lowercase).
        for ce in [&b"gzip\xff"[..], b"z\xc2\x9b2J", b"\xe9", b"IDENTITY\xff"] {
            let h = raw_headers(&[(http::header::CONTENT_TYPE, b"application/json"), (http::header::CONTENT_ENCODING, ce)]);
            let (s, b) = run_with(h, "{}", &BodyParserOptions::default()).unwrap_err();
            assert_eq!(s, 415, "{ce:?}");
            assert!(b["message"].as_str().unwrap().starts_with("unsupported content encoding"), "{b}");
        }
        let h =
            raw_headers(&[(http::header::CONTENT_TYPE, b"application/x-www-form-urlencoded"), (http::header::CONTENT_ENCODING, b"br\xff")]);
        assert_eq!(run_with(h, "a=1", &BodyParserOptions::default()).unwrap_err().0, 415);
        let h = raw_headers(&[(http::header::CONTENT_TYPE, b"application/json"), (http::header::CONTENT_ENCODING, b"Identity")]);
        assert!(run_with(h, "{}", &BodyParserOptions::default()).is_ok());
        // A quoted non-ASCII charset is read as latin1 and rejected like body-parser.
        let h = raw_headers(&[(http::header::CONTENT_TYPE, b"application/json; charset=\"\xe9\"")]);
        let (s, b) = run_with(h, "{}", &BodyParserOptions::default()).unwrap_err();
        assert_eq!((s, b["message"].as_str().unwrap()), (415, "unsupported charset \"É\""));
        // An unquoted one makes the header invalid: the body is not parsed.
        let h = raw_headers(&[(http::header::CONTENT_TYPE, b"application/json; charset=utf-8\xff")]);
        assert_eq!(run_with(h, "{}", &BodyParserOptions::default()).unwrap(), Value::Null);
    }

    #[test]
    fn json() {
        assert_eq!(run(Some("application/json"), r#"{"a":1}"#).unwrap(), serde_json::json!({ "a": 1 }));
        assert_eq!(run(Some("application/json; charset=utf-8"), "").unwrap(), serde_json::json!({}));
        assert_eq!(run(Some("application/vnd.api+json"), r#"{"a":1}"#).unwrap(), Value::Null);
        assert_eq!(run(Some("text/plain"), "hi").unwrap(), Value::Null);
        assert_eq!(run(None, r#"{"a":1}"#).unwrap(), Value::Null);
        let (s, b) = run(Some("application/json"), "\"x\"").unwrap_err();
        assert_eq!(s, 400);
        assert_eq!(b["message"], "Unexpected token '\"', \"\"x\"\" is not valid JSON");
        let (_, b) = run(Some("application/json"), "{a:1}").unwrap_err();
        assert_eq!(b["message"], "Expected property name or '}' in JSON at position 1 (line 1 column 2)");
        let (s, b) = run(Some("application/json; charset=latin1"), "{}").unwrap_err();
        assert_eq!((s, b), (415, serde_json::json!({ "statusCode": 415, "message": "unsupported charset \"LATIN1\"" })));
        let big = format!("{{\"a\":\"{}\"}}", "x".repeat(DEFAULT_BODY_LIMIT));
        let (s, b) = run(Some("application/json"), &big).unwrap_err();
        assert_eq!((s, b), (413, serde_json::json!({ "statusCode": 413, "message": "request entity too large" })));
        assert_eq!(run(Some("application/json"), "\u{FEFF}[1]").unwrap(), serde_json::json!([1]));
    }

    #[test]
    fn urlencoded() {
        assert_eq!(
            run(Some("application/x-www-form-urlencoded"), "a[b]=1&c=2").unwrap(),
            serde_json::json!({ "a": { "b": "1" }, "c": "2" })
        );
        let deep = format!("a{}=1", "[b]".repeat(33));
        let (s, b) = run(Some("application/x-www-form-urlencoded"), &deep).unwrap_err();
        assert_eq!((s, b), (400, serde_json::json!({ "statusCode": 400, "message": "The input exceeded the depth" })));
        let many = vec!["a=1"; 1001].join("&");
        assert_eq!(run(Some("application/x-www-form-urlencoded"), &many).unwrap_err().0, 413);
    }
}

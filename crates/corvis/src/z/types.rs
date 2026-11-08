//! Primitive schemas: string (and its formats), number, bigint, boolean, date, literal,
//! enum, null, undefined, void, any, unknown, never, nan, stringbool, templateLiteral.
//!
//! Each node implements zod 3 semantics in `check` (kept as they were) and zod 4 in `run_v4`.

use std::net::IpAddr;
use std::sync::{Arc, LazyLock};

use base64::Engine;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use super::core::{CheckOpts, CheckRunner, ErrorMap, ErrorParam, Payload, RawIssue, RunCtx, Val, WhenFn, has_length, run_parse_checks};
use super::regex::JsRegex;
use super::{JsonSchemaGen, ParseCtx, Schema, TypeErrors, ZodIssueCode, ZodType, invalid_type, js, regexes, v3_via_v4};

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_'+\-.]*[A-Za-z0-9_+-]@([A-Za-z0-9][A-Za-z0-9\-]*\.)+[A-Za-z]{2,}$").unwrap()
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").unwrap()
});
static CUID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^c[^\s-]{8,}$").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z][A-Za-z0-9+.\-]*://[^\s/?#]+[^\s]*$").unwrap());
static DATETIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?Z$").unwrap());
static DATETIME_OFFSET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}(:?\d{2})?)$").unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}$").unwrap());
static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{2}:\d{2}(:\d{2}(\.\d+)?)?$").unwrap());

/// Formats a number like JavaScript does (`5` instead of `5.0`).
pub(crate) fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

pub(crate) fn num_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 { json!(n as i64) } else { json!(n) }
}

pub(crate) fn issue_params(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn quote_list(values: &[String]) -> String {
    values.iter().map(|v| format!("'{v}'")).collect::<Vec<_>>().join(" | ")
}

// ---------------------------------------------------------------------------
// JS conversions on `Val` (`String(x)`, `Number(x)`, `Boolean(x)`, `BigInt(x)`, `new Date(x)`).
// ---------------------------------------------------------------------------

fn js_date_string(t: f64) -> String {
    use chrono::TimeZone;
    if !t.is_finite() {
        return "Invalid Date".into();
    }
    match chrono::Local.timestamp_millis_opt(t as i64).single() {
        Some(d) => d.format("%a %b %d %Y %H:%M:%S GMT%z").to_string(),
        None => "Invalid Date".into(),
    }
}

/// `String(value)`.
pub(crate) fn js_string_of(v: &Val) -> String {
    match v {
        Val::Undef => "undefined".into(),
        Val::Json(v) => js::to_js_string(v),
        Val::Num(n) => js::num_to_string(*n),
        Val::BigInt(b) => b.to_string(),
        Val::Date(t) => js_date_string(*t),
    }
}

/// `Number(value)`.
pub(crate) fn js_number_of(v: &Val) -> f64 {
    match v {
        Val::Undef => f64::NAN,
        Val::Json(v) => js::to_number(Some(v)),
        Val::Num(n) => *n,
        Val::BigInt(b) => *b as f64,
        Val::Date(t) => *t,
    }
}

/// `Boolean(value)`.
pub(crate) fn js_truthy(v: &Val) -> bool {
    match v {
        Val::Undef => false,
        Val::Json(v) => js::truthy(Some(v)),
        Val::Num(n) => !n.is_nan() && *n != 0.0,
        Val::BigInt(b) => *b != 0,
        Val::Date(_) => true,
    }
}

/// `BigInt(value)` (`None` when it throws).
pub(crate) fn js_bigint_of(v: &Val) -> Option<i128> {
    match v {
        Val::Undef => None,
        Val::Json(v) => js::to_bigint(Some(v)),
        Val::Num(_) => None,
        Val::BigInt(b) => Some(*b),
        Val::Date(t) => (t.is_finite() && t.fract() == 0.0).then_some(*t as i128),
    }
}

/// `new Date(value)` (`None` when it throws).
pub(crate) fn js_date_of(v: &Val) -> Option<f64> {
    match v {
        Val::Undef => Some(f64::NAN),
        Val::Json(v) => Some(js::to_date(Some(v))),
        Val::Num(_) => Some(f64::NAN),
        Val::BigInt(_) => None,
        Val::Date(t) => Some(*t),
    }
}

fn issue(fields: Vec<(&str, Value)>, input: &Val, error: Option<ErrorMap>, cont: Option<bool>) -> RawIssue {
    RawIssue::new(fields, input.clone(), error, cont)
}

fn invalid_type_v4(expected: &str, p: &mut Payload, error: Option<ErrorMap>) {
    let input = p.value.clone();
    p.push(issue(vec![("expected", json!(expected)), ("code", json!("invalid_type"))], &input, error, None));
}

/// Runs the checks of a primitive honouring `abort` / `when` (`runChecks`).
fn run_checks<C>(
    mut p: Payload,
    checks: &[C],
    opts: impl Fn(&C) -> &CheckOpts,
    default_when: impl Fn(&C) -> Option<fn(&Payload) -> bool>,
    mut apply: impl FnMut(&C, &mut Payload),
) -> Payload {
    let mut runner = CheckRunner::new(&p);
    for c in checks {
        if !runner.should_run(&p, opts(c).when.as_ref(), default_when(c)) {
            continue;
        }
        let before = p.issues.len();
        apply(c, &mut p);
        runner.after(&p, before);
    }
    p
}

macro_rules! check_builders {
    () => {
        /// `{ error }` / `message` for the last check (`.min(3).error("Too short")`);
        /// without checks (or right after a format constructor), the schema-level error.
        pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
            let e: ErrorParam = e.into();
            match self.checks.last_mut() {
                Some(c) if !c.is_self => {
                    c.message = e.message();
                    c.opts.set_error(e);
                }
                last => {
                    if let Some(c) = last {
                        c.message = e.message();
                    }
                    if let Some(m) = e.message() {
                        self.errors.invalid_type = Some(m);
                    }
                    self.error = Some(e.into_map());
                }
            }
            self
        }
        /// Custom message for the last check (`.min(3, "Too short")` → `.min(3).message("Too short")`).
        /// Without checks, it replaces the invalid type message.
        pub fn message(self, message: impl Into<String>) -> Self {
            self.error(ErrorParam::Message(message.into()))
        }
        /// `{ abort: true }` for the last check.
        pub fn abort(mut self) -> Self {
            if let Some(c) = self.checks.last_mut() {
                c.opts.abort = true;
            }
            self
        }
        /// `{ when: (payload) => boolean }` for the last check.
        pub fn when<F: Fn(&Payload) -> bool + Send + Sync + 'static>(mut self, f: F) -> Self {
            if let Some(c) = self.checks.last_mut() {
                c.opts.when = Some(Arc::new(f) as WhenFn);
            }
            self
        }
        /// `{ required_error }` (zod 3).
        pub fn required_error(mut self, message: impl Into<String>) -> Self {
            self.errors.required = Some(message.into());
            self
        }
        /// `{ invalid_type_error }` (zod 3).
        pub fn invalid_type_error(mut self, message: impl Into<String>) -> Self {
            self.errors.invalid_type = Some(message.into());
            self
        }
    };
}

#[derive(Clone)]
pub(crate) struct Chk<K> {
    pub(crate) kind: K,
    pub(crate) message: Option<String>,
    pub(crate) opts: CheckOpts,
    /// The schema's own format check (`z.email()`): the schema `error` applies to it.
    pub(crate) is_self: bool,
}

impl<K> Chk<K> {
    fn new(kind: K) -> Self {
        Chk { kind, message: None, opts: CheckOpts::default(), is_self: false }
    }
}

// ---------------------------------------------------------------------------
// string formats
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum FmtKind {
    /// `$ZodCheckStringFormat` with a pattern.
    Pattern,
    Url { hostname: Option<JsRegex>, protocol: Option<JsRegex>, normalize: bool },
    Ipv6,
    Cidrv6,
    Base64,
    Base64Url,
    Jwt(Option<String>),
    Func(Arc<dyn Fn(&str) -> bool + Send + Sync>),
    /// zod 3 `.ip()` (v4 or v6).
    Ip,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Legacy {
    None,
    Email,
    Url,
    Uuid,
    Cuid,
    Datetime(bool),
    Date,
    Time,
    Ip,
}

/// A string format (`z.email()`, `z.uuid()`, `z.iso.datetime()`, ...).
#[derive(Clone)]
pub struct Fmt {
    pub(crate) format: String,
    pub(crate) pattern: Option<JsRegex>,
    pub(crate) kind: FmtKind,
    pub(crate) legacy: Legacy,
    pub(crate) content_encoding: Option<&'static str>,
}

impl Fmt {
    fn pat(format: &str, pattern: JsRegex) -> Self {
        Fmt { format: format.into(), pattern: Some(pattern), kind: FmtKind::Pattern, legacy: Legacy::None, content_encoding: None }
    }
    fn legacy(mut self, l: Legacy) -> Self {
        self.legacy = l;
        self
    }
    fn kind(mut self, k: FmtKind) -> Self {
        self.kind = k;
        self
    }

    /// Whether `s` is valid (also used by zod 3 for the new formats).
    fn valid(&self, s: &str) -> bool {
        match &self.kind {
            FmtKind::Pattern => self.pattern.as_ref().is_none_or(|p| p.test(s)),
            FmtKind::Url { hostname, protocol, normalize } => {
                let mut p = Payload::new(Val::Json(Value::String(s.into())));
                url_check(s, hostname, protocol, *normalize, &mut p, None, None);
                p.issues.is_empty()
            }
            FmtKind::Ipv6 => ipv6_valid(s),
            FmtKind::Cidrv6 => cidrv6_valid(s),
            FmtKind::Base64 => is_valid_base64(s),
            FmtKind::Base64Url => is_valid_base64url(s),
            FmtKind::Jwt(alg) => is_valid_jwt(s, alg.as_deref()),
            FmtKind::Func(f) => f(s),
            FmtKind::Ip => regexes::IPV4.test(s) || regexes::IPV6.test(s),
        }
    }

    fn check_v4(&self, s: &str, p: &mut Payload, error: Option<ErrorMap>, cont: Option<bool>) {
        let input = p.value.clone();
        let fmt = json!(self.format);
        match &self.kind {
            FmtKind::Pattern => {
                let Some(pattern) = &self.pattern else { return };
                if !pattern.test(s) {
                    p.push(issue(
                        vec![
                            ("origin", json!("string")),
                            ("code", json!("invalid_format")),
                            ("format", fmt),
                            ("pattern", json!(pattern.to_string())),
                        ],
                        &input,
                        error,
                        cont,
                    ));
                }
            }
            FmtKind::Url { hostname, protocol, normalize } => url_check(s, hostname, protocol, *normalize, p, error, cont),
            FmtKind::Ip => {
                if !self.valid(s) {
                    p.push(issue(
                        vec![("origin", json!("string")), ("code", json!("invalid_format")), ("format", fmt)],
                        &input,
                        error,
                        cont,
                    ));
                }
            }
            _ => {
                if !self.valid(s) {
                    p.push(issue(vec![("code", json!("invalid_format")), ("format", fmt)], &input, error, cont));
                }
            }
        }
    }
}

fn url_check(
    s: &str,
    hostname: &Option<JsRegex>,
    protocol: &Option<JsRegex>,
    normalize: bool,
    p: &mut Payload,
    error: Option<ErrorMap>,
    cont: Option<bool>,
) {
    let input = p.value.clone();
    let trimmed = js::js_trim(s);
    if !normalize && protocol.as_ref().is_some_and(|r| r.source() == regexes::HTTP_PROTOCOL.source()) {
        let lower = trimmed.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://")) {
            p.push(issue(
                vec![("code", json!("invalid_format")), ("format", json!("url")), ("note", json!("Invalid URL format"))],
                &input,
                error,
                cont,
            ));
            return;
        }
    }
    let Ok(url) = url::Url::parse(trimmed) else {
        p.push(issue(vec![("code", json!("invalid_format")), ("format", json!("url"))], &input, error, cont));
        return;
    };
    if let Some(h) = hostname
        && !h.test(url.host_str().unwrap_or("")) {
            p.push(issue(
                vec![
                    ("code", json!("invalid_format")),
                    ("format", json!("url")),
                    ("note", json!("Invalid hostname")),
                    ("pattern", json!(h.source())),
                ],
                &input,
                error.clone(),
                cont,
            ));
        }
    if let Some(pr) = protocol
        && !pr.test(url.scheme()) {
            p.push(issue(
                vec![
                    ("code", json!("invalid_format")),
                    ("format", json!("url")),
                    ("note", json!("Invalid protocol")),
                    ("pattern", json!(pr.source())),
                ],
                &input,
                error,
                cont,
            ));
        }
    p.value = Val::Json(Value::String(if normalize { url.as_str().to_string() } else { trimmed.to_string() }));
}

fn ipv6_valid(s: &str) -> bool {
    url::Url::parse(&format!("http://[{s}]")).is_ok()
}

fn cidrv6_valid(s: &str) -> bool {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 || parts[1].is_empty() {
        return false;
    }
    let n = js::string_to_number(parts[1]);
    if js::num_to_string(n) != parts[1] || !(0.0..=128.0).contains(&n) {
        return false;
    }
    ipv6_valid(parts[0])
}

/// `atob(data)` (forgiving base64) → bytes.
fn atob(data: &str) -> Option<Vec<u8>> {
    let d: String = data.chars().filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\x0c' | '\r')).collect();
    let mut d = d.as_str();
    if d.len().is_multiple_of(4) {
        d = d.strip_suffix("==").or_else(|| d.strip_suffix('=')).unwrap_or(d);
    }
    if d.len() % 4 == 1 || !d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/') {
        return None;
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_allow_trailing_bits(true)
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
    );
    engine.decode(d).ok()
}

/// `isValidBase64`.
pub(crate) fn is_valid_base64(data: &str) -> bool {
    if data.is_empty() {
        return true;
    }
    if data.chars().any(|c| c.is_whitespace() || js::is_js_whitespace(c)) {
        return false;
    }
    if !data.len().is_multiple_of(4) {
        return false;
    }
    atob(data).is_some()
}

/// `isValidBase64URL`.
pub(crate) fn is_valid_base64url(data: &str) -> bool {
    if !regexes::BASE64URL.test(data) {
        return false;
    }
    let b: String = data.chars().map(|c| if c == '-' { '+' } else if c == '_' { '/' } else { c }).collect();
    let padded_len = b.len().div_ceil(4) * 4;
    let padded = format!("{b}{}", "=".repeat(padded_len - b.len()));
    is_valid_base64(&padded)
}

/// `isValidJWT`.
pub(crate) fn is_valid_jwt(token: &str, alg: Option<&str>) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 || parts[0].is_empty() {
        return false;
    }
    let Some(bytes) = atob(parts[0]) else { return false };
    let text: String = bytes.iter().map(|b| *b as char).collect();
    let Ok(Value::Object(h)) = serde_json::from_str::<Value>(&text) else { return false };
    if h.contains_key("typ") && h.get("typ") != Some(&json!("JWT")) {
        return false;
    }
    if !h.get("alg").is_some_and(|a| js::truthy(Some(a))) {
        return false;
    }
    if let Some(alg) = alg
        && h.get("alg").and_then(Value::as_str) != Some(alg) {
            return false;
        }
    true
}

/// Options of `z.iso.datetime({ precision, offset, local })`.
#[derive(Debug, Clone, Copy, Default)]
pub struct IsoDatetime {
    pub precision: Option<i32>,
    pub offset: bool,
    pub local: bool,
}

/// Options of `z.url({ hostname, protocol, normalize })`.
#[derive(Clone, Default)]
pub struct UrlOptions {
    pub hostname: Option<JsRegex>,
    pub protocol: Option<JsRegex>,
    pub normalize: bool,
}

impl Fmt {
    pub fn email() -> Self {
        Fmt::pat("email", regexes::EMAIL.clone()).legacy(Legacy::Email)
    }
    pub fn email_with(pattern: JsRegex) -> Self {
        Fmt::pat("email", pattern)
    }
    pub fn url(o: UrlOptions) -> Self {
        Fmt {
            format: "url".into(),
            pattern: None,
            kind: FmtKind::Url { hostname: o.hostname, protocol: o.protocol, normalize: o.normalize },
            legacy: Legacy::None,
            content_encoding: None,
        }
    }
    pub fn uuid(version: Option<u8>) -> Self {
        let f = Fmt::pat("uuid", regexes::uuid(version));
        if version.is_none() { f.legacy(Legacy::Uuid) } else { f }
    }
    pub fn guid() -> Self {
        Fmt::pat("guid", regexes::GUID.clone())
    }
    pub fn datetime(o: IsoDatetime) -> Self {
        Fmt::pat("datetime", regexes::datetime(o.precision, o.offset, o.local))
    }
    pub fn date() -> Self {
        Fmt::pat("date", regexes::DATE.clone())
    }
    pub fn time(precision: Option<i32>) -> Self {
        Fmt::pat("time", regexes::time(precision))
    }
    pub fn duration() -> Self {
        Fmt::pat("duration", regexes::DURATION.clone())
    }
    pub fn ipv4() -> Self {
        Fmt::pat("ipv4", regexes::IPV4.clone())
    }
    pub fn ipv6() -> Self {
        Fmt::pat("ipv6", regexes::IPV6.clone()).kind(FmtKind::Ipv6)
    }
    pub fn cidrv4() -> Self {
        Fmt::pat("cidrv4", regexes::CIDRV4.clone())
    }
    pub fn cidrv6() -> Self {
        Fmt::pat("cidrv6", regexes::CIDRV6.clone()).kind(FmtKind::Cidrv6)
    }
    pub fn mac(delimiter: Option<&str>) -> Self {
        Fmt::pat("mac", regexes::mac(delimiter))
    }
    pub fn base64() -> Self {
        let mut f = Fmt::pat("base64", regexes::BASE64.clone()).kind(FmtKind::Base64);
        f.content_encoding = Some("base64");
        f
    }
    pub fn base64url() -> Self {
        let mut f = Fmt::pat("base64url", regexes::BASE64URL.clone()).kind(FmtKind::Base64Url);
        f.content_encoding = Some("base64url");
        f
    }
    pub fn jwt(alg: Option<&str>) -> Self {
        Fmt { format: "jwt".into(), pattern: None, kind: FmtKind::Jwt(alg.map(str::to_string)), legacy: Legacy::None, content_encoding: None }
    }
    pub fn e164() -> Self {
        Fmt::pat("e164", regexes::E164.clone())
    }
    pub fn emoji() -> Self {
        Fmt::pat("emoji", regexes::emoji())
    }
    pub fn nanoid() -> Self {
        Fmt::pat("nanoid", regexes::NANOID.clone())
    }
    pub fn cuid() -> Self {
        Fmt::pat("cuid", regexes::CUID.clone()).legacy(Legacy::Cuid)
    }
    pub fn cuid2() -> Self {
        Fmt::pat("cuid2", regexes::CUID2.clone())
    }
    pub fn ulid() -> Self {
        Fmt::pat("ulid", regexes::ULID.clone())
    }
    pub fn xid() -> Self {
        Fmt::pat("xid", regexes::XID.clone())
    }
    pub fn ksuid() -> Self {
        Fmt::pat("ksuid", regexes::KSUID.clone())
    }
    pub fn lowercase() -> Self {
        Fmt::pat("lowercase", regexes::LOWERCASE.clone())
    }
    pub fn uppercase() -> Self {
        Fmt::pat("uppercase", regexes::UPPERCASE.clone())
    }
    /// `z.stringFormat(name, regex)`.
    pub fn custom_regex(format: &str, re: JsRegex) -> Self {
        let r = re.clone();
        Fmt {
            format: format.into(),
            pattern: Some(re),
            kind: FmtKind::Func(Arc::new(move |s| r.test(s))),
            legacy: Legacy::None,
            content_encoding: None,
        }
    }
    /// `z.stringFormat(name, fn)`.
    pub fn custom_fn<F: Fn(&str) -> bool + Send + Sync + 'static>(format: &str, f: F) -> Self {
        Fmt { format: format.into(), pattern: None, kind: FmtKind::Func(Arc::new(f)), legacy: Legacy::None, content_encoding: None }
    }
}

// ---------------------------------------------------------------------------
// string
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum StrKind {
    Min(usize),
    Max(usize),
    Length(usize),
    Regex(JsRegex),
    StartsWith(String),
    EndsWith(String),
    Includes(String, Option<usize>),
    Format(Box<Fmt>),
    Trim,
    ToLower,
    ToUpper,
    Normalize(String),
    Slugify,
}

type StrCheck = Chk<StrKind>;

/// `z.string()`.
#[derive(Clone, Default)]
pub struct ZString {
    pub(crate) checks: Vec<StrCheck>,
    coerce: bool,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

fn utf16_slice_from(s: &str, pos: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(units.get(pos..).unwrap_or(&[]))
}

/// `util.slugify`.
pub(crate) fn slugify(input: &str) -> String {
    let lower = input.to_lowercase();
    let t = js::js_trim(&lower);
    let kept: String = t.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || js::is_js_whitespace(*c) || *c == '-').collect();
    let mut out = String::new();
    let mut in_sep = false;
    for c in kept.chars() {
        if js::is_js_whitespace(c) || c == '_' || c == '-' {
            if !in_sep {
                out.push('-');
                in_sep = true;
            }
        } else {
            out.push(c);
            in_sep = false;
        }
    }
    out.trim_matches('-').to_string()
}

fn js_normalize(s: &str, form: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    match form {
        "NFD" => s.nfd().collect(),
        "NFKC" => s.nfkc().collect(),
        "NFKD" => s.nfkd().collect(),
        _ => s.nfc().collect(),
    }
}

impl ZString {
    pub fn new() -> Self {
        <ZString as Default>::default()
    }

    fn push(mut self, kind: StrKind) -> Self {
        self.checks.push(Chk::new(kind));
        self
    }

    fn fmt(self, f: Fmt) -> Self {
        self.push(StrKind::Format(Box::new(f)))
    }

    /// A format constructor (`z.email()`): the format check comes first and the schema `error`
    /// applies to it.
    pub(crate) fn self_fmt(f: Fmt) -> Self {
        let mut s = ZString::new();
        let mut c = Chk::new(StrKind::Format(Box::new(f)));
        c.is_self = true;
        s.checks.push(c);
        s
    }

    check_builders!();

    pub(crate) fn coerce(mut self) -> Self {
        self.coerce = true;
        self
    }

    pub fn min(self, n: usize) -> Self {
        self.push(StrKind::Min(n))
    }
    pub fn max(self, n: usize) -> Self {
        self.push(StrKind::Max(n))
    }
    pub fn length(self, n: usize) -> Self {
        self.push(StrKind::Length(n))
    }
    /// `.nonempty()` (`.min(1)`).
    pub fn nonempty(self) -> Self {
        self.push(StrKind::Min(1))
    }
    pub fn email(self) -> Self {
        self.fmt(Fmt::email())
    }
    /// `.email({ pattern: z.regexes.html5Email })`.
    pub fn email_with(self, pattern: impl Into<JsRegex>) -> Self {
        self.fmt(Fmt::email_with(pattern.into()))
    }
    pub fn url(self) -> Self {
        self.fmt(Fmt::url(UrlOptions::default()).legacy(Legacy::Url))
    }
    /// `.url({ hostname, protocol, normalize })`.
    pub fn url_with(self, o: UrlOptions) -> Self {
        self.fmt(Fmt::url(o))
    }
    pub fn uuid(self) -> Self {
        self.fmt(Fmt::uuid(None))
    }
    pub fn uuidv4(self) -> Self {
        self.fmt(Fmt::uuid(Some(4)))
    }
    pub fn uuidv6(self) -> Self {
        self.fmt(Fmt::uuid(Some(6)))
    }
    pub fn uuidv7(self) -> Self {
        self.fmt(Fmt::uuid(Some(7)))
    }
    pub fn guid(self) -> Self {
        self.fmt(Fmt::guid())
    }
    pub fn cuid(self) -> Self {
        self.fmt(Fmt::cuid())
    }
    pub fn cuid2(self) -> Self {
        self.fmt(Fmt::cuid2())
    }
    pub fn ulid(self) -> Self {
        self.fmt(Fmt::ulid())
    }
    pub fn xid(self) -> Self {
        self.fmt(Fmt::xid())
    }
    pub fn ksuid(self) -> Self {
        self.fmt(Fmt::ksuid())
    }
    pub fn nanoid(self) -> Self {
        self.fmt(Fmt::nanoid())
    }
    pub fn emoji(self) -> Self {
        self.fmt(Fmt::emoji())
    }
    pub fn ipv4(self) -> Self {
        self.fmt(Fmt::ipv4())
    }
    pub fn ipv6(self) -> Self {
        self.fmt(Fmt::ipv6())
    }
    pub fn cidrv4(self) -> Self {
        self.fmt(Fmt::cidrv4())
    }
    pub fn cidrv6(self) -> Self {
        self.fmt(Fmt::cidrv6())
    }
    pub fn mac(self) -> Self {
        self.fmt(Fmt::mac(None))
    }
    pub fn base64(self) -> Self {
        self.fmt(Fmt::base64())
    }
    pub fn base64url(self) -> Self {
        self.fmt(Fmt::base64url())
    }
    pub fn jwt(self) -> Self {
        self.fmt(Fmt::jwt(None))
    }
    pub fn e164(self) -> Self {
        self.fmt(Fmt::e164())
    }
    /// Any [`Fmt`] (`.check(z.stringFormat(...))`).
    pub fn format(self, f: Fmt) -> Self {
        self.fmt(f)
    }
    /// `.regex(/.../)`: a JS regex source (`"^[a-z]+$"`), a `(source, flags)` pair or a
    /// [`JsRegex`]. Panics if the pattern is invalid (schemas are static definitions).
    pub fn regex(self, pattern: impl Into<JsRegex>) -> Self {
        self.push(StrKind::Regex(pattern.into()))
    }
    pub fn starts_with(self, s: impl Into<String>) -> Self {
        self.push(StrKind::StartsWith(s.into()))
    }
    pub fn ends_with(self, s: impl Into<String>) -> Self {
        self.push(StrKind::EndsWith(s.into()))
    }
    pub fn includes(self, s: impl Into<String>) -> Self {
        self.push(StrKind::Includes(s.into(), None))
    }
    /// `.includes(s, { position })`.
    pub fn includes_at(self, s: impl Into<String>, position: usize) -> Self {
        self.push(StrKind::Includes(s.into(), Some(position)))
    }
    pub fn lowercase(self) -> Self {
        self.fmt(Fmt::lowercase())
    }
    pub fn uppercase(self) -> Self {
        self.fmt(Fmt::uppercase())
    }
    /// `.datetime()`: ISO 8601 in UTC (`Z`).
    pub fn datetime(self) -> Self {
        self.fmt(Fmt::datetime(IsoDatetime::default()).legacy(Legacy::Datetime(false)))
    }
    /// `.datetime({ offset: true })`.
    pub fn datetime_offset(self) -> Self {
        self.fmt(Fmt::datetime(IsoDatetime { offset: true, ..Default::default() }).legacy(Legacy::Datetime(true)))
    }
    /// `.datetime({ precision, offset, local })`.
    pub fn datetime_with(self, o: IsoDatetime) -> Self {
        self.fmt(Fmt::datetime(o))
    }
    /// `.date()`: `YYYY-MM-DD`.
    pub fn date(self) -> Self {
        self.fmt(Fmt::date().legacy(Legacy::Date))
    }
    /// `.time()`: `HH:MM[:SS[.sss]]`.
    pub fn time(self) -> Self {
        self.fmt(Fmt::time(None).legacy(Legacy::Time))
    }
    /// `.time({ precision })`.
    pub fn time_with(self, precision: i32) -> Self {
        self.fmt(Fmt::time(Some(precision)))
    }
    /// `.duration()`.
    pub fn duration(self) -> Self {
        self.fmt(Fmt::duration())
    }
    /// `.ip()` (zod 3: v4 or v6).
    pub fn ip(self) -> Self {
        let f = Fmt { format: "ip".into(), pattern: None, kind: FmtKind::Ip, legacy: Legacy::Ip, content_encoding: None };
        self.fmt(f)
    }
    pub fn trim(self) -> Self {
        self.push(StrKind::Trim)
    }
    #[allow(clippy::wrong_self_convention)]
    pub fn to_lower_case(self) -> Self {
        self.push(StrKind::ToLower)
    }
    #[allow(clippy::wrong_self_convention)]
    pub fn to_upper_case(self) -> Self {
        self.push(StrKind::ToUpper)
    }
    /// `.normalize(form)` (`"NFC"` by default).
    pub fn normalize(self, form: &str) -> Self {
        self.push(StrKind::Normalize(if form.is_empty() { "NFC".into() } else { form.into() }))
    }
    pub fn slugify(self) -> Self {
        self.push(StrKind::Slugify)
    }

    /// `.format` (the last format).
    pub fn format_name(&self) -> Option<String> {
        self.bag().format
    }
    /// `.minLength`.
    pub fn min_length(&self) -> Option<usize> {
        self.bag().minimum
    }
    /// `.maxLength`.
    pub fn max_length(&self) -> Option<usize> {
        self.bag().maximum
    }

    fn inst_error(&self) -> Option<ErrorMap> {
        self.errors.v4_map(&self.error)
    }

    fn parse_v4(&self, mut p: Payload) -> Payload {
        if self.coerce {
            p.value = Val::Json(Value::String(js_string_of(&p.value)));
        }
        if p.value.as_str().is_some() {
            return p;
        }
        invalid_type_v4("string", &mut p, self.inst_error());
        p
    }

    fn apply_v4(&self, c: &StrCheck, p: &mut Payload) {
        let err = if c.is_self { self.inst_error() } else { c.opts.error.clone() };
        let cont = c.opts.cont();
        let input = p.value.clone();
        match &c.kind {
            StrKind::Min(n) | StrKind::Max(n) | StrKind::Length(n) => {
                let Some(len) = p.value.length() else { return };
                let origin = if p.value.as_str().is_some() { "string" } else { "array" };
                let n = *n;
                match &c.kind {
                    StrKind::Min(_) if len < n => p.push(issue(
                        vec![("origin", json!(origin)), ("code", json!("too_small")), ("minimum", json!(n)), ("inclusive", json!(true))],
                        &input,
                        err,
                        cont,
                    )),
                    StrKind::Max(_) if len > n => p.push(issue(
                        vec![("origin", json!(origin)), ("code", json!("too_big")), ("maximum", json!(n)), ("inclusive", json!(true))],
                        &input,
                        err,
                        cont,
                    )),
                    StrKind::Length(_) if len != n => {
                        let (code, key) = if len > n { ("too_big", "maximum") } else { ("too_small", "minimum") };
                        p.push(issue(
                            vec![
                                ("origin", json!(origin)),
                                ("code", json!(code)),
                                (key, json!(n)),
                                ("inclusive", json!(true)),
                                ("exact", json!(true)),
                            ],
                            &input,
                            err,
                            cont,
                        ))
                    }
                    _ => {}
                }
            }
            kind => {
                let Some(s) = p.value.as_str().map(str::to_string) else { return };
                let fail = |p: &mut Payload, fields: Vec<(&str, Value)>| p.push(issue(fields, &input, err.clone(), cont));
                match kind {
                    StrKind::Regex(re) => {
                        if !re.test(&s) {
                            fail(
                                p,
                                vec![
                                    ("origin", json!("string")),
                                    ("code", json!("invalid_format")),
                                    ("format", json!("regex")),
                                    ("pattern", json!(re.to_string())),
                                ],
                            );
                        }
                    }
                    StrKind::StartsWith(pre) => {
                        if !s.starts_with(pre.as_str()) {
                            fail(
                                p,
                                vec![
                                    ("origin", json!("string")),
                                    ("code", json!("invalid_format")),
                                    ("format", json!("starts_with")),
                                    ("prefix", json!(pre)),
                                ],
                            );
                        }
                    }
                    StrKind::EndsWith(suf) => {
                        if !s.ends_with(suf.as_str()) {
                            fail(
                                p,
                                vec![
                                    ("origin", json!("string")),
                                    ("code", json!("invalid_format")),
                                    ("format", json!("ends_with")),
                                    ("suffix", json!(suf)),
                                ],
                            );
                        }
                    }
                    StrKind::Includes(inc, pos) => {
                        let ok = match pos {
                            Some(pos) => utf16_slice_from(&s, *pos).contains(inc.as_str()),
                            None => s.contains(inc.as_str()),
                        };
                        if !ok {
                            fail(
                                p,
                                vec![
                                    ("origin", json!("string")),
                                    ("code", json!("invalid_format")),
                                    ("format", json!("includes")),
                                    ("includes", json!(inc)),
                                ],
                            );
                        }
                    }
                    StrKind::Format(f) => f.check_v4(&s, p, err.clone(), cont),
                    StrKind::Trim => p.value = Val::Json(Value::String(js::js_trim(&s).to_string())),
                    StrKind::ToLower => p.value = Val::Json(Value::String(s.to_lowercase())),
                    StrKind::ToUpper => p.value = Val::Json(Value::String(s.to_uppercase())),
                    StrKind::Normalize(form) => p.value = Val::Json(Value::String(js_normalize(&s, form))),
                    StrKind::Slugify => p.value = Val::Json(Value::String(slugify(&s))),
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn bag(&self) -> StrBag {
        let mut bag = StrBag::default();
        for c in &self.checks {
            match &c.kind {
                StrKind::Min(n) => {
                    if bag.minimum.is_none_or(|m| *n > m) {
                        bag.minimum = Some(*n);
                    }
                }
                StrKind::Max(n) => {
                    if bag.maximum.is_none_or(|m| *n < m) {
                        bag.maximum = Some(*n);
                    }
                }
                StrKind::Length(n) => {
                    bag.minimum = Some(*n);
                    bag.maximum = Some(*n);
                }
                StrKind::Regex(re) => {
                    bag.format = Some("regex".into());
                    bag.add_pattern(re.clone());
                }
                StrKind::StartsWith(s) => {
                    bag.add_pattern(JsRegex::new(&format!("^{}.*", regexes::escape_regex(s)), ""));
                }
                StrKind::EndsWith(s) => {
                    bag.add_pattern(JsRegex::new(&format!(".*{}$", regexes::escape_regex(s)), ""));
                }
                StrKind::Includes(s, pos) => {
                    let e = regexes::escape_regex(s);
                    let src = match pos {
                        Some(p) => format!("^.{{{p}}}{e}"),
                        None => e,
                    };
                    bag.add_pattern(JsRegex::new(&src, ""));
                }
                StrKind::Format(f) => {
                    bag.format = Some(f.format.clone());
                    if let Some(p) = &f.pattern {
                        bag.add_pattern(p.clone());
                    }
                    if let Some(ce) = f.content_encoding {
                        bag.content_encoding = Some(ce);
                    }
                }
                _ => {}
            }
        }
        bag
    }
}

#[derive(Default)]
pub(crate) struct StrBag {
    pub minimum: Option<usize>,
    pub maximum: Option<usize>,
    pub format: Option<String>,
    pub patterns: Vec<JsRegex>,
    pub content_encoding: Option<&'static str>,
}

impl StrBag {
    fn add_pattern(&mut self, re: JsRegex) {
        if !self.patterns.iter().any(|p| p.source() == re.source() && p.flags() == re.flags()) {
            self.patterns.push(re);
        }
    }
}

pub(crate) fn string_json_schema(bag: &StrBag, jsg: &JsonSchemaGen, json: &mut Map<String, Value>) {
    json.insert("type".into(), json!("string"));
    if let Some(m) = bag.minimum {
        json.insert("minLength".into(), json!(m));
    }
    if let Some(m) = bag.maximum {
        json.insert("maxLength".into(), json!(m));
    }
    if let Some(f) = &bag.format {
        let mapped = match f.as_str() {
            "guid" => "uuid",
            "url" => "uri",
            "datetime" => "date-time",
            "json_string" => "json-string",
            "regex" | "time" => "",
            other => other,
        };
        if !mapped.is_empty() {
            json.insert("format".into(), json!(mapped));
        }
    }
    if let Some(ce) = bag.content_encoding {
        json.insert("contentEncoding".into(), json!(ce));
    }
    match bag.patterns.len() {
        0 => {}
        1 => {
            json.insert("pattern".into(), json!(bag.patterns[0].source()));
        }
        _ => {
            let all: Vec<Value> = bag
                .patterns
                .iter()
                .map(|r| {
                    if jsg.legacy_target() {
                        json!({ "type": "string", "pattern": r.source() })
                    } else {
                        json!({ "pattern": r.source() })
                    }
                })
                .collect();
            json.insert("allOf".into(), Value::Array(all));
        }
    }
}

impl ZodType for ZString {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut s = match input {
            Some(Value::String(s)) => s.clone(),
            Some(other) if self.coerce => match other {
                Value::Null => "null".to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                v => v.to_string(),
            },
            None if self.coerce => "undefined".to_string(),
            other => {
                invalid_type(ctx, "string", other, &self.errors);
                return Err(());
            }
        };
        let mut ok = true;
        for c in &self.checks {
            let len = s.chars().count();
            let fail = |ctx: &mut ParseCtx, code: ZodIssueCode, default: String, p: Map<String, Value>| {
                ctx.add(code, c.message.clone().unwrap_or(default), p);
            };
            let invalid = |ctx: &mut ParseCtx, validation: Value, default: &str| {
                ctx.add(
                    ZodIssueCode::InvalidString,
                    c.message.clone().unwrap_or_else(|| default.to_string()),
                    issue_params(&[("validation", validation)]),
                );
            };
            match &c.kind {
                StrKind::Min(n) if len < *n => {
                    ok = false;
                    fail(
                        ctx,
                        ZodIssueCode::TooSmall,
                        format!("String must contain at least {n} character(s)"),
                        issue_params(&[("minimum", json!(n)), ("type", json!("string")), ("inclusive", json!(true)), ("exact", json!(false))]),
                    );
                }
                StrKind::Max(n) if len > *n => {
                    ok = false;
                    fail(
                        ctx,
                        ZodIssueCode::TooBig,
                        format!("String must contain at most {n} character(s)"),
                        issue_params(&[("maximum", json!(n)), ("type", json!("string")), ("inclusive", json!(true)), ("exact", json!(false))]),
                    );
                }
                StrKind::Length(n) if len != *n => {
                    ok = false;
                    let (code, key) = if len < *n { (ZodIssueCode::TooSmall, "minimum") } else { (ZodIssueCode::TooBig, "maximum") };
                    fail(
                        ctx,
                        code,
                        format!("String must contain exactly {n} character(s)"),
                        issue_params(&[(key, json!(n)), ("type", json!("string")), ("inclusive", json!(true)), ("exact", json!(true))]),
                    );
                }
                StrKind::Format(f) => {
                    let bad = match f.legacy {
                        Legacy::Email => (!EMAIL.is_match(&s)).then_some(("email", "Invalid email")),
                        Legacy::Url => (!URL.is_match(&s)).then_some(("url", "Invalid url")),
                        Legacy::Uuid => (!UUID.is_match(&s)).then_some(("uuid", "Invalid uuid")),
                        Legacy::Cuid => (!CUID.is_match(&s)).then_some(("cuid", "Invalid cuid")),
                        Legacy::Datetime(offset) => {
                            let re = if offset { &DATETIME_OFFSET } else { &DATETIME };
                            (!re.is_match(&s) || chrono::DateTime::parse_from_rfc3339(&normalize_seconds(&s)).is_err())
                                .then_some(("datetime", "Invalid datetime"))
                        }
                        Legacy::Date => (!DATE.is_match(&s) || chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").is_err())
                            .then_some(("date", "Invalid date")),
                        Legacy::Time => (!TIME.is_match(&s)).then_some(("time", "Invalid time")),
                        Legacy::Ip => s.parse::<IpAddr>().is_err().then_some(("ip", "Invalid ip")),
                        Legacy::None => None,
                    };
                    if f.legacy != Legacy::None {
                        if let Some((validation, msg)) = bad {
                            ok = false;
                            invalid(ctx, json!(validation), msg);
                        }
                    } else if !f.valid(&s) {
                        ok = false;
                        let validation = if f.format == "lowercase" || f.format == "uppercase" { "regex" } else { f.format.as_str() };
                        let msg = if validation == "regex" { "Invalid".to_string() } else { format!("Invalid {}", f.format) };
                        invalid(ctx, json!(validation), &msg);
                    }
                }
                StrKind::Regex(re) if !re.test(&s) => {
                    ok = false;
                    invalid(ctx, json!("regex"), "Invalid");
                }
                StrKind::StartsWith(p) if !s.starts_with(p.as_str()) => {
                    ok = false;
                    invalid(ctx, json!({ "startsWith": p }), &format!("Invalid input: must start with \"{p}\""));
                }
                StrKind::EndsWith(p) if !s.ends_with(p.as_str()) => {
                    ok = false;
                    invalid(ctx, json!({ "endsWith": p }), &format!("Invalid input: must end with \"{p}\""));
                }
                StrKind::Includes(p, pos) => {
                    let found = match pos {
                        Some(pos) => utf16_slice_from(&s, *pos).contains(p.as_str()),
                        None => s.contains(p.as_str()),
                    };
                    if !found {
                        ok = false;
                        invalid(ctx, json!({ "includes": p }), &format!("Invalid input: must include \"{p}\""));
                    }
                }
                StrKind::Trim => s = s.trim().to_string(),
                StrKind::ToLower => s = s.to_lowercase(),
                StrKind::ToUpper => s = s.to_uppercase(),
                StrKind::Normalize(form) => s = js_normalize(&s, form),
                StrKind::Slugify => s = slugify(&s),
                _ => {}
            }
        }
        if ok { Ok(Some(Value::String(s))) } else { Err(()) }
    }

    fn type_name(&self) -> String {
        "string".into()
    }

    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        run_parse_checks(
            p,
            ctx,
            !self.checks.is_empty(),
            |p| self.parse_v4(p),
            |p| {
                run_checks(
                    p,
                    &self.checks,
                    |c| &c.opts,
                    |c| matches!(c.kind, StrKind::Min(_) | StrKind::Max(_) | StrKind::Length(_)).then_some(has_length as fn(&Payload) -> bool),
                    |c, p| self.apply_v4(c, p),
                )
            },
        )
    }

    fn pattern(&self) -> Option<String> {
        let bag = self.bag();
        match bag.patterns.last() {
            Some(p) => Some(p.source().to_string()),
            None => Some(regexes::string(bag.minimum, bag.maximum).source().to_string()),
        }
    }

    fn def_type(&self) -> &'static str {
        "string"
    }

    fn string_patterns(&self) -> Vec<String> {
        self.bag().patterns.iter().map(|p| p.source().to_string()).collect()
    }

    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        string_json_schema(&self.bag(), jsg, json);
        Ok(())
    }
}

/// `2024-01-01T10:00Z` → `2024-01-01T10:00:00Z` so chrono can parse it.
fn normalize_seconds(s: &str) -> String {
    let t = match s.find('T') {
        Some(i) => i,
        None => return s.to_string(),
    };
    let rest = &s[t + 1..];
    if rest.len() >= 5 && rest.as_bytes().get(5).is_none_or(|b| *b != b':') {
        format!("{}T{}:00{}", &s[..t], &rest[..5], &rest[5..])
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// number
// ---------------------------------------------------------------------------

/// `z.int()`, `z.int32()`, ... (`$ZodCheckNumberFormat`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NumFormat {
    SafeInt,
    Int32,
    Uint32,
    Float32,
    Float64,
}

impl NumFormat {
    fn name(self) -> &'static str {
        match self {
            NumFormat::SafeInt => "safeint",
            NumFormat::Int32 => "int32",
            NumFormat::Uint32 => "uint32",
            NumFormat::Float32 => "float32",
            NumFormat::Float64 => "float64",
        }
    }
    fn is_int(self) -> bool {
        matches!(self, NumFormat::SafeInt | NumFormat::Int32 | NumFormat::Uint32)
    }
    fn range(self) -> (f64, f64) {
        match self {
            NumFormat::SafeInt => (-MAX_SAFE, MAX_SAFE),
            NumFormat::Int32 => (-2147483648.0, 2147483647.0),
            NumFormat::Uint32 => (0.0, 4294967295.0),
            NumFormat::Float32 => (-3.4028234663852886e38, 3.4028234663852886e38),
            NumFormat::Float64 => (-f64::MAX, f64::MAX),
        }
    }
}

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

#[derive(Clone)]
pub(crate) enum NumKind {
    Int,
    Min(f64, bool),
    Max(f64, bool),
    MultipleOf(f64),
    Finite,
    Safe,
    Format(NumFormat),
}

type NumCheck = Chk<NumKind>;

/// `z.number()`.
#[derive(Clone, Default)]
pub struct ZNumber {
    checks: Vec<NumCheck>,
    coerce: bool,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

/// `util.floatSafeRemainder`.
pub(crate) fn float_safe_remainder(val: f64, step: f64) -> f64 {
    let ratio = val / step;
    let rounded = ratio.round();
    let tolerance = f64::EPSILON * ratio.abs().max(1.0);
    if (ratio - rounded).abs() < tolerance { 0.0 } else { ratio - rounded }
}

impl ZNumber {
    pub fn new() -> Self {
        <ZNumber as Default>::default()
    }
    fn push(mut self, kind: NumKind) -> Self {
        self.checks.push(Chk::new(kind));
        self
    }
    pub(crate) fn self_format(kind: NumKind) -> Self {
        let mut n = ZNumber::new();
        let mut c = Chk::new(kind);
        c.is_self = true;
        n.checks.push(c);
        n
    }

    check_builders!();

    pub(crate) fn coerce(mut self) -> Self {
        self.coerce = true;
        self
    }

    pub fn int(self) -> Self {
        self.push(NumKind::Int)
    }
    pub fn gt(self, n: impl Into<f64>) -> Self {
        self.push(NumKind::Min(n.into(), false))
    }
    pub fn gte(self, n: impl Into<f64>) -> Self {
        self.push(NumKind::Min(n.into(), true))
    }
    /// Alias of `gte`.
    pub fn min(self, n: impl Into<f64>) -> Self {
        self.gte(n)
    }
    pub fn lt(self, n: impl Into<f64>) -> Self {
        self.push(NumKind::Max(n.into(), false))
    }
    pub fn lte(self, n: impl Into<f64>) -> Self {
        self.push(NumKind::Max(n.into(), true))
    }
    /// Alias of `lte`.
    pub fn max(self, n: impl Into<f64>) -> Self {
        self.lte(n)
    }
    pub fn positive(self) -> Self {
        self.gt(0)
    }
    pub fn nonnegative(self) -> Self {
        self.gte(0)
    }
    pub fn negative(self) -> Self {
        self.lt(0)
    }
    pub fn nonpositive(self) -> Self {
        self.lte(0)
    }
    pub fn multiple_of(self, n: impl Into<f64>) -> Self {
        self.push(NumKind::MultipleOf(n.into()))
    }
    /// Alias of `multiple_of`.
    pub fn step(self, n: impl Into<f64>) -> Self {
        self.multiple_of(n)
    }
    /// `.finite()` (a no-op in zod 4: numbers are always finite).
    pub fn finite(self) -> Self {
        self.push(NumKind::Finite)
    }
    /// `.safe()`: zod 3 checks the safe range; zod 4 behaves like `.int()`.
    pub fn safe(self) -> Self {
        self.push(NumKind::Safe)
    }
    /// `.check(z.int32())`, ...
    pub fn format(self, f: NumFormat) -> Self {
        self.push(NumKind::Format(f))
    }

    fn inst_error(&self) -> Option<ErrorMap> {
        self.errors.v4_map(&self.error)
    }

    fn parse_v4(&self, mut p: Payload) -> Payload {
        if self.coerce {
            p.value = Val::number(js_number_of(&p.value));
        }
        match &p.value {
            Val::Json(Value::Number(_)) => p,
            Val::Num(n) => {
                let received = if n.is_nan() { "NaN" } else { "Infinity" };
                let input = p.value.clone();
                p.push(issue(
                    vec![("expected", json!("number")), ("code", json!("invalid_type")), ("received", json!(received))],
                    &input,
                    self.inst_error(),
                    None,
                ));
                p
            }
            _ => {
                invalid_type_v4("number", &mut p, self.inst_error());
                p
            }
        }
    }

    fn apply_v4(&self, c: &NumCheck, p: &mut Payload) {
        let Some(n) = p.value.as_f64() else { return };
        let err = if c.is_self { self.inst_error() } else { c.opts.error.clone() };
        let cont = c.opts.cont();
        let input = p.value.clone();
        match &c.kind {
            NumKind::Int | NumKind::Safe => number_format_check(NumFormat::SafeInt, n, &input, p, err, cont),
            NumKind::Format(f) => number_format_check(*f, n, &input, p, err, cont),
            NumKind::Min(v, inclusive) => {
                if !(if *inclusive { n >= *v } else { n > *v }) {
                    p.push(issue(
                        vec![
                            ("origin", json!("number")),
                            ("code", json!("too_small")),
                            ("minimum", js::num_json(*v)),
                            ("inclusive", json!(inclusive)),
                        ],
                        &input,
                        err,
                        cont,
                    ));
                }
            }
            NumKind::Max(v, inclusive) => {
                if !(if *inclusive { n <= *v } else { n < *v }) {
                    p.push(issue(
                        vec![
                            ("origin", json!("number")),
                            ("code", json!("too_big")),
                            ("maximum", js::num_json(*v)),
                            ("inclusive", json!(inclusive)),
                        ],
                        &input,
                        err,
                        cont,
                    ));
                }
            }
            NumKind::MultipleOf(m) => {
                if float_safe_remainder(n, *m) != 0.0 {
                    p.push(issue(
                        vec![("origin", json!("number")), ("code", json!("not_multiple_of")), ("divisor", js::num_json(*m))],
                        &input,
                        err,
                        cont,
                    ));
                }
            }
            NumKind::Finite => {}
        }
    }

    /// Bag (`minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `format`, `multipleOf`).
    pub(crate) fn bag(&self) -> NumBag {
        let mut b = NumBag::default();
        for c in &self.checks {
            match &c.kind {
                NumKind::Min(v, true) => {
                    if *v > b.minimum.unwrap_or(f64::NEG_INFINITY) {
                        b.minimum = Some(*v);
                    }
                }
                NumKind::Min(v, false) => {
                    if *v > b.exclusive_minimum.unwrap_or(f64::NEG_INFINITY) {
                        b.exclusive_minimum = Some(*v);
                    }
                }
                NumKind::Max(v, true) => {
                    if *v < b.maximum.unwrap_or(f64::INFINITY) {
                        b.maximum = Some(*v);
                    }
                }
                NumKind::Max(v, false) => {
                    if *v < b.exclusive_maximum.unwrap_or(f64::INFINITY) {
                        b.exclusive_maximum = Some(*v);
                    }
                }
                NumKind::MultipleOf(m) => {
                    b.multiple_of.get_or_insert(*m);
                }
                NumKind::Int | NumKind::Safe | NumKind::Format(_) => {
                    let f = match &c.kind {
                        NumKind::Format(f) => *f,
                        _ => NumFormat::SafeInt,
                    };
                    let (lo, hi) = f.range();
                    b.format = Some(f.name());
                    b.minimum = Some(lo);
                    b.maximum = Some(hi);
                }
                NumKind::Finite => {}
            }
        }
        b
    }

    /// `.isInt`.
    pub fn is_int(&self) -> bool {
        let b = self.bag();
        b.format.is_some_and(|f| f.contains("int")) || b.multiple_of.is_some_and(|m| m.fract() == 0.0 && m.abs() <= MAX_SAFE)
    }
}

#[derive(Default)]
pub(crate) struct NumBag {
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub exclusive_minimum: Option<f64>,
    pub exclusive_maximum: Option<f64>,
    pub format: Option<&'static str>,
    pub multiple_of: Option<f64>,
}

fn number_format_check(f: NumFormat, n: f64, input: &Val, p: &mut Payload, err: Option<ErrorMap>, cont: Option<bool>) {
    if f.is_int() {
        if n.fract() != 0.0 || !n.is_finite() {
            p.push(issue(
                vec![("expected", json!("int")), ("format", json!(f.name())), ("code", json!("invalid_type"))],
                input,
                err,
                Some(false),
            ));
            return;
        }
        if n.abs() > MAX_SAFE {
            let (code, key, bound) = if n > 0.0 { ("too_big", "maximum", MAX_SAFE) } else { ("too_small", "minimum", -MAX_SAFE) };
            p.push(issue(
                vec![
                    ("code", json!(code)),
                    (key, js::num_json(bound)),
                    ("note", json!("Integers must be within the safe integer range.")),
                    ("origin", json!("int")),
                    ("inclusive", json!(true)),
                ],
                input,
                err,
                cont,
            ));
            return;
        }
    }
    let (lo, hi) = f.range();
    if n < lo {
        p.push(issue(
            vec![("origin", json!("number")), ("code", json!("too_small")), ("minimum", js::num_json(lo)), ("inclusive", json!(true))],
            input,
            err.clone(),
            cont,
        ));
    }
    if n > hi {
        p.push(issue(
            vec![("origin", json!("number")), ("code", json!("too_big")), ("maximum", js::num_json(hi)), ("inclusive", json!(true))],
            input,
            err,
            cont,
        ));
    }
}

pub(crate) fn number_json_schema(b: &NumBag, jsg: &JsonSchemaGen, json: &mut Map<String, Value>) {
    json.insert("type".into(), json!(if b.format.is_some_and(|f| f.contains("int")) { "integer" } else { "number" }));
    let legacy = matches!(jsg.target(), "draft-04" | "openapi-3.0");
    let ex_min = b.exclusive_minimum.filter(|e| *e >= b.minimum.unwrap_or(f64::NEG_INFINITY));
    let ex_max = b.exclusive_maximum.filter(|e| *e <= b.maximum.unwrap_or(f64::INFINITY));
    if let Some(e) = ex_min {
        if legacy {
            json.insert("minimum".into(), js::num_json(e));
            json.insert("exclusiveMinimum".into(), json!(true));
        } else {
            json.insert("exclusiveMinimum".into(), js::num_json(e));
        }
    } else if let Some(m) = b.minimum {
        json.insert("minimum".into(), js::num_json(m));
    }
    if let Some(e) = ex_max {
        if legacy {
            json.insert("maximum".into(), js::num_json(e));
            json.insert("exclusiveMaximum".into(), json!(true));
        } else {
            json.insert("exclusiveMaximum".into(), js::num_json(e));
        }
    } else if let Some(m) = b.maximum {
        json.insert("maximum".into(), js::num_json(m));
    }
    if let Some(m) = b.multiple_of {
        json.insert("multipleOf".into(), js::num_json(m));
    }
}

impl ZodType for ZNumber {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let n: f64 = match input {
            Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
            Some(other) if self.coerce => match other {
                Value::String(s) if s.trim().is_empty() => 0.0,
                Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
                Value::Bool(b) => f64::from(u8::from(*b)),
                Value::Null => 0.0,
                _ => f64::NAN,
            },
            other => {
                invalid_type(ctx, "number", other, &self.errors);
                return Err(());
            }
        };
        if n.is_nan() {
            let mut p = Map::new();
            p.insert("expected".into(), json!("number"));
            p.insert("received".into(), json!("nan"));
            ctx.add(ZodIssueCode::InvalidType, self.errors.invalid_type.clone().unwrap_or("Expected number, received nan".into()), p);
            return Err(());
        }
        let mut ok = true;
        let mut kinds: Vec<(NumKind, &Option<String>)> = Vec::new();
        for c in &self.checks {
            match &c.kind {
                NumKind::Safe => {
                    kinds.push((NumKind::Min(-MAX_SAFE, true), &c.message));
                    kinds.push((NumKind::Max(MAX_SAFE, true), &c.message));
                }
                NumKind::Format(f) => {
                    if f.is_int() {
                        kinds.push((NumKind::Int, &c.message));
                    }
                    let (lo, hi) = f.range();
                    kinds.push((NumKind::Min(lo, true), &c.message));
                    kinds.push((NumKind::Max(hi, true), &c.message));
                }
                k => kinds.push((k.clone(), &c.message)),
            }
        }
        for (kind, message) in &kinds {
            let message: Option<String> = (**message).clone();
            let msg = |d: String| message.clone().unwrap_or(d);
            match *kind {
                NumKind::Int if n.fract() != 0.0 => {
                    ok = false;
                    ctx.add(
                        ZodIssueCode::InvalidType,
                        msg("Expected integer, received float".into()),
                        issue_params(&[("expected", json!("integer")), ("received", json!("float"))]),
                    );
                }
                NumKind::Min(v, inclusive) if (inclusive && n < v) || (!inclusive && n <= v) => {
                    ok = false;
                    let d = if inclusive {
                        format!("Number must be greater than or equal to {}", fmt_num(v))
                    } else {
                        format!("Number must be greater than {}", fmt_num(v))
                    };
                    ctx.add(
                        ZodIssueCode::TooSmall,
                        msg(d),
                        issue_params(&[("minimum", num_value(v)), ("type", json!("number")), ("inclusive", json!(inclusive)), ("exact", json!(false))]),
                    );
                }
                NumKind::Max(v, inclusive) if (inclusive && n > v) || (!inclusive && n >= v) => {
                    ok = false;
                    let d = if inclusive {
                        format!("Number must be less than or equal to {}", fmt_num(v))
                    } else {
                        format!("Number must be less than {}", fmt_num(v))
                    };
                    ctx.add(
                        ZodIssueCode::TooBig,
                        msg(d),
                        issue_params(&[("maximum", num_value(v)), ("type", json!("number")), ("inclusive", json!(inclusive)), ("exact", json!(false))]),
                    );
                }
                NumKind::MultipleOf(m) if m != 0.0 && ((n / m).round() * m - n).abs() > 1e-9 * n.abs().max(1.0) => {
                    ok = false;
                    ctx.add(
                        ZodIssueCode::NotMultipleOf,
                        msg(format!("Number must be a multiple of {}", fmt_num(m))),
                        issue_params(&[("multipleOf", num_value(m))]),
                    );
                }
                NumKind::Finite if !n.is_finite() => {
                    ok = false;
                    ctx.add(ZodIssueCode::NotFinite, msg("Number must be finite".into()), Map::new());
                }
                _ => {}
            }
        }
        if !ok {
            return Err(());
        }
        // Keep the original JSON number when it was not coerced (preserves u64/i64 exactly).
        match input {
            Some(v @ Value::Number(_)) => Ok(Some(v.clone())),
            _ => Ok(Some(num_value(n))),
        }
    }

    fn type_name(&self) -> String {
        "number".into()
    }

    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        run_parse_checks(
            p,
            ctx,
            !self.checks.is_empty(),
            |p| self.parse_v4(p),
            |p| run_checks(p, &self.checks, |c| &c.opts, |_| None, |c, p| self.apply_v4(c, p)),
        )
    }

    fn pattern(&self) -> Option<String> {
        let b = self.bag();
        Some(if b.format.is_some_and(|f| f.contains("int")) { regexes::INTEGER.source() } else { regexes::NUMBER.source() }.to_string())
    }

    fn def_type(&self) -> &'static str {
        "number"
    }

    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        number_json_schema(&self.bag(), jsg, json);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// bigint
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum BigKind {
    Min(i128, bool),
    Max(i128, bool),
    MultipleOf(i128),
    /// `int64` / `uint64`.
    #[allow(dead_code)]
    Format(&'static str, i128, i128),
}

/// `z.bigint()` (values are `BigInt`s: they come from coercion or transforms).
#[derive(Clone, Default)]
pub struct ZBigInt {
    checks: Vec<Chk<BigKind>>,
    coerce: bool,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

fn big_json(b: i128) -> Value {
    Val::BigInt(b).to_json_or_null()
}

impl ZBigInt {
    pub fn new() -> Self {
        Self::default()
    }
    fn push(mut self, k: BigKind) -> Self {
        self.checks.push(Chk::new(k));
        self
    }
    check_builders!();
    pub(crate) fn coerce(mut self) -> Self {
        self.coerce = true;
        self
    }
    pub fn gt(self, n: i128) -> Self {
        self.push(BigKind::Min(n, false))
    }
    pub fn gte(self, n: i128) -> Self {
        self.push(BigKind::Min(n, true))
    }
    pub fn min(self, n: i128) -> Self {
        self.gte(n)
    }
    pub fn lt(self, n: i128) -> Self {
        self.push(BigKind::Max(n, false))
    }
    pub fn lte(self, n: i128) -> Self {
        self.push(BigKind::Max(n, true))
    }
    pub fn max(self, n: i128) -> Self {
        self.lte(n)
    }
    pub fn positive(self) -> Self {
        self.gt(0)
    }
    pub fn negative(self) -> Self {
        self.lt(0)
    }
    pub fn nonnegative(self) -> Self {
        self.gte(0)
    }
    pub fn nonpositive(self) -> Self {
        self.lte(0)
    }
    pub fn multiple_of(self, n: i128) -> Self {
        self.push(BigKind::MultipleOf(n))
    }
    fn inst_error(&self) -> Option<ErrorMap> {
        self.errors.v4_map(&self.error)
    }
}

impl ZodType for ZBigInt {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        "bigint".into()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        run_parse_checks(
            p,
            ctx,
            !self.checks.is_empty(),
            |mut p| {
                if self.coerce
                    && let Some(b) = js_bigint_of(&p.value) {
                        p.value = Val::BigInt(b);
                    }
                if !matches!(p.value, Val::BigInt(_)) {
                    invalid_type_v4("bigint", &mut p, self.inst_error());
                }
                p
            },
            |p| {
                run_checks(
                    p,
                    &self.checks,
                    |c| &c.opts,
                    |_| None,
                    |c, p| {
                        let Val::BigInt(n) = p.value else { return };
                        let err = if c.is_self { self.inst_error() } else { c.opts.error.clone() };
                        let cont = c.opts.cont();
                        let input = p.value.clone();
                        let small = |p: &mut Payload, v: i128, incl: bool, err: Option<ErrorMap>| {
                            p.push(issue(
                                vec![
                                    ("origin", json!("bigint")),
                                    ("code", json!("too_small")),
                                    ("minimum", big_json(v)),
                                    ("inclusive", json!(incl)),
                                ],
                                &input,
                                err,
                                cont,
                            ))
                        };
                        let big = |p: &mut Payload, v: i128, incl: bool, err: Option<ErrorMap>| {
                            p.push(issue(
                                vec![
                                    ("origin", json!("bigint")),
                                    ("code", json!("too_big")),
                                    ("maximum", big_json(v)),
                                    ("inclusive", json!(incl)),
                                ],
                                &input,
                                err,
                                cont,
                            ))
                        };
                        match c.kind {
                            BigKind::Min(v, incl) if !(if incl { n >= v } else { n > v }) => small(p, v, incl, err),
                            BigKind::Max(v, incl) if !(if incl { n <= v } else { n < v }) => big(p, v, incl, err),
                            BigKind::MultipleOf(m) if m != 0 && n % m != 0 => p.push(issue(
                                vec![("origin", json!("bigint")), ("code", json!("not_multiple_of")), ("divisor", big_json(m))],
                                &input,
                                err,
                                cont,
                            )),
                            BigKind::Format(_, lo, hi) => {
                                if n < lo {
                                    small(p, lo, true, err.clone());
                                }
                                if n > hi {
                                    big(p, hi, true, err);
                                }
                            }
                            _ => {}
                        }
                    },
                )
            },
        )
    }
    fn pattern(&self) -> Option<String> {
        Some(regexes::BIGINT.source().to_string())
    }
    fn def_type(&self) -> &'static str {
        "bigint"
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        jsg.unrepresentable("BigInt cannot be represented in JSON Schema")
    }
}

// ---------------------------------------------------------------------------
// boolean, date, literal, enum, null, undefined, void, any, never, nan
// ---------------------------------------------------------------------------

/// `z.boolean()`.
#[derive(Clone, Default)]
pub struct ZBoolean {
    coerce: bool,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

impl ZBoolean {
    pub fn new() -> Self {
        <ZBoolean as Default>::default()
    }
    pub(crate) fn coerce(mut self) -> Self {
        self.coerce = true;
        self
    }
    pub fn required_error(mut self, message: impl Into<String>) -> Self {
        self.errors.required = Some(message.into());
        self
    }
    pub fn invalid_type_error(mut self, message: impl Into<String>) -> Self {
        self.errors.invalid_type = Some(message.into());
        self
    }
    /// `z.boolean({ error })`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        let e: ErrorParam = e.into();
        if let Some(m) = e.message() {
            self.errors.invalid_type = Some(m);
        }
        self.error = Some(e.into_map());
        self
    }
    pub fn message(self, m: impl Into<String>) -> Self {
        self.error(ErrorParam::Message(m.into()))
    }
}

impl ZodType for ZBoolean {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            Some(Value::Bool(b)) => Ok(Some(Value::Bool(*b))),
            // `Boolean(input)`: only `""`, `0`, `null` and `undefined` are false (`"false"` is true).
            Some(other) if self.coerce => Ok(Some(Value::Bool(js::truthy(Some(other))))),
            None if self.coerce => Ok(Some(Value::Bool(false))),
            other => {
                invalid_type(ctx, "boolean", other, &self.errors);
                Err(())
            }
        }
    }
    fn type_name(&self) -> String {
        "boolean".into()
    }
    fn run_v4(&self, mut p: Payload, _ctx: &RunCtx) -> Payload {
        if self.coerce {
            p.value = Val::Json(Value::Bool(js_truthy(&p.value)));
        }
        if !matches!(p.value, Val::Json(Value::Bool(_))) {
            invalid_type_v4("boolean", &mut p, self.errors.v4_map(&self.error));
        }
        p
    }
    fn pattern(&self) -> Option<String> {
        Some(regexes::BOOLEAN.source().to_string())
    }
    fn def_type(&self) -> &'static str {
        "boolean"
    }
    fn json_schema(&self, _gen: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("boolean"));
        Ok(())
    }
}

/// `z.date()`. zod 3: an ISO 8601 date / date-time string (with `coerce`, millisecond timestamps
/// are accepted and normalized). zod 4: a `Date` value; JSON has no dates, so they come from
/// `z.coerce.date()`, transforms, or ISO strings when [`ZodConfig::date_strings`](super::ZodConfig) is on.
#[derive(Clone, Default)]
pub struct ZDate {
    coerce: bool,
    min: Option<(chrono::DateTime<chrono::Utc>, Option<String>)>,
    max: Option<(chrono::DateTime<chrono::Utc>, Option<String>)>,
    checks: Vec<Chk<(bool, f64)>>,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

fn parse_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(&normalize_seconds(s)) {
        return Some(d.with_timezone(&chrono::Utc));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|n| n.and_utc());
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|n| n.and_utc())
}

impl ZDate {
    pub fn new() -> Self {
        <ZDate as Default>::default()
    }
    pub(crate) fn coerce(mut self) -> Self {
        self.coerce = true;
        self
    }
    check_builders!();
    /// `.min(date)`; the date is an ISO string. Panics if it does not parse.
    pub fn min(mut self, date: &str) -> Self {
        self.min = Some((parse_date(date).expect("z::date().min(): invalid date"), None));
        self.checks.push(Chk::new((true, js::parse_date(date))));
        self
    }
    /// `.max(date)`.
    pub fn max(mut self, date: &str) -> Self {
        self.max = Some((parse_date(date).expect("z::date().max(): invalid date"), None));
        self.checks.push(Chk::new((false, js::parse_date(date))));
        self
    }
    /// `.min(new Date(ms))`.
    pub fn min_ms(mut self, ms: f64) -> Self {
        self.min = chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| (d, None));
        self.checks.push(Chk::new((true, ms)));
        self
    }
    /// `.max(new Date(ms))`.
    pub fn max_ms(mut self, ms: f64) -> Self {
        self.max = chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| (d, None));
        self.checks.push(Chk::new((false, ms)));
        self
    }
    fn inst_error(&self) -> Option<ErrorMap> {
        self.errors.v4_map(&self.error)
    }
}

impl ZodType for ZDate {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let (date, original) = match input {
            Some(Value::String(s)) => (parse_date(s), Some(s.clone())),
            Some(Value::Number(n)) if self.coerce => {
                (n.as_i64().and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis), None)
            }
            other => {
                invalid_type(ctx, "date", other, &self.errors);
                return Err(());
            }
        };
        let Some(date) = date else {
            ctx.add(ZodIssueCode::InvalidDate, "Invalid date", Map::new());
            return Err(());
        };
        let mut ok = true;
        let min_msg = self.checks.iter().find(|c| c.kind.0).and_then(|c| c.message.clone());
        let max_msg = self.checks.iter().find(|c| !c.kind.0).and_then(|c| c.message.clone());
        if let Some((min, msg)) = &self.min
            && date < *min {
                ok = false;
                ctx.add(
                    ZodIssueCode::TooSmall,
                    msg.clone().or(min_msg).unwrap_or_else(|| format!("Date must be greater than or equal to {}", min.to_rfc3339())),
                    issue_params(&[("minimum", json!(min.timestamp_millis())), ("type", json!("date")), ("inclusive", json!(true)), ("exact", json!(false))]),
                );
            }
        if let Some((max, msg)) = &self.max
            && date > *max {
                ok = false;
                ctx.add(
                    ZodIssueCode::TooBig,
                    msg.clone().or(max_msg).unwrap_or_else(|| format!("Date must be smaller than or equal to {}", max.to_rfc3339())),
                    issue_params(&[("maximum", json!(max.timestamp_millis())), ("type", json!("date")), ("inclusive", json!(true)), ("exact", json!(false))]),
                );
            }
        if !ok {
            return Err(());
        }
        Ok(Some(Value::String(match (original, self.coerce) {
            (Some(s), false) => s,
            _ => date.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        })))
    }
    fn type_name(&self) -> String {
        "date".into()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        run_parse_checks(
            p,
            ctx,
            !self.checks.is_empty(),
            |mut p| {
                if self.coerce {
                    if let Some(t) = js_date_of(&p.value) {
                        p.value = Val::Date(t);
                    }
                } else if ctx.date_strings
                    && let Some(s) = p.value.as_str()
                        && js::looks_like_iso_datetime(s) {
                            p.value = Val::Date(js::parse_date(s));
                        }
                match p.value {
                    Val::Date(t) if !t.is_nan() => {}
                    Val::Date(_) => {
                        let input = p.value.clone();
                        p.push(issue(
                            vec![("expected", json!("date")), ("code", json!("invalid_type")), ("received", json!("Invalid Date"))],
                            &input,
                            self.inst_error(),
                            None,
                        ));
                    }
                    _ => invalid_type_v4("date", &mut p, self.inst_error()),
                }
                p
            },
            |p| {
                run_checks(
                    p,
                    &self.checks,
                    |c| &c.opts,
                    |_| None,
                    |c, p| {
                        let Val::Date(t) = p.value else { return };
                        let (is_min, bound) = c.kind;
                        let input = p.value.clone();
                        let ok = if is_min { t >= bound } else { t <= bound };
                        if !ok {
                            let (code, key) = if is_min { ("too_small", "minimum") } else { ("too_big", "maximum") };
                            p.push(issue(
                                vec![("origin", json!("date")), ("code", json!(code)), (key, js::num_json(bound)), ("inclusive", json!(true))],
                                &input,
                                c.opts.error.clone(),
                                c.opts.cont(),
                            ));
                        }
                    },
                )
            },
        )
    }
    fn def_type(&self) -> &'static str {
        "date"
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        jsg.unrepresentable("Date cannot be represented in JSON Schema")
    }
}

/// `z.literal(value)` / `z.literal([a, b])`.
#[derive(Clone)]
pub struct ZLiteral {
    values: Vec<Val>,
    error: Option<ErrorMap>,
}

impl ZLiteral {
    /// `z.literal(value)`; a JSON array means several values (like `z.literal([...])`).
    pub fn new(value: Value) -> Self {
        let values = match value {
            Value::Array(a) => a.into_iter().map(Val::Json).collect(),
            v => vec![Val::Json(v)],
        };
        ZLiteral { values, error: None }
    }
    /// From any JS values (`undefined`, `BigInt`, ...).
    pub fn from_vals(values: Vec<Val>) -> Self {
        assert!(!values.is_empty(), "Cannot create literal schema with no valid values");
        ZLiteral { values, error: None }
    }
    /// `.value` (the single value).
    pub fn value(&self) -> Value {
        self.values[0].to_json_or_null()
    }
    /// `.values`.
    pub fn values(&self) -> Vec<Value> {
        self.values.iter().map(Val::to_json_or_null).collect()
    }
    /// `z.literal(v, { error })`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
    pub fn message(self, m: impl Into<String>) -> Self {
        self.error(ErrorParam::Message(m.into()))
    }
    fn single(&self) -> Value {
        self.values[0].to_json_or_null()
    }
}

fn literal_pattern_part(v: &Val) -> String {
    match v {
        Val::Json(Value::String(s)) => regexes::escape_regex(s),
        Val::Undef => "undefined".into(),
        Val::Json(Value::Null) => "null".into(),
        v if !js_truthy(v) => js_string_of(v),
        v => regexes::escape_regex(&match v {
            Val::BigInt(b) => b.to_string(),
            other => js_string_of(other),
        }),
    }
}

impl ZodType for ZLiteral {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        if self.values.len() > 1 {
            return v3_via_v4(self, input, ctx);
        }
        let lit = self.single();
        match input {
            Some(v) if *v == lit => Ok(Some(v.clone())),
            Some(v) => {
                ctx.add(
                    ZodIssueCode::InvalidLiteral,
                    format!("Invalid literal value, expected {}", lit),
                    issue_params(&[("expected", lit.clone()), ("received", v.clone())]),
                );
                Err(())
            }
            None => {
                invalid_type(ctx, &lit.to_string(), None, &TypeErrors::default());
                Err(())
            }
        }
    }
    fn type_name(&self) -> String {
        self.single().to_string()
    }
    fn literal_values(&self) -> Option<Vec<Value>> {
        Some(self.values())
    }
    fn accepts_undefined(&self) -> bool {
        self.values.iter().any(Val::is_undef)
    }
    fn run_v4(&self, mut p: Payload, _ctx: &RunCtx) -> Payload {
        if !self.values.iter().any(|v| v.same_value_zero(&p.value)) {
            let input = p.value.clone();
            let values: Vec<Value> = self.values.iter().map(Val::to_json_or_null).collect();
            p.push(issue(vec![("code", json!("invalid_value")), ("values", Value::Array(values))], &input, self.error.clone(), None));
        }
        p
    }
    fn values(&self) -> Option<Vec<Val>> {
        Some(self.values.clone())
    }
    fn pattern(&self) -> Option<String> {
        Some(format!("^({})$", self.values.iter().map(literal_pattern_part).collect::<Vec<_>>().join("|")))
    }
    fn def_type(&self) -> &'static str {
        "literal"
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        let mut vals = Vec::new();
        for v in &self.values {
            match v {
                Val::Undef => jsg.unrepresentable("Literal `undefined` cannot be represented in JSON Schema")?,
                Val::BigInt(b) => {
                    jsg.unrepresentable("BigInt literals cannot be represented in JSON Schema")?;
                    vals.push(js::num_json(*b as f64));
                }
                v => vals.push(v.to_json_or_null()),
            }
        }
        let ty = |v: &Value| match v {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            _ => "object",
        };
        match vals.len() {
            0 => {}
            1 => {
                json.insert("type".into(), json!(ty(&vals[0])));
                if matches!(jsg.target(), "draft-04" | "openapi-3.0") {
                    json.insert("enum".into(), Value::Array(vec![vals[0].clone()]));
                } else {
                    json.insert("const".into(), vals[0].clone());
                }
            }
            _ => {
                for t in ["number", "string", "boolean", "null"] {
                    if vals.iter().all(|v| ty(v) == t) {
                        json.insert("type".into(), json!(t));
                    }
                }
                json.insert("enum".into(), Value::Array(vals));
            }
        }
        Ok(())
    }
}

/// `z.enum([...])` / `z.enum({ ... })` / `z.nativeEnum(...)`.
#[derive(Clone)]
pub struct ZEnum {
    values: Vec<String>,
    entries: Vec<(String, Value)>,
    message: Option<String>,
    error: Option<ErrorMap>,
}

impl ZEnum {
    pub fn new(values: Vec<String>) -> Self {
        let entries = values.iter().map(|v| (v.clone(), Value::String(v.clone()))).collect();
        ZEnum { values, entries, message: None, error: None }
    }
    /// `z.enum({ key: value, ... })` (string or number values, like a TS enum).
    pub fn from_entries(entries: Vec<(String, Value)>) -> Self {
        let values = entries.iter().map(|(_, v)| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect();
        ZEnum { values, entries, message: None, error: None }
    }
    /// `.options` (string values).
    pub fn options(&self) -> &[String] {
        &self.values
    }
    /// `.enum` (the entries).
    pub fn entries(&self) -> &[(String, Value)] {
        &self.entries
    }
    /// `util.getEnumValues(entries)`.
    pub fn enum_values(&self) -> Vec<Value> {
        let numeric: Vec<f64> = self.entries.iter().filter_map(|(_, v)| v.as_f64()).collect();
        self.entries
            .iter()
            .filter(|(k, _)| {
                let n = js::string_to_number(k);
                !numeric.contains(&n)
            })
            .map(|(_, v)| v.clone())
            .collect()
    }
    /// `.extract([...])`.
    pub fn extract(&self, keep: &[&str]) -> ZEnum {
        let mut e = ZEnum::from_entries(self.entries.iter().filter(|(k, _)| keep.contains(&k.as_str())).cloned().collect());
        e.values = self.values.iter().filter(|v| keep.contains(&v.as_str())).cloned().collect();
        e
    }
    /// `.exclude([...])`.
    pub fn exclude(&self, drop: &[&str]) -> ZEnum {
        let mut e = ZEnum::from_entries(self.entries.iter().filter(|(k, _)| !drop.contains(&k.as_str())).cloned().collect());
        e.values = self.values.iter().filter(|v| !drop.contains(&v.as_str())).cloned().collect();
        e
    }
    pub fn message(mut self, message: impl Into<String>) -> Self {
        let m = message.into();
        self.error = Some(ErrorParam::Message(m.clone()).into_map());
        self.message = Some(m);
        self
    }
    /// `z.enum([...], { error })`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        let e: ErrorParam = e.into();
        self.message = e.message();
        self.error = Some(e.into_map());
        self
    }
}

impl ZodType for ZEnum {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            Some(Value::String(s)) if self.values.iter().any(|v| v == s) => Ok(Some(Value::String(s.clone()))),
            Some(Value::String(s)) => {
                ctx.add(
                    ZodIssueCode::InvalidEnumValue,
                    self.message.clone().unwrap_or_else(|| {
                        format!("Invalid enum value. Expected {}, received '{s}'", quote_list(&self.values))
                    }),
                    issue_params(&[("received", json!(s)), ("options", json!(self.values))]),
                );
                Err(())
            }
            other => {
                let errors = TypeErrors { required: None, invalid_type: self.message.clone() };
                invalid_type(ctx, &quote_list(&self.values), other, &errors);
                Err(())
            }
        }
    }
    fn type_name(&self) -> String {
        quote_list(&self.values)
    }
    fn literal_values(&self) -> Option<Vec<Value>> {
        Some(self.enum_values())
    }
    fn run_v4(&self, mut p: Payload, _ctx: &RunCtx) -> Payload {
        let values = self.enum_values();
        if !values.iter().any(|v| Val::Json(v.clone()).same_value_zero(&p.value)) {
            let input = p.value.clone();
            p.push(issue(vec![("code", json!("invalid_value")), ("values", Value::Array(values))], &input, self.error.clone(), None));
        }
        p
    }
    fn values(&self) -> Option<Vec<Val>> {
        Some(self.enum_values().into_iter().map(Val::Json).collect())
    }
    fn pattern(&self) -> Option<String> {
        let parts: Vec<String> = self
            .enum_values()
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(regexes::escape_regex(s)),
                Value::Number(n) => Some(js::num_to_string(js::json_f64(n))),
                _ => None,
            })
            .collect();
        Some(format!("^({})$", parts.join("|")))
    }
    fn def_type(&self) -> &'static str {
        "enum"
    }
    fn json_schema(&self, _gen: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        let values = self.enum_values();
        if values.iter().all(Value::is_number) {
            json.insert("type".into(), json!("number"));
        }
        if values.iter().all(Value::is_string) {
            json.insert("type".into(), json!("string"));
        }
        json.insert("enum".into(), Value::Array(values));
        Ok(())
    }
}

macro_rules! simple_type {
    ($(#[$m:meta])* $name:ident, $expected:literal, $def:literal, |$v:ident| $ok:expr, $v3:expr, $js:expr) => {
        $(#[$m])*
        #[derive(Clone, Default)]
        pub struct $name {
            error: Option<ErrorMap>,
        }
        impl $name {
            pub fn new() -> Self {
                Self::default()
            }
            /// `{ error }`.
            pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
                self.error = Some(e.into().into_map());
                self
            }
        }
        impl ZodType for $name {
            #[allow(clippy::redundant_closure_call)]
            fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
                ($v3)(self, input, ctx)
            }
            fn type_name(&self) -> String {
                $expected.into()
            }
            fn run_v4(&self, mut p: Payload, _ctx: &RunCtx) -> Payload {
                let $v = &p.value;
                if !($ok) {
                    invalid_type_v4($expected, &mut p, self.error.clone());
                }
                p
            }
            fn def_type(&self) -> &'static str {
                $def
            }
            #[allow(clippy::redundant_closure_call)]
            fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
                ($js)(jsg, json)
            }
        }
    };
}

simple_type!(
    /// `z.null()`.
    ZNull,
    "null",
    "null",
    |v| v.is_null(),
    |_s: &ZNull, input: Option<&Value>, ctx: &mut ParseCtx| match input {
        Some(Value::Null) => Ok(Some(Value::Null)),
        other => {
            invalid_type(ctx, "null", other, &TypeErrors::default());
            Err(())
        }
    },
    |jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>| {
        if jsg.target() == "openapi-3.0" {
            json.insert("type".into(), json!("string"));
            json.insert("nullable".into(), json!(true));
            json.insert("enum".into(), json!([null]));
        } else {
            json.insert("type".into(), json!("null"));
        }
        Ok(())
    }
);

simple_type!(
    /// `z.undefined()`.
    ZUndefined,
    "undefined",
    "undefined",
    |v| v.is_undef(),
    |s: &ZUndefined, input: Option<&Value>, ctx: &mut ParseCtx| v3_via_v4(s, input, ctx),
    |jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>| jsg.unrepresentable("Undefined cannot be represented in JSON Schema")
);

simple_type!(
    /// `z.void()`.
    ZVoid,
    "void",
    "void",
    |v| v.is_undef(),
    |s: &ZVoid, input: Option<&Value>, ctx: &mut ParseCtx| v3_via_v4(s, input, ctx),
    |jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>| jsg.unrepresentable("Void cannot be represented in JSON Schema")
);

simple_type!(
    /// `z.never()`.
    ZNever,
    "never",
    "never",
    |_v| false,
    |_s: &ZNever, input: Option<&Value>, ctx: &mut ParseCtx| {
        invalid_type(ctx, "never", input, &TypeErrors::default());
        Err(())
    },
    |_gen: &mut JsonSchemaGen, json: &mut Map<String, Value>| {
        json.insert("not".into(), json!({}));
        Ok(())
    }
);

simple_type!(
    /// `z.nan()`.
    ZNaN,
    "nan",
    "nan",
    |v| matches!(v, Val::Num(n) if n.is_nan()),
    |s: &ZNaN, input: Option<&Value>, ctx: &mut ParseCtx| v3_via_v4(s, input, ctx),
    |jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>| jsg.unrepresentable("NaN cannot be represented in JSON Schema")
);


/// `z.any()` / `z.unknown()`.
#[derive(Clone)]
pub struct ZAny;
impl ZodType for ZAny {
    fn check(&self, input: Option<&Value>, _ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        Ok(input.cloned())
    }
    fn type_name(&self) -> String {
        "any".into()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn run_v4(&self, p: Payload, _ctx: &RunCtx) -> Payload {
        p
    }
    fn def_type(&self) -> &'static str {
        "any"
    }
    fn json_schema(&self, _gen: &mut JsonSchemaGen, _json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// stringbool, templateLiteral, stringFormat
// ---------------------------------------------------------------------------

/// Options of `z.stringbool({ truthy, falsy, case })`.
#[derive(Clone)]
pub struct StringBoolOptions {
    pub truthy: Vec<String>,
    pub falsy: Vec<String>,
    /// `case: "sensitive"`.
    pub case_sensitive: bool,
    pub error: Option<ErrorParam>,
}

impl Default for StringBoolOptions {
    fn default() -> Self {
        StringBoolOptions {
            truthy: ["true", "1", "yes", "on", "y", "enabled"].map(String::from).to_vec(),
            falsy: ["false", "0", "no", "off", "n", "disabled"].map(String::from).to_vec(),
            case_sensitive: false,
            error: None,
        }
    }
}

/// `z.stringbool()`: a codec `string ⇄ boolean`.
#[derive(Clone)]
pub struct ZStringBool {
    truthy: Vec<String>,
    falsy: Vec<String>,
    case_sensitive: bool,
    error: Option<ErrorMap>,
}

impl ZStringBool {
    pub fn new(o: StringBoolOptions) -> Self {
        let norm = |v: Vec<String>| if o.case_sensitive { v } else { v.into_iter().map(|s| s.to_lowercase()).collect() };
        ZStringBool {
            truthy: norm(o.truthy.clone()),
            falsy: norm(o.falsy.clone()),
            case_sensitive: o.case_sensitive,
            error: o.error.map(ErrorParam::into_map),
        }
    }
}

impl ZodType for ZStringBool {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        "boolean".into()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.backward() {
            if !matches!(p.value, Val::Json(Value::Bool(_))) {
                invalid_type_v4("boolean", &mut p, self.error.clone());
                p.aborted = true;
                return p;
            }
            let t = p.value == Val::Json(Value::Bool(true));
            let s = if t { self.truthy.first().cloned().unwrap_or("true".into()) } else { self.falsy.first().cloned().unwrap_or("false".into()) };
            p.value = Val::Json(Value::String(s));
            return p;
        }
        let Some(s) = p.value.as_str() else {
            invalid_type_v4("string", &mut p, self.error.clone());
            p.aborted = true;
            return p;
        };
        let data = if self.case_sensitive { s.to_string() } else { s.to_lowercase() };
        if self.truthy.contains(&data) {
            p.value = Val::Json(Value::Bool(true));
        } else if self.falsy.contains(&data) {
            p.value = Val::Json(Value::Bool(false));
        } else {
            let mut values: Vec<String> = Vec::new();
            for v in self.truthy.iter().chain(self.falsy.iter()) {
                if !values.contains(v) {
                    values.push(v.clone());
                }
            }
            let input = p.value.clone();
            p.push(issue(
                vec![("code", json!("invalid_value")), ("expected", json!("stringbool")), ("values", json!(values))],
                &input,
                self.error.clone(),
                Some(false),
            ));
            p.aborted = true;
        }
        p
    }
    fn def_type(&self) -> &'static str {
        "pipe"
    }
    fn is_transforming(&self) -> bool {
        true
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!(if jsg.io_input() { "string" } else { "boolean" }));
        Ok(())
    }
}

/// A part of `z.templateLiteral([...])`: a fixed value or a schema.
#[derive(Clone)]
pub enum TemplatePart {
    Value(Value),
    Schema(Schema),
}

impl From<&str> for TemplatePart {
    fn from(s: &str) -> Self {
        TemplatePart::Value(Value::String(s.into()))
    }
}
impl From<Value> for TemplatePart {
    fn from(v: Value) -> Self {
        TemplatePart::Value(v)
    }
}
impl From<Schema> for TemplatePart {
    fn from(s: Schema) -> Self {
        TemplatePart::Schema(s)
    }
}

/// `z.templateLiteral([...])`.
#[derive(Clone)]
pub struct ZTemplateLiteral {
    pattern: JsRegex,
    format: Option<String>,
    error: Option<ErrorMap>,
}

impl ZTemplateLiteral {
    pub fn new(parts: Vec<TemplatePart>) -> Self {
        let mut src = String::new();
        for p in parts {
            match p {
                TemplatePart::Schema(s) => {
                    let pat = s.node().pattern().unwrap_or_else(|| panic!("Invalid template literal part, no pattern found"));
                    let pat = pat.strip_prefix('^').unwrap_or(&pat);
                    let pat = pat.strip_suffix('$').unwrap_or(pat);
                    src.push_str(pat);
                }
                TemplatePart::Value(v) => src.push_str(&regexes::escape_regex(&js::to_js_string(&v))),
            }
        }
        ZTemplateLiteral { pattern: JsRegex::new(&format!("^{src}$"), ""), format: None, error: None }
    }
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
}

impl ZodType for ZTemplateLiteral {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        "string".into()
    }
    fn run_v4(&self, mut p: Payload, _ctx: &RunCtx) -> Payload {
        let Some(s) = p.value.as_str() else {
            let input = p.value.clone();
            p.push(issue(vec![("expected", json!("string")), ("code", json!("invalid_type"))], &input, self.error.clone(), None));
            return p;
        };
        if !self.pattern.test(s) {
            let input = p.value.clone();
            p.push(issue(
                vec![
                    ("code", json!("invalid_format")),
                    ("format", json!(self.format.clone().unwrap_or_else(|| "template_literal".into()))),
                    ("pattern", json!(self.pattern.source())),
                ],
                &input,
                self.error.clone(),
                None,
            ));
        }
        p
    }
    fn pattern(&self) -> Option<String> {
        Some(self.pattern.source().to_string())
    }
    fn def_type(&self) -> &'static str {
        "template_literal"
    }
    fn json_schema(&self, _gen: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("string"));
        json.insert("pattern".into(), json!(self.pattern.source()));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Constructors (`z.string()`, `z.email()`, ...).
// ---------------------------------------------------------------------------

/// `z.string()`.
pub fn string() -> ZString {
    ZString::new()
}
/// `z.email()`.
pub fn email() -> ZString {
    ZString::self_fmt(Fmt::email())
}
/// `z.email({ pattern })`.
pub fn email_with(pattern: impl Into<JsRegex>) -> ZString {
    ZString::self_fmt(Fmt::email_with(pattern.into()))
}
/// `z.url()`.
pub fn url() -> ZString {
    ZString::self_fmt(Fmt::url(UrlOptions::default()))
}
/// `z.url({ hostname, protocol, normalize })`.
pub fn url_with(o: UrlOptions) -> ZString {
    ZString::self_fmt(Fmt::url(o))
}
/// `z.httpUrl()`.
pub fn http_url() -> ZString {
    ZString::self_fmt(Fmt::url(UrlOptions {
        protocol: Some(regexes::HTTP_PROTOCOL.clone()),
        hostname: Some(regexes::DOMAIN.clone()),
        normalize: false,
    }))
}
/// `z.uuid()`.
pub fn uuid() -> ZString {
    ZString::self_fmt(Fmt::uuid(None))
}
/// `z.uuidv4()`.
pub fn uuidv4() -> ZString {
    ZString::self_fmt(Fmt::uuid(Some(4)))
}
/// `z.uuidv6()`.
pub fn uuidv6() -> ZString {
    ZString::self_fmt(Fmt::uuid(Some(6)))
}
/// `z.uuidv7()`.
pub fn uuidv7() -> ZString {
    ZString::self_fmt(Fmt::uuid(Some(7)))
}
/// `z.uuid({ version })`.
pub fn uuid_version(v: u8) -> ZString {
    ZString::self_fmt(Fmt::uuid(Some(v)))
}
/// `z.guid()`.
pub fn guid() -> ZString {
    ZString::self_fmt(Fmt::guid())
}
/// `z.cuid()`.
pub fn cuid() -> ZString {
    ZString::self_fmt(Fmt::cuid())
}
/// `z.cuid2()`.
pub fn cuid2() -> ZString {
    ZString::self_fmt(Fmt::cuid2())
}
/// `z.ulid()`.
pub fn ulid() -> ZString {
    ZString::self_fmt(Fmt::ulid())
}
/// `z.xid()`.
pub fn xid() -> ZString {
    ZString::self_fmt(Fmt::xid())
}
/// `z.ksuid()`.
pub fn ksuid() -> ZString {
    ZString::self_fmt(Fmt::ksuid())
}
/// `z.nanoid()`.
pub fn nanoid() -> ZString {
    ZString::self_fmt(Fmt::nanoid())
}
/// `z.emoji()`.
pub fn emoji() -> ZString {
    ZString::self_fmt(Fmt::emoji())
}
/// `z.ipv4()`.
pub fn ipv4() -> ZString {
    ZString::self_fmt(Fmt::ipv4())
}
/// `z.ipv6()`.
pub fn ipv6() -> ZString {
    ZString::self_fmt(Fmt::ipv6())
}
/// `z.cidrv4()`.
pub fn cidrv4() -> ZString {
    ZString::self_fmt(Fmt::cidrv4())
}
/// `z.cidrv6()`.
pub fn cidrv6() -> ZString {
    ZString::self_fmt(Fmt::cidrv6())
}
/// `z.mac({ delimiter })`.
pub fn mac(delimiter: Option<&str>) -> ZString {
    ZString::self_fmt(Fmt::mac(delimiter))
}
/// `z.base64()`.
pub fn base64() -> ZString {
    ZString::self_fmt(Fmt::base64())
}
/// `z.base64url()`.
pub fn base64url() -> ZString {
    ZString::self_fmt(Fmt::base64url())
}
/// `z.e164()`.
pub fn e164() -> ZString {
    ZString::self_fmt(Fmt::e164())
}
/// `z.jwt({ alg })`.
pub fn jwt(alg: Option<&str>) -> ZString {
    ZString::self_fmt(Fmt::jwt(alg))
}
/// `z.hostname()`.
pub fn hostname() -> ZString {
    ZString::self_fmt(Fmt::custom_regex("hostname", regexes::HOSTNAME.clone()))
}
/// `z.hex()`.
pub fn hex() -> ZString {
    ZString::self_fmt(Fmt::custom_regex("hex", regexes::HEX.clone()))
}
/// `z.hash(alg, { enc })`: `alg` is `md5`, `sha1`, `sha256`, `sha384` or `sha512`;
/// `enc` is `hex` (default), `base64` or `base64url`.
pub fn hash(alg: &str, enc: Option<&str>) -> ZString {
    let enc = enc.unwrap_or("hex");
    let format = format!("{alg}_{enc}");
    let re = match format.as_str() {
        "md5_hex" => &regexes::MD5_HEX,
        "md5_base64" => &regexes::MD5_BASE64,
        "md5_base64url" => &regexes::MD5_BASE64URL,
        "sha1_hex" => &regexes::SHA1_HEX,
        "sha1_base64" => &regexes::SHA1_BASE64,
        "sha1_base64url" => &regexes::SHA1_BASE64URL,
        "sha256_hex" => &regexes::SHA256_HEX,
        "sha256_base64" => &regexes::SHA256_BASE64,
        "sha256_base64url" => &regexes::SHA256_BASE64URL,
        "sha384_hex" => &regexes::SHA384_HEX,
        "sha384_base64" => &regexes::SHA384_BASE64,
        "sha384_base64url" => &regexes::SHA384_BASE64URL,
        "sha512_hex" => &regexes::SHA512_HEX,
        "sha512_base64" => &regexes::SHA512_BASE64,
        "sha512_base64url" => &regexes::SHA512_BASE64URL,
        _ => panic!("Unrecognized hash format: {format}"),
    };
    ZString::self_fmt(Fmt::custom_regex(&format, (*re).clone()))
}
/// `z.stringFormat(name, regex)`.
pub fn string_format(name: &str, re: impl Into<JsRegex>) -> ZString {
    ZString::self_fmt(Fmt::custom_regex(name, re.into()))
}
/// `z.stringFormat(name, fn)`.
pub fn string_format_fn<F: Fn(&str) -> bool + Send + Sync + 'static>(name: &str, f: F) -> ZString {
    ZString::self_fmt(Fmt::custom_fn(name, f))
}

/// `z.iso.*`.
/// `z.TimePrecision`: the `precision` of `z.iso.datetime()` / `z.iso.time()`.
pub mod time_precision {
    /// `TimePrecision.Any`: any number of fractional digits (the default).
    pub const ANY: Option<i32> = None;
    /// `TimePrecision.Minute`: `HH:MM`, no seconds.
    pub const MINUTE: Option<i32> = Some(-1);
    /// `TimePrecision.Second`: no fractional seconds.
    pub const SECOND: Option<i32> = Some(0);
    /// `TimePrecision.Millisecond`: exactly 3 digits.
    pub const MILLISECOND: Option<i32> = Some(3);
    /// `TimePrecision.Microsecond`: exactly 6 digits.
    pub const MICROSECOND: Option<i32> = Some(6);
}

pub mod iso {
    use super::*;

    /// `z.iso.datetime()`.
    pub fn datetime() -> ZString {
        ZString::self_fmt(Fmt::datetime(IsoDatetime::default()))
    }
    /// `z.iso.datetime({ precision, offset, local })`.
    pub fn datetime_with(o: IsoDatetime) -> ZString {
        ZString::self_fmt(Fmt::datetime(o))
    }
    /// `z.iso.date()`.
    pub fn date() -> ZString {
        ZString::self_fmt(Fmt::date())
    }
    /// `z.iso.time()`.
    pub fn time() -> ZString {
        ZString::self_fmt(Fmt::time(None))
    }
    /// `z.iso.time({ precision })`.
    pub fn time_with(precision: i32) -> ZString {
        ZString::self_fmt(Fmt::time(Some(precision)))
    }
    /// `z.iso.duration()`.
    pub fn duration() -> ZString {
        ZString::self_fmt(Fmt::duration())
    }
}

/// `z.number()`.
pub fn number() -> ZNumber {
    ZNumber::new()
}
/// `z.int()` (safe integers).
pub fn int() -> ZNumber {
    ZNumber::self_format(NumKind::Int)
}
/// `z.int32()`.
pub fn int32() -> ZNumber {
    ZNumber::self_format(NumKind::Format(NumFormat::Int32))
}
/// `z.uint32()`.
pub fn uint32() -> ZNumber {
    ZNumber::self_format(NumKind::Format(NumFormat::Uint32))
}
/// `z.float32()`.
pub fn float32() -> ZNumber {
    ZNumber::self_format(NumKind::Format(NumFormat::Float32))
}
/// `z.float64()`.
pub fn float64() -> ZNumber {
    ZNumber::self_format(NumKind::Format(NumFormat::Float64))
}
/// `z.bigint()`.
pub fn bigint() -> ZBigInt {
    ZBigInt::new()
}
fn big_format(name: &'static str, lo: i128, hi: i128) -> ZBigInt {
    let mut b = ZBigInt::new();
    let mut c = Chk::new(BigKind::Format(name, lo, hi));
    c.is_self = true;
    b.checks.push(c);
    b
}
/// `z.int64()`.
pub fn int64() -> ZBigInt {
    big_format("int64", i64::MIN as i128, i64::MAX as i128)
}
/// `z.uint64()`.
pub fn uint64() -> ZBigInt {
    big_format("uint64", 0, u64::MAX as i128)
}
/// `z.boolean()`.
pub fn boolean() -> ZBoolean {
    ZBoolean::new()
}
/// `z.stringbool()`.
pub fn stringbool() -> ZStringBool {
    ZStringBool::new(StringBoolOptions::default())
}
/// `z.stringbool({ truthy, falsy, case })`.
pub fn stringbool_with(o: StringBoolOptions) -> ZStringBool {
    ZStringBool::new(o)
}
/// `z.date()`.
pub fn date() -> ZDate {
    ZDate::new()
}
/// `z.literal(value)` (a JSON array is a list of values: `z.literal([...])`).
pub fn literal(value: impl Into<Value>) -> ZLiteral {
    ZLiteral::new(value.into())
}
/// `z.literal(undefined)`.
pub fn literal_undefined() -> ZLiteral {
    ZLiteral::from_vals(vec![Val::Undef])
}
/// `z.enum([...])` (`enum` is a Rust keyword).
pub fn enum_<I, S>(values: I) -> ZEnum
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ZEnum::new(values.into_iter().map(Into::into).collect())
}
/// `z.enum({ key: value })` / `z.nativeEnum(TsEnum)`.
pub fn enum_entries<I, K>(entries: I) -> ZEnum
where
    I: IntoIterator<Item = (K, Value)>,
    K: Into<String>,
{
    ZEnum::from_entries(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
/// `z.nativeEnum(...)` for a Rust enum deriving `Deserialize` (unit variants).
pub fn native_enum<T: DeserializeOwned>() -> ZEnum {
    ZEnum::new(crate::validation::rules::enum_variants::<T>().iter().map(|s| s.to_string()).collect())
}
/// `z.any()`.
pub fn any() -> ZAny {
    ZAny
}
/// `z.unknown()`.
pub fn unknown() -> ZAny {
    ZAny
}
/// `z.null()`.
pub fn null() -> ZNull {
    ZNull::new()
}
/// `z.undefined()`.
pub fn undefined() -> ZUndefined {
    ZUndefined::new()
}
/// `z.void()`.
pub fn void() -> ZVoid {
    ZVoid::new()
}
/// `z.never()`.
pub fn never() -> ZNever {
    ZNever::new()
}
/// `z.nan()`.
pub fn nan() -> ZNaN {
    ZNaN::new()
}
/// `z.templateLiteral([...])`.
pub fn template_literal<I, P>(parts: I) -> ZTemplateLiteral
where
    I: IntoIterator<Item = P>,
    P: Into<TemplatePart>,
{
    ZTemplateLiteral::new(parts.into_iter().map(Into::into).collect())
}

/// `z.coerce.*`: converts the input with JavaScript semantics before validating (zod 4:
/// `String(x)`, `Number(x)`, `Boolean(x)`, `BigInt(x)`, `new Date(x)`).
pub mod coerce {
    use super::*;

    /// `z.coerce.string()`.
    pub fn string() -> ZString {
        ZString::new().coerce()
    }
    /// `z.coerce.number()`.
    pub fn number() -> ZNumber {
        ZNumber::new().coerce()
    }
    /// `z.coerce.boolean()`. zod 3 mode: `"false"`, `"0"`, `"no"`, `"off"` and `""` become `false`;
    /// zod 4: `Boolean(x)` (any non-empty string is `true`).
    pub fn boolean() -> ZBoolean {
        ZBoolean::new().coerce()
    }
    /// `z.coerce.bigint()`.
    pub fn bigint() -> ZBigInt {
        ZBigInt::new().coerce()
    }
    /// `z.coerce.date()`.
    pub fn date() -> ZDate {
        ZDate::new().coerce()
    }
}

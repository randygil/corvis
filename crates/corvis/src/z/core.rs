//! zod v4 runtime: values, payloads, raw issues, error maps, the `en` locale,
//! global configuration and the version switch.

use std::cell::Cell;
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::{Map, Value, json};

use super::error::{ZodIssue, ZodIssueCode};
use super::js;

// ---------------------------------------------------------------------------
// Version switch.
// ---------------------------------------------------------------------------

/// Which zod semantics `parse` follows (messages, issue shapes, check ordering, coercion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ZodVersion {
    /// zod 3.x (`"String must contain at least 3 character(s)"`).
    V3,
    /// zod 4.x (`"Too small: expected string to have >=3 characters"`). The default.
    #[default]
    V4,
}

/// Default of [`ZodConfig::max_depth`].
pub const DEFAULT_MAX_DEPTH: usize = 128;

/// Global configuration (`z.config({...})`).
#[derive(Clone)]
pub struct ZodConfig {
    /// Default version for `parse` calls that do not override it.
    pub version: ZodVersion,
    /// `customError`: global error map, consulted after per-schema and per-parse maps.
    pub custom_error: Option<ErrorMap>,
    /// `localeError`: replaces the built-in `en` messages.
    pub locale_error: Option<ErrorMap>,
    /// Accept ISO 8601 strings where `z.date()` expects a `Date` (JSON has no dates; Rust
    /// structs serialize `DateTime` as strings). Off by default for zod fidelity;
    /// `ZodSerializerInterceptor` turns it on for responses.
    pub date_strings: bool,
    /// Maximum nesting of recursive schemas (`z.lazy`, `z.json()`) during one parse. Default
    /// [`DEFAULT_MAX_DEPTH`] (128). A deeper input produces a `custom` issue ("Too deeply
    /// nested...") instead of recursing until the stack overflows. zod has no such limit: in
    /// Node.js the parse throws `RangeError: Maximum call stack size exceeded`; in Rust a stack
    /// overflow aborts the whole process, so the limit is mandatory. Overridable per parse with
    /// [`ParseOptions::max_depth`](super::ParseOptions::max_depth).
    pub max_depth: usize,
}

impl Default for ZodConfig {
    fn default() -> Self {
        ZodConfig {
            version: ZodVersion::default(),
            custom_error: None,
            locale_error: None,
            date_strings: false,
            max_depth: DEFAULT_MAX_DEPTH,
        }
    }
}

static CONFIG: LazyLock<RwLock<ZodConfig>> = LazyLock::new(|| RwLock::new(ZodConfig::default()));

thread_local! {
    static THREAD_VERSION: Cell<Option<ZodVersion>> = const { Cell::new(None) };
}

/// `z.config({ ... })`: replaces the global configuration.
pub fn config(cfg: ZodConfig) {
    *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = cfg;
}

/// Updates the global configuration in place.
pub fn configure(f: impl FnOnce(&mut ZodConfig)) {
    f(&mut CONFIG.write().unwrap_or_else(|e| e.into_inner()));
}

/// Current global configuration.
pub fn get_config() -> ZodConfig {
    CONFIG.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// [`ZodConfig::max_depth`] of the global configuration.
pub(crate) fn global_max_depth() -> usize {
    CONFIG.read().unwrap_or_else(|e| e.into_inner()).max_depth
}

/// Message of the issue reported when [`ZodConfig::max_depth`] is exceeded.
pub(crate) fn too_deep_message(max: usize) -> String {
    format!("Too deeply nested: the input exceeds the maximum depth of {max}")
}

/// The `custom` issue reported when [`ZodConfig::max_depth`] is exceeded. Its `input` is left
/// `undefined` on purpose: the offending value is, by definition, a deep subtree, and copying it
/// into the issue (or into an error map) is what the limit is there to avoid.
pub(crate) fn too_deep_issue(max: usize) -> RawIssue {
    let mut iss = RawIssue::new(
        vec![("code", Value::String("custom".into())), ("message", Value::String(too_deep_message(max)))],
        Val::Undef,
        None,
        None,
    );
    iss.too_deep = true;
    iss
}

/// Sets the global default version (`z::set_version(ZodVersion::V3)` for zod 3 behaviour).
pub fn set_version(version: ZodVersion) {
    configure(|c| c.version = version);
}

/// Effective version on this thread (thread override, else global).
pub fn version() -> ZodVersion {
    THREAD_VERSION.with(|v| v.get()).unwrap_or_else(|| CONFIG.read().unwrap_or_else(|e| e.into_inner()).version)
}

/// Runs `f` with `version` as the default on the current thread (handy in tests).
pub fn with_version<R>(version: ZodVersion, f: impl FnOnce() -> R) -> R {
    let prev = THREAD_VERSION.with(|v| v.replace(Some(version)));
    struct Reset(Option<ZodVersion>);
    impl Drop for Reset {
        fn drop(&mut self) {
            THREAD_VERSION.with(|v| v.set(self.0));
        }
    }
    let _reset = Reset(prev);
    f()
}

// ---------------------------------------------------------------------------
// Values.
// ---------------------------------------------------------------------------

/// A JavaScript value as seen by zod: JSON plus `undefined`, non-finite numbers,
/// `Date` and `BigInt` (they appear through coercion and transforms).
#[derive(Debug, Clone, PartialEq)]
pub enum Val {
    Undef,
    Json(Value),
    /// `NaN`, `Infinity` or `-Infinity` (finite numbers are `Json`).
    Num(f64),
    /// A `Date` (time value in ms; NaN = `Invalid Date`).
    Date(f64),
    BigInt(i128),
}

impl Val {
    pub fn from_opt(v: Option<&Value>) -> Val {
        match v {
            Some(v) => Val::Json(v.clone()),
            None => Val::Undef,
        }
    }

    pub fn from_output(v: Option<Value>) -> Val {
        match v {
            Some(v) => Val::Json(v),
            None => Val::Undef,
        }
    }

    pub fn number(n: f64) -> Val {
        if n.is_finite() { Val::Json(js::num_json(n)) } else { Val::Num(n) }
    }

    pub fn is_undef(&self) -> bool {
        matches!(self, Val::Undef)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Val::Json(Value::Null))
    }

    pub fn json(&self) -> Option<&Value> {
        match self {
            Val::Json(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Json(Value::String(s)) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Val::Json(Value::Number(n)) => Some(js::json_f64(n)),
            Val::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// `value.length` for strings and arrays.
    pub fn length(&self) -> Option<usize> {
        match self {
            Val::Json(Value::String(s)) => Some(js::js_len(s)),
            Val::Json(Value::Array(a)) => Some(a.len()),
            _ => None,
        }
    }

    /// `JSON.stringify` view of the value (`None` = undefined / dropped).
    pub fn to_json(&self) -> Option<Value> {
        match self {
            Val::Undef => None,
            Val::Json(v) => Some(v.clone()),
            Val::Num(_) => Some(Value::Null),
            Val::Date(t) => Some(js::date_to_iso(*t).map(Value::String).unwrap_or(Value::Null)),
            Val::BigInt(b) => Some(if let Ok(i) = i64::try_from(*b) {
                json!(i)
            } else if let Ok(u) = u64::try_from(*b) {
                json!(u)
            } else {
                Value::String(b.to_string())
            }),
        }
    }

    /// Like [`Val::to_json`] but `undefined` becomes `null` (array slots).
    pub fn to_json_or_null(&self) -> Value {
        self.to_json().unwrap_or(Value::Null)
    }

    /// Like [`Val::to_json`], without copying a JSON value.
    pub fn into_json(self) -> Option<Value> {
        match self {
            Val::Json(v) => Some(v),
            other => other.to_json(),
        }
    }

    /// Like [`Val::to_json_or_null`], without copying a JSON value.
    pub fn into_json_or_null(self) -> Value {
        self.into_json().unwrap_or(Value::Null)
    }

    /// `util.parsedType(value)`.
    pub fn parsed_type(&self) -> &'static str {
        match self {
            Val::Undef => "undefined",
            Val::Json(Value::Null) => "null",
            Val::Json(Value::Bool(_)) => "boolean",
            Val::Json(Value::Number(_)) => "number",
            Val::Json(Value::String(_)) => "string",
            Val::Json(Value::Array(_)) => "array",
            Val::Json(Value::Object(_)) => "object",
            Val::Num(n) if n.is_nan() => "nan",
            Val::Num(_) => "number",
            Val::Date(_) => "Date",
            Val::BigInt(_) => "bigint",
        }
    }

    /// `SameValueZero` (used by `Set.has`, literal/enum matching, discriminators).
    pub fn same_value_zero(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Json(Value::Number(a)), Val::Json(Value::Number(b))) => js::json_f64(a) == js::json_f64(b),
            (Val::Json(a @ (Value::Null | Value::Bool(_) | Value::String(_))), Val::Json(b)) => a == b,
            (Val::Undef, Val::Undef) => true,
            (Val::Num(a), Val::Num(b)) => a == b || (a.is_nan() && b.is_nan()),
            (Val::BigInt(a), Val::BigInt(b)) => a == b,
            _ => false,
        }
    }

    /// `stringifyPrimitive` for messages.
    pub fn stringify_primitive(&self) -> String {
        match self {
            Val::Undef => "undefined".into(),
            Val::Json(Value::String(s)) => format!("\"{s}\""),
            Val::Json(Value::Number(n)) => js::num_to_string(js::json_f64(n)),
            Val::Json(Value::Null) => "null".into(),
            Val::Json(Value::Bool(b)) => b.to_string(),
            Val::Json(v) => js::to_js_string(v),
            Val::Num(n) => js::num_to_string(*n),
            Val::BigInt(b) => format!("{b}n"),
            Val::Date(t) => js::date_to_iso(*t).unwrap_or_else(|| "Invalid Date".into()),
        }
    }
}

impl From<Value> for Val {
    fn from(v: Value) -> Self {
        Val::Json(v)
    }
}

pub(crate) fn stringify_primitive_json(v: &Value) -> String {
    Val::Json(v.clone()).stringify_primitive()
}

// ---------------------------------------------------------------------------
// Issues.
// ---------------------------------------------------------------------------

/// Error map: returns a message for an issue, or `None` to fall through to the next map
/// (per-schema `error` → per-parse `error` → `customError` → locale).
pub type ErrorMap = Arc<dyn Fn(&IssueInfo) -> Option<String> + Send + Sync>;

/// The issue being reported, as handed to error maps (`iss` in `{ error: (iss) => ... }`).
pub struct IssueInfo<'a> {
    pub code: ZodIssueCode,
    /// Issue properties (`expected`, `minimum`, `origin`, `format`, ...).
    pub fields: &'a Map<String, Value>,
    pub path: &'a [Value],
    /// `iss.input` (`None` = undefined).
    pub input: Option<Value>,
    pub(crate) input_val: &'a Val,
}

impl IssueInfo<'_> {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }
    /// `iss.input === undefined`.
    pub fn input_is_undefined(&self) -> bool {
        self.input_val.is_undef()
    }
    /// `util.parsedType(iss.input)` (`"string"`, `"undefined"`, `"NaN"`-style `"nan"`, ...).
    pub fn received_type(&self) -> &'static str {
        self.input_val.parsed_type()
    }
}

/// The `error` param: a fixed message or an error map.
#[derive(Clone)]
pub enum ErrorParam {
    Message(String),
    Map(ErrorMap),
}

impl ErrorParam {
    pub(crate) fn into_map(self) -> ErrorMap {
        match self {
            ErrorParam::Message(m) => Arc::new(move |_| Some(m.clone())),
            ErrorParam::Map(f) => f,
        }
    }
    pub(crate) fn message(&self) -> Option<String> {
        match self {
            ErrorParam::Message(m) => Some(m.clone()),
            ErrorParam::Map(_) => None,
        }
    }
}

impl From<&str> for ErrorParam {
    fn from(s: &str) -> Self {
        ErrorParam::Message(s.to_string())
    }
}
impl From<String> for ErrorParam {
    fn from(s: String) -> Self {
        ErrorParam::Message(s)
    }
}
impl From<&String> for ErrorParam {
    fn from(s: &String) -> Self {
        ErrorParam::Message(s.clone())
    }
}
impl From<ErrorMap> for ErrorParam {
    fn from(f: ErrorMap) -> Self {
        ErrorParam::Map(f)
    }
}

/// `{ error: (iss) => ... }` as an [`ErrorParam`].
pub fn error_fn<F>(f: F) -> ErrorParam
where
    F: Fn(&IssueInfo) -> Option<String> + Send + Sync + 'static,
{
    ErrorParam::Map(Arc::new(f))
}

/// An issue before finalization (`$ZodRawIssue`).
#[derive(Clone)]
pub struct RawIssue {
    /// Properties in insertion order. Contains `"code"`, may contain `"message"` (explicit)
    /// and a `"path"` placeholder that marks the position of the path.
    pub(crate) fields: Map<String, Value>,
    pub(crate) path: Vec<Value>,
    pub(crate) input: Val,
    pub(crate) inst_error: Option<ErrorMap>,
    /// `continue`: `Some(true)` keeps running checks; `None`/`Some(false)` aborts.
    pub(crate) cont: Option<bool>,
    /// The issue reports that [`ZodConfig::max_depth`] was exceeded (unions surface it as is
    /// instead of wrapping it in `invalid_union`).
    pub(crate) too_deep: bool,
}

impl std::fmt::Debug for RawIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RawIssue({:?} at {:?})", self.fields, self.path)
    }
}

impl RawIssue {
    /// `fields` in the order zod pushes them (`code` included; `input`/`inst`/`continue` excluded).
    /// A `path` field (array) becomes the issue path, as in `ctx.issues.push({ ..., path })`.
    pub fn new(fields: Vec<(&str, Value)>, input: Val, inst_error: Option<ErrorMap>, cont: Option<bool>) -> Self {
        let mut path = Vec::new();
        let fields = fields
            .into_iter()
            .map(|(k, v)| match (k, v) {
                ("path", Value::Array(a)) => {
                    path = a;
                    (k.to_string(), Value::Null)
                }
                (k, v) => (k.to_string(), v),
            })
            .collect();
        RawIssue { fields, path, input, inst_error, cont, too_deep: false }
    }

    pub fn code(&self) -> ZodIssueCode {
        self.fields.get("code").and_then(Value::as_str).map(ZodIssueCode::parse).unwrap_or(ZodIssueCode::Custom)
    }

    pub(crate) fn with_path(mut self, path: Vec<Value>) -> Self {
        self.fields.insert("path".into(), Value::Null);
        self.path = path;
        self
    }

    /// `util.prefixIssues`: `iss.path ??= []; iss.path.unshift(segment)`.
    pub(crate) fn prefix(&mut self, seg: Value) {
        if !self.fields.contains_key("path") {
            self.fields.insert("path".into(), Value::Null);
        }
        self.path.insert(0, seg);
    }
}

/// `throw new $ZodEncodeError(name)`: zod throws (it is not a validation issue), so this panics.
/// [`ZodValidationPipe`](super::ZodValidationPipe) and the serializer turn it into their exceptions.
pub(crate) fn throw_encode_error(name: &str) -> ! {
    panic!("Encountered unidirectional transform during encode: {name}")
}

pub(crate) fn prefix_all(issues: Vec<RawIssue>, seg: impl Into<Value>) -> Vec<RawIssue> {
    let seg = seg.into();
    issues
        .into_iter()
        .map(|mut i| {
            i.prefix(seg.clone());
            i
        })
        .collect()
}

/// Parse direction (`decode` = forward, `encode` = backward).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    #[default]
    Forward,
    Backward,
}

/// Per-parse context (`ParseContext` + internals).
#[derive(Clone)]
pub struct RunCtx {
    pub direction: Direction,
    pub(crate) skip_checks: bool,
    /// Per-parse `error` map (`schema.parse(data, { error })`).
    pub error: Option<ErrorMap>,
    /// `reportInput`: include `input` in issues.
    pub report_input: bool,
    /// See [`ZodConfig::date_strings`].
    pub date_strings: bool,
    pub(crate) config: Arc<ZodConfigSnapshot>,
    /// Current nesting of recursive schemas (see [`ZodConfig::max_depth`]).
    pub(crate) depth: Cell<usize>,
    pub(crate) max_depth: usize,
}

impl Default for RunCtx {
    fn default() -> Self {
        RunCtx::new(&super::ParseOptions::default())
    }
}

/// Restores [`RunCtx::depth`] when a recursive schema returns (or unwinds).
pub(crate) struct DepthGuard<'a>(&'a Cell<usize>);

impl Drop for DepthGuard<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

#[derive(Default)]
pub(crate) struct ZodConfigSnapshot {
    pub custom_error: Option<ErrorMap>,
    pub locale_error: Option<ErrorMap>,
}

impl RunCtx {
    pub(crate) fn new(opts: &super::ParseOptions) -> Self {
        let cfg = get_config();
        RunCtx {
            direction: opts.direction,
            skip_checks: false,
            error: opts.error.clone(),
            report_input: opts.report_input,
            date_strings: opts.date_strings.unwrap_or(cfg.date_strings),
            config: Arc::new(ZodConfigSnapshot { custom_error: cfg.custom_error, locale_error: cfg.locale_error }),
            depth: Cell::new(0),
            max_depth: opts.max_depth.unwrap_or(cfg.max_depth),
        }
    }

    /// Enters one level of a recursive schema; `None` when [`ZodConfig::max_depth`] is reached.
    pub(crate) fn enter(&self) -> Option<DepthGuard<'_>> {
        let d = self.depth.get();
        if d >= self.max_depth {
            return None;
        }
        self.depth.set(d + 1);
        Some(DepthGuard(&self.depth))
    }

    pub(crate) fn backward(&self) -> bool {
        self.direction == Direction::Backward
    }

    /// `util.finalizeIssue`.
    pub fn finalize(&self, raw: &RawIssue) -> ZodIssue {
        let code = raw.code();
        let explicit = raw.fields.get("message").and_then(|m| m.as_str()).filter(|m| !m.is_empty()).map(str::to_string);
        let message = explicit.unwrap_or_else(|| {
            let info = IssueInfo {
                code,
                fields: &raw.fields,
                path: &raw.path,
                input: raw.input.to_json(),
                input_val: &raw.input,
            };
            raw.inst_error
                .as_ref()
                .and_then(|f| f(&info))
                .or_else(|| self.error.as_ref().and_then(|f| f(&info)))
                .or_else(|| self.config.custom_error.as_ref().and_then(|f| f(&info)))
                .or_else(|| self.config.locale_error.as_ref().and_then(|f| f(&info)))
                .unwrap_or_else(|| en_message(&info))
        });
        let mut layout: Vec<String> = raw.fields.keys().cloned().collect();
        if !raw.fields.contains_key("path") {
            layout.push("path".into());
        }
        if !raw.fields.contains_key("message") {
            layout.push("message".into());
        }
        let mut params = Map::new();
        for (k, v) in &raw.fields {
            if k != "code" && k != "path" && k != "message" {
                params.insert(k.clone(), v.clone());
            }
        }
        if self.report_input
            && let Some(input) = raw.input.to_json() {
                params.insert("input".into(), input);
                layout.push("input".into());
            }
        ZodIssue { code, path: raw.path.clone(), message, params, layout: Some(layout), v3_layout: None }
    }

    pub(crate) fn finalize_all(&self, raw: &[RawIssue]) -> Vec<ZodIssue> {
        raw.iter().map(|i| self.finalize(i)).collect()
    }
}

/// `ParsePayload`.
#[derive(Debug, Clone)]
pub struct Payload {
    pub value: Val,
    pub issues: Vec<RawIssue>,
    pub(crate) aborted: bool,
    pub(crate) fallback: bool,
}

impl Payload {
    pub fn new(value: Val) -> Self {
        Payload { value, issues: Vec::new(), aborted: false, fallback: false }
    }

    /// `util.aborted(x, startIndex)`.
    pub fn is_aborted(&self, start: usize) -> bool {
        self.aborted || self.issues.iter().skip(start).any(|i| i.cont != Some(true))
    }

    /// `util.explicitlyAborted(x, startIndex)`.
    pub fn explicitly_aborted(&self, start: usize) -> bool {
        self.aborted || self.issues.iter().skip(start).any(|i| i.cont == Some(false))
    }

    pub fn push(&mut self, issue: RawIssue) {
        self.issues.push(issue);
    }
}

// ---------------------------------------------------------------------------
// Checks.
// ---------------------------------------------------------------------------

pub type WhenFn = Arc<dyn Fn(&Payload) -> bool + Send + Sync>;

/// Shared check options: `error`, `abort`, `when`.
#[derive(Clone, Default)]
pub struct CheckOpts {
    pub(crate) error: Option<ErrorMap>,
    pub(crate) message: Option<String>,
    pub(crate) abort: bool,
    pub(crate) when: Option<WhenFn>,
}

impl CheckOpts {
    pub(crate) fn set_error(&mut self, e: ErrorParam) {
        self.message = e.message();
        self.error = Some(e.into_map());
    }
    pub(crate) fn cont(&self) -> Option<bool> {
        Some(!self.abort)
    }
}

/// Implements the `runChecks` skipping rules.
pub(crate) struct CheckRunner {
    aborted: bool,
}

impl CheckRunner {
    pub fn new(p: &Payload) -> Self {
        CheckRunner { aborted: p.is_aborted(0) }
    }

    /// Whether a check runs; `default_when` is the check's built-in `when` (length checks).
    pub fn should_run(&self, p: &Payload, when: Option<&WhenFn>, default_when: Option<fn(&Payload) -> bool>) -> bool {
        if let Some(w) = when {
            if p.explicitly_aborted(0) {
                return false;
            }
            return w(p);
        }
        if let Some(w) = default_when {
            if p.explicitly_aborted(0) {
                return false;
            }
            return w(p);
        }
        !self.aborted
    }

    pub fn after(&mut self, p: &Payload, before: usize) {
        if p.issues.len() != before && !self.aborted {
            self.aborted = p.is_aborted(before);
        }
    }
}

/// `when` of length checks: value is not nullish and has a `.length`.
pub(crate) fn has_length(p: &Payload) -> bool {
    p.value.length().is_some()
}

/// Runs `parse` then `checks` honouring direction (`encode` runs checks first on a canary).
pub(crate) fn run_parse_checks(
    p: Payload,
    ctx: &RunCtx,
    has_checks: bool,
    parse: impl Fn(Payload) -> Payload,
    checks: impl Fn(Payload) -> Payload,
) -> Payload {
    if !has_checks || ctx.skip_checks {
        return parse(p);
    }
    if ctx.backward() {
        let canary = parse(Payload::new(p.value.clone()));
        if canary.is_aborted(0) {
            let mut c = canary;
            c.aborted = true;
            return c;
        }
        return parse(checks(p));
    }
    checks(parse(p))
}

// ---------------------------------------------------------------------------
// The `en` locale.
// ---------------------------------------------------------------------------

fn sizing(origin: &str) -> Option<&'static str> {
    match origin {
        "string" => Some("characters"),
        "file" => Some("bytes"),
        "array" | "set" => Some("items"),
        "map" => Some("entries"),
        _ => None,
    }
}

fn format_noun(format: &str) -> Option<&'static str> {
    Some(match format {
        "regex" => "input",
        "email" => "email address",
        "url" => "URL",
        "emoji" => "emoji",
        "uuid" => "UUID",
        "uuidv4" => "UUIDv4",
        "uuidv6" => "UUIDv6",
        "nanoid" => "nanoid",
        "guid" => "GUID",
        "cuid" => "cuid",
        "cuid2" => "cuid2",
        "ulid" => "ULID",
        "xid" => "XID",
        "ksuid" => "KSUID",
        "datetime" => "ISO datetime",
        "date" => "ISO date",
        "time" => "ISO time",
        "duration" => "ISO duration",
        "ipv4" => "IPv4 address",
        "ipv6" => "IPv6 address",
        "mac" => "MAC address",
        "cidrv4" => "IPv4 range",
        "cidrv6" => "IPv6 range",
        "base64" => "base64-encoded string",
        "base64url" => "base64url-encoded string",
        "json_string" => "JSON string",
        "e164" => "E.164 number",
        "jwt" => "JWT",
        "template_literal" => "input",
        _ => return None,
    })
}

/// `String(x)` of a JSON value used inside template strings of messages.
fn tpl(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(v) => js::to_js_string(v),
    }
}

pub(crate) fn join_values(values: &[Value], sep: &str) -> String {
    values.iter().map(stringify_primitive_json).collect::<Vec<_>>().join(sep)
}

/// Messages of `zod/v4/locales/en.js`.
pub fn en_message(iss: &IssueInfo) -> String {
    let f = |k: &str| iss.fields.get(k);
    match iss.code {
        ZodIssueCode::InvalidType => {
            let expected = tpl(f("expected"));
            let expected = if expected == "nan" { "NaN".to_string() } else { expected };
            let received = match iss.input_val.parsed_type() {
                "nan" => "NaN",
                t => t,
            };
            format!("Invalid input: expected {expected}, received {received}")
        }
        ZodIssueCode::InvalidValue => {
            let values = f("values").and_then(Value::as_array).cloned().unwrap_or_default();
            if values.len() == 1 {
                format!("Invalid input: expected {}", stringify_primitive_json(&values[0]))
            } else {
                format!("Invalid option: expected one of {}", join_values(&values, "|"))
            }
        }
        ZodIssueCode::TooBig | ZodIssueCode::TooSmall => {
            let big = iss.code == ZodIssueCode::TooBig;
            let inclusive = f("inclusive").is_some_and(|v| v.as_bool() == Some(true));
            let adj = match (big, inclusive) {
                (true, true) => "<=",
                (true, false) => "<",
                (false, true) => ">=",
                (false, false) => ">",
            };
            let origin_v = f("origin");
            let origin = match origin_v {
                Some(Value::String(s)) => s.clone(),
                Some(v) => tpl(Some(v)),
                None if big => "value".into(),
                None => "undefined".into(),
            };
            let bound = tpl(f(if big { "maximum" } else { "minimum" }));
            let kind = if big { "Too big" } else { "Too small" };
            match origin_v.and_then(Value::as_str).and_then(sizing) {
                Some(unit) => format!("{kind}: expected {origin} to have {adj}{bound} {unit}"),
                None => format!("{kind}: expected {origin} to be {adj}{bound}"),
            }
        }
        ZodIssueCode::InvalidFormat => {
            let format = f("format").and_then(Value::as_str).unwrap_or("");
            match format {
                "starts_with" => format!("Invalid string: must start with \"{}\"", tpl(f("prefix"))),
                "ends_with" => format!("Invalid string: must end with \"{}\"", tpl(f("suffix"))),
                "includes" => format!("Invalid string: must include \"{}\"", tpl(f("includes"))),
                "regex" => format!("Invalid string: must match pattern {}", tpl(f("pattern"))),
                other => format!("Invalid {}", format_noun(other).map(str::to_string).unwrap_or_else(|| tpl(f("format")))),
            }
        }
        ZodIssueCode::NotMultipleOf => format!("Invalid number: must be a multiple of {}", tpl(f("divisor"))),
        ZodIssueCode::UnrecognizedKeys => {
            let keys = f("keys").and_then(Value::as_array).cloned().unwrap_or_default();
            format!("Unrecognized key{}: {}", if keys.len() > 1 { "s" } else { "" }, join_values(&keys, ", "))
        }
        ZodIssueCode::InvalidKey => format!("Invalid key in {}", tpl(f("origin"))),
        ZodIssueCode::InvalidUnion => match f("options").and_then(Value::as_array) {
            Some(opts) if !opts.is_empty() => format!(
                "Invalid discriminator value. Expected {}",
                opts.iter().map(|o| format!("'{}'", tpl(Some(o)))).collect::<Vec<_>>().join(" | ")
            ),
            _ => "Invalid input".into(),
        },
        ZodIssueCode::InvalidElement => format!("Invalid value in {}", tpl(f("origin"))),
        _ => "Invalid input".into(),
    }
}

/// Converts a finalized v3 issue into a raw v4 issue (adapter for custom v3 types).
pub(crate) fn raw_from_v3(issue: &ZodIssue) -> RawIssue {
    let mut fields = Map::new();
    fields.insert("code".into(), Value::String(issue.code.as_str().into()));
    for (k, v) in &issue.params {
        fields.insert(k.clone(), v.clone());
    }
    fields.insert("path".into(), Value::Null);
    fields.insert("message".into(), Value::String(issue.message.clone()));
    RawIssue { fields, path: issue.path.clone(), input: Val::Undef, inst_error: None, cont: None, too_deep: false }
}

//! `corvis::z` — schema declaration and validation with the API of [zod](https://zod.dev).
//!
//! ```ignore
//! use corvis::z::{self, ZodSchema};
//!
//! let user = z::object! {
//!     name: z::string().min(2),
//!     email: z::email(),
//!     age: z::int().nonnegative().optional(),
//!     role: z::enum_(["admin", "user"]).default("user"),
//!     tags: z::array(z::string()).max(5).default(json!([])),
//! };
//! let data = user.parse(&json!({ "name": "Ana", "email": "ana@corvis.dev" }))?;
//! let typed: CreateUserDto = user.parse_as(&body)?;          // z.infer<typeof user> via serde
//! match user.safe_parse(&input) { r if r.success => ..., r => r.error.unwrap().flatten() }
//! ```
//!
//! By default schemas follow **zod 4** semantics: the same issue codes, key order, messages
//! (`en` locale) and check ordering as zod 4.4. zod 3 behaviour is available globally with
//! [`set_version`]`(ZodVersion::V3)` or per parse with [`ParseOptions::version`].
//! Use [`ZodValidationPipe`] to validate handler arguments.

mod compound;
mod core;
mod error;
pub mod js;
mod json_schema;
pub mod locales;
mod pipe;
mod regex;
pub mod regexes;
mod types;
mod wrappers;

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

pub use self::core::{
    CheckOpts, Direction, ErrorMap, ErrorParam, IssueInfo, Payload, RawIssue, RunCtx, Val, WhenFn, ZodConfig,
    ZodVersion, config, configure, en_message, error_fn, get_config, set_version, version, with_version,
};
pub use self::regex::JsRegex;
pub use compound::*;
pub use error::{ZodError, ZodIssue, ZodIssueCode, flatten_error, format_error, prettify_error, to_dot_path, treeify_error};
pub use json_schema::{JsonSchemaGen, JsonSchemaOptions, Registry, global_registry, to_json_schema};
pub use pipe::{
    CreateValidationException, ZOD_SERIALIZER_DTO_OPTIONS, ZodDto, ZodGuard, ZodGuardSource, ZodSchemaDeclarationException, ZodSerializationException, ZodSerializerDto, ZodSerializerInterceptor,
    ZodValidationException, ZodValidationPipe, ZodValidationPipeOptions, create_zod_validation_pipe, validate, validate_opt,
};
pub use types::*;
pub use wrappers::*;

use self::core::raw_from_v3;

// ---------------------------------------------------------------------------
// Core trait and parse context.
// ---------------------------------------------------------------------------

/// Implemented by every schema node.
///
/// `check` implements zod 3 semantics: `input` is `None` when the value is absent (JavaScript
/// `undefined`); it returns `Ok(Some(v))` with the parsed value, `Ok(None)` for an absent output
/// and `Err(())` when issues were added to the context.
///
/// `run_v4` implements zod 4 semantics on a [`Payload`]. Custom types that only implement
/// `check` still work under v4 through a default adapter.
pub trait ZodType: Send + Sync + 'static {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()>;

    /// Short type name used in messages (`"string"`, `"object"`, ...).
    fn type_name(&self) -> String;

    /// `true` when absent values are accepted (`optional`, `default`, `any`, ...) under v3.
    fn accepts_undefined(&self) -> bool {
        false
    }

    /// For `.required()`: the inner schema when this is an `optional` wrapper.
    fn unwrap_optional(&self) -> Option<Schema> {
        None
    }

    /// Literal values this schema accepts (used by v3 `discriminated_union`).
    fn literal_values(&self) -> Option<Vec<Value>> {
        None
    }

    /// Object shape, when this is an object schema.
    fn object_shape(&self) -> Option<&ZObject> {
        None
    }

    // ----- zod 4 -----

    /// `_zod.run(payload, ctx)`.
    fn run_v4(&self, mut payload: Payload, _ctx: &RunCtx) -> Payload {
        let input = payload.value.to_json();
        let mut c = ParseCtx::new();
        match self.check(input.as_ref(), &mut c) {
            Ok(v) if c.issues.is_empty() => payload.value = Val::from_output(v),
            _ => {
                if c.issues.is_empty() {
                    c.add(ZodIssueCode::Custom, "Invalid input", Map::new());
                }
                payload.issues.extend(c.issues.iter().map(raw_from_v3));
            }
        }
        payload
    }

    /// `_zod.optin === "optional"`.
    fn optin(&self) -> bool {
        false
    }

    /// `_zod.optout === "optional"`.
    fn optout(&self) -> bool {
        false
    }

    /// `_zod.values`: the finite set of accepted values (literals, enums, ...).
    fn values(&self) -> Option<Vec<Val>> {
        None
    }

    /// `_zod.propValues` (discriminated unions).
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        None
    }

    /// `_zod.pattern`: regex source describing the string form (template literals).
    fn pattern(&self) -> Option<String> {
        None
    }

    /// `_zod.def.type` (`"string"`, `"object"`, `"pipe"`, ...).
    fn def_type(&self) -> &'static str {
        "custom"
    }

    /// Wrapped schema for wrappers (`optional`, `default`, `pipe` in, ...).
    fn inner_schema(&self) -> Option<Schema> {
        None
    }

    /// Fills the JSON Schema of this node (`processJSONSchema`).
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let _ = (json, path);
        jsg.unrepresentable("Custom types cannot be represented in JSON Schema")
    }

    /// Metadata attached with `.meta()` / `.describe()` on this node itself.
    fn metadata(&self) -> Option<Map<String, Value>> {
        None
    }

    /// `_zod.parent`: the schema this one was cloned from (refinements, `.meta()`, ...).
    fn parent(&self) -> Option<Schema> {
        None
    }

    /// Stable identity for JSON Schema cycle detection (defaults to the `Arc` address).
    fn identity(&self) -> Option<usize> {
        None
    }

    /// The regex sources of a string schema (`_zod.bag.patterns`).
    #[doc(hidden)]
    fn string_patterns(&self) -> Vec<String> {
        Vec::new()
    }

    /// `true` when this node transforms values (`isTransforming`).
    fn is_transforming(&self) -> bool {
        self.inner_schema().is_some_and(|s| s.0.is_transforming())
    }
}

/// zod 3 parse state: current path and collected issues.
#[derive(Debug, Default)]
pub struct ParseCtx {
    pub path: Vec<Value>,
    pub issues: Vec<ZodIssue>,
}

impl ParseCtx {
    pub fn new() -> Self {
        ParseCtx::default()
    }

    /// Adds an issue at the current path.
    pub fn add(&mut self, code: ZodIssueCode, message: impl Into<String>, params: Map<String, Value>) {
        self.issues.push(ZodIssue::new(code, self.path.clone(), message, params));
    }

    /// Runs `f` with `segment` appended to the path.
    pub fn at<R>(&mut self, segment: impl Into<Value>, f: impl FnOnce(&mut Self) -> R) -> R {
        self.path.push(segment.into());
        let r = f(self);
        self.path.pop();
        r
    }

    pub(crate) fn sub(&self) -> ParseCtx {
        ParseCtx { path: self.path.clone(), issues: Vec::new() }
    }
}

/// Runs a v4-only node under v3 (`check`): v4 semantics, issues converted to the v3 layout.
pub(crate) fn v3_via_v4(node: &dyn ZodType, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
    let rctx = RunCtx::new(&ParseOptions::default());
    let p = node.run_v4(Payload::new(Val::from_opt(input)), &rctx);
    if p.issues.is_empty() {
        return Ok(p.value.to_json());
    }
    for raw in &p.issues {
        let mut iss = rctx.finalize(raw);
        iss.layout = None;
        let mut path = ctx.path.clone();
        path.extend(iss.path);
        iss.path = path;
        ctx.issues.push(iss);
    }
    Err(())
}

// ---------------------------------------------------------------------------
// Refinement context (`ctx` of superRefine / transform).
// ---------------------------------------------------------------------------

enum RefTarget<'a> {
    V3(&'a mut ParseCtx),
    V4 { issues: &'a mut Vec<RawIssue>, input: Val, inst_error: Option<ErrorMap>, object_continue: Option<bool> },
}

/// Context handed to `super_refine` / `try_transform` callbacks (`ctx.addIssue(...)`).
pub struct RefinementCtx<'a> {
    target: RefTarget<'a>,
    failed: bool,
}

/// `ctx.addIssue({...})` builder; keys keep the order in which they are set, like a JS object literal.
///
/// ```ignore
/// ctx.add(z::issue().code(ZodIssueCode::Custom).message("Passwords don't match").path(["confirm"]));
/// ```
#[derive(Debug, Clone, Default)]
pub struct IssueBuilder {
    fields: Vec<(String, Value)>,
    path: Vec<Value>,
    fatal: bool,
    cont: Option<bool>,
}

/// Starts an [`IssueBuilder`].
pub fn issue() -> IssueBuilder {
    IssueBuilder::default()
}

impl IssueBuilder {
    fn set(mut self, key: &str, v: Value) -> Self {
        match self.fields.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = v,
            None => self.fields.push((key.to_string(), v)),
        }
        self
    }
    pub fn code(self, code: ZodIssueCode) -> Self {
        self.set("code", Value::String(code.as_str().into()))
    }
    pub fn message(self, message: impl Into<String>) -> Self {
        self.set("message", Value::String(message.into()))
    }
    pub fn path<I, P>(mut self, path: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<Value>,
    {
        self.path = path.into_iter().map(Into::into).collect();
        self.set("path", Value::Null)
    }
    /// Any other property (`expected`, `minimum`, `params`, ...).
    pub fn param(self, key: &str, value: impl Into<Value>) -> Self {
        self.set(key, value.into())
    }
    /// `fatal: true`: stops subsequent checks (kept in the issue, as zod does).
    pub fn fatal(mut self) -> Self {
        self.fatal = true;
        self.set("fatal", Value::Bool(true))
    }
    /// `input`: the value reported with `reportInput` (defaults to the checked value).
    pub fn input(self, value: impl Into<Value>) -> Self {
        self.set("input", value.into())
    }
    /// `continue: bool`.
    pub fn continue_(mut self, cont: bool) -> Self {
        self.cont = Some(cont);
        self
    }
}

impl RefinementCtx<'_> {
    pub(crate) fn v3(ctx: &mut ParseCtx) -> RefinementCtx<'_> {
        RefinementCtx { target: RefTarget::V3(ctx), failed: false }
    }

    pub(crate) fn v4<'a>(
        issues: &'a mut Vec<RawIssue>,
        input: Val,
        inst_error: Option<ErrorMap>,
        object_continue: Option<bool>,
    ) -> RefinementCtx<'a> {
        RefinementCtx { target: RefTarget::V4 { issues, input, inst_error, object_continue }, failed: false }
    }

    /// `ctx.addIssue(issue)`.
    pub fn add(&mut self, b: IssueBuilder) {
        self.failed = true;
        match &mut self.target {
            RefTarget::V3(ctx) => {
                let code = b
                    .fields
                    .iter()
                    .find(|(k, _)| k == "code")
                    .and_then(|(_, v)| v.as_str())
                    .map(ZodIssueCode::parse)
                    .unwrap_or(ZodIssueCode::Custom);
                let message = b
                    .fields
                    .iter()
                    .find(|(k, _)| k == "message")
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("Invalid input")
                    .to_string();
                let params = b
                    .fields
                    .iter()
                    .filter(|(k, _)| !matches!(k.as_str(), "code" | "message" | "path"))
                    .cloned()
                    .collect();
                // `{ ...issueData, path, message }`.
                let mut layout: Vec<String> = b.fields.iter().map(|(k, _)| k.clone()).filter(|k| k != "path").collect();
                if !layout.iter().any(|k| k == "code") {
                    layout.insert(0, "code".into());
                }
                layout.push("path".into());
                if !layout.iter().any(|k| k == "message") {
                    layout.push("message".into());
                }
                let mut path = ctx.path.clone();
                path.extend(b.path);
                let mut issue = ZodIssue::new(code, path, message, params);
                issue.v3_layout = Some(layout);
                ctx.issues.push(issue);
            }
            RefTarget::V4 { issues, input, inst_error, object_continue } => {
                let mut fields: Map<String, Value> = b.fields.into_iter().collect();
                if !fields.contains_key("code") {
                    fields.insert("code".into(), Value::String("custom".into()));
                }
                // `_issue.input ??= payload.value`: `input` is not an issue field.
                let input = match fields.shift_remove("input") {
                    Some(v) => Val::Json(v),
                    None => input.clone(),
                };
                let cont = if b.fatal { Some(false) } else { b.cont.or(*object_continue) };
                issues.push(RawIssue { fields, path: b.path, input, inst_error: inst_error.clone(), cont });
            }
        }
    }

    /// `ctx.addIssue("message")` (string form).
    pub fn add_issue(&mut self, message: impl Into<String>) {
        let message = message.into();
        match &mut self.target {
            RefTarget::V3(ctx) => {
                self.failed = true;
                ctx.add(ZodIssueCode::Custom, message, Map::new());
            }
            RefTarget::V4 { issues, input, .. } => {
                self.failed = true;
                let mut fields = Map::new();
                fields.insert("message".into(), Value::String(message));
                fields.insert("code".into(), Value::String("custom".into()));
                issues.push(RawIssue { fields, path: Vec::new(), input: input.clone(), inst_error: None, cont: None });
            }
        }
    }

    /// `ctx.addIssue({ code: "custom", message, path: [...] })` relative to the current path.
    pub fn add_issue_at<I, P>(&mut self, path: I, message: impl Into<String>)
    where
        I: IntoIterator<Item = P>,
        P: Into<Value>,
    {
        self.add(issue().code(ZodIssueCode::Custom).message(message).path(path));
    }

    /// `ctx.addIssue({ code, message, ...params })`.
    pub fn add_issue_with(&mut self, code: ZodIssueCode, message: impl Into<String>, params: Map<String, Value>) {
        let mut b = issue().code(code).message(message);
        for (k, v) in params {
            b = b.param(&k, v);
        }
        self.add(b);
    }

    /// Current path (zod 3; empty under zod 4, where paths are relative).
    pub fn path(&self) -> &[Value] {
        match &self.target {
            RefTarget::V3(ctx) => &ctx.path,
            RefTarget::V4 { .. } => &[],
        }
    }

    /// Whether an issue was reported through this context.
    pub fn has_issues(&self) -> bool {
        self.failed
    }
}

// ---------------------------------------------------------------------------
// Schema handle.
// ---------------------------------------------------------------------------

/// Type-erased, cheaply clonable schema.
#[derive(Clone)]
pub struct Schema(pub(crate) Arc<dyn ZodType>);

impl Schema {
    pub fn new<T: ZodType>(t: T) -> Self {
        Schema(Arc::new(t))
    }

    /// Parses a value at the current position of `ctx` with zod 3 semantics (custom [`ZodType`]s).
    pub fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        self.0.check(input, ctx)
    }

    /// `_zod.run(payload, ctx)` (zod 4 semantics).
    pub fn run(&self, payload: Payload, ctx: &RunCtx) -> Payload {
        self.0.run_v4(payload, ctx)
    }

    pub fn type_name(&self) -> String {
        self.0.type_name()
    }

    pub fn accepts_undefined(&self) -> bool {
        self.0.accepts_undefined()
    }

    /// The object schema behind this handle, if any (for `pick`, `extend`, ...).
    pub fn as_object(&self) -> Option<&ZObject> {
        self.0.object_shape()
    }

    /// The node behind this handle.
    pub fn node(&self) -> &dyn ZodType {
        &*self.0
    }

    pub fn optin(&self) -> bool {
        self.0.optin()
    }

    pub fn optout(&self) -> bool {
        self.0.optout()
    }

    pub fn def_type(&self) -> &'static str {
        self.0.def_type()
    }

    pub(crate) fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }

    pub(crate) fn literal_values(&self) -> Option<Vec<Value>> {
        self.0.literal_values()
    }

    pub(crate) fn unwrap_optional(&self) -> Option<Schema> {
        self.0.unwrap_optional()
    }

    pub(crate) fn id(&self) -> usize {
        self.0.identity().unwrap_or(Arc::as_ptr(&self.0) as *const () as usize)
    }

    /// `schema.meta()`: metadata in the global registry (inherited from parents, without `id`).
    pub fn get_meta(&self) -> Option<Map<String, Value>> {
        global_registry().get(self)
    }

    /// `schema.description` (from `.describe()` / `.meta({ description })`).
    pub fn description(&self) -> Option<String> {
        global_registry().get(self).and_then(|m| m.get("description").and_then(Value::as_str).map(str::to_string))
    }
}

impl std::fmt::Debug for Schema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Schema<{}>", self.type_name())
    }
}

impl<T: ZodType> From<T> for Schema {
    fn from(t: T) -> Self {
        Schema::new(t)
    }
}

/// Result of [`ZodSchema::safe_parse`] (`{ success, data, error }`).
#[derive(Debug, Clone)]
pub struct SafeParseResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<ZodError>,
}

impl SafeParseResult {
    pub fn into_result(self) -> Result<Value, ZodError> {
        match (self.data, self.error) {
            (Some(d), None) => Ok(d),
            (_, Some(e)) => Err(e),
            (None, None) => Ok(Value::Null),
        }
    }
}

/// Per-parse options (`schema.parse(data, { error, reportInput })` plus the version override).
#[derive(Clone, Default)]
pub struct ParseOptions {
    /// Overrides the global / thread version for this parse.
    pub version: Option<ZodVersion>,
    /// Per-parse error map.
    pub error: Option<ErrorMap>,
    /// `reportInput: true`.
    pub report_input: bool,
    /// Overrides [`ZodConfig::date_strings`].
    pub date_strings: Option<bool>,
    /// `decode` (forward, default) or `encode` (backward).
    pub direction: Direction,
}

impl ParseOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn version(mut self, v: ZodVersion) -> Self {
        self.version = Some(v);
        self
    }
    pub fn v3() -> Self {
        Self::default().version(ZodVersion::V3)
    }
    pub fn v4() -> Self {
        Self::default().version(ZodVersion::V4)
    }
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
    pub fn report_input(mut self, yes: bool) -> Self {
        self.report_input = yes;
        self
    }
    pub fn date_strings(mut self, yes: bool) -> Self {
        self.date_strings = Some(yes);
        self
    }
    pub fn direction(mut self, d: Direction) -> Self {
        self.direction = d;
        self
    }
}

fn run_v3(schema: &Schema, value: Option<&Value>) -> Result<Value, ZodError> {
    let mut ctx = ParseCtx::new();
    match schema.check(value, &mut ctx) {
        Ok(v) if ctx.issues.is_empty() => Ok(v.unwrap_or(Value::Null)),
        _ => {
            if ctx.issues.is_empty() {
                ctx.add(ZodIssueCode::Custom, "Invalid input", Map::new());
            }
            Err(ZodError::new(ctx.issues))
        }
    }
}

/// Parses `value` (`None` = undefined) and returns the output as a [`Val`].
pub fn parse_val(schema: &Schema, value: Option<&Value>, opts: &ParseOptions) -> Result<Val, ZodError> {
    match opts.version.unwrap_or_else(version) {
        ZodVersion::V3 => run_v3(schema, value).map(Val::Json),
        ZodVersion::V4 => {
            let ctx = RunCtx::new(opts);
            let p = schema.run(Payload::new(Val::from_opt(value)), &ctx);
            if p.issues.is_empty() { Ok(p.value) } else { Err(ZodError::new(ctx.finalize_all(&p.issues))) }
        }
    }
}

fn run(schema: &Schema, value: Option<&Value>, opts: &ParseOptions) -> Result<Value, ZodError> {
    parse_val(schema, value, opts).map(|v| v.to_json().unwrap_or(Value::Null))
}

fn serde_issue(message: String) -> ZodError {
    let issue = match version() {
        ZodVersion::V3 => ZodIssue::new(ZodIssueCode::Custom, Vec::new(), message, Map::new()),
        ZodVersion::V4 => ZodIssue {
            code: ZodIssueCode::Custom,
            path: Vec::new(),
            message,
            params: Map::new(),
            layout: Some(vec!["code".into(), "path".into(), "message".into()]),
            v3_layout: None,
        },
    };
    ZodError::new(vec![issue])
}

/// Methods shared by every schema (`parse`, `optional`, `refine`, `transform`, ...).
pub trait ZodSchema: Clone + Send + Sync + 'static {
    fn to_schema(&self) -> Schema;

    // ----- parsing -----

    /// `schema.parse(data)`: the validated (stripped / transformed) value or a [`ZodError`].
    fn parse(&self, value: &Value) -> Result<Value, ZodError> {
        run(&self.to_schema(), Some(value), &ParseOptions::default())
    }

    /// `schema.parse(data, params)`.
    fn parse_with(&self, value: &Value, opts: &ParseOptions) -> Result<Value, ZodError> {
        run(&self.to_schema(), Some(value), opts)
    }

    /// Parses an optional value (`None` = undefined).
    fn parse_opt(&self, value: Option<&Value>, opts: &ParseOptions) -> Result<Value, ZodError> {
        run(&self.to_schema(), value, opts)
    }

    /// Parses and deserializes into `T` (the Rust side of `z.infer<typeof schema>`).
    fn parse_as<T: DeserializeOwned>(&self, value: &Value) -> Result<T, ZodError> {
        let v = self.parse(value)?;
        serde_json::from_value(v).map_err(|e| serde_issue(e.to_string()))
    }

    /// Parses a JSON string.
    fn parse_str(&self, json: &str) -> Result<Value, ZodError> {
        match serde_json::from_str::<Value>(json) {
            Ok(v) => self.parse(&v),
            Err(e) => Err(serde_issue(format!("Invalid JSON: {e}"))),
        }
    }

    /// Parses "undefined" (useful to obtain defaults).
    fn parse_undefined(&self) -> Result<Value, ZodError> {
        run(&self.to_schema(), None, &ParseOptions::default())
    }

    /// `schema.safeParse(data)`.
    fn safe_parse(&self, value: &Value) -> SafeParseResult {
        self.safe_parse_with(value, &ParseOptions::default())
    }

    /// `schema.safeParse(data, params)`.
    fn safe_parse_with(&self, value: &Value, opts: &ParseOptions) -> SafeParseResult {
        match self.parse_with(value, opts) {
            Ok(v) => SafeParseResult { success: true, data: Some(v), error: None },
            Err(e) => SafeParseResult { success: false, data: None, error: Some(e) },
        }
    }

    /// `schema.safeParse(data, params)` where `None` is `undefined` (a missing body, ...).
    fn safe_parse_opt(&self, value: Option<&Value>, opts: &ParseOptions) -> SafeParseResult {
        match self.parse_opt(value, opts) {
            Ok(v) => SafeParseResult { success: true, data: Some(v), error: None },
            Err(e) => SafeParseResult { success: false, data: None, error: Some(e) },
        }
    }

    /// `schema.decode(data)` (same as `parse`, typed input).
    fn decode(&self, value: &Value) -> Result<Value, ZodError> {
        self.parse(value)
    }

    /// `schema.encode(data)`: runs codecs backwards (output → input).
    fn encode(&self, value: &Value) -> Result<Value, ZodError> {
        self.parse_with(value, &ParseOptions::v4().direction(Direction::Backward))
    }

    /// `schema.safeDecode(data)`.
    fn safe_decode(&self, value: &Value) -> SafeParseResult {
        self.safe_parse(value)
    }

    /// `schema.safeEncode(data)`.
    fn safe_encode(&self, value: &Value) -> SafeParseResult {
        self.safe_parse_with(value, &ParseOptions::v4().direction(Direction::Backward))
    }

    fn is_valid(&self, value: &Value) -> bool {
        self.parse(value).is_ok()
    }

    /// `schema.isOptional()`.
    fn is_optional(&self) -> bool {
        self.parse_undefined().is_ok()
    }

    /// `schema.isNullable()`.
    fn is_nullable(&self) -> bool {
        self.parse(&Value::Null).is_ok()
    }

    /// `z.toJSONSchema(schema)` / `schema.toJSONSchema()`.
    fn to_json_schema(&self) -> Value {
        to_json_schema(&self.to_schema(), &JsonSchemaOptions::default()).unwrap_or(Value::Null)
    }

    // ----- modifiers -----

    /// `.optional()`: accepts a missing value.
    fn optional(&self) -> Schema {
        Schema::new(ZOptional(self.to_schema()))
    }

    /// `.exactOptional()`: the key may be absent, but `undefined` is not accepted as a value.
    fn exact_optional(&self) -> Schema {
        Schema::new(ZExactOptional(self.to_schema()))
    }

    /// `.nullable()`: accepts `null`.
    fn nullable(&self) -> Schema {
        Schema::new(ZNullable(self.to_schema()))
    }

    /// `.nullish()`: `.nullable().optional()`.
    fn nullish(&self) -> Schema {
        Schema::new(ZOptional(Schema::new(ZNullable(self.to_schema()))))
    }

    /// `.nonoptional()`.
    fn nonoptional(&self) -> Schema {
        Schema::new(ZNonOptional::new(self.to_schema()))
    }

    /// `.default(value)`: used when the value is missing (not validated, like zod 4).
    fn default(&self, value: impl Into<Value>) -> Schema {
        Schema::new(ZDefault(self.to_schema(), value.into()))
    }

    /// `.default(() => value)`.
    fn default_fn<F: Fn() -> Value + Send + Sync + 'static>(&self, f: F) -> Schema {
        Schema::new(ZDefaultFn::new(self.to_schema(), f, false))
    }

    /// `.prefault(value)`: the value is parsed when the input is missing.
    fn prefault(&self, value: impl Into<Value>) -> Schema {
        let v = value.into();
        Schema::new(ZDefaultFn::new(self.to_schema(), move || v.clone(), true))
    }

    /// `.catch(value)`: used when parsing fails.
    fn catch(&self, value: impl Into<Value>) -> Schema {
        Schema::new(ZCatch(self.to_schema(), value.into()))
    }

    /// `.catch((ctx) => value)` (`ctx.error`, `ctx.input`).
    fn catch_fn<F: Fn(&CatchCtx) -> Value + Send + Sync + 'static>(&self, f: F) -> Schema {
        Schema::new(ZCatchFn::new(self.to_schema(), f))
    }

    /// `.readonly()`.
    fn readonly(&self) -> Schema {
        Schema::new(ZReadonly(self.to_schema()))
    }

    /// `.brand()` (type-level only).
    fn brand(&self) -> Schema {
        self.to_schema()
    }

    /// `.array()`.
    fn array(&self) -> ZArray {
        ZArray::new(self.to_schema())
    }

    /// `.or(other)`: `z.union([this, other])`.
    fn or(&self, other: impl Into<Schema>) -> Schema {
        Schema::new(ZUnion(vec![self.to_schema(), other.into()]))
    }

    /// `.and(other)`: `z.intersection(this, other)`.
    fn and(&self, other: impl Into<Schema>) -> Schema {
        Schema::new(ZIntersection(self.to_schema(), other.into()))
    }

    /// `.refine(check, "message")` / `.refine(check, z::params().message(..).path([..]).abort())`
    /// / `.refine(check, ())`.
    fn refine<F>(&self, check: F, params: impl Into<RefineParams>) -> Schema
    where
        F: Fn(&Value) -> bool + Send + Sync + 'static,
    {
        Schema::new(ZRefine::new(self.to_schema(), Arc::new(check), params.into()))
    }

    /// `.refine(check, { message, path: [...] })`.
    fn refine_at<F, I, P>(&self, check: F, message: impl Into<String>, path: I) -> Schema
    where
        F: Fn(&Value) -> bool + Send + Sync + 'static,
        I: IntoIterator<Item = P>,
        P: Into<Value>,
    {
        Schema::new(ZRefine::new(self.to_schema(), Arc::new(check), RefineParams::from(message.into()).path(path)))
    }

    /// `.superRefine((val, ctx) => ctx.addIssue(...))`.
    fn super_refine<F>(&self, f: F) -> Schema
    where
        F: Fn(&Value, &mut RefinementCtx) + Send + Sync + 'static,
    {
        Schema::new(ZSuperRefine::new(self.to_schema(), Arc::new(f), RefineParams::default()))
    }

    /// `.check((ctx) => { ... })`: a raw check with access to the payload (zod 4).
    fn check_fn<F>(&self, f: F) -> Schema
    where
        F: Fn(&mut Payload) + Send + Sync + 'static,
    {
        Schema::new(ZCheckFn { inner: self.to_schema(), f: Arc::new(f) })
    }

    /// `.overwrite(fn)`: transforms the value without changing its type (runs as a check).
    fn overwrite<F>(&self, f: F) -> Schema
    where
        F: Fn(Value) -> Value + Send + Sync + 'static,
    {
        Schema::new(ZOverwrite { inner: self.to_schema(), f: Arc::new(f) })
    }

    /// `.transform(fn)`.
    fn transform<F>(&self, f: F) -> Schema
    where
        F: Fn(Value) -> Value + Send + Sync + 'static,
    {
        Schema::new(ZTransform::new(self.to_schema(), Arc::new(move |v, _| Some(f(v)))))
    }

    /// `.transform((val, ctx) => ...)` that may report issues (return `None` after `ctx.add_issue`).
    fn try_transform<F>(&self, f: F) -> Schema
    where
        F: Fn(Value, &mut RefinementCtx) -> Option<Value> + Send + Sync + 'static,
    {
        Schema::new(ZTransform::new(self.to_schema(), Arc::new(f)))
    }

    /// `.pipe(other)`: feeds the output of this schema into `other`.
    fn pipe(&self, other: impl Into<Schema>) -> Schema {
        Schema::new(ZPipe(self.to_schema(), other.into()))
    }

    /// `.describe(description)`.
    fn describe(&self, description: impl Into<String>) -> Schema {
        let mut m = Map::new();
        m.insert("description".into(), Value::String(description.into()));
        Schema::new(ZMeta::new(self.to_schema(), m))
    }

    /// `.meta({ ... })` (`id`, `title`, `description`, `examples`, ...).
    fn meta(&self, meta: Value) -> Schema {
        let m = match meta {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        Schema::new(ZMeta::new(self.to_schema(), m))
    }

    /// `.register(registry, meta)`.
    fn register(&self, registry: &Registry, meta: Value) -> Schema {
        let s = self.to_schema();
        registry.add(&s, meta);
        s
    }

    /// `.apply(fn)`.
    fn apply<R, F: FnOnce(Self) -> R>(self, f: F) -> R {
        f(self)
    }
}

impl<T: ZodType + Clone> ZodSchema for T {
    fn to_schema(&self) -> Schema {
        Schema::new(self.clone())
    }
}

impl ZodSchema for Schema {
    fn to_schema(&self) -> Schema {
        self.clone()
    }
}

/// `z.object({ key: schema, ... })`:
///
/// ```ignore
/// let schema = z::object! {
///     name: z::string(),
///     "first-name": z::string().optional(),   // string keys are allowed too
/// };
/// ```
#[doc(hidden)]
#[macro_export]
macro_rules! __z_object {
    () => { $crate::z::ZObject::new() };
    ($($key:tt : $schema:expr),+ $(,)?) => {
        $crate::z::ZObject::from_fields(::std::vec![
            $( ($crate::__z_key!($key).to_string(), $crate::z::Schema::from($schema)) ),+
        ])
    };
}

/// `z.strictObject({...})`.
#[doc(hidden)]
#[macro_export]
macro_rules! __z_strict_object {
    ($($t:tt)*) => { $crate::__z_object!($($t)*).strict() };
}

/// `z.looseObject({...})`.
#[doc(hidden)]
#[macro_export]
macro_rules! __z_loose_object {
    ($($t:tt)*) => { $crate::__z_object!($($t)*).loose() };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __z_key {
    ($k:ident) => {
        ::std::stringify!($k)
    };
    ($k:literal) => {
        $k
    };
}

pub use crate::__z_loose_object as loose_object;
pub use crate::__z_object as object;
pub use crate::__z_strict_object as strict_object;

/// Implements [`ZodDto`] for a type so `ZodValidationPipe` (global) validates it
/// (`createZodDto`):
///
/// ```ignore
/// #[derive(Deserialize)]
/// pub struct CreateCatDto { name: String, age: u32 }
/// zod_dto!(CreateCatDto, z::object! { name: z::string().min(1), age: z::int().nonnegative() });
/// zod_dto!(EventDto, schema(), codec = true);   // createZodDto(schema, { codec: true })
/// ```
#[macro_export]
macro_rules! zod_dto {
    ($ty:ty, $schema:expr $(,)?) => {
        impl $crate::z::ZodDto for $ty {
            fn schema() -> &'static $crate::z::Schema {
                static SCHEMA: ::std::sync::OnceLock<$crate::z::Schema> = ::std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| $crate::z::Schema::from($schema))
            }
        }
    };
    ($ty:ty, $schema:expr, codec = $codec:expr $(,)?) => {
        impl $crate::z::ZodDto for $ty {
            fn schema() -> &'static $crate::z::Schema {
                static SCHEMA: ::std::sync::OnceLock<$crate::z::Schema> = ::std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| $crate::z::Schema::from($schema))
            }
            fn codec() -> bool {
                $codec
            }
        }
    };
}

pub(crate) fn invalid_type(ctx: &mut ParseCtx, expected: &str, input: Option<&Value>, errors: &TypeErrors) {
    let received = received_type(input);
    let message = match input {
        None => errors.required.clone().unwrap_or_else(|| "Required".to_string()),
        Some(_) => errors.invalid_type.clone().unwrap_or_else(|| format!("Expected {expected}, received {received}")),
    };
    let mut params = Map::new();
    params.insert("expected".into(), json!(expected));
    params.insert("received".into(), json!(received));
    ctx.add(ZodIssueCode::InvalidType, message, params);
}

pub(crate) fn received_type(input: Option<&Value>) -> &'static str {
    match input {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// `{ required_error, invalid_type_error }` options of every primitive.
#[derive(Debug, Clone, Default)]
pub struct TypeErrors {
    pub required: Option<String>,
    pub invalid_type: Option<String>,
}

impl TypeErrors {
    /// Error map for zod 4: `error` param first, then the v3-style messages.
    pub(crate) fn v4_map(&self, error: &Option<ErrorMap>) -> Option<ErrorMap> {
        if self.required.is_none() && self.invalid_type.is_none() {
            return error.clone();
        }
        let te = self.clone();
        let error = error.clone();
        Some(Arc::new(move |iss: &IssueInfo| {
            if let Some(m) = error.as_ref().and_then(|f| f(iss)) {
                return Some(m);
            }
            if iss.code != ZodIssueCode::InvalidType {
                return None;
            }
            if iss.input_is_undefined() { te.required.clone().or_else(|| te.invalid_type.clone()) } else { te.invalid_type.clone() }
        }))
    }
}

#[cfg(test)]
mod tests;

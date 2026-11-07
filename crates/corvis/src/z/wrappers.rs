//! Wrapper and effect schemas: optional, nullable, default, prefault, catch, nonoptional,
//! readonly, refine, superRefine, check, overwrite, transform, pipe, preprocess, codec, lazy, meta.

use std::any::TypeId;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use serde_json::{Map, Value, json};

use super::core::{CheckRunner, ErrorMap, ErrorParam, Payload, RawIssue, RunCtx, Val, WhenFn};
use super::{
    JsonSchemaGen, ParseCtx, RefinementCtx, Schema, ZObject, ZodError, ZodIssueCode, ZodType, v3_via_v4,
};

fn null_or<'a>(v: &'a Option<Value>, null: &'a Value) -> &'a Value {
    v.as_ref().unwrap_or(null)
}

// ---------------------------------------------------------------------------
// optional / exactOptional / nullable / nonoptional
// ---------------------------------------------------------------------------

/// `.optional()`.
#[derive(Clone)]
pub struct ZOptional(pub Schema);
impl ZodType for ZOptional {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            None => Ok(None),
            Some(v) => self.0.check(Some(v), ctx),
        }
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn unwrap_optional(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        if self.0.optin() {
            let input_undef = p.value.is_undef();
            let r = self.0.run(p, ctx);
            if input_undef && (!r.issues.is_empty() || r.fallback) {
                return Payload::new(Val::Undef);
            }
            return r;
        }
        if p.value.is_undef() {
            return p;
        }
        self.0.run(p, ctx)
    }
    fn optin(&self) -> bool {
        true
    }
    fn optout(&self) -> bool {
        true
    }
    fn values(&self) -> Option<Vec<Val>> {
        let mut v = self.0.values()?;
        v.push(Val::Undef);
        Some(v)
    }
    fn pattern(&self) -> Option<String> {
        self.0.node().pattern().map(|p| format!("^({})?$", clean_regex(&p)))
    }
    fn def_type(&self) -> &'static str {
        "optional"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.0, path)
    }
}

pub(crate) fn clean_regex(source: &str) -> &str {
    let s = source.strip_prefix('^').unwrap_or(source);
    s.strip_suffix('$').unwrap_or(s)
}

/// `.exactOptional()`.
#[derive(Clone)]
pub struct ZExactOptional(pub Schema);
impl ZodType for ZExactOptional {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            None => Ok(None),
            Some(v) => self.0.check(Some(v), ctx),
        }
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn unwrap_optional(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        self.0.run(p, ctx)
    }
    fn optin(&self) -> bool {
        true
    }
    fn optout(&self) -> bool {
        true
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }
    fn pattern(&self) -> Option<String> {
        self.0.node().pattern()
    }
    fn def_type(&self) -> &'static str {
        "optional"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.0, path)
    }
}

/// `.nullable()`.
#[derive(Clone)]
pub struct ZNullable(pub Schema);
impl ZodType for ZNullable {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            Some(Value::Null) => Ok(Some(Value::Null)),
            other => self.0.check(other, ctx),
        }
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.0.accepts_undefined()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        if p.value.is_null() {
            return p;
        }
        self.0.run(p, ctx)
    }
    fn optin(&self) -> bool {
        self.0.optin()
    }
    fn optout(&self) -> bool {
        self.0.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        let mut v = self.0.values()?;
        v.push(Val::Json(Value::Null));
        Some(v)
    }
    fn pattern(&self) -> Option<String> {
        self.0.node().pattern().map(|p| format!("^({}|null)$", clean_regex(&p)))
    }
    fn def_type(&self) -> &'static str {
        "nullable"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let inner = jsg.process(&self.0, path)?;
        if jsg.target() == "openapi-3.0" {
            jsg.set_ref(&self.0);
            json.insert("nullable".into(), Value::Bool(true));
        } else {
            json.insert("anyOf".into(), json!([inner, { "type": "null" }]));
        }
        Ok(())
    }
}

/// `.nonoptional()`.
#[derive(Clone)]
pub struct ZNonOptional {
    pub(crate) inner: Schema,
    pub(crate) error: Option<ErrorMap>,
    /// zod 3 behaviour of `.required()` (the unwrapped schema).
    pub(crate) v3: Option<Schema>,
}
impl ZNonOptional {
    pub fn new(inner: Schema) -> Self {
        ZNonOptional { inner, error: None, v3: None }
    }
    /// `{ error }` param.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
}
impl ZodType for ZNonOptional {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match &self.v3 {
            Some(s) => s.check(input, ctx),
            None => v3_via_v4(self, input, ctx),
        }
    }
    fn accepts_undefined(&self) -> bool {
        self.v3.as_ref().is_some_and(|s| s.accepts_undefined())
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut r = self.inner.run(p, ctx);
        if r.issues.is_empty() && r.value.is_undef() {
            r.push(RawIssue::new(
                vec![("code", json!("invalid_type")), ("expected", json!("nonoptional"))],
                Val::Undef,
                self.error.clone(),
                None,
            ));
        }
        r
    }
    fn values(&self) -> Option<Vec<Val>> {
        Some(self.inner.values()?.into_iter().filter(|v| !v.is_undef()).collect())
    }
    fn def_type(&self) -> &'static str {
        "nonoptional"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.inner, path)
    }
}

// ---------------------------------------------------------------------------
// default / prefault / catch / readonly / success
// ---------------------------------------------------------------------------

/// `.default(value)`.
#[derive(Clone)]
pub struct ZDefault(pub Schema, pub Value);
impl ZodType for ZDefault {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            None => self.0.check(Some(&self.1), ctx),
            Some(v) => self.0.check(Some(v), ctx),
        }
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn unwrap_optional(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.backward() {
            return self.0.run(p, ctx);
        }
        if p.value.is_undef() {
            p.value = Val::Json(self.1.clone());
            return p;
        }
        let mut r = self.0.run(p, ctx);
        if r.value.is_undef() {
            r.value = Val::Json(self.1.clone());
        }
        r
    }
    fn optin(&self) -> bool {
        true
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }
    fn def_type(&self) -> &'static str {
        "default"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.0, path)?;
        json.insert("default".into(), self.1.clone());
        Ok(())
    }
}

type ValueFn = Arc<dyn Fn() -> Value + Send + Sync>;

/// `.default(() => value)` and `.prefault(value)`.
#[derive(Clone)]
pub struct ZDefaultFn {
    inner: Schema,
    f: ValueFn,
    prefault: bool,
}
impl ZDefaultFn {
    pub fn new<F: Fn() -> Value + Send + Sync + 'static>(inner: Schema, f: F, prefault: bool) -> Self {
        ZDefaultFn { inner, f: Arc::new(f), prefault }
    }
}
impl ZodType for ZDefaultFn {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        match input {
            None => self.inner.check(Some(&(self.f)()), ctx),
            Some(v) => self.inner.check(Some(v), ctx),
        }
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn unwrap_optional(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.backward() {
            return self.inner.run(p, ctx);
        }
        if self.prefault {
            if p.value.is_undef() {
                p.value = Val::Json((self.f)());
            }
            return self.inner.run(p, ctx);
        }
        if p.value.is_undef() {
            p.value = Val::Json((self.f)());
            return p;
        }
        let mut r = self.inner.run(p, ctx);
        if r.value.is_undef() {
            r.value = Val::Json((self.f)());
        }
        r
    }
    fn optin(&self) -> bool {
        true
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn def_type(&self) -> &'static str {
        if self.prefault { "prefault" } else { "default" }
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.inner, path)?;
        if !self.prefault {
            json.insert("default".into(), (self.f)());
        } else if jsg.io_input() {
            json.insert("_prefault".into(), (self.f)());
        }
        Ok(())
    }
}

/// `.catch(value)`.
#[derive(Clone)]
pub struct ZCatch(pub Schema, pub Value);
impl ZodType for ZCatch {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut sub = ctx.sub();
        match self.0.check(input, &mut sub) {
            Ok(v) if sub.issues.is_empty() => Ok(v),
            _ => Ok(Some(self.1.clone())),
        }
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let v = self.1.clone();
        catch_v4(&self.0, p, ctx, &move |_| v.clone())
    }
    fn optin(&self) -> bool {
        true
    }
    fn optout(&self) -> bool {
        self.0.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }
    fn def_type(&self) -> &'static str {
        "catch"
    }
    fn is_transforming(&self) -> bool {
        false
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.0, path)?;
        json.insert("default".into(), self.1.clone());
        Ok(())
    }
}

/// The `ctx` of `.catch((ctx) => ...)`.
pub struct CatchCtx {
    /// `ctx.error` (the issues of the failed parse).
    pub error: ZodError,
    /// `ctx.input` (`None` = undefined).
    pub input: Option<Value>,
}

type CatchFn = Arc<dyn Fn(&CatchCtx) -> Value + Send + Sync>;

fn catch_v4(inner: &Schema, p: Payload, ctx: &RunCtx, f: &dyn Fn(&CatchCtx) -> Value) -> Payload {
    if ctx.backward() {
        return inner.run(p, ctx);
    }
    let r = inner.run(p, ctx);
    if r.issues.is_empty() {
        return r;
    }
    let cctx = CatchCtx { error: ZodError::new(ctx.finalize_all(&r.issues)), input: r.value.to_json() };
    let mut out = Payload::new(Val::Json(f(&cctx)));
    out.fallback = true;
    out
}

/// `.catch((ctx) => value)`.
#[derive(Clone)]
pub struct ZCatchFn {
    inner: Schema,
    f: CatchFn,
}
impl ZCatchFn {
    pub fn new<F: Fn(&CatchCtx) -> Value + Send + Sync + 'static>(inner: Schema, f: F) -> Self {
        ZCatchFn { inner, f: Arc::new(f) }
    }
}
impl ZodType for ZCatchFn {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut sub = ctx.sub();
        match self.inner.check(input, &mut sub) {
            Ok(v) if sub.issues.is_empty() => Ok(v),
            _ => Ok(Some((self.f)(&CatchCtx { error: ZodError::new(sub.issues), input: input.cloned() }))),
        }
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        catch_v4(&self.inner, p, ctx, &*self.f)
    }
    fn optin(&self) -> bool {
        true
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn def_type(&self) -> &'static str {
        "catch"
    }
    fn is_transforming(&self) -> bool {
        false
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.inner, path)?;
        let v = (self.f)(&CatchCtx { error: ZodError::default(), input: None });
        json.insert("default".into(), v);
        Ok(())
    }
}

/// `.readonly()`.
#[derive(Clone)]
pub struct ZReadonly(pub Schema);
impl ZodType for ZReadonly {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        self.0.check(input, ctx)
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.0.accepts_undefined()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.0.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        self.0.run(p, ctx)
    }
    fn optin(&self) -> bool {
        self.0.optin()
    }
    fn optout(&self) -> bool {
        self.0.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.0.node().prop_values()
    }
    fn def_type(&self) -> &'static str {
        "readonly"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.0, path)?;
        json.insert("readOnly".into(), Value::Bool(true));
        Ok(())
    }
}

/// `z.success(schema)`: `true` when `schema` accepts the input.
#[derive(Clone)]
pub struct ZSuccess(pub Schema);
impl ZodType for ZSuccess {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut sub = ctx.sub();
        let ok = self.0.check(input, &mut sub).is_ok() && sub.issues.is_empty();
        Ok(Some(Value::Bool(ok)))
    }
    fn type_name(&self) -> String {
        "boolean".into()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        // zod runs the inner schema on the same payload: its issues stay.
        if ctx.backward() {
            super::core::throw_encode_error("ZodSuccess");
        }
        let mut out = self.0.run(p, ctx);
        out.value = Val::Json(Value::Bool(out.issues.is_empty()));
        out
    }
    fn def_type(&self) -> &'static str {
        "success"
    }
    fn is_transforming(&self) -> bool {
        false
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn json_schema(&self, _gen: &mut JsonSchemaGen, json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("boolean"));
        Ok(())
    }
}

/// `z.success(schema)`.
pub fn success(schema: impl Into<Schema>) -> Schema {
    Schema::new(ZSuccess(schema.into()))
}

// ---------------------------------------------------------------------------
// refine / superRefine / check / overwrite
// ---------------------------------------------------------------------------

type RefineFn = Arc<dyn Fn(&Value) -> bool + Send + Sync>;
type SuperRefineFn = Arc<dyn Fn(&Value, &mut RefinementCtx) + Send + Sync>;
type TransformFn = Arc<dyn Fn(Value, &mut RefinementCtx) -> Option<Value> + Send + Sync>;

/// Params of `.refine()` / `.superRefine()` / `z.custom()`:
/// `{ message | error, path, abort, when, params }`.
#[derive(Clone, Default)]
pub struct RefineParams {
    pub(crate) message: Option<String>,
    pub(crate) error: Option<ErrorMap>,
    pub(crate) path: Vec<Value>,
    pub(crate) abort: Option<bool>,
    pub(crate) when: Option<WhenFn>,
    pub(crate) params: Option<Map<String, Value>>,
}

/// Starts [`RefineParams`] (`z::params().message("..").path(["confirm"])`).
pub fn params() -> RefineParams {
    RefineParams::default()
}

impl RefineParams {
    pub fn new() -> Self {
        Self::default()
    }
    /// `message`.
    pub fn message(mut self, m: impl Into<String>) -> Self {
        let m = m.into();
        self.error = Some(Arc::new({
            let m = m.clone();
            move |_| Some(m.clone())
        }));
        self.message = Some(m);
        self
    }
    /// `error` (string or [`error_fn`](super::error_fn)).
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        let e = e.into();
        self.message = e.message();
        self.error = Some(e.into_map());
        self
    }
    /// `path: [...]` (relative to the refined value).
    pub fn path<I, P>(mut self, path: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<Value>,
    {
        self.path = path.into_iter().map(Into::into).collect();
        self
    }
    /// `abort: true`.
    pub fn abort(mut self) -> Self {
        self.abort = Some(true);
        self
    }
    /// `abort: false` (the default of `.refine`; `z.custom` aborts by default).
    pub fn no_abort(mut self) -> Self {
        self.abort = Some(false);
        self
    }
    /// `when: (payload) => boolean`.
    pub fn when<F: Fn(&Payload) -> bool + Send + Sync + 'static>(mut self, f: F) -> Self {
        self.when = Some(Arc::new(f));
        self
    }
    /// `params: {...}` (copied into the issue).
    pub fn params(mut self, p: Value) -> Self {
        self.params = p.as_object().cloned();
        self
    }
}

impl From<&str> for RefineParams {
    fn from(s: &str) -> Self {
        RefineParams::new().message(s)
    }
}
impl From<String> for RefineParams {
    fn from(s: String) -> Self {
        RefineParams::new().message(s)
    }
}
impl From<&String> for RefineParams {
    fn from(s: &String) -> Self {
        RefineParams::new().message(s.clone())
    }
}
impl From<()> for RefineParams {
    fn from(_: ()) -> Self {
        RefineParams::default()
    }
}
impl From<ErrorParam> for RefineParams {
    fn from(e: ErrorParam) -> Self {
        RefineParams::new().error(e)
    }
}

fn check_should_run(p: &Payload, when: Option<&WhenFn>) -> bool {
    CheckRunner::new(p).should_run(p, when, None)
}

/// `.refine(fn, params)`.
#[derive(Clone)]
pub struct ZRefine {
    inner: Schema,
    check: RefineFn,
    params: RefineParams,
}

impl ZRefine {
    pub(crate) fn new(inner: Schema, check: RefineFn, params: RefineParams) -> Self {
        ZRefine { inner, check, params }
    }
}

pub(crate) fn refine_issue(params: &RefineParams, input: Val) -> RawIssue {
    let mut fields = vec![("code", json!("custom")), ("path", Value::Null)];
    if let Some(p) = &params.params {
        fields.push(("params", Value::Object(p.clone())));
    }
    RawIssue::new(fields, input, params.error.clone(), Some(!params.abort.unwrap_or(false))).with_path(params.path.clone())
}

impl ZodType for ZRefine {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let out = self.inner.check(input, ctx)?;
        if let Some(v) = &out {
            if !(self.check)(v) {
                let base = ctx.path.len();
                ctx.path.extend(self.params.path.iter().cloned());
                ctx.add(ZodIssueCode::Custom, self.params.message.clone().unwrap_or_else(|| "Invalid input".into()), Map::new());
                ctx.path.truncate(base);
                return Err(());
            }
        }
        Ok(out)
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.inner.accepts_undefined()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut p = self.inner.run(p, ctx);
        if ctx.skip_checks || !check_should_run(&p, self.params.when.as_ref()) {
            return p;
        }
        let null = Value::Null;
        let v = p.value.to_json();
        if !(self.check)(null_or(&v, &null)) {
            let iss = refine_issue(&self.params, p.value.clone());
            p.push(iss);
        }
        p
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.inner.node().prop_values()
    }
    fn pattern(&self) -> Option<String> {
        self.inner.node().pattern()
    }
    fn def_type(&self) -> &'static str {
        self.inner.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit_parent(&self.inner, json, path)
    }
    fn parent(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
}

/// `.superRefine((val, ctx) => ...)`.
#[derive(Clone)]
pub struct ZSuperRefine {
    inner: Schema,
    f: SuperRefineFn,
    params: RefineParams,
}

impl ZSuperRefine {
    pub(crate) fn new(inner: Schema, f: SuperRefineFn, params: RefineParams) -> Self {
        ZSuperRefine { inner, f, params }
    }
}

impl ZodType for ZSuperRefine {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let out = self.inner.check(input, ctx)?;
        if let Some(v) = &out {
            let mut rc = RefinementCtx::v3(ctx);
            (self.f)(v, &mut rc);
            if rc.has_issues() {
                return Err(());
            }
        }
        Ok(out)
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.inner.accepts_undefined()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut p = self.inner.run(p, ctx);
        if ctx.skip_checks || !check_should_run(&p, self.params.when.as_ref()) {
            return p;
        }
        let null = Value::Null;
        let v = p.value.to_json();
        let input = p.value.clone();
        let mut rc = RefinementCtx::v4(&mut p.issues, input, self.params.error.clone(), Some(true));
        (self.f)(null_or(&v, &null), &mut rc);
        p
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.inner.node().prop_values()
    }
    fn def_type(&self) -> &'static str {
        self.inner.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit_parent(&self.inner, json, path)
    }
    fn parent(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
}

type CheckFn = Arc<dyn Fn(&mut Payload) + Send + Sync>;

/// `.check(fn)` with raw payload access.
#[derive(Clone)]
pub struct ZCheckFn {
    pub(crate) inner: Schema,
    pub(crate) f: CheckFn,
}
impl ZodType for ZCheckFn {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut p = self.inner.run(p, ctx);
        if ctx.skip_checks || !check_should_run(&p, None) {
            return p;
        }
        (self.f)(&mut p);
        p
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn def_type(&self) -> &'static str {
        self.inner.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit_parent(&self.inner, json, path)
    }
    fn parent(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
}

type OverwriteFn = Arc<dyn Fn(Value) -> Value + Send + Sync>;

/// `.overwrite(fn)`.
#[derive(Clone)]
pub struct ZOverwrite {
    pub(crate) inner: Schema,
    pub(crate) f: OverwriteFn,
}
impl ZodType for ZOverwrite {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        Ok(self.inner.check(input, ctx)?.map(|v| (self.f)(v)))
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.inner.accepts_undefined()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut p = self.inner.run(p, ctx);
        if ctx.skip_checks || !check_should_run(&p, None) {
            return p;
        }
        p.value = Val::Json((self.f)(p.value.to_json_or_null()));
        p
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn def_type(&self) -> &'static str {
        self.inner.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit_parent(&self.inner, json, path)
    }
    fn parent(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
}

// ---------------------------------------------------------------------------
// transform / pipe / preprocess / codec
// ---------------------------------------------------------------------------

/// `.transform(fn)` (`pipe(schema, z.transform(fn))`).
#[derive(Clone)]
pub struct ZTransform {
    inner: Schema,
    f: TransformFn,
}

impl ZTransform {
    pub(crate) fn new(inner: Schema, f: TransformFn) -> Self {
        ZTransform { inner, f }
    }
}

fn apply_transform(f: &TransformFn, mut p: Payload, ctx: &RunCtx) -> Payload {
    if ctx.backward() {
        super::core::throw_encode_error("ZodTransform");
    }
    let input = p.value.clone();
    let v = input.to_json().unwrap_or(Value::Null);
    let mut rc = RefinementCtx::v4(&mut p.issues, input, None, None);
    let out = f(v, &mut rc);
    p.value = Val::from_output(out);
    p.fallback = true;
    p
}

impl ZodType for ZTransform {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let out = self.inner.check(input, ctx)?;
        match out {
            None => Ok(None),
            Some(v) => {
                let mut rc = RefinementCtx::v3(ctx);
                let r = (self.f)(v, &mut rc);
                if rc.has_issues() {
                    return Err(());
                }
                Ok(Some(r.unwrap_or(Value::Null)))
            }
        }
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.inner.accepts_undefined()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let mut left = self.inner.run(p, ctx);
        if !left.issues.is_empty() {
            left.aborted = true;
            return left;
        }
        let next = Payload { value: left.value, issues: left.issues, aborted: false, fallback: left.fallback };
        apply_transform(&self.f, next, ctx)
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.inner.node().prop_values()
    }
    fn def_type(&self) -> &'static str {
        "pipe"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn is_transforming(&self) -> bool {
        true
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        if jsg.io_input() {
            jsg.inherit(&self.inner, path)
        } else {
            jsg.unrepresentable("Transforms cannot be represented in JSON Schema")
        }
    }
}

/// `z.transform(fn)`: a standalone transform (accepts any input, including undefined).
#[derive(Clone)]
pub struct ZTransformFn {
    f: TransformFn,
}
impl ZodType for ZTransformFn {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut rc = RefinementCtx::v3(ctx);
        let r = (self.f)(input.cloned().unwrap_or(Value::Null), &mut rc);
        if rc.has_issues() { Err(()) } else { Ok(r) }
    }
    fn type_name(&self) -> String {
        "transform".into()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        apply_transform(&self.f, p, ctx)
    }
    fn optin(&self) -> bool {
        true
    }
    fn def_type(&self) -> &'static str {
        "transform"
    }
    fn is_transforming(&self) -> bool {
        true
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, _path: &[Value]) -> Result<(), String> {
        jsg.unrepresentable("Transforms cannot be represented in JSON Schema")
    }
}

/// `z.transform(fn)`.
pub fn transform<F>(f: F) -> Schema
where
    F: Fn(Value, &mut RefinementCtx) -> Option<Value> + Send + Sync + 'static,
{
    Schema::new(ZTransformFn { f: Arc::new(f) })
}

/// `.pipe(other)`.
#[derive(Clone)]
pub struct ZPipe(pub Schema, pub Schema);

fn pipe_next(left: Payload, next: &Schema, ctx: &RunCtx) -> Payload {
    if !left.issues.is_empty() {
        let mut l = left;
        l.aborted = true;
        return l;
    }
    next.run(Payload { value: left.value, issues: left.issues, aborted: false, fallback: left.fallback }, ctx)
}

impl ZodType for ZPipe {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mid = self.0.check(input, ctx)?;
        self.1.check(mid.as_ref(), ctx)
    }
    fn type_name(&self) -> String {
        self.1.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.0.accepts_undefined()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.backward() {
            let right = self.1.run(p, ctx);
            return pipe_next(right, &self.0, ctx);
        }
        let left = self.0.run(p, ctx);
        pipe_next(left, &self.1, ctx)
    }
    fn optin(&self) -> bool {
        self.0.optin()
    }
    fn optout(&self) -> bool {
        self.1.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.0.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.0.node().prop_values()
    }
    fn def_type(&self) -> &'static str {
        "pipe"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.0.clone())
    }
    fn is_transforming(&self) -> bool {
        self.0.node().is_transforming() || self.1.node().is_transforming()
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let in_is_transform = self.0.def_type() == "transform";
        let inner = if jsg.io_input() { if in_is_transform { &self.1 } else { &self.0 } } else { &self.1 };
        jsg.inherit(inner, path)
    }
}

type PreprocessFn = Arc<dyn Fn(Option<&Value>) -> Option<Value> + Send + Sync>;

/// `z.preprocess(fn, schema)`.
#[derive(Clone)]
pub struct ZPreprocess {
    f: PreprocessFn,
    inner: Schema,
}
impl ZodType for ZPreprocess {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let v = (self.f)(input);
        self.inner.check(v.as_ref(), ctx)
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.backward() {
            let right = self.inner.run(p, ctx);
            if !right.issues.is_empty() {
                return right;
            }
            super::core::throw_encode_error("ZodTransform");
        }
        let input = p.value.to_json();
        p.value = Val::from_output((self.f)(input.as_ref()));
        p.fallback = true;
        self.inner.run(p, ctx)
    }
    fn optin(&self) -> bool {
        true
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn def_type(&self) -> &'static str {
        "pipe"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn is_transforming(&self) -> bool {
        true
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit(&self.inner, path)
    }
}

/// `z.preprocess(fn, schema)`.
pub fn preprocess<F>(f: F, schema: impl Into<Schema>) -> Schema
where
    F: Fn(Option<&Value>) -> Option<Value> + Send + Sync + 'static,
{
    Schema::new(ZPreprocess { f: Arc::new(f), inner: schema.into() })
}

type CodecFn = Arc<dyn Fn(Value, &mut RefinementCtx) -> Value + Send + Sync>;

/// `z.codec(input, output, { decode, encode })`.
#[derive(Clone)]
pub struct ZCodec {
    input: Schema,
    output: Schema,
    decode: CodecFn,
    encode: CodecFn,
}

impl ZCodec {
    /// `z.invertCodec(codec)`.
    pub fn invert(&self) -> ZCodec {
        ZCodec { input: self.output.clone(), output: self.input.clone(), decode: self.encode.clone(), encode: self.decode.clone() }
    }
}

/// `z.invertCodec(codec)`: swaps input/output and decode/encode.
pub fn invert_codec(codec: &ZCodec) -> ZCodec {
    codec.invert()
}

/// `z.codec(in, out, { decode, encode })`.
pub fn codec<D, E>(input: impl Into<Schema>, output: impl Into<Schema>, decode: D, encode: E) -> ZCodec
where
    D: Fn(Value, &mut RefinementCtx) -> Value + Send + Sync + 'static,
    E: Fn(Value, &mut RefinementCtx) -> Value + Send + Sync + 'static,
{
    ZCodec { input: input.into(), output: output.into(), decode: Arc::new(decode), encode: Arc::new(encode) }
}

impl ZodType for ZCodec {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        self.output.type_name()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        let (first, tx, next) =
            if ctx.backward() { (&self.output, &self.encode, &self.input) } else { (&self.input, &self.decode, &self.output) };
        let mut r = first.run(p, ctx);
        if !r.issues.is_empty() {
            r.aborted = true;
            return r;
        }
        let input = r.value.clone();
        let v = input.to_json_or_null();
        let out = {
            let mut rc = RefinementCtx::v4(&mut r.issues, input, None, None);
            tx(v, &mut rc)
        };
        if !r.issues.is_empty() {
            r.aborted = true;
            return r;
        }
        next.run(Payload { value: Val::Json(out), issues: r.issues, aborted: false, fallback: false }, ctx)
    }
    fn optin(&self) -> bool {
        self.input.optin()
    }
    fn optout(&self) -> bool {
        self.output.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.input.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.input.node().prop_values()
    }
    fn def_type(&self) -> &'static str {
        "pipe"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.input.clone())
    }
    fn is_transforming(&self) -> bool {
        true
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let inner = if jsg.io_input() { &self.input } else { &self.output };
        jsg.inherit(inner, path)
    }
}

// ---------------------------------------------------------------------------
// lazy / custom / meta
// ---------------------------------------------------------------------------

/// `z.lazy(() => schema)`.
#[derive(Clone)]
pub struct ZLazy {
    f: Arc<dyn Fn() -> Schema + Send + Sync>,
    cache: Arc<OnceLock<Schema>>,
    ident: Option<usize>,
}
impl ZLazy {
    fn get(&self) -> &Schema {
        self.cache.get_or_init(|| (self.f)())
    }
}
impl ZodType for ZLazy {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        self.get().check(input, ctx)
    }
    fn type_name(&self) -> String {
        "lazy".into()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.get().as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        self.get().run(p, ctx)
    }
    fn optin(&self) -> bool {
        self.get().optin()
    }
    fn optout(&self) -> bool {
        self.get().optout()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.get().node().prop_values()
    }
    fn pattern(&self) -> Option<String> {
        self.get().node().pattern()
    }
    fn def_type(&self) -> &'static str {
        "lazy"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.get().clone())
    }
    fn identity(&self) -> Option<usize> {
        self.ident
    }
    fn is_transforming(&self) -> bool {
        false
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, _json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let inner = self.get().clone();
        jsg.inherit(&inner, path)
    }
}

/// `z.lazy(() => schema)` for recursive schemas.
pub fn lazy<F: Fn() -> Schema + Send + Sync + 'static>(f: F) -> Schema {
    // Getters without captures (`z::lazy(category)`) share an identity, so recursive
    // schemas built by a function are detected as cycles by `to_json_schema`.
    let ident = if std::mem::size_of::<F>() == 0 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        TypeId::of::<F>().hash(&mut h);
        Some((h.finish() as usize) | 1)
    } else {
        None
    };
    Schema::new(ZLazy { f: Arc::new(f), cache: Arc::new(OnceLock::new()), ident })
}

/// `z.custom(check, params)`: accepts any value for which `check` returns `true`
/// (aborts by default, unlike `.refine`).
#[derive(Clone)]
pub struct ZCustom {
    check: RefineFn,
    params: RefineParams,
}
impl ZodType for ZCustom {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let null = Value::Null;
        if (self.check)(input.unwrap_or(&null)) {
            Ok(input.cloned())
        } else {
            let base = ctx.path.len();
            ctx.path.extend(self.params.path.iter().cloned());
            ctx.add(ZodIssueCode::Custom, self.params.message.clone().unwrap_or_else(|| "Invalid input".into()), Map::new());
            ctx.path.truncate(base);
            Err(())
        }
    }
    fn type_name(&self) -> String {
        "custom".into()
    }
    fn accepts_undefined(&self) -> bool {
        true
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if ctx.skip_checks || !check_should_run(&p, self.params.when.as_ref()) {
            return p;
        }
        let null = Value::Null;
        let v = p.value.to_json();
        if !(self.check)(null_or(&v, &null)) {
            let mut params = self.params.clone();
            params.abort = Some(params.abort.unwrap_or(true));
            let iss = refine_issue(&params, p.value.clone());
            p.push(iss);
        }
        p
    }
    fn def_type(&self) -> &'static str {
        "custom"
    }
}

/// `z.custom(check, params)`.
pub fn custom<F>(check: F, params: impl Into<RefineParams>) -> Schema
where
    F: Fn(&Value) -> bool + Send + Sync + 'static,
{
    Schema::new(ZCustom { check: Arc::new(check), params: params.into() })
}

/// `z.custom()` without a check (accepts anything).
pub fn custom_any() -> Schema {
    custom(|_| true, ())
}

/// `.describe()` / `.meta()`.
#[derive(Clone)]
pub struct ZMeta {
    inner: Schema,
    meta: Map<String, Value>,
}
impl ZMeta {
    pub(crate) fn new(inner: Schema, meta: Map<String, Value>) -> Self {
        // `.meta()` on a schema that already has metadata replaces it (a new registry entry);
        // `.describe()` keeps previous keys.
        ZMeta { inner, meta }
    }
    pub fn inner(&self) -> &Schema {
        &self.inner
    }
}
impl ZodType for ZMeta {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        self.inner.check(input, ctx)
    }
    fn type_name(&self) -> String {
        self.inner.type_name()
    }
    fn accepts_undefined(&self) -> bool {
        self.inner.accepts_undefined()
    }
    fn unwrap_optional(&self) -> Option<Schema> {
        self.inner.unwrap_optional()
    }
    fn literal_values(&self) -> Option<Vec<Value>> {
        self.inner.literal_values()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        self.inner.as_object()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        self.inner.run(p, ctx)
    }
    fn optin(&self) -> bool {
        self.inner.optin()
    }
    fn optout(&self) -> bool {
        self.inner.optout()
    }
    fn values(&self) -> Option<Vec<Val>> {
        self.inner.values()
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        self.inner.node().prop_values()
    }
    fn pattern(&self) -> Option<String> {
        self.inner.node().pattern()
    }
    fn def_type(&self) -> &'static str {
        self.inner.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
    fn metadata(&self) -> Option<Map<String, Value>> {
        Some(self.meta.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        jsg.inherit_parent(&self.inner, json, path)
    }
    fn parent(&self) -> Option<Schema> {
        Some(self.inner.clone())
    }
}

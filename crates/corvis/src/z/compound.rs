//! Composite schemas: array, tuple, record, object, union, xor, discriminatedUnion, intersection.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::core::{CheckOpts, CheckRunner, ErrorMap, ErrorParam, Payload, RawIssue, RunCtx, Val, WhenFn, has_length, prefix_all, run_parse_checks};
use super::types::{ZAny, ZEnum, ZNever, issue_params, js_string_of};
use super::{JsonSchemaGen, ParseCtx, Schema, TypeErrors, ZNonOptional, ZOptional, ZodIssueCode, ZodSchema, ZodType, invalid_type, js, regexes, v3_via_v4};

fn type_issue(expected: &str, input: &Val, error: Option<ErrorMap>) -> RawIssue {
    RawIssue::new(vec![("expected", json!(expected)), ("code", json!("invalid_type"))], input.clone(), error, None)
}

fn finalized_json(ctx: &RunCtx, issues: &[RawIssue]) -> Value {
    Value::Array(ctx.finalize_all(issues).iter().map(|i| i.to_json()).collect())
}

// ---------------------------------------------------------------------------
// array
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) enum ArrKind {
    Min(usize),
    Max(usize),
    Length(usize),
}

#[derive(Clone)]
struct ArrCheck {
    kind: ArrKind,
    message: Option<String>,
    opts: CheckOpts,
}

/// `z.array(schema)`.
#[derive(Clone)]
pub struct ZArray {
    item: Schema,
    checks: Vec<ArrCheck>,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

impl ZArray {
    pub fn new(item: Schema) -> Self {
        ZArray { item, checks: Vec::new(), errors: TypeErrors::default(), error: None }
    }
    /// `.element`.
    pub fn element(&self) -> &Schema {
        &self.item
    }
    /// `.unwrap()`.
    pub fn unwrap(&self) -> Schema {
        self.item.clone()
    }
    fn push(mut self, kind: ArrKind) -> Self {
        self.checks.push(ArrCheck { kind, message: None, opts: CheckOpts::default() });
        self
    }
    pub fn min(self, n: usize) -> Self {
        self.push(ArrKind::Min(n))
    }
    pub fn max(self, n: usize) -> Self {
        self.push(ArrKind::Max(n))
    }
    pub fn length(self, n: usize) -> Self {
        self.push(ArrKind::Length(n))
    }
    pub fn nonempty(self) -> Self {
        self.min(1)
    }
    /// `{ error }` / `message` for the last size check (schema-level without checks).
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        let e: ErrorParam = e.into();
        match self.checks.last_mut() {
            Some(c) => {
                c.message = e.message();
                c.opts.set_error(e);
            }
            None => {
                self.errors.invalid_type = e.message();
                self.error = Some(e.into_map());
            }
        }
        self
    }
    /// Custom message for the last size check.
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
    /// `{ when }` for the last check.
    pub fn when<F: Fn(&Payload) -> bool + Send + Sync + 'static>(mut self, f: F) -> Self {
        if let Some(c) = self.checks.last_mut() {
            c.opts.when = Some(Arc::new(f) as WhenFn);
        }
        self
    }
    pub fn required_error(mut self, message: impl Into<String>) -> Self {
        self.errors.required = Some(message.into());
        self
    }
    fn last_v3(&self, f: impl Fn(ArrKind) -> Option<usize>) -> Option<(usize, Option<String>)> {
        self.checks.iter().rev().find_map(|c| f(c.kind).map(|n| (n, c.message.clone())))
    }
    pub(crate) fn bag(&self) -> (Option<usize>, Option<usize>) {
        let (mut min, mut max): (Option<usize>, Option<usize>) = (None, None);
        for c in &self.checks {
            match c.kind {
                ArrKind::Min(n) => {
                    if min.is_none_or(|m| n > m) {
                        min = Some(n)
                    }
                }
                ArrKind::Max(n) => {
                    if max.is_none_or(|m| n < m) {
                        max = Some(n)
                    }
                }
                ArrKind::Length(n) => {
                    min = Some(n);
                    max = Some(n);
                }
            }
        }
        (min, max)
    }
}

pub(crate) fn length_check(kind: ArrKind, p: &mut Payload, err: Option<ErrorMap>, cont: Option<bool>) {
    let Some(len) = p.value.length() else { return };
    let origin = if p.value.as_str().is_some() { "string" } else { "array" };
    let input = p.value.clone();
    let fields: Option<Vec<(&str, Value)>> = match kind {
        ArrKind::Min(n) if len < n => {
            Some(vec![("origin", json!(origin)), ("code", json!("too_small")), ("minimum", json!(n)), ("inclusive", json!(true))])
        }
        ArrKind::Max(n) if len > n => {
            Some(vec![("origin", json!(origin)), ("code", json!("too_big")), ("maximum", json!(n)), ("inclusive", json!(true))])
        }
        ArrKind::Length(n) if len != n => {
            let (code, key) = if len > n { ("too_big", "maximum") } else { ("too_small", "minimum") };
            Some(vec![("origin", json!(origin)), ("code", json!(code)), (key, json!(n)), ("inclusive", json!(true)), ("exact", json!(true))])
        }
        _ => None,
    };
    if let Some(f) = fields {
        p.push(RawIssue::new(f, input, err, cont));
    }
}

impl ZodType for ZArray {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let Some(Value::Array(items)) = input else {
            invalid_type(ctx, "array", input, &self.errors);
            return Err(());
        };
        let mut ok = true;
        let len = items.len();
        if let Some((n, m)) = self.last_v3(|k| if let ArrKind::Length(n) = k { Some(n) } else { None })
            && len != n {
                ok = false;
                let (code, key) = if len < n { (ZodIssueCode::TooSmall, "minimum") } else { (ZodIssueCode::TooBig, "maximum") };
                ctx.add(
                    code,
                    m.unwrap_or_else(|| format!("Array must contain exactly {n} element(s)")),
                    issue_params(&[(key, json!(n)), ("type", json!("array")), ("inclusive", json!(true)), ("exact", json!(true))]),
                );
            }
        if let Some((n, m)) = self.last_v3(|k| if let ArrKind::Min(n) = k { Some(n) } else { None })
            && len < n {
                ok = false;
                ctx.add(
                    ZodIssueCode::TooSmall,
                    m.unwrap_or_else(|| format!("Array must contain at least {n} element(s)")),
                    issue_params(&[("minimum", json!(n)), ("type", json!("array")), ("inclusive", json!(true)), ("exact", json!(false))]),
                );
            }
        if let Some((n, m)) = self.last_v3(|k| if let ArrKind::Max(n) = k { Some(n) } else { None })
            && len > n {
                ok = false;
                ctx.add(
                    ZodIssueCode::TooBig,
                    m.unwrap_or_else(|| format!("Array must contain at most {n} element(s)")),
                    issue_params(&[("maximum", json!(n)), ("type", json!("array")), ("inclusive", json!(true)), ("exact", json!(false))]),
                );
            }
        let mut out = Vec::with_capacity(len);
        for (i, item) in items.iter().enumerate() {
            match ctx.at(i, |ctx| self.item.check(Some(item), ctx)) {
                Ok(v) => out.push(v.unwrap_or(Value::Null)),
                Err(()) => ok = false,
            }
        }
        if ok { Ok(Some(Value::Array(out))) } else { Err(()) }
    }
    fn type_name(&self) -> String {
        "array".into()
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        run_parse_checks(
            p,
            ctx,
            !self.checks.is_empty(),
            |mut p| {
                // The elements are moved into their payloads: copying the array at every level
                // makes recursive schemas quadratic.
                let items = match std::mem::replace(&mut p.value, Val::Undef) {
                    Val::Json(Value::Array(items)) => items,
                    other => {
                        p.value = other;
                        let input = p.value.clone();
                        p.push(type_issue("array", &input, self.errors.v4_map(&self.error)));
                        return p;
                    }
                };
                let mut out = Vec::with_capacity(items.len());
                for (i, item) in items.into_iter().enumerate() {
                    let r = self.item.run(Payload::new(Val::Json(item)), ctx);
                    if !r.issues.is_empty() {
                        p.issues.extend(prefix_all(r.issues, i));
                    }
                    out.push(r.value.into_json_or_null());
                }
                p.value = Val::Json(Value::Array(out));
                p
            },
            |mut p| {
                let mut runner = CheckRunner::new(&p);
                for c in &self.checks {
                    if !runner.should_run(&p, c.opts.when.as_ref(), Some(has_length)) {
                        continue;
                    }
                    let before = p.issues.len();
                    length_check(c.kind, &mut p, c.opts.error.clone(), c.opts.cont());
                    runner.after(&p, before);
                }
                p
            },
        )
    }
    fn def_type(&self) -> &'static str {
        "array"
    }
    fn inner_schema(&self) -> Option<Schema> {
        Some(self.item.clone())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let (min, max) = self.bag();
        if let Some(m) = min {
            json.insert("minItems".into(), json!(m));
        }
        if let Some(m) = max {
            json.insert("maxItems".into(), json!(m));
        }
        json.insert("type".into(), json!("array"));
        let mut p = path.to_vec();
        p.push(json!("items"));
        let items = jsg.process(&self.item, &p)?;
        json.insert("items".into(), items);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// tuple
// ---------------------------------------------------------------------------

/// `z.tuple([...])`.
#[derive(Clone)]
pub struct ZTuple {
    pub(crate) items: Vec<Schema>,
    pub(crate) rest: Option<Schema>,
    pub(crate) error: Option<ErrorMap>,
}

impl ZTuple {
    /// `.rest(schema)`.
    pub fn rest(mut self, schema: impl Into<Schema>) -> Self {
        self.rest = Some(schema.into());
        self
    }
    /// `{ error }`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
}

fn tuple_opt_start(items: &[Schema], optin: bool) -> usize {
    for i in (0..items.len()).rev() {
        let opt = if optin { items[i].optin() } else { items[i].optout() };
        if !opt {
            return i + 1;
        }
    }
    0
}

impl ZodType for ZTuple {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let Some(Value::Array(values)) = input else {
            invalid_type(ctx, "array", input, &TypeErrors::default());
            return Err(());
        };
        let n = self.items.len();
        if values.len() < n {
            ctx.add(
                ZodIssueCode::TooSmall,
                format!("Array must contain at least {n} element(s)"),
                issue_params(&[("minimum", json!(n)), ("inclusive", json!(true)), ("exact", json!(false)), ("type", json!("array"))]),
            );
            return Err(());
        }
        if self.rest.is_none() && values.len() > n {
            ctx.add(
                ZodIssueCode::TooBig,
                format!("Array must contain at most {n} element(s)"),
                issue_params(&[("maximum", json!(n)), ("inclusive", json!(true)), ("exact", json!(false)), ("type", json!("array"))]),
            );
            return Err(());
        }
        let mut ok = true;
        let mut out = Vec::with_capacity(values.len());
        for (i, v) in values.iter().enumerate() {
            let schema = self.items.get(i).or(self.rest.as_ref()).expect("checked above");
            match ctx.at(i, |ctx| schema.check(Some(v), ctx)) {
                Ok(v) => out.push(v.unwrap_or(Value::Null)),
                Err(()) => ok = false,
            }
        }
        if ok { Ok(Some(Value::Array(out))) } else { Err(()) }
    }
    fn type_name(&self) -> String {
        "array".into()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if !matches!(p.value, Val::Json(Value::Array(_))) {
            let input = p.value.clone();
            p.push(RawIssue::new(vec![("expected", json!("tuple")), ("code", json!("invalid_type"))], input, self.error.clone(), None));
            return p;
        }
        let optin_start = tuple_opt_start(&self.items, true);
        let optout_start = tuple_opt_start(&self.items, false);
        let len = p.value.length().unwrap_or(0);
        if self.rest.is_none() {
            if len < optin_start {
                let input_val = p.value.clone();
                p.push(RawIssue::new(
                    vec![("code", json!("too_small")), ("minimum", json!(optin_start)), ("inclusive", json!(true)), ("origin", json!("array"))],
                    input_val,
                    self.error.clone(),
                    None,
                ));
                return p;
            }
            if len > self.items.len() {
                let input_val = p.value.clone();
                p.push(RawIssue::new(
                    vec![("code", json!("too_big")), ("maximum", json!(self.items.len())), ("inclusive", json!(true)), ("origin", json!("array"))],
                    input_val,
                    self.error.clone(),
                    None,
                ));
            }
        }
        // Elements are moved into their payloads (no copy of the array per level).
        let Val::Json(Value::Array(mut input)) = std::mem::replace(&mut p.value, Val::Undef) else { unreachable!() };
        let results: Vec<Payload> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, s)| s.run(Payload::new(Val::from_output(input.get_mut(i).map(std::mem::take))), ctx))
            .collect();
        let mut out: Vec<Val> = Vec::new();
        let mut rest_vals: Vec<(usize, Val)> = Vec::new();
        if let Some(rest) = &self.rest {
            for (i, el) in input.iter_mut().enumerate().skip(self.items.len()) {
                let r = rest.run(Payload::new(Val::Json(std::mem::take(el))), ctx);
                if !r.issues.is_empty() {
                    p.issues.extend(prefix_all(r.issues, i));
                }
                rest_vals.push((i, r.value));
            }
        }
        let mut truncated = None;
        for (i, r) in results.into_iter().enumerate() {
            let present = i < input.len();
            if !r.issues.is_empty() {
                if !present && i >= optout_start {
                    truncated = Some(i);
                    break;
                }
                p.issues.extend(prefix_all(r.issues, i));
            }
            out.push(r.value);
        }
        let mut full: Vec<Val> = out;
        if truncated.is_none() {
            for (_, v) in rest_vals {
                full.push(v);
            }
        }
        while full.len() > input.len() {
            let i = full.len() - 1;
            if i < self.items.len() && self.items[i].optout() && full[i].is_undef() {
                full.pop();
            } else {
                break;
            }
        }
        p.value = Val::Json(Value::Array(full.into_iter().map(Val::into_json_or_null).collect()));
        p
    }
    fn def_type(&self) -> &'static str {
        "tuple"
    }
    fn is_transforming(&self) -> bool {
        self.items.iter().any(|s| s.node().is_transforming()) || self.rest.as_ref().is_some_and(|s| s.node().is_transforming())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("array"));
        let t = jsg.target().to_string();
        let prefix_path = if t == "draft-2020-12" { "prefixItems" } else { "items" };
        let rest_path = if t == "draft-2020-12" || t == "openapi-3.0" { "items" } else { "additionalItems" };
        let mut prefix = Vec::new();
        for (i, s) in self.items.iter().enumerate() {
            let mut p = path.to_vec();
            p.push(json!(prefix_path));
            p.push(json!(i));
            prefix.push(jsg.process(s, &p)?);
        }
        let rest = match &self.rest {
            Some(r) => {
                let mut p = path.to_vec();
                p.push(json!(rest_path));
                if t == "openapi-3.0" {
                    p.push(json!(self.items.len()));
                }
                Some(jsg.process(r, &p)?)
            }
            None => None,
        };
        if t == "draft-2020-12" {
            json.insert("prefixItems".into(), Value::Array(prefix));
            if let Some(r) = rest {
                json.insert("items".into(), r);
            }
        } else if t == "openapi-3.0" {
            let n = prefix.len();
            let mut any_of = prefix;
            let has_rest = rest.is_some();
            if let Some(r) = rest {
                any_of.push(r);
            }
            json.insert("items".into(), json!({ "anyOf": any_of }));
            json.insert("minItems".into(), json!(n));
            if !has_rest {
                json.insert("maxItems".into(), json!(n));
            }
        } else {
            json.insert("items".into(), Value::Array(prefix));
            if let Some(r) = rest {
                json.insert("additionalItems".into(), r);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// record
// ---------------------------------------------------------------------------

/// `z.record(key, value)` / `z.partialRecord(...)` / `z.looseRecord(...)`.
#[derive(Clone)]
pub struct ZRecord {
    pub(crate) key: Option<Schema>,
    pub(crate) value: Schema,
    pub(crate) loose: bool,
    pub(crate) partial: bool,
    pub(crate) error: Option<ErrorMap>,
}

impl ZRecord {
    pub fn new(key: Option<Schema>, value: Schema) -> Self {
        ZRecord { key, value, loose: false, partial: false, error: None }
    }
    /// `.keyType`.
    pub fn key_type(&self) -> Schema {
        self.key.clone().unwrap_or_else(|| Schema::new(super::string()))
    }
    /// `.valueType`.
    pub fn value_type(&self) -> &Schema {
        &self.value
    }
    /// `{ error }`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
}

fn is_plain_object(v: &Val) -> Option<&Map<String, Value>> {
    match v {
        Val::Json(Value::Object(m)) => Some(m),
        _ => None,
    }
}

impl ZodType for ZRecord {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let Some(Value::Object(map)) = input else {
            invalid_type(ctx, "object", input, &TypeErrors::default());
            return Err(());
        };
        let mut ok = true;
        let mut out = Map::new();
        for (k, v) in map {
            let key = match &self.key {
                Some(ks) => match ctx.at(k.as_str(), |ctx| ks.check(Some(&Value::String(k.clone())), ctx)) {
                    Ok(Some(Value::String(s))) => s,
                    Ok(_) => k.clone(),
                    Err(()) => {
                        ok = false;
                        continue;
                    }
                },
                None => k.clone(),
            };
            match ctx.at(k.as_str(), |ctx| self.value.check(Some(v), ctx)) {
                Ok(Some(v)) => {
                    out.insert(key, v);
                }
                Ok(None) => {}
                Err(()) => ok = false,
            }
        }
        if ok { Ok(Some(Value::Object(super::js::js_map(out)))) } else { Err(()) }
    }
    fn type_name(&self) -> String {
        "object".into()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if is_plain_object(&p.value).is_none() {
            let input = p.value.clone();
            p.push(type_issue("record", &input, self.error.clone()));
            return p;
        }
        let key_schema = self.key_type();
        let mut out = Map::new();
        let values = if self.partial { None } else { key_schema.values() };
        if let Some(values) = values {
            let key_str_of = |key: &Val| match key {
                Val::Json(Value::String(s)) => Some(s.clone()),
                Val::Json(Value::Number(n)) => Some(js::num_to_string(js::json_f64(n))),
                _ => None,
            };
            let record_keys: Vec<String> = values.iter().filter_map(key_str_of).collect();
            // The `unrecognized_keys` issue reports the original object: copy it only then.
            let unrecognized: Vec<Value> = match &p.value {
                Val::Json(Value::Object(m)) => m.keys().filter(|k| !record_keys.contains(k)).map(|k| json!(k)).collect(),
                _ => Vec::new(),
            };
            let unrecognized_input = (!unrecognized.is_empty()).then(|| p.value.clone());
            // Values are moved into their payloads (no copy of the object per level); a key that
            // the key schema lists twice gets a copy until its last use.
            let Val::Json(Value::Object(mut input)) = std::mem::replace(&mut p.value, Val::Undef) else { unreachable!() };
            let mut uses: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
            for k in &record_keys {
                *uses.entry(k.clone()).or_default() += 1;
            }
            for key in values {
                let Some(key_str) = key_str_of(&key) else { continue };
                let last_use = match uses.get_mut(&key_str) {
                    Some(n) => {
                        *n -= 1;
                        *n == 0
                    }
                    None => true,
                };
                let key_json = key.to_json_or_null();
                let kr = key_schema.run(Payload::new(key.clone()), ctx);
                if !kr.issues.is_empty() {
                    let mut iss = RawIssue::new(
                        vec![("code", json!("invalid_key")), ("origin", json!("record")), ("issues", finalized_json(ctx, &kr.issues))],
                        key.clone(),
                        self.error.clone(),
                        None,
                    );
                    iss = iss.with_path(vec![key_json]);
                    p.push(iss);
                    continue;
                }
                let out_key = js_string_of(&kr.value);
                let value = if last_use { input.get_mut(&key_str).map(std::mem::take) } else { input.get(&key_str).cloned() };
                let r = self.value.run(Payload::new(Val::from_output(value)), ctx);
                if !r.issues.is_empty() {
                    p.issues.extend(prefix_all(r.issues, key_json));
                }
                if let Some(v) = r.value.into_json() {
                    out.insert(out_key, v);
                }
            }
            if let Some(input) = unrecognized_input {
                p.push(RawIssue::new(
                    vec![("code", json!("unrecognized_keys")), ("keys", Value::Array(unrecognized))],
                    input,
                    self.error.clone(),
                    None,
                ));
            }
        } else {
            let Val::Json(Value::Object(input)) = std::mem::replace(&mut p.value, Val::Undef) else { unreachable!() };
            for (key, v) in input {
                if key == "__proto__" {
                    continue;
                }
                let mut kr = key_schema.run(Payload::new(Val::Json(json!(key))), ctx);
                if !kr.issues.is_empty() && regexes::NUMBER.test(&key) {
                    let retry = key_schema.run(Payload::new(Val::number(js::string_to_number(&key))), ctx);
                    if retry.issues.is_empty() {
                        kr = retry;
                    }
                }
                if !kr.issues.is_empty() {
                    if self.loose {
                        out.insert(key, v);
                    } else {
                        let iss = RawIssue::new(
                            vec![("code", json!("invalid_key")), ("origin", json!("record")), ("issues", finalized_json(ctx, &kr.issues))],
                            Val::Json(json!(key)),
                            self.error.clone(),
                            None,
                        )
                        .with_path(vec![json!(key)]);
                        p.push(iss);
                    }
                    continue;
                }
                let r = self.value.run(Payload::new(Val::Json(v)), ctx);
                if !r.issues.is_empty() {
                    p.issues.extend(prefix_all(r.issues, key));
                }
                if let Some(v) = r.value.into_json() {
                    out.insert(js_string_of(&kr.value), v);
                }
            }
        }
        p.value = Val::Json(Value::Object(super::js::js_map(out)));
        p
    }
    fn def_type(&self) -> &'static str {
        "record"
    }
    fn is_transforming(&self) -> bool {
        self.key.as_ref().is_some_and(|k| k.node().is_transforming()) || self.value.node().is_transforming()
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("object"));
        let key = self.key_type();
        let patterns: Vec<String> = key.node().string_patterns();
        if self.loose && !patterns.is_empty() {
            let mut p = path.to_vec();
            p.push(json!("patternProperties"));
            p.push(json!("*"));
            let v = jsg.process(&self.value, &p)?;
            let mut pp = Map::new();
            for pat in patterns {
                pp.insert(pat, v.clone());
            }
            json.insert("patternProperties".into(), Value::Object(pp));
        } else {
            if matches!(jsg.target(), "draft-07" | "draft-2020-12") {
                let mut p = path.to_vec();
                p.push(json!("propertyNames"));
                let k = jsg.process(&key, &p)?;
                json.insert("propertyNames".into(), k);
            }
            let mut p = path.to_vec();
            p.push(json!("additionalProperties"));
            let v = jsg.process(&self.value, &p)?;
            json.insert("additionalProperties".into(), v);
        }
        if !self.partial
            && let Some(values) = key.values() {
                let req: Vec<Value> = values
                    .into_iter()
                    .filter_map(|v| match v {
                        Val::Json(v @ (Value::String(_) | Value::Number(_))) => Some(v),
                        _ => None,
                    })
                    .collect();
                if !req.is_empty() {
                    json.insert("required".into(), Value::Array(req));
                }
            }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// object
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum UnknownKeys {
    Strip,
    Strict(Option<String>),
    Passthrough,
    Catchall(Schema),
}

/// `z.object({...})`.
#[derive(Clone)]
pub struct ZObject {
    shape: Vec<(String, Schema)>,
    unknown: UnknownKeys,
    errors: TypeErrors,
    error: Option<ErrorMap>,
}

impl Default for ZObject {
    fn default() -> Self {
        ZObject::new()
    }
}

impl ZObject {
    pub fn new() -> Self {
        ZObject { shape: Vec::new(), unknown: UnknownKeys::Strip, errors: TypeErrors::default(), error: None }
    }

    pub fn from_fields(fields: Vec<(String, Schema)>) -> Self {
        let mut o = ZObject::new();
        for (k, s) in fields {
            o = o.field(k, s);
        }
        o
    }

    /// Adds (or replaces) a field.
    pub fn field(mut self, name: impl Into<String>, schema: impl Into<Schema>) -> Self {
        let name = name.into();
        let schema = schema.into();
        match self.shape.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = schema,
            None => self.shape.push((name, schema)),
        }
        self
    }

    /// `.shape`.
    pub fn shape(&self) -> &[(String, Schema)] {
        &self.shape
    }

    /// `.shape.key`.
    pub fn get(&self, key: &str) -> Option<&Schema> {
        self.shape.iter().find(|(k, _)| k == key).map(|(_, s)| s)
    }

    /// `.keyof()`.
    pub fn keyof(&self) -> ZEnum {
        ZEnum::new(self.shape.iter().map(|(k, _)| k.clone()).collect())
    }

    /// `.strict()`: unknown keys are an error.
    pub fn strict(mut self) -> Self {
        self.unknown = UnknownKeys::Strict(None);
        self
    }
    /// `.strict(message)`.
    pub fn strict_with(mut self, message: impl Into<String>) -> Self {
        self.unknown = UnknownKeys::Strict(Some(message.into()));
        self
    }
    /// `.strip()` (default): unknown keys are removed.
    pub fn strip(mut self) -> Self {
        self.unknown = UnknownKeys::Strip;
        self
    }
    /// `.passthrough()`: unknown keys are kept.
    pub fn passthrough(mut self) -> Self {
        self.unknown = UnknownKeys::Passthrough;
        self
    }
    /// `.loose()` (zod 4 name of `.passthrough()`).
    pub fn loose(self) -> Self {
        self.passthrough()
    }
    /// `.catchall(schema)`: unknown keys are validated with `schema`.
    pub fn catchall(mut self, schema: impl Into<Schema>) -> Self {
        self.unknown = UnknownKeys::Catchall(schema.into());
        self
    }
    /// `{ error }` (object-level issues: invalid type, unrecognized keys).
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        let e: ErrorParam = e.into();
        self.errors.invalid_type = e.message();
        self.error = Some(e.into_map());
        self
    }
    pub fn message(self, m: impl Into<String>) -> Self {
        self.error(ErrorParam::Message(m.into()))
    }

    /// `.extend({...})`.
    pub fn extend(mut self, other: ZObject) -> Self {
        for (k, s) in other.shape {
            self = self.field(k, s);
        }
        self
    }
    /// `.safeExtend({...})`.
    pub fn safe_extend(self, other: ZObject) -> Self {
        self.extend(other)
    }
    /// `.merge(other)`: like `extend`, and takes the unknown-keys policy of `other`.
    pub fn merge(self, other: ZObject) -> Self {
        let unknown = other.unknown.clone();
        let mut o = self.extend(other);
        o.unknown = unknown;
        o
    }
    /// `.pick({ a: true })`.
    pub fn pick(mut self, keys: &[&str]) -> Self {
        self.shape.retain(|(k, _)| keys.contains(&k.as_str()));
        self
    }
    /// `.omit({ a: true })`.
    pub fn omit(mut self, keys: &[&str]) -> Self {
        self.shape.retain(|(k, _)| !keys.contains(&k.as_str()));
        self
    }
    /// `.partial()`: every field optional (PartialType).
    pub fn partial(mut self) -> Self {
        for (_, s) in &mut self.shape {
            *s = Schema::new(ZOptional(s.clone()));
        }
        self
    }
    /// `.partial({ a: true })`.
    pub fn partial_fields(mut self, keys: &[&str]) -> Self {
        for (k, s) in &mut self.shape {
            if keys.contains(&k.as_str()) {
                *s = Schema::new(ZOptional(s.clone()));
            }
        }
        self
    }
    /// `.deepPartial()` (zod 3).
    pub fn deep_partial(mut self) -> Self {
        for (_, s) in &mut self.shape {
            let inner = match s.as_object() {
                Some(o) => Schema::new(o.clone().deep_partial()),
                None => s.clone(),
            };
            *s = if inner.accepts_undefined() { inner } else { inner.optional() };
        }
        self
    }
    fn required_one(s: &Schema) -> Schema {
        let mut unwrapped = s.clone();
        while let Some(inner) = unwrapped.unwrap_optional() {
            unwrapped = inner;
        }
        let mut n = ZNonOptional::new(s.clone());
        n.v3 = Some(unwrapped);
        Schema::new(n)
    }
    /// `.required()`: every field non-optional.
    pub fn required(mut self) -> Self {
        for (_, s) in &mut self.shape {
            *s = Self::required_one(s);
        }
        self
    }
    /// `.required({ a: true })`.
    pub fn required_fields(mut self, keys: &[&str]) -> Self {
        for (k, s) in &mut self.shape {
            if keys.contains(&k.as_str()) {
                *s = Self::required_one(s);
            }
        }
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

    fn catchall_schema(&self) -> Option<Schema> {
        match &self.unknown {
            UnknownKeys::Strip => None,
            UnknownKeys::Strict(_) => Some(Schema::new(ZNever::new())),
            UnknownKeys::Passthrough => Some(Schema::new(ZAny)),
            UnknownKeys::Catchall(s) => Some(s.clone()),
        }
    }

    fn inst_error(&self) -> Option<ErrorMap> {
        self.errors.v4_map(&self.error)
    }
}

fn handle_property(r: Payload, p: &mut Payload, out: &mut Map<String, Value>, key: &str, present: bool, optin: bool, optout: bool) {
    let had_issues = !r.issues.is_empty();
    if had_issues {
        if optin && optout && !present {
            return;
        }
        p.issues.extend(prefix_all(r.issues, key));
    }
    if !present && !optin {
        if !had_issues {
            p.push(
                RawIssue::new(vec![("code", json!("invalid_type")), ("expected", json!("nonoptional"))], Val::Undef, None, None)
                    .with_path(vec![json!(key)]),
            );
        }
        return;
    }
    if let Some(v) = r.value.into_json() {
        out.insert(key.to_string(), v);
    }
}

impl ZodType for ZObject {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let Some(Value::Object(map)) = input else {
            invalid_type(ctx, "object", input, &self.errors);
            return Err(());
        };
        let mut ok = true;
        let mut out = Map::new();
        for (key, schema) in &self.shape {
            match ctx.at(key.as_str(), |ctx| schema.check(map.get(key), ctx)) {
                Ok(Some(v)) => {
                    out.insert(key.clone(), v);
                }
                Ok(None) => {}
                Err(()) => ok = false,
            }
        }
        let extra: Vec<&String> = map.keys().filter(|k| !self.shape.iter().any(|(s, _)| s == *k)).collect();
        if !extra.is_empty() {
            match &self.unknown {
                UnknownKeys::Strip => {}
                UnknownKeys::Strict(msg) => {
                    ok = false;
                    let keys = extra.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
                    ctx.add(
                        ZodIssueCode::UnrecognizedKeys,
                        msg.clone().unwrap_or_else(|| format!("Unrecognized key(s) in object: {keys}")),
                        issue_params(&[("keys", json!(extra))]),
                    );
                }
                UnknownKeys::Passthrough => {
                    for k in extra {
                        out.insert(k.clone(), map[k].clone());
                    }
                }
                UnknownKeys::Catchall(schema) => {
                    for k in extra {
                        match ctx.at(k.as_str(), |ctx| schema.check(Some(&map[k]), ctx)) {
                            Ok(Some(v)) => {
                                out.insert(k.clone(), v);
                            }
                            Ok(None) => {}
                            Err(()) => ok = false,
                        }
                    }
                }
            }
        }
        if ok { Ok(Some(Value::Object(super::js::js_map(out)))) } else { Err(()) }
    }
    fn type_name(&self) -> String {
        "object".into()
    }
    fn object_shape(&self) -> Option<&ZObject> {
        Some(self)
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if is_plain_object(&p.value).is_none() {
            let input = p.value.clone();
            p.push(type_issue("object", &input, self.inst_error()));
            return p;
        }
        let catchall = self.catchall_schema();
        let is_never = catchall.as_ref().is_some_and(|c| matches!(self.unknown, UnknownKeys::Strict(_)) || c.def_type() == "never");
        let is_extra = |key: &str| key != "__proto__" && !self.shape.iter().any(|(k, _)| k == key);
        // The `unrecognized_keys` issue reports the original object: copy it only in that case.
        let input_val = match &p.value {
            Val::Json(Value::Object(m)) if is_never && m.keys().any(|k| is_extra(k)) => Some(p.value.clone()),
            _ => None,
        };
        // Property values are moved into their payloads: copying the object at every level
        // makes recursive schemas quadratic.
        let Val::Json(Value::Object(mut input)) = std::mem::replace(&mut p.value, Val::Undef) else { unreachable!() };
        let mut out = Map::new();
        for (key, schema) in &self.shape {
            let present = input.contains_key(key);
            let r = schema.run(Payload::new(Val::from_output(input.get_mut(key).map(std::mem::take))), ctx);
            handle_property(r, &mut p, &mut out, key, present, schema.optin(), schema.optout());
        }
        if let Some(catchall) = catchall {
            let mut unrecognized = Vec::new();
            for (key, v) in input.iter_mut() {
                if !is_extra(key) {
                    continue;
                }
                if is_never {
                    unrecognized.push(json!(key));
                    continue;
                }
                let r = catchall.run(Payload::new(Val::Json(std::mem::take(v))), ctx);
                handle_property(r, &mut p, &mut out, key, true, catchall.optin(), catchall.optout());
            }
            if let (false, Some(input_val)) = (unrecognized.is_empty(), input_val) {
                let err = match &self.unknown {
                    UnknownKeys::Strict(Some(m)) => Some(ErrorParam::Message(m.clone()).into_map()),
                    _ => self.inst_error(),
                };
                p.push(RawIssue::new(vec![("code", json!("unrecognized_keys")), ("keys", Value::Array(unrecognized))], input_val, err, None));
            }
        }
        p.value = Val::Json(Value::Object(super::js::js_map(out)));
        p
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        Some(self.shape.iter().filter_map(|(k, s)| s.values().map(|v| (k.clone(), v))).collect())
    }
    fn def_type(&self) -> &'static str {
        "object"
    }
    fn is_transforming(&self) -> bool {
        self.shape.iter().any(|(_, s)| s.node().is_transforming())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        json.insert("type".into(), json!("object"));
        let mut props = Map::new();
        for (k, s) in &self.shape {
            let mut p = path.to_vec();
            p.push(json!("properties"));
            p.push(json!(k));
            props.insert(k.clone(), jsg.process(s, &p)?);
        }
        json.insert("properties".into(), Value::Object(props));
        let input = jsg.io_input();
        let required: Vec<Value> =
            self.shape.iter().filter(|(_, s)| if input { !s.optin() } else { !s.optout() }).map(|(k, _)| json!(k)).collect();
        if !required.is_empty() {
            json.insert("required".into(), Value::Array(required));
        }
        match self.catchall_schema() {
            None => {
                if !input {
                    json.insert("additionalProperties".into(), json!(false));
                }
            }
            Some(c) if c.def_type() == "never" => {
                json.insert("additionalProperties".into(), json!(false));
            }
            Some(c) => {
                let mut p = path.to_vec();
                p.push(json!("additionalProperties"));
                let v = jsg.process(&c, &p)?;
                json.insert("additionalProperties".into(), v);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// union, xor, discriminated union, intersection
// ---------------------------------------------------------------------------

/// `z.union([...])`.
#[derive(Clone)]
pub struct ZUnion(pub Vec<Schema>);

fn union_values(options: &[Schema]) -> Option<Vec<Val>> {
    let mut out = Vec::new();
    for o in options {
        out.extend(o.values()?);
    }
    Some(out)
}

fn union_pattern(options: &[Schema]) -> Option<String> {
    let mut parts = Vec::new();
    for o in options {
        parts.push(super::wrappers::clean_regex(&o.node().pattern()?).to_string());
    }
    Some(format!("^({})$", parts.join("|")))
}

fn union_json_schema(options: &[Schema], exclusive: bool, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
    let key = if exclusive { "oneOf" } else { "anyOf" };
    let mut out = Vec::new();
    for (i, o) in options.iter().enumerate() {
        let mut p = path.to_vec();
        p.push(json!(key));
        p.push(json!(i));
        out.push(jsg.process(o, &p)?);
    }
    json.insert(key.into(), Value::Array(out));
    Ok(())
}

pub(crate) fn union_v4(options: &[Schema], p: Payload, ctx: &RunCtx, error: Option<ErrorMap>) -> Payload {
    if options.len() == 1 {
        return options[0].run(p, ctx);
    }
    let mut results = Vec::with_capacity(options.len());
    for o in options {
        let r = o.run(Payload::new(p.value.clone()), ctx);
        if r.issues.is_empty() {
            return r;
        }
        results.push(r);
    }
    // A nesting-limit issue is reported as is, not as one more `invalid_union` per level.
    if let Some(i) = results.iter().position(|r| r.issues.iter().any(|i| i.too_deep)) {
        return results.swap_remove(i);
    }
    let nonaborted: Vec<usize> = (0..results.len()).filter(|i| !results[*i].is_aborted(0)).collect();
    if nonaborted.len() == 1 {
        return results.swap_remove(nonaborted[0]);
    }
    let errors: Vec<Value> = results.iter().map(|r| finalized_json(ctx, &r.issues)).collect();
    let mut p = p;
    let input = p.value.clone();
    p.push(RawIssue::new(vec![("code", json!("invalid_union")), ("errors", Value::Array(errors))], input, error, None));
    p
}

impl ZodType for ZUnion {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let mut failures = Vec::with_capacity(self.0.len());
        for option in &self.0 {
            let mut sub = ctx.sub();
            match option.check(input, &mut sub) {
                Ok(v) if sub.issues.is_empty() => return Ok(v),
                _ => failures.push(sub.issues),
            }
        }
        // zod returns the issues of a "dirty" option: one whose type matched but
        // whose checks failed. Otherwise it reports `invalid_union`.
        let depth = ctx.path.len();
        if let Some(dirty) = failures
            .iter()
            .find(|issues| issues.iter().any(|i| i.code != ZodIssueCode::InvalidType || i.path.len() > depth))
        {
            ctx.issues.extend(dirty.iter().cloned());
            return Err(());
        }
        let union_errors: Vec<Value> = failures.iter().map(|issues| json!({ "issues": issues, "name": "ZodError" })).collect();
        ctx.add(ZodIssueCode::InvalidUnion, "Invalid input", issue_params(&[("unionErrors", Value::Array(union_errors))]));
        Err(())
    }
    fn type_name(&self) -> String {
        self.0.iter().map(|s| s.type_name()).collect::<Vec<_>>().join(" | ")
    }
    fn accepts_undefined(&self) -> bool {
        self.0.iter().any(|s| s.accepts_undefined())
    }
    fn literal_values(&self) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        for s in &self.0 {
            out.extend(s.literal_values()?);
        }
        Some(out)
    }
    fn run_v4(&self, p: Payload, ctx: &RunCtx) -> Payload {
        union_v4(&self.0, p, ctx, None)
    }
    fn optin(&self) -> bool {
        self.0.iter().any(|o| o.optin())
    }
    fn optout(&self) -> bool {
        self.0.iter().any(|o| o.optout())
    }
    fn values(&self) -> Option<Vec<Val>> {
        union_values(&self.0)
    }
    fn pattern(&self) -> Option<String> {
        union_pattern(&self.0)
    }
    fn def_type(&self) -> &'static str {
        "union"
    }
    fn is_transforming(&self) -> bool {
        self.0.iter().any(|s| s.node().is_transforming())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        union_json_schema(&self.0, false, jsg, json, path)
    }
}

/// `z.xor([...])`: exactly one option must match.
#[derive(Clone)]
pub struct ZXor(pub Vec<Schema>);

impl ZodType for ZXor {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        v3_via_v4(self, input, ctx)
    }
    fn type_name(&self) -> String {
        self.0.iter().map(|s| s.type_name()).collect::<Vec<_>>().join(" | ")
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if self.0.len() == 1 {
            return self.0[0].run(p, ctx);
        }
        let mut results: Vec<Payload> = self.0.iter().map(|o| o.run(Payload::new(p.value.clone()), ctx)).collect();
        if let Some(i) = results.iter().position(|r| r.issues.iter().any(|i| i.too_deep)) {
            return results.swap_remove(i);
        }
        let successes: Vec<&Payload> = results.iter().filter(|r| r.issues.is_empty()).collect();
        let input = p.value.clone();
        match successes.len() {
            1 => p.value = successes[0].value.clone(),
            0 => {
                let errors: Vec<Value> = results.iter().map(|r| finalized_json(ctx, &r.issues)).collect();
                p.push(RawIssue::new(vec![("code", json!("invalid_union")), ("errors", Value::Array(errors))], input, None, None));
            }
            _ => {
                p.push(RawIssue::new(
                    vec![("code", json!("invalid_union")), ("errors", json!([])), ("inclusive", json!(false))],
                    input,
                    None,
                    None,
                ));
            }
        }
        p
    }
    fn optin(&self) -> bool {
        self.0.iter().any(|o| o.optin())
    }
    fn optout(&self) -> bool {
        self.0.iter().any(|o| o.optout())
    }
    fn values(&self) -> Option<Vec<Val>> {
        union_values(&self.0)
    }
    fn def_type(&self) -> &'static str {
        "union"
    }
    fn is_transforming(&self) -> bool {
        self.0.iter().any(|s| s.node().is_transforming())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        union_json_schema(&self.0, true, jsg, json, path)
    }
}

/// `z.discriminatedUnion(key, [...])`.
#[derive(Clone)]
pub struct ZDiscriminatedUnion {
    key: String,
    options: Vec<(Vec<Value>, Schema)>,
    map: Vec<(Val, usize)>,
    union_fallback: bool,
    error: Option<ErrorMap>,
}

impl ZDiscriminatedUnion {
    pub fn new(key: &str, options: Vec<Schema>) -> Self {
        let mut map: Vec<(Val, usize)> = Vec::new();
        for (i, o) in options.iter().enumerate() {
            let values = o
                .node()
                .prop_values()
                .and_then(|pv| pv.into_iter().find(|(k, _)| k == key).map(|(_, v)| v))
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| panic!("Invalid discriminated union option at index \"{i}\""));
            for v in values {
                if map.iter().any(|(m, _)| m.same_value_zero(&v)) {
                    panic!("Duplicate discriminator value \"{}\"", js_string_of(&v));
                }
                map.push((v, i));
            }
        }
        let options = options
            .into_iter()
            .map(|s| {
                let values = s.as_object().and_then(|o| o.get(key)).and_then(|d| d.literal_values()).unwrap_or_default();
                (values, s)
            })
            .collect();
        ZDiscriminatedUnion { key: key.to_string(), options, map, union_fallback: false, error: None }
    }
    /// `{ unionFallback: true }`.
    pub fn union_fallback(mut self) -> Self {
        self.union_fallback = true;
        self
    }
    /// `{ error }`.
    pub fn error(mut self, e: impl Into<ErrorParam>) -> Self {
        self.error = Some(e.into().into_map());
        self
    }
    /// `.options`.
    pub fn options(&self) -> Vec<Schema> {
        self.options.iter().map(|(_, s)| s.clone()).collect()
    }
    fn schemas(&self) -> Vec<Schema> {
        self.options()
    }
}

impl ZodType for ZDiscriminatedUnion {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        let Some(Value::Object(map)) = input else {
            invalid_type(ctx, "object", input, &TypeErrors::default());
            return Err(());
        };
        let disc = map.get(&self.key);
        match self.options.iter().find(|(values, _)| disc.is_some_and(|d| values.contains(d))) {
            Some((_, schema)) => schema.check(input, ctx),
            None => {
                let all: Vec<Value> = self.options.iter().flat_map(|(v, _)| v.clone()).collect();
                let expected = all
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => format!("'{s}'"),
                        o => o.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                ctx.at(self.key.as_str(), |ctx| {
                    ctx.add(
                        ZodIssueCode::InvalidUnionDiscriminator,
                        format!("Invalid discriminator value. Expected {expected}"),
                        issue_params(&[("options", Value::Array(all.clone()))]),
                    )
                });
                Err(())
            }
        }
    }
    fn type_name(&self) -> String {
        "object".into()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        let Some(input) = is_plain_object(&p.value) else {
            let input = p.value.clone();
            p.push(RawIssue::new(vec![("code", json!("invalid_type")), ("expected", json!("object"))], input, self.error.clone(), None));
            return p;
        };
        let disc = Val::from_opt(input.get(&self.key));
        if let Some((_, i)) = self.map.iter().find(|(v, _)| v.same_value_zero(&disc)) {
            return self.options[*i].1.run(p, ctx);
        }
        if self.union_fallback || ctx.backward() {
            return union_v4(&self.schemas(), p, ctx, self.error.clone());
        }
        let options: Vec<Value> = self.map.iter().map(|(v, _)| v.to_json_or_null()).collect();
        let input = p.value.clone();
        p.push(
            RawIssue::new(
                vec![
                    ("code", json!("invalid_union")),
                    ("errors", json!([])),
                    ("note", json!("No matching discriminator")),
                    ("discriminator", json!(self.key)),
                    ("options", Value::Array(options)),
                ],
                input,
                self.error.clone(),
                None,
            )
            .with_path(vec![json!(self.key)]),
        );
        p
    }
    fn optin(&self) -> bool {
        self.options.iter().any(|(_, o)| o.optin())
    }
    fn optout(&self) -> bool {
        self.options.iter().any(|(_, o)| o.optout())
    }
    fn values(&self) -> Option<Vec<Val>> {
        union_values(&self.schemas())
    }
    fn prop_values(&self) -> Option<Vec<(String, Vec<Val>)>> {
        let mut out: Vec<(String, Vec<Val>)> = Vec::new();
        for (_, o) in &self.options {
            for (k, vs) in o.node().prop_values().unwrap_or_default() {
                match out.iter_mut().find(|(kk, _)| *kk == k) {
                    Some(slot) => slot.1.extend(vs),
                    None => out.push((k, vs)),
                }
            }
        }
        Some(out)
    }
    fn def_type(&self) -> &'static str {
        "union"
    }
    fn is_transforming(&self) -> bool {
        self.options.iter().any(|(_, s)| s.node().is_transforming())
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        union_json_schema(&self.schemas(), true, jsg, json, path)
    }
}

/// `z.intersection(a, b)` / `a.and(b)`.
#[derive(Clone)]
pub struct ZIntersection(pub Schema, pub Schema);

fn merge_values(a: Value, b: Value) -> Option<Value> {
    match (a, b) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                let merged = match a.remove(&k) {
                    Some(prev) => merge_values(prev, v)?,
                    None => v,
                };
                a.insert(k, merged);
            }
            Some(Value::Object(a))
        }
        (a, b) if a == b => Some(a),
        _ => None,
    }
}

/// `mergeValues` of zod 4 (`Err(path)` when the values cannot be merged).
fn merge_vals(a: &Val, b: &Val) -> Result<Val, Vec<Value>> {
    if a.same_value_zero(b) {
        return Ok(a.clone());
    }
    match (a, b) {
        (Val::Date(x), Val::Date(y)) if x == y => Ok(a.clone()),
        (Val::Json(Value::Object(x)), Val::Json(Value::Object(y))) => {
            let mut out = x.clone();
            for (k, v) in y {
                out.insert(k.clone(), v.clone());
            }
            for (k, xv) in x {
                if let Some(yv) = y.get(k) {
                    match merge_vals(&Val::Json(xv.clone()), &Val::Json(yv.clone())) {
                        Ok(v) => {
                            out.insert(k.clone(), v.to_json_or_null());
                        }
                        Err(mut path) => {
                            path.insert(0, json!(k));
                            return Err(path);
                        }
                    }
                }
            }
            Ok(Val::Json(Value::Object(super::js::js_map(out))))
        }
        (Val::Json(Value::Array(x)), Val::Json(Value::Array(y))) => {
            if x.len() != y.len() {
                return Err(Vec::new());
            }
            let mut out = Vec::with_capacity(x.len());
            for (i, (xv, yv)) in x.iter().zip(y).enumerate() {
                match merge_vals(&Val::Json(xv.clone()), &Val::Json(yv.clone())) {
                    Ok(v) => out.push(v.to_json_or_null()),
                    Err(mut path) => {
                        path.insert(0, json!(i));
                        return Err(path);
                    }
                }
            }
            Ok(Val::Json(Value::Array(out)))
        }
        _ => Err(Vec::new()),
    }
}

impl ZodType for ZIntersection {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        // Both sides see the full input; for objects use passthrough semantics so that
        // each side keeps the keys the other one declares.
        let a = self.0.check(input, ctx);
        let b = self.1.check(input, ctx);
        match (a, b) {
            (Ok(a), Ok(b)) => match (a, b) {
                (None, None) => Ok(None),
                (Some(a), None) => Ok(Some(a)),
                (None, Some(b)) => Ok(Some(b)),
                (Some(a), Some(b)) => match merge_values(a, b) {
                    Some(v) => Ok(Some(v)),
                    None => {
                        ctx.add(ZodIssueCode::InvalidIntersectionTypes, "Intersection results could not be merged", Map::new());
                        Err(())
                    }
                },
            },
            _ => Err(()),
        }
    }
    fn type_name(&self) -> String {
        format!("{} & {}", self.0.type_name(), self.1.type_name())
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        let left = self.0.run(Payload::new(p.value.clone()), ctx);
        let right = self.1.run(Payload::new(p.value.clone()), ctx);
        let mut unrec: Vec<(String, bool, bool)> = Vec::new();
        let mut unrec_issue: Option<RawIssue> = None;
        for (side, r) in [(0, &left), (1, &right)] {
            for iss in &r.issues {
                if iss.code() == ZodIssueCode::UnrecognizedKeys {
                    if side == 0 && unrec_issue.is_none() {
                        unrec_issue = Some(iss.clone());
                    }
                    for k in iss.fields.get("keys").and_then(Value::as_array).cloned().unwrap_or_default() {
                        let k = k.as_str().unwrap_or_default().to_string();
                        let slot = match unrec.iter().position(|(kk, _, _)| *kk == k) {
                            Some(i) => i,
                            None => {
                                unrec.push((k, false, false));
                                unrec.len() - 1
                            }
                        };
                        if side == 0 {
                            unrec[slot].1 = true;
                        } else {
                            unrec[slot].2 = true;
                        }
                    }
                } else {
                    p.issues.push(iss.clone());
                }
            }
        }
        let both: Vec<Value> = unrec.iter().filter(|(_, l, r)| *l && *r).map(|(k, _, _)| json!(k)).collect();
        if let (false, Some(mut iss)) = (both.is_empty(), unrec_issue) {
            iss.fields.insert("keys".into(), Value::Array(both));
            p.issues.push(iss);
        }
        if p.is_aborted(0) {
            return p;
        }
        match merge_vals(&left.value, &right.value) {
            Ok(v) => p.value = v,
            // zod throws a plain `Error` (not a validation issue).
            Err(path) => panic!("Unmergable intersection. Error path: {}", Value::Array(path)),
        }
        p
    }
    fn def_type(&self) -> &'static str {
        "intersection"
    }
    fn is_transforming(&self) -> bool {
        self.0.node().is_transforming() || self.1.node().is_transforming()
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let mut pa = path.to_vec();
        pa.push(json!("allOf"));
        pa.push(json!(0));
        let a = jsg.process(&self.0, &pa)?;
        let mut pb = path.to_vec();
        pb.push(json!("allOf"));
        pb.push(json!(1));
        let b = jsg.process(&self.1, &pb)?;
        let mut all = Vec::new();
        for v in [a, b] {
            let peek = jsg.peek(&v);
            match peek.as_object() {
                Some(m) if m.len() == 1 && m.contains_key("allOf") => all.extend(m["allOf"].as_array().cloned().unwrap_or_default()),
                _ => all.push(v),
            }
        }
        json.insert("allOf".into(), Value::Array(all));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Constructors.
// ---------------------------------------------------------------------------

/// `z.array(schema)`.
pub fn array(item: impl Into<Schema>) -> ZArray {
    ZArray::new(item.into())
}
/// `z.object({})` builder; prefer the [`object!`](crate::z::object) macro.
pub fn object() -> ZObject {
    ZObject::new()
}
/// `z.record(valueSchema)` (zod 3 form; the key is `z.string()`).
pub fn record(value: impl Into<Schema>) -> ZRecord {
    ZRecord::new(None, value.into())
}
/// `z.record(keySchema, valueSchema)`. With an enum / literal key the record is exhaustive.
pub fn record_with(key: impl Into<Schema>, value: impl Into<Schema>) -> ZRecord {
    ZRecord::new(Some(key.into()), value.into())
}
/// `z.partialRecord(keySchema, valueSchema)`.
pub fn partial_record(key: impl Into<Schema>, value: impl Into<Schema>) -> ZRecord {
    let mut r = ZRecord::new(Some(key.into()), value.into());
    r.partial = true;
    r
}
/// `z.looseRecord(keySchema, valueSchema)`: keys that do not match pass through.
pub fn loose_record(key: impl Into<Schema>, value: impl Into<Schema>) -> ZRecord {
    let mut r = ZRecord::new(Some(key.into()), value.into());
    r.loose = true;
    r
}
/// `z.tuple([...])`.
pub fn tuple<I: IntoIterator<Item = Schema>>(items: I) -> ZTuple {
    ZTuple { items: items.into_iter().collect(), rest: None, error: None }
}
/// `z.union([...])`.
pub fn union<I: IntoIterator<Item = Schema>>(options: I) -> Schema {
    Schema::new(ZUnion(options.into_iter().collect()))
}
/// `z.xor([...])`.
pub fn xor<I: IntoIterator<Item = Schema>>(options: I) -> Schema {
    Schema::new(ZXor(options.into_iter().collect()))
}
/// `z.discriminatedUnion(key, [...])`.
pub fn discriminated_union<I: IntoIterator<Item = Schema>>(key: &str, options: I) -> Schema {
    Schema::new(ZDiscriminatedUnion::new(key, options.into_iter().collect()))
}
/// `z.intersection(a, b)`.
pub fn intersection(a: impl Into<Schema>, b: impl Into<Schema>) -> Schema {
    Schema::new(ZIntersection(a.into(), b.into()))
}
/// `z.keyof(object)`.
pub fn keyof(o: &ZObject) -> ZEnum {
    o.keyof()
}
/// `z.json()`: any JSON value.
///
/// zod defines it as `z.lazy(() => z.union([string, number, boolean, null, array(json),
/// record(string, json)]))`; that is the schema used for JSON Schema output, zod 3 and non-JSON
/// inputs. A JSON input always matches exactly one option, so it is validated in a single pass
/// with the same result (`__proto__` keys dropped, keys in V8 order) and the same nesting limit
/// ([`ZodConfig::max_depth`](super::ZodConfig::max_depth)) as the union of lazies.
pub fn json() -> Schema {
    Schema::new(ZJson(super::lazy(json_node)))
}

fn json_node() -> Schema {
    // One union shared by every level (the lazies resolve to it instead of building a new level
    // of schemas per level of input).
    static NODE: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();
    NODE.get_or_init(|| {
        union([
            Schema::new(super::string()),
            Schema::new(super::number()),
            Schema::new(super::boolean()),
            Schema::new(super::null()),
            Schema::new(array(super::lazy(json_node))),
            Schema::new(record_with(super::string(), super::lazy(json_node))),
        ])
    })
    .clone()
}

/// `z.json()` (see [`json`]): the zod definition plus a single-pass path for JSON inputs.
#[derive(Clone)]
struct ZJson(Schema);

impl ZJson {
    /// One element of an array or record: it goes through `z.lazy` in zod's definition.
    fn child(v: Value, ctx: &RunCtx, path: &mut Vec<Value>, issues: &mut Vec<RawIssue>) -> Value {
        match ctx.enter() {
            Some(_guard) => Self::value(v, ctx, path, issues),
            None => {
                issues.push(super::core::too_deep_issue(ctx.max_depth).with_path(path.clone()));
                Value::Null
            }
        }
    }

    fn value(v: Value, ctx: &RunCtx, path: &mut Vec<Value>, issues: &mut Vec<RawIssue>) -> Value {
        match v {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| {
                        path.push(json!(i));
                        let out = Self::child(item, ctx, path, issues);
                        path.pop();
                        out
                    })
                    .collect(),
            ),
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, item) in map {
                    if k == "__proto__" {
                        continue;
                    }
                    path.push(json!(k));
                    let v = Self::child(item, ctx, path, issues);
                    path.pop();
                    out.insert(k, v);
                }
                Value::Object(super::js::js_map(out))
            }
            other => other,
        }
    }
}

impl ZodType for ZJson {
    fn check(&self, input: Option<&Value>, ctx: &mut ParseCtx) -> Result<Option<Value>, ()> {
        self.0.check(input, ctx)
    }
    fn type_name(&self) -> String {
        self.0.type_name()
    }
    fn run_v4(&self, mut p: Payload, ctx: &RunCtx) -> Payload {
        if !matches!(p.value, Val::Json(_)) {
            return self.0.run(p, ctx);
        }
        let Some(_guard) = ctx.enter() else {
            p.push(super::core::too_deep_issue(ctx.max_depth));
            return p;
        };
        let Val::Json(v) = std::mem::replace(&mut p.value, Val::Undef) else { unreachable!() };
        let mut path = Vec::new();
        p.value = Val::Json(Self::value(v, ctx, &mut path, &mut p.issues));
        p
    }
    fn optin(&self) -> bool {
        self.0.optin()
    }
    fn optout(&self) -> bool {
        self.0.optout()
    }
    fn def_type(&self) -> &'static str {
        self.0.def_type()
    }
    fn inner_schema(&self) -> Option<Schema> {
        self.0.node().inner_schema()
    }
    fn identity(&self) -> Option<usize> {
        Some(self.0.id())
    }
    fn is_transforming(&self) -> bool {
        self.0.node().is_transforming()
    }
    fn json_schema(&self, jsg: &mut JsonSchemaGen, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        self.0.node().json_schema(jsg, json, path)
    }
}

impl ZObject {
    /// `z.strictObject({...})` from a builder.
    pub fn strict_object(fields: Vec<(String, Schema)>) -> Self {
        ZObject::from_fields(fields).strict()
    }
}

#[allow(dead_code)]
fn _assert_traits() {
    fn is_schema<T: ZodSchema>() {}
    is_schema::<ZObject>();
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use crate::z::{ParseOptions, parse_val};

    /// The single-pass path gives the same result as zod's union of lazies.
    #[test]
    fn json_fast_path_matches_union_of_lazies() {
        let fast = json();
        let slow = super::super::lazy(json_node);
        let cases = [
            json!(null),
            json!("s"),
            json!(1.5),
            json!(12345678901234567890u64),
            json!(-0.0),
            json!([1, "a", [true, null], {}]),
            json!({ "b": 1, "2": [ { "__proto__": { "x": 1 }, "y": "z" } ], "1": null, "__proto__": 5 }),
        ];
        for c in cases {
            let a = parse_val(&fast, Some(&c), &ParseOptions::v4()).unwrap();
            let b = parse_val(&slow, Some(&c), &ParseOptions::v4()).unwrap();
            assert_eq!(a, b, "{c}");
            assert_eq!(serde_json::to_string(&a.to_json()).unwrap(), serde_json::to_string(&b.to_json()).unwrap());
        }
        let mut deep = json!(1);
        for i in 0..10 {
            deep = if i % 2 == 0 { json!([0, deep]) } else { json!({ "k": deep, "j": 1 }) };
        }
        let opts = ParseOptions::v4().max_depth(6);
        let a = parse_val(&fast, Some(&deep), &opts).unwrap_err().to_json();
        let b = parse_val(&slow, Some(&deep), &opts).unwrap_err().to_json();
        assert_eq!(a, b);
    }
}

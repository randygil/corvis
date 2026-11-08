//! `ZodError` / `ZodIssue` with zod v3 and v4 compatible codes, messages and JSON shape,
//! plus the v4 error utilities (`flattenError`, `treeifyError`, `prettifyError`, `formatError`).

use std::fmt;

use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Map, Value, json};

/// `ZodIssueCode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ZodIssueCode {
    InvalidType,
    InvalidLiteral,
    #[default]
    Custom,
    InvalidUnion,
    InvalidUnionDiscriminator,
    InvalidEnumValue,
    UnrecognizedKeys,
    InvalidArguments,
    InvalidReturnType,
    InvalidDate,
    InvalidString,
    TooSmall,
    TooBig,
    InvalidIntersectionTypes,
    NotMultipleOf,
    NotFinite,
    /// v4: `z.enum` / `z.literal` mismatch.
    InvalidValue,
    /// v4: string formats (`email`, `uuid`, `regex`, `starts_with`, ...).
    InvalidFormat,
    /// v4: invalid record / map key.
    InvalidKey,
    /// v4: invalid map / set element.
    InvalidElement,
}

impl ZodIssueCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ZodIssueCode::InvalidType => "invalid_type",
            ZodIssueCode::InvalidLiteral => "invalid_literal",
            ZodIssueCode::Custom => "custom",
            ZodIssueCode::InvalidUnion => "invalid_union",
            ZodIssueCode::InvalidUnionDiscriminator => "invalid_union_discriminator",
            ZodIssueCode::InvalidEnumValue => "invalid_enum_value",
            ZodIssueCode::UnrecognizedKeys => "unrecognized_keys",
            ZodIssueCode::InvalidArguments => "invalid_arguments",
            ZodIssueCode::InvalidReturnType => "invalid_return_type",
            ZodIssueCode::InvalidDate => "invalid_date",
            ZodIssueCode::InvalidString => "invalid_string",
            ZodIssueCode::TooSmall => "too_small",
            ZodIssueCode::TooBig => "too_big",
            ZodIssueCode::InvalidIntersectionTypes => "invalid_intersection_types",
            ZodIssueCode::NotMultipleOf => "not_multiple_of",
            ZodIssueCode::NotFinite => "not_finite",
            ZodIssueCode::InvalidValue => "invalid_value",
            ZodIssueCode::InvalidFormat => "invalid_format",
            ZodIssueCode::InvalidKey => "invalid_key",
            ZodIssueCode::InvalidElement => "invalid_element",
        }
    }

    /// Inverse of [`ZodIssueCode::as_str`] (unknown codes map to `Custom`).
    pub fn parse(s: &str) -> ZodIssueCode {
        match s {
            "invalid_type" => ZodIssueCode::InvalidType,
            "invalid_literal" => ZodIssueCode::InvalidLiteral,
            "invalid_union" => ZodIssueCode::InvalidUnion,
            "invalid_union_discriminator" => ZodIssueCode::InvalidUnionDiscriminator,
            "invalid_enum_value" => ZodIssueCode::InvalidEnumValue,
            "unrecognized_keys" => ZodIssueCode::UnrecognizedKeys,
            "invalid_arguments" => ZodIssueCode::InvalidArguments,
            "invalid_return_type" => ZodIssueCode::InvalidReturnType,
            "invalid_date" => ZodIssueCode::InvalidDate,
            "invalid_string" => ZodIssueCode::InvalidString,
            "too_small" => ZodIssueCode::TooSmall,
            "too_big" => ZodIssueCode::TooBig,
            "invalid_intersection_types" => ZodIssueCode::InvalidIntersectionTypes,
            "not_multiple_of" => ZodIssueCode::NotMultipleOf,
            "not_finite" => ZodIssueCode::NotFinite,
            "invalid_value" => ZodIssueCode::InvalidValue,
            "invalid_format" => ZodIssueCode::InvalidFormat,
            "invalid_key" => ZodIssueCode::InvalidKey,
            "invalid_element" => ZodIssueCode::InvalidElement,
            _ => ZodIssueCode::Custom,
        }
    }
}

impl fmt::Display for ZodIssueCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single validation problem. v3 issues serialize as `{ code, ...params, path, message }`;
/// v4 issues keep the exact key order zod 4 produces (`{ expected, code, path, message }`, ...).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ZodIssue {
    pub code: ZodIssueCode,
    /// Path to the offending value (`["address", "zip"]`, `["tags", 0]`).
    pub path: Vec<Value>,
    pub message: String,
    /// Extra fields (`expected`, `received`, `minimum`, `inclusive`, `keys`, ...).
    pub params: Map<String, Value>,
    /// v4 key order (`"code"`, `"path"`, `"message"` and the params keys). `None` = v3 layout.
    pub layout: Option<Vec<String>>,
    /// Explicit v3 key order (`ctx.addIssue({...})`); otherwise it follows zod 3 per issue code.
    pub(crate) v3_layout: Option<Vec<String>>,
}

/// `ZodParsedType` values (zod 3): an `expected` outside this set comes from an enum.
const PARSED_TYPES: &[&str] = &[
    "string",
    "nan",
    "number",
    "integer",
    "float",
    "boolean",
    "date",
    "bigint",
    "symbol",
    "function",
    "undefined",
    "null",
    "array",
    "object",
    "unknown",
    "promise",
    "void",
    "never",
    "map",
    "set",
];

impl ZodIssue {
    /// Builds a v3-style issue (`{ code, ...params, path, message }`).
    pub fn new(code: ZodIssueCode, path: Vec<Value>, message: impl Into<String>, params: Map<String, Value>) -> Self {
        ZodIssue { code, path, message: message.into(), params, layout: None, v3_layout: None }
    }

    /// Key order of a zod 3 issue: `{ ...issueData, path, message }` where `issueData` is the
    /// object zod builds at each call site (check issues carry `message` in it).
    fn v3_keys(&self) -> Vec<String> {
        if let Some(l) = &self.v3_layout {
            return l.clone();
        }
        let p = &self.params;
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        let bound = if p.contains_key("minimum") { "minimum" } else { "maximum" };
        let pos = |k: &str| p.keys().position(|x| x == k);
        let listed: Vec<&str> = match self.code {
            ZodIssueCode::InvalidType if s("expected") == Some("integer") && s("received") == Some("float") => {
                vec!["code", "expected", "received", "message", "path"]
            }
            ZodIssueCode::InvalidType if s("expected").is_some_and(|e| !PARSED_TYPES.contains(&e)) => {
                vec!["expected", "received", "code", "path", "message"]
            }
            ZodIssueCode::InvalidLiteral => vec!["received", "code", "expected", "path", "message"],
            ZodIssueCode::InvalidEnumValue => vec!["received", "code", "options", "path", "message"],
            ZodIssueCode::InvalidString => match s("validation") {
                Some(v) if !matches!(v, "datetime" | "date" | "time") => vec!["validation", "code", "message", "path"],
                _ => vec!["code", "validation", "message", "path"],
            },
            ZodIssueCode::TooSmall | ZodIssueCode::TooBig => match s("type") {
                Some("date") => vec!["code", "message", "inclusive", "exact", bound, "type", "path"],
                Some("bigint") => vec!["code", "type", bound, "inclusive", "message", "path"],
                // tuples: `{ code, minimum, inclusive, exact, type }` without `message`.
                Some("array") if pos("inclusive") < pos("type") => vec!["code", bound, "inclusive", "exact", "type", "path", "message"],
                _ => vec!["code", bound, "type", "inclusive", "exact", "message", "path"],
            },
            ZodIssueCode::NotMultipleOf => vec!["code", "multipleOf", "message", "path"],
            ZodIssueCode::NotFinite | ZodIssueCode::Custom => vec!["code", "message", "path"],
            _ => vec!["code", "path", "message"],
        };
        let mut keys: Vec<String> =
            listed.iter().filter(|k| matches!(**k, "code" | "path" | "message") || p.contains_key(**k)).map(|k| k.to_string()).collect();
        // Params without a fixed position go right after the listed data keys.
        let extra: Vec<String> = p.keys().filter(|k| !listed.contains(&k.as_str())).cloned().collect();
        let at = keys.iter().position(|k| k == "message" || k == "path").unwrap_or(keys.len());
        keys.splice(at..at, extra);
        keys
    }

    /// `true` for issues produced with zod v4 semantics.
    pub fn is_v4(&self) -> bool {
        self.layout.is_some()
    }

    /// `path.join(".")` (`tags.0.name`).
    pub fn path_string(&self) -> String {
        self.path
            .iter()
            .map(|p| match p {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(".")
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Parses an issue from its JSON form (the nested `errors` of `invalid_union`, ...).
    pub fn from_json(v: &Value) -> Option<ZodIssue> {
        let obj = v.as_object()?;
        let code = ZodIssueCode::parse(obj.get("code")?.as_str()?);
        let path = obj.get("path").and_then(Value::as_array).cloned().unwrap_or_default();
        let message = obj.get("message").and_then(Value::as_str).unwrap_or_default().to_string();
        let params =
            obj.iter().filter(|(k, _)| !matches!(k.as_str(), "code" | "path" | "message")).map(|(k, v)| (k.clone(), v.clone())).collect();
        Some(ZodIssue { code, path, message, params, layout: Some(obj.keys().cloned().collect()), v3_layout: None })
    }

    fn nested(&self, key: &str) -> Vec<ZodIssue> {
        self.params.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(ZodIssue::from_json).collect()).unwrap_or_default()
    }
}

impl Serialize for ZodIssue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let v3;
        let layout = match &self.layout {
            Some(l) => l,
            None => {
                v3 = self.v3_keys();
                &v3
            }
        };
        let mut map = serializer.serialize_map(Some(layout.len()))?;
        for key in layout {
            match key.as_str() {
                "code" => map.serialize_entry("code", self.code.as_str())?,
                "path" => map.serialize_entry("path", &self.path)?,
                "message" => map.serialize_entry("message", &self.message)?,
                k => {
                    if let Some(v) = self.params.get(k) {
                        map.serialize_entry(k, v)?;
                    }
                }
            }
        }
        map.end()
    }
}

/// `ZodError`: the list of issues found while parsing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ZodError {
    pub issues: Vec<ZodIssue>,
}

fn key_of(seg: &Value) -> String {
    match seg {
        Value::String(s) => s.clone(),
        Value::Number(n) => super::js::num_to_string(n.as_f64().unwrap_or(f64::NAN)),
        other => other.to_string(),
    }
}

impl ZodError {
    pub fn new(issues: Vec<ZodIssue>) -> Self {
        ZodError { issues }
    }

    /// `error.errors` (alias of `issues`).
    pub fn errors(&self) -> &[ZodIssue] {
        &self.issues
    }

    /// `error.isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.issues.is_empty()
    }

    /// `error.addIssue(issue)`.
    pub fn add_issue(&mut self, issue: ZodIssue) {
        self.issues.push(issue);
    }

    /// Just the messages, in order.
    pub fn messages(&self) -> Vec<String> {
        self.issues.iter().map(|i| i.message.clone()).collect()
    }

    /// `error.message`: `JSON.stringify(issues, null, 2)`.
    pub fn message(&self) -> String {
        self.to_string()
    }

    /// `error.flatten()` / `z.flattenError(error)` →
    /// `{ formErrors: [...], fieldErrors: { name: [...] } }` (grouped by the first path segment).
    pub fn flatten(&self) -> Value {
        self.flatten_with(|i| Value::String(i.message.clone()))
    }

    /// `z.flattenError(error, mapper)`.
    pub fn flatten_with(&self, mapper: impl Fn(&ZodIssue) -> Value) -> Value {
        let mut form = Vec::new();
        let mut fields: Map<String, Value> = Map::new();
        for issue in &self.issues {
            match issue.path.first() {
                None => form.push(mapper(issue)),
                Some(first) => {
                    let key = if issue.is_v4() {
                        key_of(first)
                    } else {
                        match first {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    };
                    let entry = fields.entry(key).or_insert_with(|| Value::Array(Vec::new()));
                    if let Value::Array(a) = entry {
                        a.push(mapper(issue));
                    }
                }
            }
        }
        super::js::js_key_order(json!({ "formErrors": form, "fieldErrors": fields }))
    }

    /// `error.format()` → nested `{ _errors: [], name: { _errors: ["..."] } }`.
    /// Deprecated in zod v4 in favour of [`ZodError::treeify`].
    pub fn format(&self) -> Value {
        if self.issues.iter().any(ZodIssue::is_v4) {
            return self.format_with(|i| Value::String(i.message.clone()));
        }
        let mut root = json!({ "_errors": [] });
        for issue in &self.issues {
            let mut node = &mut root;
            for seg in &issue.path {
                let key = match seg {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                let obj = node.as_object_mut().expect("format node is an object");
                node = obj.entry(key).or_insert_with(|| json!({ "_errors": [] }));
            }
            if let Some(Value::Array(errs)) = node.get_mut("_errors") {
                errs.push(Value::String(issue.message.clone()));
            }
        }
        super::js::js_key_order(root)
    }

    /// `z.formatError(error, mapper)` (v4 algorithm: recurses into union / key / element errors).
    pub fn format_with(&self, mapper: impl Fn(&ZodIssue) -> Value) -> Value {
        fn process(issues: &[ZodIssue], base: &[Value], root: &mut Value, mapper: &dyn Fn(&ZodIssue) -> Value) {
            for issue in issues {
                let full: Vec<Value> = base.iter().chain(issue.path.iter()).cloned().collect();
                match issue.code {
                    ZodIssueCode::InvalidUnion if issue.params.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) => {
                        for group in issue.params["errors"].as_array().into_iter().flatten() {
                            let sub: Vec<ZodIssue> = group.as_array().into_iter().flatten().filter_map(ZodIssue::from_json).collect();
                            process(&sub, &full, root, mapper);
                        }
                    }
                    ZodIssueCode::InvalidKey | ZodIssueCode::InvalidElement => {
                        process(&issue.nested("issues"), &full, root, mapper);
                    }
                    _ => {
                        let mut node = &mut *root;
                        for seg in &full {
                            let obj = node.as_object_mut().expect("format node is an object");
                            node = obj.entry(key_of(seg)).or_insert_with(|| json!({ "_errors": [] }));
                        }
                        if let Some(Value::Array(errs)) = node.get_mut("_errors") {
                            errs.push(mapper(issue));
                        }
                    }
                }
            }
        }
        let mut root = json!({ "_errors": [] });
        process(&self.issues, &[], &mut root, &mapper);
        super::js::js_key_order(root)
    }

    /// `z.treeifyError(error)` → `{ errors: [], properties: { name: { errors: [...] } }, items: [...] }`.
    pub fn treeify(&self) -> Value {
        self.treeify_with(|i| Value::String(i.message.clone()))
    }

    /// `z.treeifyError(error, mapper)`.
    pub fn treeify_with(&self, mapper: impl Fn(&ZodIssue) -> Value) -> Value {
        fn process(issues: &[ZodIssue], base: &[Value], root: &mut Value, mapper: &dyn Fn(&ZodIssue) -> Value) {
            for issue in issues {
                let full: Vec<Value> = base.iter().chain(issue.path.iter()).cloned().collect();
                match issue.code {
                    ZodIssueCode::InvalidUnion if issue.params.get("errors").and_then(Value::as_array).is_some_and(|e| !e.is_empty()) => {
                        for group in issue.params["errors"].as_array().into_iter().flatten() {
                            let sub: Vec<ZodIssue> = group.as_array().into_iter().flatten().filter_map(ZodIssue::from_json).collect();
                            process(&sub, &full, root, mapper);
                        }
                    }
                    ZodIssueCode::InvalidKey | ZodIssueCode::InvalidElement => {
                        process(&issue.nested("issues"), &full, root, mapper);
                    }
                    _ => {
                        let mut node = &mut *root;
                        for seg in &full {
                            let obj = node.as_object_mut().expect("tree node is an object");
                            node = match seg {
                                Value::Number(n) => {
                                    let idx = n.as_u64().unwrap_or(0) as usize;
                                    let items = obj.entry("items").or_insert_with(|| json!([]));
                                    let arr = items.as_array_mut().expect("items is an array");
                                    while arr.len() <= idx {
                                        arr.push(Value::Null);
                                    }
                                    if arr[idx].is_null() {
                                        arr[idx] = json!({ "errors": [] });
                                    }
                                    &mut arr[idx]
                                }
                                other => {
                                    let props = obj.entry("properties").or_insert_with(|| json!({}));
                                    let props = props.as_object_mut().expect("properties is an object");
                                    props.entry(key_of(other)).or_insert_with(|| json!({ "errors": [] }))
                                }
                            };
                        }
                        if let Some(Value::Array(errs)) = node.get_mut("errors") {
                            errs.push(mapper(issue));
                        }
                    }
                }
            }
        }
        let mut root = json!({ "errors": [] });
        process(&self.issues, &[], &mut root, &mapper);
        super::js::js_key_order(root)
    }

    /// `z.prettifyError(error)`:
    ///
    /// ```text
    /// ✖ Invalid input: expected string, received number
    ///   → at name
    /// ```
    pub fn prettify(&self) -> String {
        let mut issues: Vec<&ZodIssue> = self.issues.iter().collect();
        issues.sort_by_key(|i| i.path.len());
        let mut lines = Vec::new();
        for issue in issues {
            lines.push(format!("✖ {}", issue.message));
            if !issue.path.is_empty() {
                lines.push(format!("  → at {}", to_dot_path(&issue.path)));
            }
        }
        lines.join("\n")
    }

    pub fn to_json(&self) -> Value {
        Value::Array(self.issues.iter().map(ZodIssue::to_json).collect())
    }
}

/// `z.core.toDotPath(path)`.
pub fn to_dot_path(path: &[Value]) -> String {
    let mut segs = String::new();
    for seg in path {
        match seg {
            Value::Number(n) => segs.push_str(&format!("[{}]", super::js::num_to_string(n.as_f64().unwrap_or(f64::NAN)))),
            Value::String(s) => {
                if s.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')) {
                    segs.push_str(&format!("[{}]", serde_json::to_string(s).unwrap_or_default()));
                } else {
                    if !segs.is_empty() {
                        segs.push('.');
                    }
                    segs.push_str(s);
                }
            }
            other => segs.push_str(&format!("[{other}]")),
        }
    }
    segs
}

/// `z.flattenError(error)`.
pub fn flatten_error(error: &ZodError) -> Value {
    error.flatten()
}

/// `z.treeifyError(error)`.
pub fn treeify_error(error: &ZodError) -> Value {
    error.treeify()
}

/// `z.prettifyError(error)`.
pub fn prettify_error(error: &ZodError) -> String {
    error.prettify()
}

/// `z.formatError(error)`.
pub fn format_error(error: &ZodError) -> Value {
    error.format()
}

impl Serialize for ZodError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.issues.serialize(serializer)
    }
}

impl fmt::Display for ZodError {
    /// Like zod, the message is the pretty-printed list of issues.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = serde_json::to_string_pretty(&self.issues).map_err(|_| fmt::Error)?;
        f.write_str(&s)
    }
}

impl std::error::Error for ZodError {}

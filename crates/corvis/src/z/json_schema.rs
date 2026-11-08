//! `z.toJSONSchema()`: a port of `zod/v4/core/to-json-schema` and its processors, plus
//! registries (`z.registry()`, `z.globalRegistry`).
//!
//! Like zod, nodes are processed once (the `seen` map), shared nodes are referenced by
//! placeholders and cycles / ids / reused schemas are extracted into `$defs`.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use serde_json::{Map, Value, json};

use super::Schema;

// ---------------------------------------------------------------------------
// Registries.
// ---------------------------------------------------------------------------

#[derive(Default)]
struct RegistryInner {
    entries: Vec<(usize, Schema, Map<String, Value>)>,
}

/// `z.registry()`: metadata attached to schemas (`.register(registry, meta)`).
#[derive(Clone, Default)]
pub struct Registry {
    inner: Arc<RwLock<RegistryInner>>,
    global: bool,
}

static GLOBAL: LazyLock<Registry> = LazyLock::new(|| Registry { inner: Arc::default(), global: true });

/// `z.globalRegistry` (`.meta()` / `.describe()` metadata).
pub fn global_registry() -> &'static Registry {
    &GLOBAL
}

impl Registry {
    /// `z.registry()`.
    pub fn new() -> Self {
        Registry::default()
    }

    /// `registry.add(schema, meta)`.
    pub fn add(&self, schema: &Schema, meta: Value) -> &Self {
        let meta = match meta {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        let id = schema.id();
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if let Some(slot) = inner.entries.iter_mut().find(|(i, _, _)| *i == id) {
            slot.2 = meta;
        } else {
            inner.entries.push((id, schema.clone(), meta));
        }
        drop(inner);
        self
    }

    /// `registry.remove(schema)`.
    pub fn remove(&self, schema: &Schema) -> &Self {
        let id = schema.id();
        self.inner.write().unwrap_or_else(|e| e.into_inner()).entries.retain(|(i, _, _)| *i != id);
        self
    }

    /// `registry.clear()`.
    pub fn clear(&self) -> &Self {
        self.inner.write().unwrap_or_else(|e| e.into_inner()).entries.clear();
        self
    }

    /// `registry.has(schema)`.
    pub fn has(&self, schema: &Schema) -> bool {
        self.own(schema).is_some()
    }

    fn lookup(&self, id: usize) -> Option<Map<String, Value>> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.entries.iter().find(|(i, _, _)| *i == id).map(|(_, _, m)| m.clone())
    }

    fn own(&self, schema: &Schema) -> Option<Map<String, Value>> {
        let reg = self.lookup(schema.id());
        if self.global {
            match (reg, schema.node().metadata()) {
                (Some(mut r), Some(m)) => {
                    for (k, v) in m {
                        r.entry(k).or_insert(v);
                    }
                    Some(r)
                }
                (r, m) => r.or(m),
            }
        } else {
            reg
        }
    }

    /// `registry.get(schema)`: own metadata merged over the parent's (without `id`).
    pub fn get(&self, schema: &Schema) -> Option<Map<String, Value>> {
        match schema.node().parent() {
            Some(p) => {
                let mut f = self.get(&p).unwrap_or_default();
                f.shift_remove("id");
                if let Some(own) = self.own(schema) {
                    for (k, v) in own {
                        f.insert(k, v);
                    }
                }
                if f.is_empty() { None } else { Some(f) }
            }
            None => self.own(schema),
        }
    }

    /// Registered schemas that carry an `id` (`registry._idmap`).
    pub fn schemas_by_id(&self) -> Vec<(String, Schema)> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .entries
            .iter()
            .filter_map(|(_, s, m)| m.get("id").and_then(Value::as_str).map(|id| (id.to_string(), s.clone())))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Options.
// ---------------------------------------------------------------------------

/// `override` callback: `(zodSchema, jsonSchema, path)`.
pub type JsonSchemaOverride = Arc<dyn Fn(&Schema, &mut Map<String, Value>, &[Value]) + Send + Sync>;

/// Parameters of `z.toJSONSchema(schema, params)`.
#[derive(Clone)]
pub struct JsonSchemaOptions {
    /// `"draft-2020-12"` (default), `"draft-07"`, `"draft-04"`, `"openapi-3.0"`.
    pub target: String,
    /// `"output"` (default) or `"input"`.
    pub io: String,
    /// `"throw"` (default) or `"any"`.
    pub unrepresentable: String,
    /// `"ref"` (default) or `"throw"`.
    pub cycles: String,
    /// `"inline"` (default) or `"ref"`.
    pub reused: String,
    /// `metadata` registry (default: the global registry).
    pub metadata: Option<Registry>,
    pub override_fn: Option<JsonSchemaOverride>,
}

impl Default for JsonSchemaOptions {
    fn default() -> Self {
        JsonSchemaOptions {
            target: "draft-2020-12".into(),
            io: "output".into(),
            unrepresentable: "throw".into(),
            cycles: "ref".into(),
            reused: "inline".into(),
            metadata: None,
            override_fn: None,
        }
    }
}

impl JsonSchemaOptions {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn target(mut self, t: impl Into<String>) -> Self {
        let t = t.into();
        self.target = match t.as_str() {
            "draft-4" => "draft-04".into(),
            "draft-7" => "draft-07".into(),
            _ => t,
        };
        self
    }
    pub fn io(mut self, io: impl Into<String>) -> Self {
        self.io = io.into();
        self
    }
    pub fn input(self) -> Self {
        self.io("input")
    }
    pub fn unrepresentable(mut self, u: impl Into<String>) -> Self {
        self.unrepresentable = u.into();
        self
    }
    pub fn cycles(mut self, c: impl Into<String>) -> Self {
        self.cycles = c.into();
        self
    }
    pub fn reused(mut self, r: impl Into<String>) -> Self {
        self.reused = r.into();
        self
    }
    pub fn metadata(mut self, r: Registry) -> Self {
        self.metadata = Some(r);
        self
    }
    pub fn override_with<F: Fn(&Schema, &mut Map<String, Value>, &[Value]) + Send + Sync + 'static>(mut self, f: F) -> Self {
        self.override_fn = Some(Arc::new(f));
        self
    }
}

// ---------------------------------------------------------------------------
// Generator.
// ---------------------------------------------------------------------------

const PLACEHOLDER: &str = "$__zseen";

#[derive(Clone, Copy, PartialEq)]
enum RefState {
    Unset,
    To(usize),
    Done,
}

struct Seen {
    node: Schema,
    schema: Map<String, Value>,
    count: usize,
    cycle: Option<Vec<Value>>,
    path: Vec<Value>,
    ref_: RefState,
    def: Option<Map<String, Value>>,
    def_id: Option<String>,
    parent: Option<usize>,
}

/// JSON Schema generation context (`ToJSONSchemaContext`), handed to [`ZodType::json_schema`](super::ZodType::json_schema).
pub struct JsonSchemaGen {
    opts: JsonSchemaOptions,
    registry: Registry,
    seen: Vec<Seen>,
    index: HashMap<usize, usize>,
    stack: Vec<usize>,
    counter: usize,
}

fn placeholder(i: usize) -> Value {
    json!({ PLACEHOLDER: i })
}

fn placeholder_index(v: &Value) -> Option<usize> {
    match v {
        Value::Object(m) if m.len() == 1 => m.get(PLACEHOLDER).and_then(Value::as_u64).map(|i| i as usize),
        _ => None,
    }
}

impl JsonSchemaGen {
    pub fn new(opts: JsonSchemaOptions) -> Self {
        let registry = opts.metadata.clone().unwrap_or_else(|| global_registry().clone());
        JsonSchemaGen { opts, registry, seen: Vec::new(), index: HashMap::new(), stack: Vec::new(), counter: 0 }
    }

    /// `ctx.target`.
    pub fn target(&self) -> &str {
        &self.opts.target
    }

    /// `ctx.io === "input"`.
    pub fn io_input(&self) -> bool {
        self.opts.io == "input"
    }

    /// Targets that cannot combine `$ref` with siblings and use `items` arrays.
    pub fn legacy_target(&self) -> bool {
        matches!(self.opts.target.as_str(), "draft-07" | "draft-04" | "openapi-3.0")
    }

    /// Reports an unrepresentable type (`throw` → error, `any` → `{}`).
    pub fn unrepresentable(&self, message: &str) -> Result<(), String> {
        if self.opts.unrepresentable == "throw" { Err(message.to_string()) } else { Ok(()) }
    }

    fn current(&self) -> usize {
        *self.stack.last().expect("json schema: no current schema")
    }

    /// `process(schema, ctx, params)`: returns a reference to the (shared) JSON Schema of `schema`.
    pub fn process(&mut self, schema: &Schema, path: &[Value]) -> Result<Value, String> {
        let id = schema.id();
        if let Some(&i) = self.index.get(&id) {
            self.seen[i].count += 1;
            if self.stack.contains(&i) {
                self.seen[i].cycle = Some(path.to_vec());
            }
            return Ok(placeholder(i));
        }
        let i = self.seen.len();
        self.seen.push(Seen {
            node: schema.clone(),
            schema: Map::new(),
            count: 1,
            cycle: None,
            path: path.to_vec(),
            ref_: RefState::Unset,
            def: None,
            def_id: None,
            parent: None,
        });
        self.index.insert(id, i);
        self.stack.push(i);
        let mut json = Map::new();
        let r = schema.node().json_schema(self, &mut json, path);
        self.stack.pop();
        r?;
        let mut out = json;
        if let Some(meta) = self.registry.get(schema) {
            for (k, v) in meta {
                out.insert(k, v);
            }
        }
        if self.io_input() && is_transforming(schema) {
            out.shift_remove("examples");
            out.shift_remove("default");
        }
        if self.io_input()
            && let Some(p) = out.get("_prefault").cloned() {
                out.entry("default").or_insert(p);
            }
        out.shift_remove("_prefault");
        self.seen[i].schema = out;
        Ok(placeholder(i))
    }

    fn idx(&self, schema: &Schema) -> usize {
        self.index[&schema.id()]
    }

    /// Processes `inner` and makes the current schema inherit from it (`seen.ref = innerType`).
    pub fn inherit(&mut self, inner: &Schema, path: &[Value]) -> Result<(), String> {
        self.process(inner, path)?;
        let cur = self.current();
        self.seen[cur].ref_ = RefState::To(self.idx(inner));
        Ok(())
    }

    /// `seen.ref = inner` for an already processed `inner`.
    pub fn set_ref(&mut self, inner: &Schema) {
        let cur = self.current();
        if let Some(&i) = self.index.get(&inner.id()) {
            self.seen[cur].ref_ = RefState::To(i);
        }
    }

    /// For clones with a `_zod.parent` (refinements, `.meta()`, `.describe()`): runs the base
    /// processor on `json`, then processes the parent and links to it.
    pub fn inherit_parent(&mut self, parent: &Schema, json: &mut Map<String, Value>, path: &[Value]) -> Result<(), String> {
        let mut base = parent.clone();
        while let Some(p) = base.node().parent() {
            base = p;
        }
        base.node().json_schema(self, json, path)?;
        let cur = self.current();
        self.process(parent, path)?;
        let pi = self.idx(parent);
        self.seen[cur].parent = Some(pi);
        if self.seen[cur].ref_ == RefState::Unset {
            self.seen[cur].ref_ = RefState::To(pi);
        }
        Ok(())
    }

    /// The current JSON of a processed schema reference (for inspection by processors).
    pub fn peek(&self, v: &Value) -> Value {
        match placeholder_index(v) {
            Some(i) => Value::Object(self.seen[i].schema.clone()),
            None => v.clone(),
        }
    }

    fn meta_id(&self, i: usize) -> Option<String> {
        self.registry.get(&self.seen[i].node).and_then(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
    }

    fn extract_defs(&mut self, root: usize) -> Result<(), String> {
        let mut ids: HashMap<String, usize> = HashMap::new();
        for i in 0..self.seen.len() {
            if let Some(id) = self.meta_id(i) {
                if let Some(&other) = ids.get(&id)
                    && self.seen[other].node.id() != self.seen[i].node.id() {
                        return Err(format!(
                            "Duplicate schema id \"{id}\" detected during JSON Schema conversion. Two different schemas cannot share the same id when converted together."
                        ));
                    }
                ids.insert(id, i);
            }
        }
        if self.opts.cycles == "throw" {
            for s in &self.seen {
                if let Some(c) = &s.cycle {
                    let p = c.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>();
                    return Err(format!(
                        "Cycle detected: #/{}/<root>\n\nSet the `cycles` parameter to `\"ref\"` to resolve cyclical schemas with defs.",
                        p.join("/")
                    ));
                }
            }
        }
        for i in 0..self.seen.len() {
            if i == root {
                self.extract_to_def(i, root);
                continue;
            }
            if self.meta_id(i).is_some() {
                self.extract_to_def(i, root);
                continue;
            }
            if self.seen[i].cycle.is_some() {
                self.extract_to_def(i, root);
                continue;
            }
            if self.seen[i].count > 1 && self.opts.reused == "ref" {
                self.extract_to_def(i, root);
            }
        }
        Ok(())
    }

    fn extract_to_def(&mut self, i: usize, root: usize) {
        if self.seen[i].schema.contains_key("$ref") {
            return;
        }
        let segment = if self.opts.target == "draft-2020-12" { "$defs" } else { "definitions" };
        let (r, def_id) = if i == root {
            ("#".to_string(), None)
        } else {
            let id = match self.seen[i].schema.get("id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => {
                    let id = format!("__schema{}", self.counter);
                    self.counter += 1;
                    id
                }
            };
            (format!("#/{segment}/{id}"), Some(id))
        };
        let s = &mut self.seen[i];
        s.def = Some(std::mem::take(&mut s.schema));
        if def_id.is_some() {
            s.def_id = def_id;
        }
        s.schema.insert("$ref".into(), Value::String(r));
    }

    fn flatten_ref(&mut self, i: usize) {
        if self.seen[i].ref_ == RefState::Done {
            return;
        }
        let in_def = self.seen[i].def.is_some();
        let mut schema = if in_def { self.seen[i].def.clone().unwrap() } else { self.seen[i].schema.clone() };
        let cached = schema.clone();
        let r = self.seen[i].ref_;
        self.seen[i].ref_ = RefState::Done;
        let mut ref_idx = None;
        if let RefState::To(r) = r {
            ref_idx = Some(r);
            self.flatten_ref(r);
            let ref_schema = self.seen[r].schema.clone();
            if ref_schema.contains_key("$ref") && self.legacy_target() {
                let all_of = schema.entry("allOf").or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(a) = all_of {
                    a.push(Value::Object(ref_schema.clone()));
                }
            } else {
                for (k, v) in &ref_schema {
                    schema.insert(k.clone(), v.clone());
                }
            }
            for (k, v) in &cached {
                schema.insert(k.clone(), v.clone());
            }
            if self.seen[i].parent == Some(r) {
                schema.retain(|k, _| k == "$ref" || k == "allOf" || cached.contains_key(k));
            }
            if ref_schema.contains_key("$ref")
                && let Some(def) = &self.seen[r].def {
                    let def = def.clone();
                    schema.retain(|k, v| k == "$ref" || k == "allOf" || def.get(k) != Some(v));
                }
        }
        if let Some(p) = self.seen[i].parent
            && Some(p) != ref_idx {
                self.flatten_ref(p);
                if let Some(pr) = self.seen[p].schema.get("$ref").cloned() {
                    schema.insert("$ref".into(), pr);
                    if let Some(def) = self.seen[p].def.clone() {
                        schema.retain(|k, v| k == "$ref" || k == "allOf" || def.get(k) != Some(v));
                    }
                }
            }
        if let Some(f) = self.opts.override_fn.clone() {
            let node = self.seen[i].node.clone();
            let path = self.seen[i].path.clone();
            f(&node, &mut schema, &path);
        }
        if in_def {
            self.seen[i].def = Some(schema);
        } else {
            self.seen[i].schema = schema;
        }
    }

    fn resolve(&self, v: &Value, depth: usize) -> Result<Value, String> {
        if depth > 512 {
            return Err("Error converting schema to JSON.".into());
        }
        if let Some(i) = placeholder_index(v) {
            return self.resolve(&Value::Object(self.seen[i].schema.clone()), depth + 1);
        }
        Ok(match v {
            Value::Object(m) => {
                let mut out = Map::new();
                for (k, v) in m {
                    out.insert(k.clone(), self.resolve(v, depth + 1)?);
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.iter().map(|v| self.resolve(v, depth + 1)).collect::<Result<_, _>>()?),
            v => v.clone(),
        })
    }

    fn finalize(&mut self, root: usize, root_schema: &Schema) -> Result<Value, String> {
        for i in (0..self.seen.len()).rev() {
            self.flatten_ref(i);
        }
        let mut result = Map::new();
        match self.opts.target.as_str() {
            "draft-2020-12" => {
                result.insert("$schema".into(), json!("https://json-schema.org/draft/2020-12/schema"));
            }
            "draft-07" => {
                result.insert("$schema".into(), json!("http://json-schema.org/draft-07/schema#"));
            }
            "draft-04" => {
                result.insert("$schema".into(), json!("http://json-schema.org/draft-04/schema#"));
            }
            _ => {}
        }
        let body = self.seen[root].def.clone().unwrap_or_else(|| self.seen[root].schema.clone());
        for (k, v) in body {
            result.insert(k, v);
        }
        let root_meta_id = self.registry.get(root_schema).and_then(|m| m.get("id").cloned());
        if root_meta_id.is_some() && result.get("id") == root_meta_id.as_ref() {
            result.shift_remove("id");
        }
        let mut defs = Map::new();
        for s in &mut self.seen {
            if let (Some(def), Some(def_id)) = (&mut s.def, &s.def_id) {
                if def.get("id").and_then(Value::as_str) == Some(def_id.as_str()) {
                    def.shift_remove("id");
                }
                defs.insert(def_id.clone(), Value::Object(def.clone()));
            }
        }
        if !defs.is_empty() {
            let key = if self.opts.target == "draft-2020-12" { "$defs" } else { "definitions" };
            result.insert(key.into(), Value::Object(defs));
        }
        self.resolve(&Value::Object(result), 0)
    }
}

/// `isTransforming(schema)`.
pub(crate) fn is_transforming(schema: &Schema) -> bool {
    schema.node().is_transforming()
}

/// `z.toJSONSchema(schema, params)`.
pub fn to_json_schema(schema: &Schema, opts: &JsonSchemaOptions) -> Result<Value, String> {
    let mut jsg = JsonSchemaGen::new(opts.clone());
    jsg.process(schema, &[])?;
    let root = jsg.idx(schema);
    jsg.extract_defs(root)?;
    jsg.finalize(root, schema)
}

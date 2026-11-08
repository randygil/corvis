//! zod DTO integration for OpenAPI: the metadata factory of zod DTOs
//! (`_OPENAPI_METADATA_FACTORY`) and [`cleanup_open_api_doc`].
//!
//! The JSON Schema of a zod DTO comes from [`corvis::z::to_json_schema`] (zod 4
//! `z.toJSONSchema(schema, { io })`).

use corvis::z::{JsonSchemaOptions, Schema};
use serde_json::{Value, json};

use crate::document::OpenApiDocument;
use crate::js::{self, Obj};
use crate::types::ZodIo;

const PREFIX: &str = "x-corvis_zod";

pub const EMPTY_TYPE_KEY: &str = "x-corvis_zod-empty-type";
pub const DEFS_KEY: &str = "x-corvis_zod-$defs";
pub const PARENT_ID_KEY: &str = "x-corvis_zod-parent-id";
pub const PARENT_ADDITIONAL_PROPERTIES_KEY: &str = "x-corvis_zod-parent-additional-properties";
pub const PARENT_HAS_REFS_KEY: &str = "x-corvis_zod-parent-has-refs";
pub const UNWRAP_ROOT_KEY: &str = "x-corvis_zod-unwrap-root";
pub const USES_THREE_POINT_ONE_SYNTAX_KEY: &str = "x-corvis_zod-uses-3-point-1-syntax";
pub const SELF_REQUIRED_KEY: &str = "x-corvis_zod-self-required";
pub const PARENT_METADATA_KEY: &str = "x-corvis_zod-parent-metadata";

#[allow(dead_code)]
const _PREFIX_IS_USED: &str = PREFIX;

// --- JSON Schema walking ------------------------------------------------------------------

/// `walkJsonSchema(schema, callback)` (the callback may replace the node).
pub fn walk_json_schema(schema: Value, cb: &mut dyn FnMut(Value) -> Result<Value, String>) -> Result<Value, String> {
    let mut schema = cb(schema)?;
    let Value::Object(o) = &mut schema else {
        return Ok(schema);
    };
    let ty = o.get("type").and_then(Value::as_str).map(str::to_string);
    if ty.as_deref() == Some("object")
        && let Some(Value::Object(props)) = o.get_mut("properties")
    {
        let keys: Vec<String> = props.keys().cloned().collect();
        for k in keys {
            let v = props.get(&k).cloned().unwrap_or(Value::Null);
            props.insert(k, walk_json_schema(v, cb)?);
        }
    }
    if ty.as_deref() == Some("array") {
        if let Some(Value::Array(items)) = o.get("items").cloned() {
            let mut out = Vec::new();
            for i in items {
                out.push(walk_json_schema(i, cb)?);
            }
            o.insert("items".into(), Value::Array(out));
        }
        if let Some(items) = o.get("items").cloned().filter(|i| js::truthy(Some(i))) {
            o.insert("items".into(), walk_json_schema(items, cb)?);
        }
    }
    for comb in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(list)) = o.get(comb).cloned() {
            let mut out = Vec::new();
            for i in list {
                out.push(walk_json_schema(i, cb)?);
            }
            o.insert(comb.into(), Value::Array(out));
        }
    }
    if let Some(ap @ Value::Object(_)) = o.get("additionalProperties").cloned() {
        o.insert("additionalProperties".into(), walk_json_schema(ap, cb)?);
    }
    if let Some(pn @ Value::Object(_)) = o.get("propertyNames").cloned() {
        o.insert("propertyNames".into(), walk_json_schema(pn, cb)?);
    }
    Ok(schema)
}

fn walk(schema: Value, mut cb: impl FnMut(&mut Obj) -> Result<Option<Value>, String>) -> Result<Value, String> {
    walk_json_schema(schema, &mut |mut v| {
        if let Value::Object(o) = &mut v
            && let Some(replacement) = cb(o)?
        {
            return Ok(replacement);
        }
        Ok(v)
    })
}

/// `fixAllRefs({ schema, defRenames, rootSchemaName })`.
pub fn fix_all_refs(schema: Value, def_renames: &Obj, root_schema_name: Option<&str>) -> Result<Value, String> {
    walk(schema, |s| {
        if let Some(r) = s.get("$ref").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string) {
            let mut r = r;
            if let Some(old) = r.strip_prefix("#/$defs/")
                && let Some(new) = def_renames.get(old).and_then(Value::as_str).filter(|n| !n.is_empty())
            {
                r = format!("#/$defs/{new}");
            }
            let mut r = r.replacen("#/$defs/", "#/components/schemas/", 1);
            if r == "#" {
                let Some(root) = root_schema_name else {
                    return Err("[fixAllRefs] rootSchemaName is required when fixing a ref to #".into());
                };
                r = format!("#/components/schemas/{root}");
            }
            s.insert("$ref".into(), Value::String(r));
        }
        Ok(None)
    })
}

/// `convertToOpenApi3Point0(schema)`: OpenAPI 3.1 JSON Schema -> OpenAPI 3.0 "subset superset".
pub fn convert_to_open_api_3_point_0(schema: Value) -> Value {
    walk(schema, |s| {
        js::delete(s, "id");
        if js::truthy(s.get("anyOf")) {
            let mut any_of = s.get("anyOf").and_then(Value::as_array).cloned().unwrap_or_default();
            let Some(idx) = any_of.iter().position(|x| x.get("type").and_then(Value::as_str) == Some("null")) else {
                return Ok(None);
            };
            any_of.remove(idx);
            let rest = js::omit(s, &["anyOf"]);
            if any_of.len() == 1 {
                let sole = any_of[0].clone();
                let sole_obj = js::obj_of(&sole);
                if js::truthy(sole_obj.get("$ref")) && sole_obj.len() == 1 {
                    let mut out = Obj::new();
                    out.insert("allOf".into(), json!([sole]));
                    out.insert("nullable".into(), Value::Bool(true));
                    js::assign(&mut out, &rest);
                    return Ok(Some(Value::Object(out)));
                }
                let mut out = js::spread(&[&sole_obj, &rest]);
                if let Some(Value::Array(values)) = sole_obj.get("enum")
                    && !values.contains(&Value::Null)
                {
                    let mut e = values.clone();
                    e.push(Value::Null);
                    out.insert("enum".into(), Value::Array(e));
                }
                out.insert("nullable".into(), Value::Bool(true));
                return Ok(Some(Value::Object(out)));
            }
            let mut out = rest;
            out.insert("anyOf".into(), Value::Array(any_of));
            out.insert("nullable".into(), Value::Bool(true));
            return Ok(Some(Value::Object(out)));
        }
        if let Some(c) = s.get("const").cloned() {
            s.insert("enum".into(), json!([c]));
            js::delete(s, "const");
        }
        js::delete(s, "propertyNames");
        if let Some(n @ Value::Number(_)) = s.get("exclusiveMinimum").cloned() {
            s.insert("minimum".into(), n);
            s.insert("exclusiveMinimum".into(), Value::Bool(true));
        }
        if let Some(n @ Value::Number(_)) = s.get("exclusiveMaximum").cloned() {
            s.insert("maximum".into(), n);
            s.insert("exclusiveMaximum".into(), Value::Bool(true));
        }
        Ok(None)
    })
    .unwrap_or(Value::Null)
}

/// `cleanupRefs(schema, io)`: `_Output` suffix on `$defs` refs for output DTOs, no `id`.
fn cleanup_refs(schema: Value, io: ZodIo) -> Value {
    walk(schema, |s| {
        if let Some(r) = s.get("$ref").and_then(Value::as_str).map(str::to_string)
            && let Some(key) = r.strip_prefix("#/$defs/")
            && !key.is_empty()
            && io == ZodIo::Output
        {
            s.insert("$ref".into(), Value::String(format!("#/$defs/{key}_Output")));
        }
        js::delete(s, "id");
        Ok(None)
    })
    .unwrap_or(Value::Null)
}

/// The JSON Schema of a zod schema (`z.toJSONSchema(schema, { io })`).
pub fn zod_json_schema(schema: &Schema, io: ZodIo) -> Value {
    let opts = JsonSchemaOptions::new().io(match io {
        ZodIo::Input => "input",
        ZodIo::Output => "output",
    });
    match corvis::z::to_json_schema(schema, &opts) {
        Ok(v) => v,
        Err(e) => {
            corvis::Logger::new("SwaggerModule").error(format!("[zod] z.toJSONSchema failed: {e}"));
            json!({})
        }
    }
}

/// `generateJsonSchema(schema, io)`.
fn generate_json_schema(schema: &Schema, io: ZodIo) -> Obj {
    let generated = zod_json_schema(schema, io);
    let defs = generated.get("$defs").filter(|d| js::truthy(Some(d))).cloned();
    let mut new_schema = js::obj_of(&cleanup_refs(generated, io));
    if let Some(Value::Object(defs)) = defs {
        let mut new_defs = Obj::new();
        for (k, v) in defs {
            let key = if io == ZodIo::Output { format!("{k}_Output") } else { k };
            if new_defs.contains_key(&key) {
                panic!("[zod] Duplicate id in $defs: {key}");
            }
            new_defs.insert(key, cleanup_refs(v, io));
        }
        new_schema.insert("$defs".into(), Value::Object(new_defs));
    }
    new_schema
}

fn schema_metadata(json_schema: &Value) -> (bool, bool) {
    let mut has_refs = false;
    let mut uses_31 = false;
    let _ = walk(json_schema.clone(), |s| {
        if s.get("type").and_then(Value::as_str) == Some("null")
            || js::truthy(s.get("const"))
            || js::truthy(s.get("id"))
            || s.contains_key("propertyNames")
            || s.get("exclusiveMinimum").is_some_and(Value::is_number)
            || s.get("exclusiveMaximum").is_some_and(Value::is_number)
        {
            uses_31 = true;
        }
        if js::truthy(s.get("$ref")) {
            has_refs = true;
        }
        Ok(None)
    });
    (has_refs, uses_31)
}

fn is_object_type_with_properties(s: &Obj) -> bool {
    s.get("type").and_then(Value::as_str) == Some("object") && s.get("properties").and_then(Value::as_object).is_some_and(|p| !p.is_empty())
}

/// `openApiMetadataFactory({ schema, io })`: the `@ApiProperty` metadata of every property.
pub fn open_api_metadata_factory(schema: &Schema, io: ZodIo) -> Vec<(String, Obj)> {
    let generated_full = generate_json_schema(schema, io);
    let defs = generated_full.get("$defs").cloned();
    let generated = js::omit(&generated_full, &["$defs", "$schema"]);
    let zod_id = corvis::z::global_registry().get(schema).and_then(|m| m.get("id").and_then(Value::as_str).map(str::to_string));
    let root_id = zod_id.map(|id| if io == ZodIo::Output { format!("{id}_Output") } else { id });

    let json_schema = if !is_object_type_with_properties(&generated) {
        let mut root = generated.clone();
        root.insert(UNWRAP_ROOT_KEY.into(), Value::Bool(true));
        let mut o = Obj::new();
        o.insert("type".into(), js::s("object"));
        if let Some(t) = generated.get("title") {
            o.insert("title".into(), t.clone());
        }
        let mut props = Obj::new();
        props.insert("root".into(), Value::Object(root));
        o.insert("properties".into(), Value::Object(props));
        if let Some(d) = &defs {
            o.insert("$defs".into(), d.clone());
        }
        o
    } else {
        let mut o = generated.clone();
        if let Some(d) = &defs {
            o.insert("$defs".into(), d.clone());
        }
        o
    };
    let (has_refs, uses_31) = schema_metadata(&Value::Object(json_schema.clone()));
    let required: Vec<Value> = json_schema.get("required").and_then(Value::as_array).cloned().unwrap_or_default();
    let reserved = ["type", "properties", "required", "additionalProperties", "$defs", "id"];
    let mut parent_metadata = Obj::new();
    for (k, v) in &json_schema {
        if !reserved.contains(&k.as_str()) {
            parent_metadata.insert(k.clone(), v.clone());
        }
    }

    let mut out = Vec::new();
    let props = json_schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, property_schema) in props {
        let ps = js::obj_of(&property_schema);
        let mut p = ps.clone();
        let t = match ps.get("type") {
            Some(t) if js::truthy(Some(t)) => t.clone(),
            _ => js::s(""),
        };
        p.insert("type".into(), t);
        if uses_31 {
            p.insert(USES_THREE_POINT_ONE_SYNTAX_KEY.into(), Value::Bool(true));
        }
        if has_refs {
            p.insert(PARENT_HAS_REFS_KEY.into(), Value::Bool(true));
        }
        if !ps.get("type").is_some_and(Value::is_string) {
            p.insert(EMPTY_TYPE_KEY.into(), Value::Bool(true));
        }
        let is_required = required.contains(&Value::String(key.clone()));
        if p.get("type").and_then(Value::as_str) == Some("object") {
            p.insert("selfRequired".into(), Value::Bool(is_required));
            p.insert(SELF_REQUIRED_KEY.into(), Value::Bool(is_required));
        } else {
            p.insert("required".into(), Value::Bool(is_required));
        }
        if let Some(d) = json_schema.get("$defs").filter(|d| js::truthy(Some(d))) {
            p.insert(DEFS_KEY.into(), d.clone());
        }
        if let Some(id) = &root_id {
            p.insert(PARENT_ID_KEY.into(), js::s(id.clone()));
        }
        if let Some(Value::Bool(b)) = json_schema.get("additionalProperties") {
            p.insert(PARENT_ADDITIONAL_PROPERTIES_KEY.into(), Value::Bool(*b));
        }
        if !parent_metadata.is_empty() {
            p.insert(PARENT_METADATA_KEY.into(), Value::Object(parent_metadata.clone()));
        }
        out.push((key, p));
    }
    out
}

// --- cleanupOpenApiDoc --------------------------------------------------------------------

/// OpenAPI version handling of [`cleanup_open_api_doc_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenApiVersion {
    /// From `doc.openapi` (`3.1.x` -> 3.1, otherwise 3.0).
    #[default]
    Auto,
    V3_0,
    V3_1,
}

fn register_schema(registry: &mut Obj, name: &str, schema: Value) -> Result<(), String> {
    if let Some(existing) = registry.get(name)
        && !js::deep_equal(existing, &schema)
    {
        return Err(format!(
            "[cleanupOpenApiDoc] Found multiple schemas with name `{name}`.  Please review your schemas to ensure that you are not using the same schema name for different schemas"
        ));
    }
    registry.insert(name.to_string(), schema);
    Ok(())
}

fn schema_name_from_ref(r: &str) -> &str {
    r.rsplit('/').next().unwrap_or(r)
}

fn fix_refs_in_bodies(method: &mut Obj, renames: &Obj) {
    let fix = |content: &mut Obj| {
        for (_, body) in content.iter_mut() {
            let Some(schema) = body.get_mut("schema").and_then(Value::as_object_mut) else { continue };
            let Some(r) = schema.get("$ref").and_then(Value::as_str).map(str::to_string) else { continue };
            let old = schema_name_from_ref(&r).to_string();
            if let Some(new) = renames.get(&old).and_then(Value::as_str) {
                schema.insert("$ref".into(), Value::String(r.replacen(&format!("/{old}"), &format!("/{new}"), 1)));
            }
        }
    };
    if let Some(Value::Object(rb)) = method.get_mut("requestBody")
        && let Some(Value::Object(content)) = rb.get_mut("content")
    {
        fix(content);
    }
    if let Some(Value::Object(responses)) = method.get_mut("responses") {
        for (_, r) in responses.iter_mut() {
            if let Some(Value::Object(content)) = r.get_mut("content") {
                fix(content);
            }
        }
    }
}

struct Cleaned {
    schema: Value,
    name: String,
    additional: Obj,
}

fn cleanup_schema(old_name: &str, old_schema: &Value, version: OpenApiVersion) -> Result<Cleaned, String> {
    let mut additional = Obj::new();
    if old_schema.get("type").and_then(Value::as_str) != Some("object") {
        return Ok(Cleaned { schema: old_schema.clone(), name: old_name.to_string(), additional });
    }
    let mut name = old_name.to_string();
    let mut added_defs = false;
    let mut has_refs = false;
    let mut uses_31 = false;
    let mut def_renames = Obj::new();
    let mut s = js::obj_of(old_schema);
    let prop_names: Vec<String> = s.get("properties").and_then(Value::as_object).map(|p| p.keys().cloned().collect()).unwrap_or_default();
    for prop_name in prop_names {
        let mut p = s["properties"][&prop_name].as_object().cloned().unwrap_or_default();
        if p.contains_key(SELF_REQUIRED_KEY) {
            if !js::truthy(p.get(SELF_REQUIRED_KEY))
                && let Some(Value::Array(req)) = s.get_mut("required")
            {
                if let Some(i) = req.iter().position(|v| v.as_str() == Some(prop_name.as_str())) {
                    req.remove(i);
                }
                if req.is_empty() {
                    js::delete(&mut s, "required");
                }
            }
            js::delete(&mut p, SELF_REQUIRED_KEY);
            js::delete(&mut p, "selfRequired");
        }
        if p.contains_key(USES_THREE_POINT_ONE_SYNTAX_KEY) {
            uses_31 = js::truthy(p.get(USES_THREE_POINT_ONE_SYNTAX_KEY));
            js::delete(&mut p, USES_THREE_POINT_ONE_SYNTAX_KEY);
        }
        if p.contains_key(PARENT_HAS_REFS_KEY) {
            has_refs = js::truthy(p.get(PARENT_HAS_REFS_KEY));
            js::delete(&mut p, PARENT_HAS_REFS_KEY);
        }
        if js::truthy(p.get(EMPTY_TYPE_KEY)) {
            js::delete(&mut p, EMPTY_TYPE_KEY);
            if p.get("type").and_then(Value::as_str) == Some("") {
                js::delete(&mut p, "type");
            }
        }
        if let Some(Value::String(id)) = p.get(PARENT_ID_KEY).cloned() {
            name = id;
            js::delete(&mut p, PARENT_ID_KEY);
        }
        if let Some(Value::Object(meta)) = p.get(PARENT_METADATA_KEY).cloned() {
            js::assign(&mut s, &meta);
            js::delete(&mut p, PARENT_METADATA_KEY);
        }
        if let Some(Value::Bool(b)) = p.get(PARENT_ADDITIONAL_PROPERTIES_KEY).cloned() {
            s.insert("additionalProperties".into(), Value::Bool(b));
            js::delete(&mut p, PARENT_ADDITIONAL_PROPERTIES_KEY);
        }
        if p.contains_key(DEFS_KEY) {
            let defs = js::delete(&mut p, DEFS_KEY).unwrap_or(Value::Null);
            if !added_defs {
                let defs = js::obj_of(&defs);
                for k in defs.keys() {
                    if k.starts_with("__schema") {
                        def_renames.insert(k.clone(), Value::String(format!("{name}{k}")));
                    }
                }
                for (k, d) in &defs {
                    let mut fixed = fix_all_refs(d.clone(), &def_renames, Some(&name))?;
                    if version == OpenApiVersion::V3_0 {
                        fixed = convert_to_open_api_3_point_0(fixed);
                    }
                    let key = def_renames.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(k).to_string();
                    register_schema(&mut additional, &key, fixed)?;
                }
                added_defs = true;
            }
        }
        if let Some(Value::Object(props)) = s.get_mut("properties") {
            props.insert(prop_name.clone(), Value::Object(p));
        }
    }
    if name != old_name && s.contains_key("id") {
        s.insert("id".into(), Value::String(name.clone()));
    }
    let mut schema = Value::Object(s);
    if has_refs {
        schema = fix_all_refs(schema, &def_renames, Some(&name))?;
    }
    if uses_31 && version == OpenApiVersion::V3_0 {
        schema = convert_to_open_api_3_point_0(schema);
    }
    let unwrap = schema
        .get("properties")
        .and_then(|p| p.get("root"))
        .and_then(Value::as_object)
        .filter(|r| r.contains_key(UNWRAP_ROOT_KEY))
        .cloned();
    if let Some(mut root) = unwrap {
        let replace = js::truthy(root.get(UNWRAP_ROOT_KEY));
        js::delete(&mut root, UNWRAP_ROOT_KEY);
        if replace {
            schema = Value::Object(root);
        } else if let Some(Value::Object(props)) = schema.get_mut("properties") {
            props.insert("root".into(), Value::Object(root));
        }
    }
    if let Some(existing) = additional.get(&name)
        && !js::deep_equal(existing, &schema)
    {
        return Err(format!(
            "[cleanupOpenApiDoc] Found multiple schemas with name `{name}`.  Please review your schemas to ensure that you are not using the same schema name for different schemas"
        ));
    }
    Ok(Cleaned { schema, name, additional })
}

fn fix_parameter(input: &Value, version: OpenApiVersion) -> Result<Obj, String> {
    let mut p = js::obj_of(input);
    let mut uses_31 = false;
    if p.contains_key(EMPTY_TYPE_KEY) {
        js::delete(&mut p, EMPTY_TYPE_KEY);
        if let Some(Value::Object(s)) = p.get_mut("schema") {
            js::delete(s, "type");
        }
    }
    if p.contains_key(UNWRAP_ROOT_KEY) {
        return Err("[cleanupOpenApiDoc] Query or url parameters must be an object type".into());
    }
    if p.contains_key("const") && p.get("schema").is_some_and(|s| js::truthy(Some(s))) {
        let c = p["const"].clone();
        if let Some(Value::Object(s)) = p.get_mut("schema") {
            s.insert("const".into(), c);
        }
        js::delete(&mut p, "const");
    }
    js::delete(&mut p, PARENT_ID_KEY);
    js::delete(&mut p, PARENT_METADATA_KEY);
    js::delete(&mut p, PARENT_ADDITIONAL_PROPERTIES_KEY);
    if p.contains_key(PARENT_HAS_REFS_KEY) {
        js::delete(&mut p, PARENT_HAS_REFS_KEY);
        if let Some(schema) = p.get("schema").cloned() {
            let fixed = fix_all_refs(schema, &Obj::new(), None).map_err(|e| {
                if e.starts_with("[fixAllRefs]") {
                    "[cleanupOpenApiDoc] Recursive schemas are not supported for parameters".to_string()
                } else {
                    e
                }
            })?;
            p.insert("schema".into(), fixed);
        }
    }
    if p.contains_key(USES_THREE_POINT_ONE_SYNTAX_KEY) {
        uses_31 = js::truthy(p.get(USES_THREE_POINT_ONE_SYNTAX_KEY));
        js::delete(&mut p, USES_THREE_POINT_ONE_SYNTAX_KEY);
    }
    if version == OpenApiVersion::V3_0
        && uses_31
        && let Some(schema) = p.get("schema").cloned().filter(|s| js::truthy(Some(s)))
    {
        p.insert("schema".into(), convert_to_open_api_3_point_0(schema));
    }
    Ok(p)
}

/// `cleanupOpenApiDoc(doc)` of the zod DTO integration.
///
/// # Panics
/// On the same conditions where the JavaScript integration throws (two different schemas with the same name,
/// recursive schemas in parameters, non-object query DTOs). Use
/// [`try_cleanup_open_api_doc`] to get the error instead.
pub fn cleanup_open_api_doc(doc: OpenApiDocument) -> OpenApiDocument {
    match try_cleanup_open_api_doc(doc, OpenApiVersion::Auto) {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    }
}

/// `cleanupOpenApiDoc(doc, { version })`.
pub fn cleanup_open_api_doc_with(doc: OpenApiDocument, version: OpenApiVersion) -> OpenApiDocument {
    match try_cleanup_open_api_doc(doc, version) {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    }
}

/// `cleanupOpenApiDoc(doc, { version })` returning the error message instead of panicking.
pub fn try_cleanup_open_api_doc(mut doc: OpenApiDocument, version: OpenApiVersion) -> Result<OpenApiDocument, String> {
    let version = match version {
        OpenApiVersion::Auto => {
            if doc.get("openapi").and_then(Value::as_str).unwrap_or("").starts_with("3.1") {
                OpenApiVersion::V3_1
            } else {
                OpenApiVersion::V3_0
            }
        }
        v => v,
    };
    let mut schemas = Obj::new();
    let mut renames = Obj::new();
    let old_schemas = doc.get("components").and_then(|c| c.get("schemas")).and_then(Value::as_object).cloned().unwrap_or_default();
    for (old_name, old_schema) in &old_schemas {
        let c = cleanup_schema(old_name, old_schema, version)?;
        for (k, v) in c.additional {
            register_schema(&mut schemas, &k, v)?;
        }
        if *old_name != c.name {
            renames.insert(old_name.clone(), Value::String(c.name.clone()));
        }
        register_schema(&mut schemas, &c.name, c.schema)?;
    }

    let mut paths = doc.get("paths").and_then(Value::as_object).cloned().unwrap_or_default();
    for (_, item) in paths.iter_mut() {
        let Value::Object(item) = item else { continue };
        for method in ["get", "patch", "post", "delete", "put", "head"] {
            let Some(Value::Object(m)) = item.get_mut(method) else { continue };
            fix_refs_in_bodies(m, &renames);
            let n = m.get("parameters").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            for i in 0..n {
                let mut p = fix_parameter(&m["parameters"][i], version)?;
                if p.contains_key(SELF_REQUIRED_KEY) {
                    if p.contains_key("schema") {
                        let is_object = p.get("schema").and_then(|s| s.get("type")).and_then(Value::as_str) == Some("object");
                        if p.get("in").and_then(Value::as_str) == Some("query") && !p.contains_key("style") && is_object {
                            p.insert("style".into(), js::s("deepObject"));
                        }
                        if let Some(Value::Array(req)) = p.get("required").cloned()
                            && let Some(Value::Object(s)) = p.get_mut("schema")
                            && is_object
                            && !s.contains_key("required")
                        {
                            s.insert("required".into(), Value::Array(req));
                        }
                        let sr = js::truthy(p.get(SELF_REQUIRED_KEY));
                        p.insert("required".into(), Value::Bool(sr));
                    }
                    js::delete(&mut p, SELF_REQUIRED_KEY);
                }
                if p.contains_key(DEFS_KEY) {
                    let defs = js::delete(&mut p, DEFS_KEY).unwrap_or(Value::Null);
                    for (id, def) in js::obj_of(&defs) {
                        let mut fixed = fix_all_refs(def, &Obj::new(), None).map_err(|e| {
                            if e.starts_with("[fixAllRefs]") {
                                "[cleanupOpenApiDoc] Recursive schemas are not supported for parameters".to_string()
                            } else {
                                e
                            }
                        })?;
                        if version == OpenApiVersion::V3_0 {
                            fixed = convert_to_open_api_3_point_0(fixed);
                        }
                        register_schema(&mut schemas, &id, fixed)?;
                    }
                }
                if let Some(Value::Array(params)) = m.get_mut("parameters") {
                    params[i] = Value::Object(p);
                }
            }
        }
    }

    let root = doc.obj_mut();
    root.insert("paths".into(), Value::Object(paths));
    let mut components = root.get("components").and_then(Value::as_object).cloned().unwrap_or_default();
    components.insert("schemas".into(), Value::Object(schemas));
    root.insert("components".into(), Value::Object(components));
    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_nullable() {
        let s = json!({ "type": "object", "properties": { "a": { "anyOf": [{ "type": "string", "format": "email" }, { "type": "null" }], "description": "d" } } });
        let out = convert_to_open_api_3_point_0(s);
        assert_eq!(
            js::stringify(&out),
            r#"{"type":"object","properties":{"a":{"type":"string","format":"email","description":"d","nullable":true}}}"#
        );
        let out = convert_to_open_api_3_point_0(json!({ "const": "x", "exclusiveMinimum": 0 }));
        assert_eq!(js::stringify(&out), r#"{"exclusiveMinimum":true,"enum":["x"],"minimum":0}"#);
        let out = convert_to_open_api_3_point_0(json!({ "anyOf": [{ "$ref": "#/a" }, { "type": "null" }] }));
        assert_eq!(js::stringify(&out), r##"{"allOf":[{"$ref":"#/a"}],"nullable":true}"##);
    }

    #[test]
    fn refs() {
        let out = fix_all_refs(json!({ "$ref": "#/$defs/A" }), &Obj::new(), None).unwrap();
        assert_eq!(out, json!({ "$ref": "#/components/schemas/A" }));
        assert!(fix_all_refs(json!({ "$ref": "#" }), &Obj::new(), None).is_err());
        let out = fix_all_refs(json!({ "$ref": "#" }), &Obj::new(), Some("Root")).unwrap();
        assert_eq!(out, json!({ "$ref": "#/components/schemas/Root" }));
    }

    #[test]
    fn cleanup_removes_markers() {
        let schema = json!({
            "type": "object",
            "properties": {
                "a": { "type": "", "anyOf": [{ "type": "string" }, { "type": "null" }], "x-corvis_zod-empty-type": true, "x-corvis_zod-uses-3-point-1-syntax": true },
                "o": { "type": "object", "properties": {}, "x-corvis_zod-self-required": false, "selfRequired": false }
            },
            "required": ["a", "o"]
        });
        let c = cleanup_schema("Dto", &schema, OpenApiVersion::V3_0).unwrap();
        assert_eq!(
            js::stringify(&c.schema),
            r#"{"type":"object","properties":{"a":{"type":"string","nullable":true},"o":{"type":"object","properties":{}}},"required":["a"]}"#
        );
    }
}

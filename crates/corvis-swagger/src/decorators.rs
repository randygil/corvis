//! The OpenAPI decorators (`@ApiTags()`, `@ApiOkResponse()`, ...) as [`CustomDecorator`] factories.
//!
//! You normally use them through the attributes (`#[api_tags("cats")]`,
//! `#[api_ok_response(type = CatDto)]`, ...), which `#[controller]` / `#[routes]` forward to
//! the `api_*!` macros, which call these functions. They can also be combined with
//! `#[apply_decorators(...)]`:
//!
//! ```ignore
//! fn auth_docs() -> CustomDecorator {
//!     apply_decorators([decorators::api_bearer_auth(None), api_unauthorized_response!(description = "No token")])
//! }
//! ```
//!
//! Every decorator stores one metadata entry under the usual `swagger/*` key
//! (`swagger/apiResponse`, `swagger/apiUseTags`, ...). The value is the *increment* the
//! decorator applies; [`crate::metadata`] folds the entries of a class or method in TypeScript
//! decorator application order (bottom to top, methods before the class).

use corvis::CustomDecorator;
use serde_json::{Map, Value, json};

use crate::js::{self, Obj};

/// Metadata keys (the `swagger/*` decorator keys).
pub mod keys {
    pub const API_OPERATION: &str = "swagger/apiOperation";
    pub const API_RESPONSE: &str = "swagger/apiResponse";
    pub const API_PRODUCES: &str = "swagger/apiProduces";
    pub const API_CONSUMES: &str = "swagger/apiConsumes";
    pub const API_TAGS: &str = "swagger/apiUseTags";
    pub const API_CALLBACKS: &str = "swagger/apiCallbacks";
    pub const API_PARAMETERS: &str = "swagger/apiParameters";
    pub const API_HEADERS: &str = "swagger/apiHeaders";
    pub const API_SECURITY: &str = "swagger/apiSecurity";
    pub const API_EXCLUDE_ENDPOINT: &str = "swagger/apiExcludeEndpoint";
    pub const API_INCLUDE_ENDPOINT: &str = "swagger/apiIncludeEndpoint";
    pub const API_EXCLUDE_CONTROLLER: &str = "swagger/apiExcludeController";
    pub const API_EXTRA_MODELS: &str = "swagger/apiExtraModels";
    pub const API_EXTENSION: &str = "swagger/apiExtension";
    /// `ApiHeader` (a parameter on a method, `API_HEADERS` on a class).
    pub const API_HEADER: &str = "swagger/apiHeader";
}

fn single(key: &str, value: Value) -> CustomDecorator {
    CustomDecorator::from((key.to_string(), value))
}

fn obj(v: Value) -> Obj {
    match v {
        Value::Object(o) => o,
        _ => Obj::new(),
    }
}

/// lodash `pickBy(o, negate(isUndefined))`: JSON has no `undefined`, so this is a copy.
fn defined(o: &Obj) -> Obj {
    o.clone()
}

/// `getTypeIsArrayTuple(type, isArray)`.
pub(crate) fn type_is_array_tuple(o: &mut Obj) {
    let is_array = o.get("isArray").cloned();
    let Some(t) = o.get("type").cloned() else {
        return;
    };
    if js::truthy(is_array.as_ref()) {
        return;
    }
    if !js::truthy(Some(&t)) {
        return;
    }
    match t {
        Value::Array(a) => {
            o.insert("type".into(), a.first().cloned().unwrap_or(Value::Null));
            o.insert("isArray".into(), Value::Bool(true));
        }
        _ => {
            o.insert("isArray".into(), Value::Bool(false));
        }
    }
}

// --- enum helpers (utils/enum.utils) -------------------------------------------------------

/// `getEnumValues(enum)`: an array is used as is; an object (TypeScript enum) gives its values.
pub(crate) fn enum_values(e: &Value) -> Vec<Value> {
    match e {
        Value::Array(a) => a.clone(),
        Value::Object(o) => {
            let numeric: Vec<String> = o.values().filter(|v| v.is_number()).map(js_number_string).collect();
            o.iter().filter(|(k, _)| !numeric.contains(k)).map(|(_, v)| v.clone()).collect()
        }
        _ => Vec::new(),
    }
}

fn js_number_string(v: &Value) -> String {
    match v.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e21 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => String::new(),
    }
}

/// `getEnumType(values)`.
pub(crate) fn enum_type(values: &[Value]) -> &'static str {
    if values.iter().any(Value::is_string) {
        "string"
    } else if values.iter().any(Value::is_boolean) {
        "boolean"
    } else {
        "number"
    }
}

fn add_enum_schema(param: &mut Obj, options: &Obj) {
    let mut schema = param.get("schema").and_then(Value::as_object).cloned().unwrap_or_default();
    let values = enum_values(options.get("enum").unwrap_or(&Value::Null));
    schema.insert("enum".into(), Value::Array(values.clone()));
    schema.insert("type".into(), js::s(enum_type(&values)));
    param.insert("schema".into(), Value::Object(schema));
    for k in ["enumName", "enumSchema"] {
        if let Some(v) = options.get(k).filter(|v| js::truthy(Some(v))) {
            param.insert(k.into(), v.clone());
        }
    }
}

fn add_enum_array_schema(param: &mut Obj, options: &Obj) {
    let mut schema = param.get("schema").and_then(Value::as_object).cloned().unwrap_or_default();
    schema.insert("type".into(), js::s("array"));
    js::delete(param, "isArray");
    let values = enum_values(options.get("enum").unwrap_or(&Value::Null));
    schema.insert("items".into(), json!({ "type": enum_type(&values), "enum": values }));
    param.insert("schema".into(), Value::Object(schema));
    for k in ["enumName", "enumSchema"] {
        if let Some(v) = options.get(k).filter(|v| js::truthy(Some(v))) {
            param.insert(k.into(), v.clone());
        }
    }
}

fn is_enum_array(o: &Obj) -> bool {
    js::truthy(o.get("isArray")) && js::truthy(o.get("enum"))
}

fn apply_param_extensions(param: &mut Obj, options: &Obj) {
    if let Some(Value::Object(ext)) = options.get("extensions") {
        for (k, v) in ext {
            let key = if k.starts_with("x-") { k.clone() } else { format!("x-{k}") };
            param.insert(key, v.clone());
        }
    }
}

/// `createParamDecorator(metadata, initial)`: `{ ...initial, ...pickBy(metadata) }`.
fn param_decorator(param: Obj, initial: Value) -> CustomDecorator {
    let mut out = obj(initial);
    js::assign(&mut out, &defined(&param));
    single(keys::API_PARAMETERS, Value::Object(out))
}

// --- operation / responses ----------------------------------------------------------------

/// `@ApiOperation(options)` (`summary` defaults to `''`).
pub fn api_operation(options: Value) -> CustomDecorator {
    let mut o = Obj::new();
    o.insert("summary".into(), js::s(""));
    js::assign(&mut o, &obj(options));
    single(keys::API_OPERATION, Value::Object(o))
}

/// `@ApiResponse(options)`.
pub fn api_response(options: Value) -> CustomDecorator {
    let mut o = obj(options);
    // `type`/`isArray` are reassigned in place (`undefined` values do not exist in JSON).
    type_is_array_tuple(&mut o);
    let desc = o.get("description").filter(|d| js::truthy(Some(d))).cloned().unwrap_or_else(|| js::s(""));
    o.insert("description".into(), desc);
    let status = match o.get("status") {
        Some(Value::Number(n)) => js_number_string(&Value::Number(n.clone())),
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => "default".to_string(),
    };
    let entry = js::omit(&o, &["status"]);
    single(keys::API_RESPONSE, json!({ "status": status, "entry": Value::Object(entry) }))
}

/// `Api<Status>Response(options)`: `ApiResponse({ ...options, status })`.
pub fn api_status_response(status: u16, options: Value) -> CustomDecorator {
    let mut o = obj(options);
    o.insert("status".into(), json!(status));
    api_response(Value::Object(o))
}

/// `@ApiDefaultResponse(options)`.
pub fn api_default_response(options: Value) -> CustomDecorator {
    let mut o = obj(options);
    o.insert("status".into(), js::s("default"));
    api_response(Value::Object(o))
}

/// `mergeResponseEntry(existing, incoming)` (two `@ApiResponse` with the same status).
pub(crate) fn merge_response_entry(existing: Option<&Obj>, incoming: &Obj) -> Obj {
    let Some(existing) = existing else {
        return incoming.clone();
    };
    let ed = existing.get("description").and_then(Value::as_str).unwrap_or("");
    let id = incoming.get("description").and_then(Value::as_str).unwrap_or("");
    let desc = if !ed.is_empty() && !id.is_empty() {
        format!("{ed}\n\n{id}")
    } else if !ed.is_empty() {
        ed.to_string()
    } else {
        id.to_string()
    };
    let examples = match (existing.get("examples"), incoming.get("examples")) {
        (Some(Value::Object(a)), Some(Value::Object(b))) => Some(Value::Object(js::spread(&[a, b]))),
        (e, i) => i.or(e).cloned(),
    };
    let mut out = js::spread(&[existing, incoming]);
    out.insert("description".into(), Value::String(desc));
    if let Some(ex) = examples {
        out.insert("examples".into(), ex);
    }
    out
}

/// The zod DTO integration's `@ZodResponse({ status, description, type: Dto })`: `@HttpCode(status)` (when
/// given) + `@ZodSerializerDto(Dto)` + `@ApiResponse({ status, description, type: Dto.Output })`
/// (`Dto` itself for codec DTOs). Use it with
/// `#[apply_decorators(corvis_swagger::decorators::zod_response::<UserDto>(Some(201), None))]` and
/// register `ZodSerializerInterceptor` (globally or with `#[use_interceptors]`).
pub fn zod_response<T: corvis::z::ZodDto + 'static>(status: Option<u16>, description: Option<&str>) -> CustomDecorator {
    zod_response_impl::<T>(status, description, false)
}

/// `@ZodResponse({ status, description, type: [Dto] })`.
pub fn zod_response_array<T: corvis::z::ZodDto + 'static>(status: Option<u16>, description: Option<&str>) -> CustomDecorator {
    zod_response_impl::<T>(status, description, true)
}

fn zod_response_impl<T: corvis::z::ZodDto + 'static>(status: Option<u16>, description: Option<&str>, array: bool) -> CustomDecorator {
    use crate::types::SwaggerType;
    let mut out = CustomDecorator::new();
    if let Some(s) = status {
        out = out.set_metadata(corvis::HTTP_CODE_METADATA, s);
    }
    let serializer = if array { corvis::z::ZodSerializerDto::dto_array::<T>() } else { corvis::z::ZodSerializerDto::dto::<T>() };
    out = out.set_metadata(corvis::z::ZOD_SERIALIZER_DTO_OPTIONS, serializer);
    let ty = if T::codec() {
        crate::__private::probe::ZodProbe::__swagger_type_marker(&&crate::__private::probe::Probe::<T>::new())
    } else {
        crate::types::Output::<T>::swagger_type()
    };
    let mut o = Obj::new();
    if let Some(s) = status {
        o.insert("status".into(), json!(s));
    }
    if let Some(d) = description {
        o.insert("description".into(), js::s(d));
    }
    o.insert("type".into(), if array { Value::Array(vec![ty]) } else { ty });
    out.and(api_response(Value::Object(o)))
}

/// `@ApiProduces(...mimeTypes)`.
pub fn api_produces(mime_types: Vec<Value>) -> CustomDecorator {
    single(keys::API_PRODUCES, Value::Array(mime_types))
}

/// `@ApiConsumes(...mimeTypes)`.
pub fn api_consumes(mime_types: Vec<Value>) -> CustomDecorator {
    single(keys::API_CONSUMES, Value::Array(mime_types))
}

/// `@ApiTags(...tags)` (a tag object contributes its `name`).
pub fn api_tags(tags: Vec<Value>) -> CustomDecorator {
    let tags = tags
        .into_iter()
        .map(|t| match t {
            Value::Object(o) => {
                if o.contains_key("parent") || o.contains_key("kind") {
                    let name = o.get("name").and_then(Value::as_str).unwrap_or("");
                    corvis::Logger::new("ApiTags").warn(format!(
                        "Tag \"{name}\" includes hierarchy fields (parent/kind) defined via @ApiTags, but those values are ignored there. Define them with DocumentBuilder.addTag(\"{name}\", description, {{ parent, kind }}) to include them in the document's root-level tags."
                    ));
                }
                o.get("name").cloned().unwrap_or(Value::Null)
            }
            other => other,
        })
        .collect();
    single(keys::API_TAGS, Value::Array(tags))
}

/// `@ApiCallbacks(...callbacks)`.
pub fn api_callbacks(callbacks: Vec<Value>) -> CustomDecorator {
    single(keys::API_CALLBACKS, Value::Array(callbacks))
}

// --- parameters ------------------------------------------------------------------------

/// `@ApiParam(options)`.
pub fn api_param(options: Value) -> CustomDecorator {
    let options = obj(options);
    let mut param = Obj::new();
    let name = match options.get("name") {
        None | Some(Value::Null) => js::s(""),
        Some(v) => v.clone(),
    };
    param.insert("name".into(), name);
    param.insert("in".into(), js::s("path"));
    js::assign(&mut param, &js::omit(&options, &["enum", "extensions"]));
    if js::truthy(options.get("enum")) {
        add_enum_schema(&mut param, &options);
    }
    apply_param_extensions(&mut param, &options);
    param_decorator(param, json!({ "name": "", "required": true }))
}

/// `@ApiQuery(options)`.
pub fn api_query(options: Value) -> CustomDecorator {
    let options = obj(options);
    let mut typed = options.clone();
    type_is_array_tuple(&mut typed);
    let is_array = js::truthy(typed.get("isArray"));
    let mut param = Obj::new();
    param.insert("name".into(), options.get("name").cloned().unwrap_or_else(|| js::s("")));
    param.insert("in".into(), js::s("query"));
    js::assign(&mut param, &js::omit(&options, &["enum", "extensions"]));
    match typed.get("type") {
        Some(t) => {
            param.insert("type".into(), t.clone());
        }
        None => {
            js::delete(&mut param, "type");
        }
    }
    if is_enum_array(&options) {
        add_enum_array_schema(&mut param, &options);
    } else if js::truthy(options.get("enum")) {
        add_enum_schema(&mut param, &options);
    }
    if is_array {
        param.insert("isArray".into(), Value::Bool(true));
    }
    apply_param_extensions(&mut param, &options);
    param_decorator(param, json!({ "name": "", "required": true }))
}

/// `@ApiHeader(options)`.
pub fn api_header(options: Value) -> CustomDecorator {
    let options = obj(options);
    let schema = if options.contains_key("content") {
        None
    } else {
        let mut s = Obj::new();
        s.insert("type".into(), js::s("string"));
        if let Some(e) = options.get("example").filter(|v| js::truthy(Some(v))) {
            s.insert("example".into(), e.clone());
        }
        if let Some(Value::Object(extra)) = options.get("schema") {
            js::assign(&mut s, extra);
        }
        Some(Value::Object(s))
    };
    let mut param = Obj::new();
    let name = match options.get("name") {
        None | Some(Value::Null) => js::s(""),
        Some(v) => v.clone(),
    };
    param.insert("name".into(), name);
    param.insert("in".into(), js::s("header"));
    for k in ["description", "required", "deprecated", "allowEmptyValue", "style", "explode", "allowReserved", "content", "examples"] {
        if let Some(v) = options.get(k) {
            param.insert(k.into(), v.clone());
        }
    }
    if let Some(s) = schema {
        param.insert("schema".into(), s);
    }
    if js::truthy(options.get("enum")) {
        let values = enum_values(&options["enum"]);
        let mut s = param.get("schema").and_then(Value::as_object).cloned().unwrap_or_default();
        s.insert("enum".into(), Value::Array(values.clone()));
        s.insert("type".into(), js::s(enum_type(&values)));
        param.insert("schema".into(), Value::Object(s));
    }
    single(keys::API_HEADER, Value::Object(param))
}

/// `@ApiHeaders([...])`.
pub fn api_headers(headers: Value) -> CustomDecorator {
    let mut out = CustomDecorator::new();
    if let Value::Array(list) = headers {
        for h in list {
            out = out.and(api_header(h));
        }
    }
    out
}

/// `@ApiBody(options)`.
pub fn api_body(options: Value) -> CustomDecorator {
    let options = obj(options);
    let mut typed = options.clone();
    type_is_array_tuple(&mut typed);
    let mut param = Obj::new();
    param.insert("in".into(), js::s("body"));
    js::assign(&mut param, &js::omit(&options, &["enum"]));
    match typed.get("type") {
        Some(t) => {
            param.insert("type".into(), t.clone());
        }
        None => {
            js::delete(&mut param, "type");
        }
    }
    match typed.get("isArray") {
        Some(a) => {
            param.insert("isArray".into(), a.clone());
        }
        None => {
            js::delete(&mut param, "isArray");
        }
    }
    if is_enum_array(&options) {
        add_enum_array_schema(&mut param, &options);
    } else if js::truthy(options.get("enum")) {
        add_enum_schema(&mut param, &options);
    }
    param_decorator(param, json!({ "type": crate::types::ctor_marker("String"), "required": true }))
}

// --- security ----------------------------------------------------------------------------

/// `@ApiSecurity(name, requirements)` / `@ApiSecurity({ name: [...] })`.
pub fn api_security(name: Value, requirements: Value) -> CustomDecorator {
    let entry = match name {
        Value::String(n) => {
            let mut m = Map::new();
            m.insert(n, if requirements.is_null() { json!([]) } else { requirements });
            Value::Object(m)
        }
        other => other,
    };
    single(keys::API_SECURITY, json!([entry]))
}

/// `@ApiBearerAuth(name = 'bearer')`.
pub fn api_bearer_auth(name: Option<String>) -> CustomDecorator {
    api_security(js::s(name.unwrap_or_else(|| "bearer".into())), json!([]))
}

/// `@ApiBasicAuth(name = 'basic')`.
pub fn api_basic_auth(name: Option<String>) -> CustomDecorator {
    api_security(js::s(name.unwrap_or_else(|| "basic".into())), json!([]))
}

/// `@ApiCookieAuth(name = 'cookie')`.
pub fn api_cookie_auth(name: Option<String>) -> CustomDecorator {
    api_security(js::s(name.unwrap_or_else(|| "cookie".into())), json!([]))
}

/// `@ApiOAuth2(scopes, name = 'oauth2')`.
pub fn api_oauth2(scopes: Value, name: Option<String>) -> CustomDecorator {
    api_security(js::s(name.unwrap_or_else(|| "oauth2".into())), scopes)
}

// --- inclusion / extensions / models -------------------------------------------------------

/// `@ApiExcludeEndpoint(disable = true)`.
pub fn api_exclude_endpoint(disable: bool) -> CustomDecorator {
    single(keys::API_EXCLUDE_ENDPOINT, json!({ "disable": disable }))
}

/// `@ApiIncludeEndpoint(disable = true)` (used with `only_include_decorated_endpoints`).
pub fn api_include_endpoint(disable: bool) -> CustomDecorator {
    single(keys::API_INCLUDE_ENDPOINT, json!({ "disable": disable }))
}

/// `@ApiExcludeController(disable = true)`.
pub fn api_exclude_controller(disable: bool) -> CustomDecorator {
    single(keys::API_EXCLUDE_CONTROLLER, json!([disable]))
}

/// `@ApiExtension('x-key', value)`.
///
/// # Panics
/// When the key is not prefixed with `x-` (the same error the Node.js generator throws).
pub fn api_extension(key: String, value: Value) -> CustomDecorator {
    if !key.starts_with("x-") {
        panic!("Extension key is not prefixed. Please ensure you prefix it with `x-`.");
    }
    single(keys::API_EXTENSION, json!({ "key": key, "value": value }))
}

/// `@ApiExtraModels(...models)` (markers created by the `api_extra_models!` macro).
pub fn api_extra_models(models: Vec<Value>) -> CustomDecorator {
    single(keys::API_EXTRA_MODELS, Value::Array(models))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(d: &CustomDecorator) -> (&str, &Value) {
        let (k, v) = &d.entries()[0];
        (k.as_str(), v)
    }

    #[test]
    fn operation_defaults_summary() {
        let d = api_operation(json!({ "description": "x" }));
        assert_eq!(entry(&d), (keys::API_OPERATION, &json!({ "summary": "", "description": "x" })));
    }

    #[test]
    fn response_entry() {
        let d = api_status_response(200, json!({ "type": [ { "$model": "A" } ] }));
        assert_eq!(
            js::stringify(entry(&d).1),
            r#"{"status":"200","entry":{"type":{"$model":"A"},"isArray":true,"description":""}}"#
        );
        let d = api_default_response(json!({ "description": "d" }));
        assert_eq!(entry(&d).1["status"], "default");
    }

    #[test]
    fn merge_responses() {
        let a = obj(json!({ "description": "a", "type": 1 }));
        let b = obj(json!({ "description": "b" }));
        assert_eq!(Value::Object(merge_response_entry(Some(&a), &b)), json!({ "description": "a\n\nb", "type": 1 }));
    }

    #[test]
    fn params() {
        let d = api_param(json!({ "name": "id", "enum": ["a", "b"] }));
        assert_eq!(
            js::stringify(entry(&d).1),
            r#"{"name":"id","required":true,"in":"path","schema":{"enum":["a","b"],"type":"string"}}"#
        );
        let d = api_query(json!({ "name": "q", "type": [{ "$ctor": "String" }], "required": false }));
        assert_eq!(
            js::stringify(entry(&d).1),
            r#"{"name":"q","required":false,"in":"query","type":{"$ctor":"String"},"isArray":true}"#
        );
        let d = api_header(json!({ "name": "X-Key", "description": "k" }));
        assert_eq!(
            js::stringify(entry(&d).1),
            r#"{"name":"X-Key","in":"header","description":"k","schema":{"type":"string"}}"#
        );
    }

    #[test]
    fn security() {
        assert_eq!(entry(&api_bearer_auth(None)).1, &json!([{ "bearer": [] }]));
        assert_eq!(entry(&api_oauth2(json!(["read"]), None)).1, &json!([{ "oauth2": ["read"] }]));
    }

    #[test]
    fn enum_values_of_ts_enum() {
        // enum E { A = 0, B = 1 } compiles to { "0": "A", "1": "B", "A": 0, "B": 1 }
        let e = json!({ "0": "A", "1": "B", "A": 0, "B": 1 });
        assert_eq!(enum_values(&e), vec![json!(0), json!(1)]);
    }
}

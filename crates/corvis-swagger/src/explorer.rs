//! The route scanner of the document generator: walks the controllers and handlers, reads
//! their decorator metadata and builds the paths, operations and response objects.

use std::any::TypeId;
use std::sync::Arc;

use corvis::router::controller::VersionValue;
use corvis::router::versioning::VersioningType;
use corvis::{ControllerInfo, CorvisApplication, ParamDef, ParamKind, RequestMethod, RouteMeta};
use serde_json::{Value, json};

use crate::decorators::keys;
use crate::document::OpenApiDocument;
use crate::js::{self, Obj};
use crate::metadata::{self, Metadata};
use crate::schema_factory::{self, SchemaObjectFactory, Schemas, SwaggerTypesMapper, Ty, ty};
use crate::types;

/// Builds the method key of a handler from its name.
pub type MethodKeyFactory = Arc<dyn Fn(&str) -> String + Send + Sync>;

/// `operationIdFactory(controllerKey, methodKey, version)`.
pub type OperationIdFactory = Arc<dyn Fn(&str, &str, Option<&str>) -> String + Send + Sync>;

/// `SwaggerDocumentOptions`.
#[derive(Clone)]
pub struct SwaggerDocumentOptions {
    /// Modules to scan (`include: [CatsModule]`); empty = every module.
    pub include: Vec<TypeId>,
    /// Extra models added to `components.schemas` (`extraModels`), as type markers
    /// (see [`crate::model`]).
    pub extra_models: Vec<Value>,
    /// Leave the global prefix out of the paths (`ignoreGlobalPrefix`).
    pub ignore_global_prefix: bool,
    /// Also scan the controllers of imported (non-global) modules (`deepScanRoutes`).
    pub deep_scan_routes: bool,
    /// `operationIdFactory` (default: `${controller}_${method}[_${version}]`).
    pub operation_id_factory: Option<OperationIdFactory>,
    /// Name of a handler as a method key (default: `snake_case` -> `camelCase`, so
    /// `find_all` gives the same `UsersController_findAll` as the TypeScript method).
    pub method_key_factory: Option<MethodKeyFactory>,
    /// Tag controllers without `#[api_tags]` with their name minus `Controller`
    /// (`autoTagControllers`, default `true`).
    pub auto_tag_controllers: bool,
    /// Only document handlers with `#[api_include_endpoint]` (`onlyIncludeDecoratedEndpoints`).
    pub only_include_decorated_endpoints: bool,
    /// `exampleMaxDepth`.
    pub example_max_depth: Option<i64>,
}

impl Default for SwaggerDocumentOptions {
    fn default() -> Self {
        SwaggerDocumentOptions {
            include: Vec::new(),
            extra_models: Vec::new(),
            ignore_global_prefix: false,
            deep_scan_routes: false,
            operation_id_factory: None,
            method_key_factory: None,
            auto_tag_controllers: true,
            only_include_decorated_endpoints: false,
            example_max_depth: None,
        }
    }
}

impl std::fmt::Debug for SwaggerDocumentOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwaggerDocumentOptions")
            .field("include", &self.include)
            .field("ignore_global_prefix", &self.ignore_global_prefix)
            .field("deep_scan_routes", &self.deep_scan_routes)
            .field("auto_tag_controllers", &self.auto_tag_controllers)
            .field("only_include_decorated_endpoints", &self.only_include_decorated_endpoints)
            .finish_non_exhaustive()
    }
}

/// `find_all` -> `findAll`.
pub fn snake_to_camel(s: &str) -> String {
    let s = s.strip_prefix("r#").unwrap_or(s);
    let mut out = String::with_capacity(s.len());
    let mut upper = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' && i > 0 {
            upper = true;
            continue;
        }
        if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn default_operation_id(controller: &str, method: &str, version: Option<&str>) -> String {
    match (version, controller.is_empty()) {
        (Some(v), false) => format!("{controller}_{method}_{v}"),
        (Some(v), true) => format!("{method}_{v}"),
        (None, false) => format!("{controller}_{method}"),
        (None, true) => method.to_string(),
    }
}

/// `SwaggerScanner.scanApplication(app, options)` + `SwaggerModule.createDocument`.
pub fn create_document(app: &CorvisApplication, config: &OpenApiDocument, options: &SwaggerDocumentOptions) -> OpenApiDocument {
    let mut explorer = Explorer { app, options, config, schemas: Obj::new() };
    let modules = app.modules_info();
    let mut denormalized: Vec<Obj> = Vec::new();
    for module in &modules {
        if !options.include.is_empty() && !options.include.contains(&module.type_id) {
            continue;
        }
        if options.deep_scan_routes {
            for &i in &module.imports {
                let Some(imported) = modules.get(i) else { continue };
                if imported.global {
                    continue;
                }
                for c in &imported.controllers {
                    denormalized.extend(explorer.explore_controller(c));
                }
            }
        }
        for c in &module.controllers {
            denormalized.extend(explorer.explore_controller(c));
        }
    }
    let mut schemas = std::mem::take(&mut explorer.schemas);
    for m in &options.extra_models {
        SchemaObjectFactory::explore_model_schema(m, &mut schemas, &mut Vec::new());
    }
    apply_example_max_depth(&mut schemas, options.example_max_depth);

    let paths = normalize_paths(denormalized);

    // SwaggerModule.createDocument
    let config_obj = js::omit(&config.obj(), &["webhooks"]);
    let mut components = Obj::new();
    for src in [config_obj.get("components"), Some(&json!({ "schemas": Value::Object(schemas) }))] {
        if let Some(Value::Object(src)) = src {
            for (k, v) in src {
                let mut cur = components.get(k).and_then(Value::as_object).cloned().unwrap_or_default();
                if let Value::Object(v) = v {
                    js::assign(&mut cur, v);
                }
                components.insert(k.clone(), Value::Object(cur));
            }
        }
    }
    let mut doc = Obj::new();
    doc.insert("openapi".into(), js::s("3.0.0"));
    doc.insert("paths".into(), json!({}));
    js::assign(&mut doc, &config_obj);
    doc.insert("paths".into(), Value::Object(paths));
    doc.insert("components".into(), Value::Object(components));
    OpenApiDocument::from_value(Value::Object(doc))
}

struct Explorer<'a> {
    app: &'a CorvisApplication,
    options: &'a SwaggerDocumentOptions,
    config: &'a OpenApiDocument,
    schemas: Schemas,
}

impl Explorer<'_> {
    fn method_key(&self, handler: &str) -> String {
        match &self.options.method_key_factory {
            Some(f) => f(handler),
            None => snake_to_camel(handler),
        }
    }

    fn operation_id(&self, controller: &str, method: &str, version: Option<&str>) -> String {
        match &self.options.operation_id_factory {
            Some(f) => f(controller, method, version),
            None => default_operation_id(controller, method, version),
        }
    }

    fn register_extra_models(&mut self, models: Option<&Vec<Value>>) {
        for m in models.cloned().unwrap_or_default() {
            SchemaObjectFactory::explore_model_schema(&m, &mut self.schemas, &mut Vec::new());
        }
    }

    /// `exploreController` + `generateDenormalizedDocument`.
    fn explore_controller(&mut self, ctrl: &ControllerInfo) -> Vec<Obj> {
        let class = metadata::fold_class(&ctrl.metadata.metadata);
        if class.array(keys::API_EXCLUDE_CONTROLLER).and_then(|a| a.first()) == Some(&Value::Bool(true)) {
            return Vec::new();
        }
        let global = self.explore_global_metadata(ctrl, &class);
        self.register_extra_models(class.array(keys::API_EXTRA_MODELS));

        let mut out = Vec::new();
        for route in &ctrl.routes {
            let method = metadata::fold_method(&route.metadata, &ctrl.metadata.metadata);
            let include = method.get(keys::API_INCLUDE_ENDPOINT);
            if self.options.only_include_decorated_endpoints && include.is_none() {
                continue;
            }
            if method.get(keys::API_EXCLUDE_ENDPOINT).is_some_and(|e| js::truthy(e.get("disable"))) {
                continue;
            }
            self.register_extra_models(method.array(keys::API_EXTRA_MODELS));

            // documentResolvers, in order: root, security, tags, callbacks, responses.
            let mut roots = self.explore_route_path_and_method(ctrl, route, &method);
            if let Some(Value::Object(op)) = method.get(keys::API_OPERATION) {
                for r in roots.iter_mut() {
                    js::assign(r, op);
                }
            }
            let parameters = self.explore_api_parameters(route, &method);
            for r in roots.iter_mut() {
                r.insert("parameters".into(), Value::Array(parameters.iter().cloned().map(Value::Object).collect()));
            }
            let security = method.get(keys::API_SECURITY).cloned().unwrap_or(json!({}));
            let tags = method.get(keys::API_TAGS).cloned().unwrap_or(json!({}));
            let callbacks = explore_callbacks(&method).map(Value::Object).unwrap_or(json!({}));
            let responses = self.explore_api_response(route, &method, &class).map(Value::Object).unwrap_or(json!({}));

            for root in roots {
                let mut endpoint = Obj::new();
                endpoint.insert("root".into(), Value::Object(root));
                endpoint.insert("security".into(), security.clone());
                endpoint.insert("tags".into(), tags.clone());
                endpoint.insert("callbacks".into(), callbacks.clone());
                endpoint.insert("responses".into(), responses.clone());
                endpoint.retain(|_, v| !js::is_empty(Some(v)));
                let merged = merge_metadata(&global, endpoint);
                let mut doc = Obj::new();
                doc.insert("responses".into(), json!({}));
                js::assign(&mut doc, &js::omit(&global, &["chunks"]));
                js::assign(&mut doc, &merged);
                self.migrate_operation_schema(&mut doc, &class, &method);
                if doc.get("root").and_then(|r| r.get("path")).is_some_and(|p| js::truthy(Some(p))) {
                    out.push(doc);
                }
            }
        }
        out
    }

    /// `exploreGlobalMetadata(metatype)`.
    fn explore_global_metadata(&mut self, ctrl: &ControllerInfo, class: &Metadata) -> Obj {
        let mut global = Obj::new();
        // tags
        let tags = class.array(keys::API_TAGS).filter(|t| !t.is_empty());
        if let Some(t) = tags {
            global.insert("tags".into(), Value::Array(t.clone()));
        } else if self.options.auto_tag_controllers {
            let name = ctrl.name.strip_suffix("Controller").unwrap_or(&ctrl.name).to_string();
            global.insert("tags".into(), json!([name]));
        }
        // security
        if let Some(s) = class.get(keys::API_SECURITY) {
            global.insert("security".into(), s.clone());
        }
        // responses
        let global_responses = self.config.global_responses.clone();
        let mapped_global = self.map_responses(&global_responses, None);
        if let Some(Value::Object(responses)) = class.get(keys::API_RESPONSE) {
            let produces = class.array(keys::API_PRODUCES).cloned();
            let mapped = self.map_responses(responses, produces);
            global.insert("responses".into(), Value::Object(js::spread(&[&mapped_global, &mapped])));
        } else {
            global.insert("responses".into(), Value::Object(mapped_global));
        }
        // headers (a chunk merged into root.parameters, depth 1)
        if let Some(headers) = class.array(keys::API_HEADERS) {
            global.insert("chunks".into(), json!([{ "root": { "parameters": Value::Array(headers.clone()) }, "depth": 1 }]));
        }
        global
    }

    /// `exploreRoutePathAndMethod`.
    fn explore_route_path_and_method(&self, ctrl: &ControllerInfo, route: &RouteMeta, method: &Metadata) -> Vec<Obj> {
        let app = self.app;
        let cmeta = &ctrl.metadata;
        let all = app.resolve_route_paths_with(cmeta, route, !self.options.ignore_global_prefix);
        let versioning = app.versioning_options();
        let uri = versioning.is_some_and(|v| v.kind == VersioningType::Uri);
        let versions: Vec<String> = match (uri, app.effective_route_version(cmeta, route)) {
            (true, Some(VersionValue::Versions(list))) => {
                let prefix = versioning.map(|v| v.prefix.clone()).unwrap_or_default();
                list.iter().map(|v| format!("{prefix}{v}")).collect()
            }
            _ => Vec::new(),
        };
        let non_path_version: Option<String> = match versioning {
            Some(v) if v.kind != VersioningType::Uri => {
                match route.version.clone().or_else(|| cmeta.version.clone()).or_else(|| v.default_version.clone()) {
                    Some(VersionValue::Versions(list)) => list.first().cloned(),
                    _ => None,
                }
            }
            _ => None,
        };
        let extension = method.get(keys::API_EXTENSION).and_then(Value::as_object).cloned().unwrap_or_default();
        let method_name = self.method_key(route.handler_name);
        let mut out = Vec::new();
        for (index, rp) in all.iter().enumerate() {
            let full = validate_route_path(&rp.path);
            let path = if full.is_empty() { "/".to_string() } else { full.clone() };
            if route.method == RequestMethod::All {
                for m in ["get", "post", "put", "delete", "patch", "options", "head", "search"] {
                    let mut o = Obj::new();
                    o.insert("method".into(), js::s(m));
                    o.insert("path".into(), js::s(path.clone()));
                    o.insert("operationId".into(), js::s(format!("{}_{m}", self.operation_id(&ctrl.name, &method_name, None))));
                    js::assign(&mut o, &extension);
                    out.push(o);
                }
                continue;
            }
            let path_version = versions.iter().find(|v| full.contains(&format!("/{v}/")) || full.ends_with(&format!("/{v}"))).cloned();
            let is_alias = all.len() > 1 && all.len() != versions.len();
            let method_key = if is_alias { format!("{method_name}[{index}]") } else { method_name.clone() };
            let version = path_version.or_else(|| non_path_version.clone());
            let mut o = Obj::new();
            o.insert("method".into(), js::s(route.method.as_str().to_lowercase()));
            o.insert("path".into(), js::s(path));
            o.insert("operationId".into(), js::s(self.operation_id(&ctrl.name, &method_key, version.as_deref())));
            js::assign(&mut o, &extension);
            out.push(o);
        }
        out
    }

    /// `exploreApiParametersMetadata`.
    fn explore_api_parameters(&mut self, route: &RouteMeta, method: &Metadata) -> Vec<Obj> {
        let explicit: Vec<Obj> = method.array(keys::API_PARAMETERS).map(|a| a.iter().map(js::obj_of).collect()).unwrap_or_default();
        let global: Vec<Obj> = self.config.global_parameters.iter().map(js::obj_of).collect();

        // ParameterMetadataAccessor.explore + ParametersMetadataMapper.transformModelToProperties
        let mut properties: Vec<Obj> = Vec::new();
        for p in &route.params {
            let Some(reflected) = reflected_param(p) else { continue };
            let t = ty(reflected.get("type"));
            if matches!(t, Ty::Undefined | Ty::Ctor("Object")) {
                continue;
            }
            if js::truthy(reflected.get("name")) {
                properties.push(reflected);
                continue;
            }
            if reflected.get("in").and_then(Value::as_str) == Some("body") {
                let name = match t {
                    Ty::Model(k) => schema_factory::model_info(k).map(|m| m.class_name).unwrap_or_default(),
                    Ty::Ctor(c) => c.to_string(),
                    Ty::Str(s) => s.to_string(),
                    _ => String::new(),
                };
                let mut o = reflected.clone();
                o.insert("name".into(), js::s(name));
                properties.push(o);
                continue;
            }
            if let Ty::Model(k) = t
                && let Some(info) = schema_factory::model_info(k)
            {
                for (key, meta) in info.properties {
                    let mut o = reflected.clone();
                    js::assign(&mut o, &meta);
                    let name = match meta.get("name") {
                        Some(n) if js::truthy(Some(n)) => n.clone(),
                        _ => js::s(key),
                    };
                    o.insert("name".into(), name);
                    properties.push(o);
                }
            }
        }

        // removeBodyMetadataIfExplicitExists
        if properties.iter().any(is_body) && explicit.iter().any(is_body) {
            properties.retain(|p| !is_body(p));
        }
        // merge implicit and explicit by name
        for item in properties.iter_mut() {
            if let Some(found) = explicit.iter().find(|e| e.get("name") == item.get("name")) {
                js::assign(item, found);
            }
        }
        // unionWith(properties, explicit, global, sameNameAndIn)
        let mut union: Vec<Obj> = Vec::new();
        for p in properties.into_iter().chain(explicit).chain(global) {
            let dup = union.iter().any(|u| u.get("name") == p.get("name") && u.get("in") == p.get("in"));
            if !dup {
                union.push(p);
            }
        }
        let with_defs = SchemaObjectFactory::create_from_model(union, &mut self.schemas);
        SwaggerTypesMapper::map_param_types(with_defs)
    }

    /// `exploreApiResponseMetadata`.
    fn explore_api_response(&mut self, route: &RouteMeta, method: &Metadata, class: &Metadata) -> Option<Obj> {
        if let Some(Value::Object(responses)) = method.get(keys::API_RESPONSE) {
            let produces = merge_and_uniq(class.array(keys::API_PRODUCES), method.array(keys::API_PRODUCES));
            return Some(self.map_responses(responses, Some(produces)));
        }
        let status = route.http_code.unwrap_or(if route.method == RequestMethod::Post { 201 } else { 200 });
        let mut o = Obj::new();
        o.insert(status.to_string(), json!({ "description": "" }));
        Some(o)
    }

    /// `mapResponsesToSwaggerResponses(responses, schemas, produces)`.
    fn map_responses(&mut self, responses: &Obj, produces: Option<Vec<Value>>) -> Obj {
        let produces = match produces {
            Some(p) if !p.is_empty() => p,
            _ => vec![js::s("application/json")],
        };
        let mut out = Obj::new();
        for (status, response) in responses {
            let r = self.create_response(js::obj_of(response), &produces);
            out.insert(status.clone(), Value::Object(js::omit(&r, &["type"])));
        }
        out
    }

    /// `ResponseObjectFactory.create(response, produces, schemas)`.
    fn create_response(&mut self, response: Obj, produces: &[Value]) -> Obj {
        let t = response.get("type").cloned();
        let is_array = js::truthy(response.get("isArray"));
        let response = js::omit(&response, &["isArray"]);
        let example_keys = ["example", "examples"];
        let Some(t) = t.filter(|t| js::truthy(Some(t))) else {
            // wrapSchemaWithContent
            if !js::truthy(response.get("schema")) && !response.contains_key("example") && !response.contains_key("examples") {
                return response;
            }
            let mut inner = Obj::new();
            if let Some(s) = response.get("schema") {
                inner.insert("schema".into(), s.clone());
            }
            js::assign(&mut inner, &js::pick(&response, &example_keys));
            let mut out = js::omit(&response, &["example", "examples", "schema"]);
            out.insert("content".into(), wrap_content(produces, &inner));
            return out;
        };
        let nullable = js::truthy(response.get("nullable"));
        let schema = match ty(Some(&t)) {
            Ty::Ctor(c @ ("String" | "Boolean" | "Number" | "Object" | "Array")) => {
                let swagger_type = schema_factory::map_type_to_openapi(c).unwrap_or_default();
                if is_array { json!({ "type": "array", "items": { "type": swagger_type } }) } else { json!({ "type": swagger_type }) }
            }
            _ => {
                let name = SchemaObjectFactory::explore_model_schema(&t, &mut self.schemas, &mut Vec::new());
                let r = schema_factory::get_schema_path(&name);
                if is_array { json!({ "type": "array", "items": { "$ref": r } }) } else { json!({ "$ref": r }) }
            }
        };
        let schema = if nullable { json!({ "oneOf": [schema, { "type": "null" }] }) } else { schema };
        let mut inner = Obj::new();
        inner.insert("schema".into(), schema);
        js::assign(&mut inner, &js::pick(&response, &example_keys));
        let mut out = js::omit(&response, &["example", "examples", "nullable"]);
        out.insert("content".into(), wrap_content(produces, &inner));
        out
    }

    /// `migrateOperationSchema`: the body parameter becomes `requestBody`.
    fn migrate_operation_schema(&self, doc: &mut Obj, class: &Metadata, method: &Metadata) {
        let Some(Value::Object(root)) = doc.get_mut("root") else { return };
        let Some(Value::Array(params)) = root.get_mut("parameters") else { return };
        let Some(idx) = params.iter().position(|p| p.get("in").and_then(Value::as_str) == Some("body")) else {
            return;
        };
        let body = js::obj_of(&params.remove(idx));
        let consumes = merge_and_uniq(class.array(keys::API_CONSUMES), method.array(keys::API_CONSUMES));
        let consumes = if consumes.is_empty() { vec![js::s("application/json")] } else { consumes };
        let mut rb = js::omit(&body, &["schema", "in", "name", "examples", "encoding"]);
        rb.insert("content".into(), wrap_content(&consumes, &js::pick(&body, &["schema", "examples", "encoding"])));
        root.insert("requestBody".into(), Value::Object(rb));
    }
}

fn is_body(p: &Obj) -> bool {
    p.get("in").and_then(Value::as_str) == Some("body")
}

/// `MimetypeContentWrapper.wrap(mimetypes, obj)`.
fn wrap_content(mimetypes: &[Value], obj: &Obj) -> Value {
    let mut content = Obj::new();
    for m in mimetypes {
        let key = match m {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        content.insert(key, Value::Object(obj.clone()));
    }
    Value::Object(content)
}

/// `mergeAndUniq(a, b)` = lodash `uniq(merge(a, b))` (arrays are merged index by index).
fn merge_and_uniq(a: Option<&Vec<Value>>, b: Option<&Vec<Value>>) -> Vec<Value> {
    let a = a.cloned().unwrap_or_default();
    let b = b.cloned().unwrap_or_default();
    let mut merged = a.clone();
    for (i, v) in b.into_iter().enumerate() {
        if i < merged.len() {
            merged[i] = v;
        } else {
            merged.push(v);
        }
    }
    let mut out: Vec<Value> = Vec::new();
    for v in merged {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// The `{ type, name, required: true, in }` reflected for a handler argument (`None` for
/// arguments that are not `@Body`/`@Param`/`@Query`/`@Headers`, and for named bodies).
fn reflected_param(p: &ParamDef) -> Option<Obj> {
    let location = match p.kind {
        ParamKind::Body => "body",
        ParamKind::Param => "path",
        ParamKind::Query => "query",
        ParamKind::Headers => "header",
        _ => return None,
    };
    if location == "body" && p.data.is_some_and(|d| !d.is_empty()) {
        return None;
    }
    let t = param_type(p);
    let mut o = Obj::new();
    o.insert("type".into(), t);
    // `name: param.data` exists even when undefined, so it keeps its place before `required`.
    o.insert("name".into(), p.data.map(js::s).unwrap_or(Value::Null));
    o.insert("required".into(), Value::Bool(true));
    o.insert("in".into(), js::s(location));
    Some(o)
}

/// `design:paramtypes[index]` of a handler argument.
fn param_type(p: &ParamDef) -> Value {
    if let Some(r) = p.api_schema {
        return types::api_schema_marker(r);
    }
    let name = p.metatype.name;
    if let Some(z) = p.metatype.zod {
        let short = corvis::openapi::short_type_name(name);
        return types::zod_marker(&format!("zod:{name}"), &short, z, types::ZodIo::Input);
    }
    match types::ctor_from_type_name(name) {
        Some(c) => types::ctor_marker(c),
        None => types::opaque_marker(name),
    }
}

/// `exploreApiCallbacksMetadata`.
fn explore_callbacks(method: &Metadata) -> Option<Obj> {
    let list = method.array(keys::API_CALLBACKS)?;
    let mut acc = Obj::new();
    for cb in list {
        let name = cb.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let url = cb.get("callbackUrl").and_then(Value::as_str).unwrap_or("").to_string();
        let m = cb.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let body_type = cb.get("requestBody").and_then(|b| b.get("type")).cloned().unwrap_or(Value::Null);
        let status = match cb.get("expectedResponse").and_then(|r| r.get("status")) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => "undefined".into(),
        };
        let description = cb
            .get("expectedResponse")
            .and_then(|r| r.get("description"))
            .filter(|d| js::truthy(Some(d)))
            .cloned()
            .unwrap_or_else(|| js::s("Your server returns this code if it accepts the callback"));
        let mut responses = Obj::new();
        responses.insert(status, json!({ "description": description }));
        let op = json!({
            "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": schema_factory::schema_path_of(&body_type) } } } },
            "responses": Value::Object(responses),
        });
        let mut by_method = Obj::new();
        by_method.insert(m, op);
        let mut by_url = Obj::new();
        by_url.insert(url, Value::Object(by_method));
        acc.insert(name, Value::Object(by_url));
    }
    Some(acc)
}

/// `mergeMetadata(globalMetadata, methodMetadata)`.
fn merge_metadata(global: &Obj, mut method: Obj) -> Obj {
    if let Some(Value::Object(root)) = method.get_mut("root")
        && !root.contains_key("parameters")
    {
        root.insert("parameters".into(), json!([]));
    }
    if let Some(Value::Array(chunks)) = global.get("chunks") {
        for chunk in chunks {
            let chunk = js::obj_of(chunk);
            let depth = chunk.get("depth").and_then(Value::as_u64);
            for (key, value) in method.iter_mut() {
                let Some(gv) = chunk.get(key).filter(|v| js::truthy(Some(v))) else { continue };
                *value = match depth {
                    Some(d) if d > 0 => deep_merge_metadata(gv, value, d, 0),
                    _ => merge_values(gv, value),
                };
            }
        }
    }
    for (key, value) in method.iter_mut() {
        let Some(gv) = global.get(key).filter(|v| js::truthy(Some(v))) else { continue };
        *value = merge_values(gv, value);
    }
    method
}

fn deep_merge_metadata(global: &Value, method: &Value, max: u64, cur: u64) -> Value {
    if cur == max {
        return merge_values(global, method);
    }
    let Value::Object(m) = method else { return method.clone() };
    let mut out = Obj::new();
    for (k, v) in m {
        let nv = match global.get(k) {
            Some(g) => deep_merge_metadata(g, v, max, cur + 1),
            None => v.clone(),
        };
        out.insert(k.clone(), nv);
    }
    Value::Object(out)
}

fn merge_values(global: &Value, method: &Value) -> Value {
    match global {
        Value::Array(g) => {
            let mut out = g.clone();
            match method {
                Value::Array(m) => out.extend(m.iter().cloned()),
                other => out.push(other.clone()),
            }
            Value::Array(out)
        }
        Value::Object(g) => {
            let mut out = g.clone();
            if let Value::Object(m) = method {
                js::assign(&mut out, m);
            }
            Value::Object(out)
        }
        _ => method.clone(),
    }
}

/// `validateRoutePath`: `/cats/:id` -> `/cats/{id}`, `''` for the root.
pub fn validate_route_path(path: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = path.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            ':' | '*' => {
                let mut j = i + 1;
                let mut name = String::new();
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '$') {
                    name.push(chars[j]);
                    j += 1;
                }
                if name.is_empty() {
                    if c == '*' {
                        // legacy `*` wildcard (`LegacyRouteConverter`): `*path`
                        out.push_str("{path}");
                    } else {
                        out.push(c);
                    }
                    i = j;
                    continue;
                }
                // legacy `:id(\d+)` / `:id?`
                if j < chars.len() && chars[j] == '(' {
                    let mut depth = 0;
                    while j < chars.len() {
                        if chars[j] == '(' {
                            depth += 1;
                        } else if chars[j] == ')' {
                            depth -= 1;
                            if depth == 0 {
                                j += 1;
                                break;
                            }
                        }
                        j += 1;
                    }
                }
                if j < chars.len() && chars[j] == '?' {
                    j += 1;
                }
                out.push('{');
                out.push_str(&name);
                out.push('}');
                i = j;
            }
            '{' | '}' => {
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    if out == "/" {
        return String::new();
    }
    if out.starts_with('/') { out } else { format!("/{out}") }
}

/// `SwaggerTransformer.normalizePaths`.
fn normalize_paths(denormalized: Vec<Obj>) -> Obj {
    let mut paths = Obj::new();
    for route in denormalized {
        let Some(Value::Object(root)) = route.get("root").cloned() else { continue };
        let path = root.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let method = root.get("method").and_then(Value::as_str).unwrap_or("").to_string();
        let mut merged = js::omit(&route, &["root"]);
        js::assign(&mut merged, &js::omit(&root, &["method", "path", "isWebhook", "webhookName"]));
        let mut keys: Vec<String> = merged.keys().cloned().collect();
        keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        let mut sorted = Obj::new();
        for k in keys {
            if let Some(v) = merged.get(&k) {
                sorted.insert(k, v.clone());
            }
        }
        let entry = paths.entry(path).or_insert_with(|| json!({}));
        if let Value::Object(e) = entry {
            e.insert(method, Value::Object(sorted));
        }
    }
    paths
}

/// `applyExampleMaxDepth(schemas, exampleMaxDepth)`.
fn apply_example_max_depth(schemas: &mut Obj, global: Option<i64>) {
    fn trim(v: &Value, depth: i64) -> Value {
        match v {
            Value::Array(a) => {
                if depth <= 0 {
                    json!([])
                } else {
                    Value::Array(a.iter().map(|x| trim(x, depth - 1)).collect())
                }
            }
            Value::Object(o) => {
                if depth <= 0 {
                    json!({})
                } else {
                    Value::Object(o.iter().map(|(k, x)| (k.clone(), trim(x, depth - 1))).collect())
                }
            }
            other => other.clone(),
        }
    }
    fn walk(s: &mut Value, global: Option<i64>) {
        let Value::Object(o) = s else { return };
        let per_prop = o.get("exampleMaxDepth").and_then(Value::as_i64);
        if let Some(max) = per_prop.or(global) {
            if let Some(e) = o.get("example").cloned() {
                o.insert("example".into(), trim(&e, max));
            }
            if let Some(Value::Array(list)) = o.get("examples").cloned() {
                o.insert("examples".into(), Value::Array(list.iter().map(|e| trim(e, max)).collect()));
            }
        }
        js::delete(o, "exampleMaxDepth");
        if let Some(Value::Object(props)) = o.get_mut("properties") {
            for (_, p) in props.iter_mut() {
                walk(p, global);
            }
        }
        if let Some(items) = o.get_mut("items") {
            walk(items, global);
        }
        for k in ["allOf", "oneOf", "anyOf"] {
            if let Some(Value::Array(list)) = o.get_mut(k) {
                for s in list.iter_mut() {
                    walk(s, global);
                }
            }
        }
        if let Some(ap @ Value::Object(_)) = o.get_mut("additionalProperties") {
            walk(ap, global);
        }
    }
    for (_, s) in schemas.iter_mut() {
        walk(s, global);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_paths() {
        assert_eq!(validate_route_path("/cats/:id"), "/cats/{id}");
        assert_eq!(validate_route_path("/"), "");
        assert_eq!(validate_route_path("/files/*path"), "/files/{path}");
        assert_eq!(validate_route_path("/a{/:b}"), "/a/{b}");
        assert_eq!(validate_route_path("/x/:id(\\d+)"), "/x/{id}");
    }

    #[test]
    fn camel() {
        assert_eq!(snake_to_camel("find_all"), "findAll");
        assert_eq!(snake_to_camel("login"), "login");
        assert_eq!(snake_to_camel("get_v2_data"), "getV2Data");
    }

    #[test]
    fn merge_uniq_is_index_wise() {
        let a = vec![json!("application/json")];
        let b = vec![json!("application/pdf")];
        assert_eq!(merge_and_uniq(Some(&a), Some(&b)), vec![json!("application/pdf")]);
        assert_eq!(merge_and_uniq(None, Some(&b)), vec![json!("application/pdf")]);
    }

    #[test]
    fn operation_ids() {
        assert_eq!(default_operation_id("CatsController", "findAll", None), "CatsController_findAll");
        assert_eq!(default_operation_id("CatsController", "findAll", Some("v1")), "CatsController_findAll_v1");
    }
}

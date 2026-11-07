//! `DocumentBuilder` and the OpenAPI document (`OpenAPIObject`).

use serde::Serialize;
use serde_json::{Value, json};

use crate::js::{self, Obj};

/// An OpenAPI document (`OpenAPIObject`). The content is kept as JSON so key order and
/// extensions are preserved exactly; serialize it with [`OpenApiDocument::to_json`],
/// [`to_json_pretty`](OpenApiDocument::to_json_pretty) or [`to_yaml`](OpenApiDocument::to_yaml).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenApiDocument {
    value: Value,
    /// `addGlobalParameters(...)` (the Node.js generator keeps them in a process-wide storage).
    pub(crate) global_parameters: Vec<Value>,
    /// `addGlobalResponse(...)`, grouped by status.
    pub(crate) global_responses: Obj,
}

impl OpenApiDocument {
    pub fn from_value(value: Value) -> Self {
        OpenApiDocument { value, global_parameters: Vec::new(), global_responses: Obj::new() }
    }

    pub fn as_value(&self) -> &Value {
        &self.value
    }

    pub fn as_value_mut(&mut self) -> &mut Value {
        &mut self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    pub(crate) fn obj(&self) -> Obj {
        js::obj_of(&self.value)
    }

    /// The root object (the value is turned into an object when it is not one).
    pub(crate) fn obj_mut(&mut self) -> &mut Obj {
        if !self.value.is_object() {
            self.value = Value::Object(Obj::new());
        }
        match &mut self.value {
            Value::Object(o) => o,
            _ => unreachable!(),
        }
    }

    /// `document.openapi`, `document.info`, ... (`None` when absent).
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.value.get(key)
    }

    /// `JSON.stringify(document)`, the body of `/<path>-json`.
    pub fn to_json(&self) -> String {
        js::stringify(&self.value)
    }

    /// `JSON.stringify(document, null, 2)`.
    pub fn to_json_pretty(&self) -> String {
        js::stringify_pretty(&self.value, 2)
    }

    /// `jsYaml.dump(document, { skipInvalid: true, noRefs: true })`, the body of `/<path>-yaml`.
    pub fn to_yaml(&self) -> String {
        let mut v = self.value.clone();
        js::js_key_order(&mut v);
        crate::yaml::dump(&v)
    }
}

impl Serialize for OpenApiDocument {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut v = self.value.clone();
        js::js_key_order(&mut v);
        v.serialize(s)
    }
}

impl From<Value> for OpenApiDocument {
    fn from(v: Value) -> Self {
        OpenApiDocument::from_value(v)
    }
}

/// Where `add_extension` writes (`'root'`, `'info'`, ...).
pub const ROOT: &str = "root";

/// `new DocumentBuilder()`: builds the base document passed to `SwaggerModule::create_document`.
///
/// ```ignore
/// let config = DocumentBuilder::new()
///     .set_title("Cats example")
///     .set_description("The cats API description")
///     .set_version("1.0")
///     .add_tag("cats", "")
///     .add_bearer_auth()
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct DocumentBuilder {
    document: Obj,
    global_parameters: Vec<Value>,
    global_responses: Obj,
}

impl Default for DocumentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn to_value(v: impl Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// Removes `null` members (JavaScript `undefined` never reaches `JSON.stringify`).
fn defined(pairs: Vec<(&str, Option<Value>)>) -> Obj {
    let mut o = Obj::new();
    for (k, v) in pairs {
        if let Some(v) = v {
            o.insert(k.to_string(), v);
        }
    }
    o
}

impl DocumentBuilder {
    pub fn new() -> Self {
        DocumentBuilder {
            document: js::obj_of(&json!({
                "openapi": "3.0.0",
                "info": { "title": "", "description": "", "version": "1.0.0", "contact": {} },
                "tags": [],
                "servers": [],
                "components": {}
            })),
            global_parameters: Vec::new(),
            global_responses: Obj::new(),
        }
    }

    fn info(&mut self) -> &mut Obj {
        self.document.get_mut("info").and_then(Value::as_object_mut).expect("info")
    }

    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.info().insert("title".into(), Value::String(title.into()));
        self
    }

    pub fn set_description(mut self, description: impl Into<String>) -> Self {
        self.info().insert("description".into(), Value::String(description.into()));
        self
    }

    pub fn set_version(mut self, version: impl Into<String>) -> Self {
        self.info().insert("version".into(), Value::String(version.into()));
        self
    }

    pub fn set_terms_of_service(mut self, terms_of_service: impl Into<String>) -> Self {
        self.info().insert("termsOfService".into(), Value::String(terms_of_service.into()));
        self
    }

    /// `setContact(name, url, email)`.
    pub fn set_contact(mut self, name: impl Into<String>, url: impl Into<String>, email: impl Into<String>) -> Self {
        let c = json!({ "name": name.into(), "url": url.into(), "email": email.into() });
        self.info().insert("contact".into(), c);
        self
    }

    /// `setContact(...)` with optional members.
    pub fn set_contact_opt(mut self, name: Option<&str>, url: Option<&str>, email: Option<&str>) -> Self {
        let c = defined(vec![("name", name.map(js::s)), ("url", url.map(js::s)), ("email", email.map(js::s))]);
        self.info().insert("contact".into(), Value::Object(c));
        self
    }

    /// `setLicense(name, url)`.
    pub fn set_license(mut self, name: impl Into<String>, url: impl Into<String>) -> Self {
        let l = json!({ "name": name.into(), "url": url.into() });
        self.info().insert("license".into(), l);
        self
    }

    /// `setOpenAPIVersion('3.1.0')` (ignored with a warning when not `x.y.z`).
    pub fn set_open_api_version(mut self, version: &str) -> Self {
        let ok = {
            let parts: Vec<&str> = version.split('.').collect();
            parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        if ok {
            self.document.insert("openapi".into(), js::s(version));
        } else {
            corvis::Logger::new("DocumentBuilder").warn("The OpenApi version is invalid. Expecting format \"x.x.x\"");
        }
        self
    }

    /// `addServer(url, description?, variables?)`.
    pub fn add_server(self, url: impl Into<String>, description: Option<&str>, variables: Option<Value>) -> Self {
        self.add_server_with(url, description, variables, None)
    }

    /// `addServer(url, description, variables, serverExtraProperties)`.
    pub fn add_server_with(
        mut self,
        url: impl Into<String>,
        description: Option<&str>,
        variables: Option<Value>,
        extra: Option<Value>,
    ) -> Self {
        let mut server = defined(vec![("url", Some(Value::String(url.into()))), ("description", description.map(js::s)), ("variables", variables)]);
        if let Some(Value::Object(extra)) = extra {
            for (k, v) in extra {
                if matches!(k.as_str(), "url" | "description" | "variables") {
                    continue;
                }
                server.insert(k, v);
            }
        }
        if let Some(Value::Array(a)) = self.document.get_mut("servers") {
            a.push(Value::Object(server));
        }
        self
    }

    /// `setExternalDoc(description, url)`.
    pub fn set_external_doc(mut self, description: impl Into<String>, url: impl Into<String>) -> Self {
        self.document.insert("externalDocs".into(), json!({ "description": description.into(), "url": url.into() }));
        self
    }

    /// Deprecated; only logs a warning.
    pub fn set_base_path(self, _path: &str) -> Self {
        corvis::Logger::new("DocumentBuilder").warn(
            "The \"setBasePath\" method has been deprecated. Now, a global prefix is populated automatically. Use `ignore_global_prefix` to leave it out. Alternatively, you can use \"addServer\" method to set up multiple different paths.",
        );
        self
    }

    /// `addTag(name, description = '')`.
    pub fn add_tag(self, name: impl Into<String>, description: impl Into<String>) -> Self {
        self.add_tag_with(name, description, None, TagOptions::default())
    }

    /// `addTag(name, description, externalDocs, { summary, parent, kind })`.
    pub fn add_tag_with(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        external_docs: Option<Value>,
        options: TagOptions,
    ) -> Self {
        let tag = defined(vec![
            ("name", Some(Value::String(name.into()))),
            ("summary", options.summary.map(Value::String)),
            ("description", Some(Value::String(description.into()))),
            ("externalDocs", external_docs),
            ("parent", options.parent.map(Value::String)),
            ("kind", options.kind.map(Value::String)),
        ]);
        if let Some(Value::Array(a)) = self.document.get_mut("tags") {
            a.push(Value::Object(tag));
        }
        self
    }

    /// `addExtension('x-key', value, location = 'root')`.
    ///
    /// # Panics
    /// When the key does not start with `x-` (the same error the Node.js generator throws).
    pub fn add_extension(mut self, key: &str, value: impl Serialize, location: &str) -> Self {
        assert!(key.starts_with("x-"), "Extension key is not prefixed. Please ensure you prefix it with `x-`.");
        let value = to_value(value);
        if location == ROOT {
            self.document.insert(key.to_string(), value);
        } else if let Some(Value::Object(target)) = self.document.get_mut(location) {
            target.insert(key.to_string(), value);
        } else {
            let mut o = Obj::new();
            o.insert(key.to_string(), value);
            self.document.insert(location.to_string(), Value::Object(o));
        }
        self
    }

    /// `addSecurity(name, securitySchemeObject)`.
    pub fn add_security(mut self, name: impl Into<String>, options: impl Serialize) -> Self {
        let components = self.document.entry("components").or_insert_with(|| json!({}));
        let Value::Object(components) = components else { return self };
        let mut schemes = components.get("securitySchemes").and_then(Value::as_object).cloned().unwrap_or_default();
        schemes.insert(name.into(), to_value(options));
        components.insert("securitySchemes".into(), Value::Object(schemes));
        self
    }

    /// `addGlobalResponse(...responses)`: each response may carry a `status` (default
    /// `'default'`).
    pub fn add_global_response(mut self, responses: impl IntoIterator<Item = Value>) -> Self {
        for r in responses {
            let mut r = js::obj_of(&r);
            let status = match js::delete(&mut r, "status") {
                Some(Value::String(s)) => s,
                Some(Value::Number(n)) => n.to_string(),
                _ => "default".to_string(),
            };
            self.global_responses.insert(status, Value::Object(r));
        }
        self
    }

    /// `addGlobalParameters(...parameters)`.
    pub fn add_global_parameters(mut self, parameters: impl IntoIterator<Item = Value>) -> Self {
        self.global_parameters.extend(parameters);
        self
    }

    /// `addSecurityRequirements(name, requirements = [])`.
    pub fn add_security_requirements(mut self, name: impl Into<String>, requirements: &[&str]) -> Self {
        let mut req = Obj::new();
        req.insert(name.into(), json!(requirements));
        self.push_security(Value::Object(req));
        self
    }

    /// `addSecurityRequirements({ name: [...], ... })`.
    pub fn add_security_requirements_object(mut self, requirement: Value) -> Self {
        self.push_security(requirement);
        self
    }

    fn push_security(&mut self, v: Value) {
        match self.document.get_mut("security") {
            Some(Value::Array(a)) => a.push(v),
            _ => {
                self.document.insert("security".into(), Value::Array(vec![v]));
            }
        }
    }

    fn with_defaults(defaults: Value, options: Option<Value>, fallback: Value) -> Value {
        let mut out = js::obj_of(&defaults);
        js::assign(&mut out, &js::obj_of(&options.unwrap_or(fallback)));
        Value::Object(out)
    }

    /// `addBearerAuth()`: `bearer` scheme `{ scheme: 'bearer', bearerFormat: 'JWT', type: 'http' }`.
    pub fn add_bearer_auth(self) -> Self {
        self.add_bearer_auth_with(None, "bearer")
    }

    /// `addBearerAuth(options, name)`.
    pub fn add_bearer_auth_with(self, options: Option<Value>, name: &str) -> Self {
        let v = Self::with_defaults(json!({ "scheme": "bearer", "bearerFormat": "JWT" }), options, json!({ "type": "http" }));
        self.add_security(name, v)
    }

    /// `addOAuth2()`.
    pub fn add_oauth2(self) -> Self {
        self.add_oauth2_with(None, "oauth2")
    }

    /// `addOAuth2(options, name)`.
    pub fn add_oauth2_with(self, options: Option<Value>, name: &str) -> Self {
        let v = Self::with_defaults(json!({ "type": "oauth2", "flows": {} }), options, json!({ "type": "oauth2" }));
        self.add_security(name, v)
    }

    /// `addApiKey()`: `{ type: 'apiKey', in: 'header', name: 'api_key' }`.
    pub fn add_api_key(self) -> Self {
        self.add_api_key_with(None, "api_key")
    }

    /// `addApiKey(options, name)`.
    pub fn add_api_key_with(self, options: Option<Value>, name: &str) -> Self {
        let v = Self::with_defaults(json!({ "type": "apiKey", "in": "header", "name": name }), options, json!({ "type": "apiKey" }));
        self.add_security(name, v)
    }

    /// `addBasicAuth()`.
    pub fn add_basic_auth(self) -> Self {
        self.add_basic_auth_with(None, "basic")
    }

    /// `addBasicAuth(options, name)`.
    pub fn add_basic_auth_with(self, options: Option<Value>, name: &str) -> Self {
        let v = Self::with_defaults(json!({ "type": "http", "scheme": "basic" }), options, json!({ "type": "http" }));
        self.add_security(name, v)
    }

    /// `addCookieAuth()`: cookie `connect.sid`, scheme name `cookie`.
    pub fn add_cookie_auth(self) -> Self {
        self.add_cookie_auth_with("connect.sid", None, "cookie")
    }

    /// `addCookieAuth(cookieName, options, securityName)`.
    pub fn add_cookie_auth_with(self, cookie_name: &str, options: Option<Value>, security_name: &str) -> Self {
        let v = Self::with_defaults(json!({ "type": "apiKey", "in": "cookie", "name": cookie_name }), options, json!({ "type": "apiKey" }));
        self.add_security(security_name, v)
    }

    /// `build()`.
    pub fn build(self) -> OpenApiDocument {
        OpenApiDocument {
            value: Value::Object(self.document),
            global_parameters: self.global_parameters,
            global_responses: self.global_responses,
        }
    }
}

/// Options of [`DocumentBuilder::add_tag_with`] (OpenAPI 3.2 tag fields).
#[derive(Debug, Clone, Default)]
pub struct TagOptions {
    pub summary: Option<String>,
    pub parent: Option<String>,
    pub kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_document() {
        let d = DocumentBuilder::new().build();
        assert_eq!(
            d.to_json(),
            r#"{"openapi":"3.0.0","info":{"title":"","description":"","version":"1.0.0","contact":{}},"tags":[],"servers":[],"components":{}}"#
        );
    }

    #[test]
    fn security_schemes() {
        let d = DocumentBuilder::new()
            .add_bearer_auth()
            .add_basic_auth()
            .add_api_key()
            .add_oauth2()
            .add_cookie_auth()
            .add_bearer_auth_with(Some(json!({ "type": "http", "description": "x", "bearerFormat": "opaque" })), "custom")
            .build();
        assert_eq!(
            js::stringify(&d.as_value()["components"]),
            concat!(
                r#"{"securitySchemes":{"bearer":{"scheme":"bearer","bearerFormat":"JWT","type":"http"},"#,
                r#""basic":{"type":"http","scheme":"basic"},"api_key":{"type":"apiKey","in":"header","name":"api_key"},"#,
                r#""oauth2":{"type":"oauth2","flows":{}},"cookie":{"type":"apiKey","in":"cookie","name":"connect.sid"},"#,
                r#""custom":{"scheme":"bearer","bearerFormat":"opaque","type":"http","description":"x"}}}"#
            )
        );
    }
}

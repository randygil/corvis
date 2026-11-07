//! Folds the decorator entries of a controller or handler into the metadata TypeScript
//! decorators would have stored with `Reflect.defineMetadata`.
//!
//! TypeScript applies decorators bottom to top, and the decorators of every method before the
//! ones of the class. Corvis keeps attributes in declaration order, so entries are applied in
//! reverse. Class-level `@ApiParam`/`@ApiQuery`/`@ApiHeader`/`@ApiBody` and `@ApiExtension`
//! apply to every route handler of the class (after the handler's own decorators).

use serde_json::{Value, json};

use crate::decorators::{keys, merge_response_entry};
use crate::js::{self, Obj};

/// `Reflect.getMetadata(key, target)` for one class or method.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub values: Obj,
}

impl Metadata {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn array(&self, key: &str) -> Option<&Vec<Value>> {
        self.values.get(key).and_then(Value::as_array)
    }

    fn concat(&mut self, key: &str, v: &Value) {
        let items = v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]);
        match self.values.get_mut(key) {
            Some(Value::Array(a)) => a.extend(items),
            _ => {
                self.values.insert(key.to_string(), Value::Array(items));
            }
        }
    }

    fn assign(&mut self, key: &str, v: &Value) {
        let mut cur = self.values.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
        if let Value::Object(o) = v {
            js::assign(&mut cur, o);
        }
        self.values.insert(key.to_string(), Value::Object(cur));
    }

    fn push_param(&mut self, param: &Value) {
        self.concat(keys::API_PARAMETERS, &json!([param]));
    }

    fn add_response(&mut self, v: &Value) {
        let status = v.get("status").and_then(Value::as_str).unwrap_or("default").to_string();
        let entry = v.get("entry").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut responses = self.values.get(keys::API_RESPONSE).and_then(Value::as_object).cloned().unwrap_or_default();
        let merged = merge_response_entry(responses.get(&status).and_then(Value::as_object), &entry);
        responses.insert(status, Value::Object(merged));
        self.values.insert(keys::API_RESPONSE.to_string(), Value::Object(responses));
    }

    fn add_extension(&mut self, v: &Value) {
        let key = v.get("key").and_then(Value::as_str).unwrap_or_default().to_string();
        let value = v.get("value").cloned().unwrap_or(Value::Null);
        // `{ [key]: value, ...existing }`
        let mut out = Obj::new();
        out.insert(key, value);
        if let Some(Value::Object(existing)) = self.values.get(keys::API_EXTENSION) {
            js::assign(&mut out, existing);
        }
        self.values.insert(keys::API_EXTENSION.to_string(), Value::Object(out));
    }
}

fn header_param(v: &Value) -> Value {
    // `createParamDecorator(param, { name: '' })`
    let mut out = js::obj_of(&json!({ "name": "" }));
    if let Value::Object(o) = v {
        js::assign(&mut out, o);
    }
    Value::Object(out)
}

/// Metadata of a controller class.
pub fn fold_class(entries: &[(String, Value)]) -> Metadata {
    let mut m = Metadata::default();
    for (k, v) in entries.iter().rev() {
        match k.as_str() {
            keys::API_RESPONSE => m.add_response(v),
            keys::API_PRODUCES | keys::API_CONSUMES | keys::API_TAGS | keys::API_CALLBACKS | keys::API_SECURITY
            | keys::API_EXTRA_MODELS | keys::API_EXCLUDE_CONTROLLER => m.concat(k, v),
            keys::API_HEADER => m.concat(keys::API_HEADERS, &json!([v])),
            keys::API_OPERATION | keys::API_EXCLUDE_ENDPOINT | keys::API_INCLUDE_ENDPOINT => m.assign(k, v),
            // Applied to the methods (see `fold_method`).
            keys::API_PARAMETERS | keys::API_EXTENSION => {}
            _ => {}
        }
    }
    m
}

/// Metadata of a route handler (`class_entries` are the controller's entries).
pub fn fold_method(entries: &[(String, Value)], class_entries: &[(String, Value)]) -> Metadata {
    let mut m = Metadata::default();
    for (k, v) in entries.iter().rev() {
        match k.as_str() {
            keys::API_RESPONSE => m.add_response(v),
            keys::API_PRODUCES | keys::API_CONSUMES | keys::API_TAGS | keys::API_CALLBACKS | keys::API_SECURITY
            | keys::API_EXTRA_MODELS => m.concat(k, v),
            keys::API_PARAMETERS => m.push_param(v),
            keys::API_HEADER => m.push_param(&header_param(v)),
            keys::API_OPERATION | keys::API_EXCLUDE_ENDPOINT | keys::API_INCLUDE_ENDPOINT => m.assign(k, v),
            keys::API_EXTENSION => m.add_extension(v),
            _ => {}
        }
    }
    for (k, v) in class_entries.iter().rev() {
        match k.as_str() {
            keys::API_PARAMETERS => m.push_param(v),
            keys::API_EXTENSION => m.add_extension(v),
            _ => {}
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decorators::*;

    fn entries(ds: Vec<corvis::CustomDecorator>) -> Vec<(String, Value)> {
        ds.into_iter().flat_map(|d| d.into_entries()).collect()
    }

    #[test]
    fn bottom_up_order() {
        // @ApiTags('a') @ApiTags('b') -> ['b', 'a']
        let m = fold_method(&entries(vec![api_tags(vec![json!("a")]), api_tags(vec![json!("b")])]), &[]);
        assert_eq!(m.get(keys::API_TAGS), Some(&json!(["b", "a"])));
    }

    #[test]
    fn responses_merge() {
        let e = entries(vec![
            api_status_response(200, json!({ "description": "top" })),
            api_status_response(404, json!({})),
            api_status_response(200, json!({ "description": "bottom" })),
        ]);
        let m = fold_method(&e, &[]);
        assert_eq!(
            js::stringify(m.get(keys::API_RESPONSE).unwrap()),
            r#"{"200":{"description":"bottom\n\ntop"},"404":{"description":""}}"#
        );
    }

    #[test]
    fn class_params_after_method_params() {
        let class = entries(vec![api_query(json!({ "name": "c" }))]);
        let m = fold_method(&entries(vec![api_query(json!({ "name": "m" }))]), &class);
        let names: Vec<_> = m.array(keys::API_PARAMETERS).unwrap().iter().map(|p| p["name"].clone()).collect();
        assert_eq!(names, vec![json!("m"), json!("c")]);
    }

    #[test]
    fn class_headers() {
        let class = fold_class(&entries(vec![api_header(json!({ "name": "X-A" }))]));
        assert_eq!(class.array(keys::API_HEADERS).unwrap().len(), 1);
    }
}

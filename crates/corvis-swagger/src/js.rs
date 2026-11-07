//! JavaScript object semantics on top of `serde_json` (built with `preserve_order`).
//!
//! The OpenAPI and zod DTO generators are ported literally from JavaScript, so key order
//! follows the same rules as in JavaScript: `Object.assign` keeps the position of existing
//! keys and appends new ones, `delete` keeps the order of the rest, lodash `pick` follows the
//! order of the requested keys, and integer-like keys (`"200"`, `"404"`) come first in
//! ascending order when the document is serialized (see [`js_key_order`]).

use serde_json::{Map, Value};

pub type Obj = Map<String, Value>;

/// `Object.assign(target, source)`.
pub fn assign(target: &mut Obj, source: &Obj) {
    for (k, v) in source {
        target.insert(k.clone(), v.clone());
    }
}

/// `{ ...a, ...b, ... }`.
pub fn spread(parts: &[&Obj]) -> Obj {
    let mut out = Obj::new();
    for p in parts {
        assign(&mut out, p);
    }
    out
}

/// lodash `omit(obj, keys)`.
pub fn omit(o: &Obj, keys: &[&str]) -> Obj {
    o.iter().filter(|(k, _)| !keys.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// lodash `pick(obj, keys)` (result in the order of `keys`).
pub fn pick(o: &Obj, keys: &[&str]) -> Obj {
    let mut out = Obj::new();
    for k in keys {
        if let Some(v) = o.get(*k) {
            out.insert((*k).to_string(), v.clone());
        }
    }
    out
}

/// `delete obj[key]` (keeps the order of the remaining keys).
pub fn delete(o: &mut Obj, key: &str) -> Option<Value> {
    o.shift_remove(key)
}

/// lodash `isEmpty(value)`: `undefined`/`null`, empty strings, arrays and objects, and every
/// number/boolean are empty.
pub fn is_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(_)) | Some(Value::Number(_)) => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
    }
}

/// JavaScript truthiness.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0 && !f.is_nan()).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `typeof v === 'string'`.
pub fn as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

pub fn obj_of(v: &Value) -> Obj {
    v.as_object().cloned().unwrap_or_default()
}

pub fn s(v: impl Into<String>) -> Value {
    Value::String(v.into())
}

/// `true` for canonical array-index keys (`"0"`, `"200"`, not `"01"` or `"-1"`), which
/// JavaScript orders before every other key.
pub fn is_array_index(k: &str) -> bool {
    if k.is_empty() || k.len() > 10 || !k.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if k.len() > 1 && k.starts_with('0') {
        return false;
    }
    k.parse::<u64>().map(|n| n < 4_294_967_295).unwrap_or(false)
}

/// Reorders every object like a JavaScript engine would: integer-like keys first, ascending,
/// then the rest in insertion order.
pub fn js_key_order(v: &mut Value) {
    match v {
        Value::Object(o) => {
            if o.keys().any(|k| is_array_index(k)) {
                let old = std::mem::take(o);
                let mut ints: Vec<(u64, String, Value)> = Vec::new();
                let mut rest: Vec<(String, Value)> = Vec::new();
                for (k, v) in old {
                    if is_array_index(&k) {
                        ints.push((k.parse().unwrap_or(0), k, v));
                    } else {
                        rest.push((k, v));
                    }
                }
                ints.sort_by_key(|(n, _, _)| *n);
                for (_, k, v) in ints {
                    o.insert(k, v);
                }
                for (k, v) in rest {
                    o.insert(k, v);
                }
            }
            for (_, child) in o.iter_mut() {
                js_key_order(child);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(js_key_order),
        _ => {}
    }
}

/// `JSON.stringify(value)` (compact) with JavaScript key order.
pub fn stringify(v: &Value) -> String {
    let mut v = v.clone();
    js_key_order(&mut v);
    serde_json::to_string(&v).unwrap_or_default()
}

/// `JSON.stringify(value, null, indent)` with JavaScript key order.
pub fn stringify_pretty(v: &Value, indent: usize) -> String {
    let mut v = v.clone();
    js_key_order(&mut v);
    let ind = " ".repeat(indent);
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(ind.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(&v, &mut ser).ok();
    String::from_utf8(buf).unwrap_or_default()
}

/// Deep equality (`isDeepStrictEqual` / lodash `isEqual`): key order does not matter.
pub fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w))),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(v, w)| deep_equal(v, w)),
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_like_js() {
        let mut v = json!({ "default": 1, "404": 2, "b": 3, "200": 4, "01": 5 });
        js_key_order(&mut v);
        assert_eq!(serde_json::to_string(&v).unwrap(), r#"{"200":4,"404":2,"default":1,"b":3,"01":5}"#);
    }

    #[test]
    fn assign_keeps_position() {
        let mut a = obj_of(&json!({ "a": 1, "b": 2 }));
        assign(&mut a, &obj_of(&json!({ "c": 3, "a": 9 })));
        assert_eq!(Value::Object(a), json!({ "a": 9, "b": 2, "c": 3 }));
        let mut o = obj_of(&json!({ "a": 1, "b": 2, "c": 3 }));
        delete(&mut o, "a");
        assert_eq!(serde_json::to_string(&o).unwrap(), r#"{"b":2,"c":3}"#);
        assert_eq!(serde_json::to_string(&pick(&obj_of(&json!({ "a": 1, "b": 2 })), &["b", "a"])).unwrap(), r#"{"b":2,"a":1}"#);
    }

    #[test]
    fn lodash_is_empty() {
        assert!(is_empty(None));
        assert!(is_empty(Some(&json!(5))));
        assert!(is_empty(Some(&json!({}))));
        assert!(!is_empty(Some(&json!([1]))));
        assert!(!is_empty(Some(&json!("x"))));
    }

    #[test]
    fn pretty() {
        assert_eq!(stringify_pretty(&json!({ "a": [1] }), 2), "{\n  \"a\": [\n    1\n  ]\n}");
        assert_eq!(stringify(&json!({ "a": 1 })), r#"{"a":1}"#);
        assert!(deep_equal(&json!({ "a": 1, "b": 2 }), &json!({ "b": 2, "a": 1 })));
    }
}

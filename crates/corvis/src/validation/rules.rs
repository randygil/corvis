//! Validation rules used by `#[derive(Validate)]` (class-validator equivalents).
//! Every rule receives `Option<&Value>` (`None` = property missing).

use std::sync::LazyLock;

use regex::Regex;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use super::{ValidationError, Validate, ValidatorOptions};

type V<'a> = Option<&'a Value>;

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)+$").unwrap()
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$").unwrap()
});
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?i)(https?|ftp)://(([a-z0-9\-._~%!$&'()*+,;=]+(:[^@\s]*)?@)?([a-z0-9\-]+\.)+[a-z]{2,}|localhost|\d{1,3}(\.\d{1,3}){3})(:\d{1,5})?(/[^\s]*)?$").unwrap()
});
static ISO8601: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}([T ]\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$").unwrap()
});
static HEX_COLOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#?([0-9a-fA-F]{3}|[0-9a-fA-F]{4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$").unwrap());
static JWT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.?[A-Za-z0-9_-]*$").unwrap());
static MONGO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-fA-F]{24}$").unwrap());

fn as_str(v: V<'_>) -> Option<&str> {
    v.and_then(|v| v.as_str())
}

pub fn is_missing(v: V<'_>) -> bool {
    matches!(v, None | Some(Value::Null))
}
pub fn is_defined(v: V<'_>) -> bool {
    !is_missing(v)
}
pub fn is_string(v: V<'_>) -> bool {
    matches!(v, Some(Value::String(_)))
}
pub fn is_int(v: V<'_>) -> bool {
    match v {
        Some(Value::Number(n)) => n.is_i64() || n.is_u64() || n.as_f64().map(|f| f.fract() == 0.0).unwrap_or(false),
        _ => false,
    }
}
pub fn is_number(v: V<'_>) -> bool {
    matches!(v, Some(Value::Number(_)))
}
pub fn is_boolean(v: V<'_>) -> bool {
    matches!(v, Some(Value::Bool(_)))
}
pub fn is_array(v: V<'_>) -> bool {
    matches!(v, Some(Value::Array(_)))
}
pub fn is_object(v: V<'_>) -> bool {
    matches!(v, Some(Value::Object(_)))
}
pub fn is_not_empty(v: V<'_>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}
pub fn is_empty(v: V<'_>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.is_empty(),
        _ => false,
    }
}
pub fn is_email(v: V<'_>) -> bool {
    as_str(v).map(|s| EMAIL.is_match(s)).unwrap_or(false)
}
pub fn is_url(v: V<'_>) -> bool {
    as_str(v).map(|s| URL.is_match(s)).unwrap_or(false)
}
pub fn is_uuid(s: &str) -> bool {
    UUID.is_match(s)
}
pub fn is_uuid_value(v: V<'_>) -> bool {
    as_str(v).map(is_uuid).unwrap_or(false)
}
pub fn is_alpha(v: V<'_>) -> bool {
    as_str(v).map(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())).unwrap_or(false)
}
pub fn is_alphanumeric(v: V<'_>) -> bool {
    as_str(v).map(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or(false)
}
pub fn is_number_string(v: V<'_>) -> bool {
    as_str(v).map(|s| !s.trim().is_empty() && s.trim().parse::<f64>().is_ok()).unwrap_or(false)
}
pub fn is_date_string(v: V<'_>) -> bool {
    as_str(v).map(|s| ISO8601.is_match(s)).unwrap_or(false)
}
pub fn is_hex_color(v: V<'_>) -> bool {
    as_str(v).map(|s| HEX_COLOR.is_match(s)).unwrap_or(false)
}
pub fn is_jwt(v: V<'_>) -> bool {
    as_str(v).map(|s| JWT.is_match(s)).unwrap_or(false)
}
pub fn is_mongo_id(v: V<'_>) -> bool {
    as_str(v).map(|s| MONGO_ID.is_match(s)).unwrap_or(false)
}
pub fn is_ip(v: V<'_>) -> bool {
    as_str(v).map(|s| s.parse::<std::net::IpAddr>().is_ok()).unwrap_or(false)
}
pub fn is_lowercase(v: V<'_>) -> bool {
    as_str(v).map(|s| s == s.to_lowercase()).unwrap_or(false)
}
pub fn is_uppercase(v: V<'_>) -> bool {
    as_str(v).map(|s| s == s.to_uppercase()).unwrap_or(false)
}
pub fn min_length(v: V<'_>, n: usize) -> bool {
    as_str(v).map(|s| s.chars().count() >= n).unwrap_or(false)
}
pub fn max_length(v: V<'_>, n: usize) -> bool {
    as_str(v).map(|s| s.chars().count() <= n).unwrap_or(false)
}
pub fn min(v: V<'_>, n: f64) -> bool {
    v.and_then(|v| v.as_f64()).map(|x| x >= n).unwrap_or(false)
}
pub fn max(v: V<'_>, n: f64) -> bool {
    v.and_then(|v| v.as_f64()).map(|x| x <= n).unwrap_or(false)
}
pub fn is_positive(v: V<'_>) -> bool {
    v.and_then(|v| v.as_f64()).map(|x| x > 0.0).unwrap_or(false)
}
pub fn is_negative(v: V<'_>) -> bool {
    v.and_then(|v| v.as_f64()).map(|x| x < 0.0).unwrap_or(false)
}
pub fn matches(v: V<'_>, re: &Regex) -> bool {
    as_str(v).map(|s| re.is_match(s)).unwrap_or(false)
}
pub fn contains(v: V<'_>, seed: &str) -> bool {
    as_str(v).map(|s| s.contains(seed)).unwrap_or(false)
}
pub fn not_contains(v: V<'_>, seed: &str) -> bool {
    as_str(v).map(|s| !s.contains(seed)).unwrap_or(false)
}
pub fn is_in(v: V<'_>, allowed: &[Value]) -> bool {
    v.map(|v| allowed.contains(v)).unwrap_or(false)
}
pub fn is_not_in(v: V<'_>, disallowed: &[Value]) -> bool {
    v.map(|v| !disallowed.contains(v)).unwrap_or(false)
}
pub fn equals(v: V<'_>, expected: &Value) -> bool {
    v == Some(expected)
}
pub fn not_equals(v: V<'_>, expected: &Value) -> bool {
    v != Some(expected)
}
pub fn array_min_size(v: V<'_>, n: usize) -> bool {
    v.and_then(|v| v.as_array()).map(|a| a.len() >= n).unwrap_or(false)
}
pub fn array_max_size(v: V<'_>, n: usize) -> bool {
    v.and_then(|v| v.as_array()).map(|a| a.len() <= n).unwrap_or(false)
}
pub fn array_not_empty(v: V<'_>) -> bool {
    v.and_then(|v| v.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
}
/// `arrayUnique`: no two elements are equal (`serde_json::Value` equality). Linear: elements are
/// bucketed by a hash consistent with that equality and only compared within a bucket (the
/// previous pairwise scan was O(n²), ~350 ms for a 100 KB body).
pub fn array_unique(v: V<'_>) -> bool {
    let Some(a) = v.and_then(|v| v.as_array()) else { return false };
    if a.len() < 2 {
        return true;
    }
    let mut buckets: std::collections::HashMap<u64, Vec<&Value>> = std::collections::HashMap::with_capacity(a.len());
    for x in a {
        let bucket = buckets.entry(value_hash(x)).or_default();
        if bucket.contains(&x) {
            return false;
        }
        bucket.push(x);
    }
    true
}

/// A hash consistent with `Value`'s `PartialEq` (object keys in any order are equal).
fn value_hash(v: &Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match v {
        Value::Null => 0u8.hash(&mut h),
        Value::Bool(b) => (1u8, b).hash(&mut h),
        Value::Number(n) => (2u8, n).hash(&mut h),
        Value::String(s) => (3u8, s).hash(&mut h),
        Value::Array(a) => {
            (4u8, a.len()).hash(&mut h);
            for x in a {
                value_hash(x).hash(&mut h);
            }
        }
        Value::Object(o) => {
            // Order-independent: the entries' hashes are combined commutatively.
            let sum = o.iter().fold(0u64, |acc, (k, x)| {
                let mut e = std::collections::hash_map::DefaultHasher::new();
                (k, value_hash(x)).hash(&mut e);
                acc.wrapping_add(e.finish())
            });
            (5u8, o.len(), sum).hash(&mut h);
        }
    }
    h.finish()
}
pub fn is_enum<T: DeserializeOwned>(v: V<'_>) -> bool {
    v.map(|v| serde_json::from_value::<T>(v.clone()).is_ok()).unwrap_or(false)
}

/// Applies `rule` to every element (`{ each: true }`).
pub fn each(v: V<'_>, rule: impl Fn(V<'_>) -> bool) -> bool {
    match v {
        Some(Value::Array(a)) => a.iter().all(|x| rule(Some(x))),
        other => rule(other),
    }
}

/// Variant names of a serde enum (respecting `rename_all`).
pub fn enum_variants<T: DeserializeOwned>() -> &'static [&'static str] {
    match T::deserialize(crate::common::lenient::VariantsCapture) {
        Err(crate::common::lenient::CaptureError(Some(v))) => v,
        _ => &[],
    }
}

/// Builds the final message: the custom one (with `$property` / `$value` tokens) or the default.
pub fn message(custom: Option<&str>, default: String, property: &str, value: V<'_>) -> String {
    match custom {
        Some(c) => {
            let val = match value {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => "undefined".into(),
            };
            c.replace("$property", property).replace("$value", &val)
        }
        None => default,
    }
}

pub fn join_values(values: &[Value]) -> String {
    values
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Primitive kind used for implicit conversion of query/route params.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    Int,
    Float,
    Bool,
    Other,
}

/// Converts `"42"` / `"true"` into numbers / booleans in place.
pub fn coerce(obj: &mut Map<String, Value>, prop: &str, kind: Primitive) {
    let Some(v) = obj.get_mut(prop) else { return };
    if let Value::String(s) = v {
        let s = s.trim().to_string();
        match kind {
            Primitive::Int => {
                if let Ok(n) = s.parse::<i64>() {
                    *v = Value::from(n);
                } else if let Ok(n) = s.parse::<u64>() {
                    *v = Value::from(n);
                }
            }
            Primitive::Float => {
                if let Ok(n) = s.parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(n) {
                        *v = Value::Number(num);
                    }
                }
            }
            Primitive::Bool => match s.as_str() {
                "true" | "1" => *v = Value::Bool(true),
                "false" | "0" => *v = Value::Bool(false),
                _ => {}
            },
            Primitive::Other => {}
        }
    }
}

/// Strips (or rejects) properties not declared in the DTO.
pub fn whitelist(obj: &mut Map<String, Value>, allowed: &[&str], forbid: bool, errors: &mut Vec<ValidationError>) {
    let unknown: Vec<String> = obj.keys().filter(|k| !allowed.contains(&k.as_str())).cloned().collect();
    for k in unknown {
        if forbid {
            let mut e = ValidationError::new(k.clone());
            e.value = obj.get(&k).cloned();
            e.constraints.push(("whitelistValidation".into(), format!("property {k} should not exist")));
            errors.push(e);
        } else {
            obj.shift_remove(&k);
        }
    }
}

/// Validates a nested DTO (object or array of objects).
pub fn nested<T: Validate>(
    value: Option<&mut Value>,
    property: &str,
    options: &ValidatorOptions,
    constraints: &mut Vec<(String, String)>,
) -> Vec<ValidationError> {
    match value {
        Some(v @ Value::Object(_)) => T::validate_value(v, options),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for (i, item) in items.iter_mut().enumerate() {
                if item.is_object() {
                    let children = T::validate_value(item, options);
                    if !children.is_empty() {
                        let mut e = ValidationError::new(i.to_string());
                        e.children = children;
                        out.push(e);
                    }
                } else {
                    constraints.push((
                        "nestedValidation".into(),
                        format!("each value in nested property {property} must be either object or array"),
                    ));
                    break;
                }
            }
            out
        }
        None | Some(Value::Null) => Vec::new(),
        Some(_) => {
            constraints.push(("nestedValidation".into(), format!("nested property {property} must be either object or array")));
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules() {
        assert!(is_email(Some(&json!("a@b.com"))));
        assert!(!is_email(Some(&json!("nope"))));
        assert!(is_uuid("123e4567-e89b-12d3-a456-426614174000"));
        assert!(is_url(Some(&json!("https://example.com/docs"))));
        assert!(is_int(Some(&json!(3))));
        assert!(!is_int(Some(&json!(3.5))));
        assert!(min_length(Some(&json!("abc")), 3));
        assert!(array_unique(Some(&json!([1, 2, 3]))));
        assert!(!array_unique(Some(&json!([1, 1]))));
        assert!(is_date_string(Some(&json!("2024-01-31T10:00:00Z"))));
    }

    #[test]
    fn array_unique_semantics() {
        assert!(array_unique(Some(&json!([]))));
        assert!(!array_unique(Some(&json!({}))));
        assert!(!array_unique(None));
        assert!(array_unique(Some(&json!([1, "1", true, null, [1], { "a": 1 }]))));
        assert!(!array_unique(Some(&json!([{ "a": 1, "b": [2] }, { "b": [2], "a": 1 }]))));
        assert!(array_unique(Some(&json!([{ "a": 1 }, { "a": 2 }, [1, 2], [2, 1]]))));
        assert!(!array_unique(Some(&json!([[1, [2]], [1, [2]]]))));
        assert!(!array_unique(Some(&json!(["x", "y", "x"]))));
    }

    /// Before: pairwise comparison, O(n²) (18 518 numbers from a 100 KB body took 353 ms in
    /// release and minutes in a debug build for this size).
    #[test]
    fn array_unique_is_linear() {
        let mut items: Vec<Value> = (0..100_000).map(|i| json!(i)).collect();
        let t = std::time::Instant::now();
        assert!(array_unique(Some(&Value::Array(items.clone()))));
        items.push(json!(0));
        assert!(!array_unique(Some(&Value::Array(items))));
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn variants() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "lowercase")]
        #[allow(dead_code)]
        enum Role {
            Admin,
            User,
        }
        assert_eq!(enum_variants::<Role>(), &["admin", "user"]);
    }
}

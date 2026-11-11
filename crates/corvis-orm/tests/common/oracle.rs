//! How a result of corvis-orm is compared with one recorded from the reference client (`tests/pg_oracle.rs`,
//! `tests/sqlite_oracle.rs`): results as JSON text (key order included) after normalizing the clock, errors by class
//! name, code, meta and core message (validation errors by class name only).
#![allow(dead_code)]

use corvis_orm::OrmError;
use serde_json::{Value, json};

/// `updatedAt` values depend on the clock; BigInts beyond `Number.MAX_SAFE_INTEGER` are recorded as strings.
pub fn normalize(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if k == "updatedAt" && x.is_string() {
                    *x = json!("<now>");
                } else {
                    normalize(x);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalize),
        // The recorder (`plain()` in cases.js) writes BigInts beyond Number.MAX_SAFE_INTEGER as strings.
        Value::Number(n) if n.as_i64().is_some_and(|i| i.unsigned_abs() > 9_007_199_254_740_991) => {
            *v = Value::String(n.to_string());
        }
        _ => {}
    }
}

/// `{ "$float": "NaN" | "Infinity" | "-Infinity" }` in the recorded arguments is a non-finite JavaScript number. The
/// reference client sends it as JSON `null` (`JSON.stringify(NaN)`, recorded in `fin.*`), so a JSON query carries
/// `null` too; the typed helpers refuse such values instead (`tests/typed_params.rs`).
///
/// `{ "$type": "DbNull" | "JsonNull" | "AnyNull" }` is the reference client's null sentinel ([`corvis_orm::db_null`],
/// ...); `{ "$json": v }` is `v` as plain JSON data, not revived (a request body can carry `{ "$type": "DbNull" }`).
pub fn revive(v: &mut Value) {
    match v {
        Value::Object(m) if m.len() == 1 && m.contains_key("$float") => *v = Value::Null,
        Value::Object(m) if m.len() == 1 && m.contains_key("$json") => *v = m["$json"].take(),
        Value::Object(m) if m.len() == 1 && m.get("$type").is_some_and(|t| t == "DbNull") => *v = corvis_orm::db_null(),
        Value::Object(m) if m.len() == 1 && m.get("$type").is_some_and(|t| t == "JsonNull") => *v = corvis_orm::json_null(),
        Value::Object(m) if m.len() == 1 && m.get("$type").is_some_and(|t| t == "AnyNull") => *v = corvis_orm::any_null(),
        Value::Object(m) => m.values_mut().for_each(revive),
        Value::Array(a) => a.iter_mut().for_each(revive),
        _ => {}
    }
}

/// The core message (the reference client prefixes it with the invocation and argument tree).
pub fn core_message(msg: &str) -> String {
    let m = msg.trim();
    m.rsplit("\n\n").next().unwrap_or(m).trim().to_string()
}

/// The error class names recorded in the fixture (the reference client's `error.name`).
pub fn recorded_name(e: &OrmError) -> &'static str {
    match e {
        OrmError::KnownRequest(_) => "PrismaClientKnownRequestError",
        OrmError::Validation(_) => "PrismaClientValidationError",
        OrmError::Initialization(_) => "PrismaClientInitializationError",
        OrmError::UnknownRequest(_) => "PrismaClientUnknownRequestError",
    }
}

fn error_json(e: &OrmError) -> Value {
    json!({ "name": recorded_name(e), "code": e.code(), "meta": e.meta().cloned(), "message": core_message(e.message()) })
}

/// `None` when `res` equals the recorded case, else a description of the difference.
pub fn compare(case: &Value, res: corvis_orm::Result<Value>) -> Option<String> {
    let label = case["label"].as_str().unwrap_or("?");
    let (got, expected) = match (res, case.get("result"), case.get("error")) {
        (Ok(mut v), Some(exp), _) => {
            let mut exp = exp.clone();
            normalize(&mut v);
            normalize(&mut exp);
            (v, exp)
        }
        (Err(e), _, Some(exp)) => {
            let mut exp = exp.clone();
            let is_validation = exp["name"] == "PrismaClientValidationError";
            exp["message"] = json!(core_message(exp["message"].as_str().unwrap_or("")));
            let mut got = error_json(&e);
            if is_validation {
                // The argument tree of validation errors is not reproduced: compare the name only.
                got = json!({ "name": got["name"] });
                exp = json!({ "name": exp["name"] });
            }
            (got, exp)
        }
        (Ok(v), None, Some(exp)) => (json!({ "ok": v }), json!({ "error": exp })),
        (Err(e), Some(exp), _) => (json!({ "error": error_json(&e) }), json!({ "ok": exp })),
        _ => (json!("no recording"), json!("a result or an error")),
    };
    // Compare as text so that key order matters too.
    let (g, x) = (serde_json::to_string(&got).unwrap(), serde_json::to_string(&expected).unwrap());
    (g != x).then(|| format!("[{label}]\n  expected: {x}\n  got:      {g}"))
}

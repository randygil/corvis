//! Compares the built-in pipes and the number, address and length rules with the original framework.
//!
//! `fixtures/builtin_pipes/builtin_pipes.json` is recorded by `fixtures/builtin_pipes/generate.mjs` from the original
//! framework's pipes and from class-validator; every record is replayed here and all mismatches are reported at once.

use corvis::common::pipe::{
    ArgumentMetadata, DefaultValuePipe, Metatype, ParamKind, ParseArrayPipe, ParseBoolPipe, ParseFloatPipe, ParseIntPipe, PipeTransform,
};
use corvis::validation::rules;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/builtin_pipes/builtin_pipes.json")).unwrap()
}

fn pipe(name: &str, options: &Value) -> Box<dyn PipeTransform> {
    match name {
        "ParseArrayPipe" => {
            let mut p = ParseArrayPipe::new();
            if options["items"] == "Number" {
                p = p.numbers();
            }
            if let Some(sep) = options["separator"].as_str() {
                p = p.separator(sep);
            }
            Box::new(p)
        }
        "ParseIntPipe" => Box::new(ParseIntPipe::new()),
        "ParseFloatPipe" => Box::new(ParseFloatPipe::new()),
        "ParseBoolPipe" => Box::new(ParseBoolPipe::new()),
        "DefaultValuePipe" => Box::new(DefaultValuePipe::new(options["default"].clone())),
        other => panic!("no Rust pipe for `{other}`"),
    }
}

/// JSON equality where numbers compare by value: integers exactly, other numbers as `f64` (`-0` equals `0`, as
/// `JSON.stringify` writes it).
fn same(got: &Value, want: &Value) -> bool {
    match (got, want) {
        (Value::Number(a), Value::Number(b)) => match (a.as_i64(), b.as_i64()) {
            (Some(a), Some(b)) => a == b,
            _ => a.as_f64() == b.as_f64(),
        },
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b)),
        (Value::Object(a), Value::Object(b)) => a.len() == b.len() && a.iter().zip(b).all(|((ka, va), (kb, vb))| ka == kb && same(va, vb)),
        (a, b) => a == b,
    }
}

#[tokio::test]
async fn builtin_pipes_follow_the_original() {
    let fx = fixture();
    let meta = ArgumentMetadata { kind: ParamKind::Query, data: None, metatype: Metatype { name: "String", validator: None, zod: None } };
    let mut failures = Vec::new();
    for case in fx["pipes"].as_array().unwrap() {
        let name = case["pipe"].as_str().unwrap();
        // A missing value reaches the Rust pipes as `null`.
        let input = if case["input"].get("$undefined").is_some() { Value::Null } else { case["input"].clone() };
        // A value Corvis cannot represent is refused with the error recorded for it.
        let want = case.get("divergence").map_or(&case["result"], |d| &d["result"]);
        let got = match pipe(name, &case["options"]).transform(input.clone(), &meta).await {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "status": e.get_status(), "body": e.get_response() }),
        };
        if !same(&got, want) {
            failures.push(format!("* {name} {} <- {input}\n   got  {got}\n   want {want}", case["options"]));
        }
    }
    assert!(failures.is_empty(), "{} pipe mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn number_address_and_length_rules_follow_the_original() {
    let fx = fixture();
    let mut failures = Vec::new();
    for case in fx["rules"].as_array().unwrap() {
        let rule = case["rule"].as_str().unwrap();
        let input = Some(&case["input"]);
        let n = || case["args"][0].as_u64().unwrap() as usize;
        let got = match rule {
            "isNumberString" => rules::is_number_string(input),
            "isIP" => rules::is_ip(input),
            "minLength" => rules::min_length(input, n()),
            "maxLength" => rules::max_length(input, n()),
            other => panic!("no Rust rule for `{other}`"),
        };
        if json!(got) != case["result"] {
            failures.push(format!("* {rule}{} <- {}: got {got}, want {}", case["args"], case["input"], case["result"]));
        }
    }
    assert!(failures.is_empty(), "{} rule mismatches:\n{}", failures.len(), failures.join("\n"));
}

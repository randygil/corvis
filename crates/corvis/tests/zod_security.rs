//! Security regressions of `corvis::z`: nesting limit of recursive schemas (`z.json()`,
//! `z.lazy`). The linear traversal of nested inputs is in `zod_linear_traversal.rs`.

use std::sync::Mutex;

use corvis::z::{self, ParseOptions, ZodSchema, ZodVersion};
use serde_json::{Value, json};

/// Tests that touch the global `z::config` run one at a time.
static GLOBAL: Mutex<()> = Mutex::new(());

/// tokio's default worker stack size: where `ZodValidationPipe` runs.
fn on_worker_stack<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new().stack_size(2 * 1024 * 1024).spawn(f).unwrap().join().unwrap()
}

fn nested_arrays(depth: usize) -> Value {
    let mut v = json!([]);
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

fn tree_schema() -> z::Schema {
    z::Schema::from(z::object! { name: z::string().max(50), children: z::array(z::lazy(tree_schema)).max(10) })
}

fn obj(entries: Vec<(&str, Value)>) -> Value {
    Value::Object(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Built by moving `v` (`json!({ "c": v })` would serialize `v` again at every level).
fn tree(depth: usize) -> Value {
    let mut v = obj(vec![("name", json!("a")), ("children", json!([]))]);
    for _ in 0..depth {
        v = obj(vec![("name", json!("a")), ("children", Value::Array(vec![v]))]);
    }
    v
}

fn too_deep(max: usize) -> String {
    format!("Too deeply nested: the input exceeds the maximum depth of {max}")
}

/// Before: `{"meta": [[[...]]]}` with 1000 levels (2 KB) overflowed the stack and aborted the
/// process. Now it is one `custom` issue at the first level past the limit.
#[test]
fn json_deep_input_is_an_issue_not_a_stack_overflow() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let issues = on_worker_stack(|| {
        let schema = z::object! { meta: z::json() };
        let err = schema.parse(&json!({ "meta": nested_arrays(1000) })).unwrap_err();
        err.to_json()
    });
    let issues = issues.as_array().unwrap();
    assert_eq!(issues.len(), 1, "{issues:?}");
    let mut path = vec![json!("meta")];
    path.extend(std::iter::repeat_n(json!(0), 128));
    assert_eq!(issues[0], json!({ "code": "custom", "message": too_deep(128), "path": path }));
}

/// Before: a recursive DTO (`z.lazy`) with a few hundred levels (8 KB) aborted the process,
/// even with `.max(10)` on the array (the PoC used 800 levels against a release build; 300 is
/// already past what an unlimited parse survives in a debug build on 2 MiB).
#[test]
fn lazy_deep_input_is_an_issue_not_a_stack_overflow() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    for version in [ZodVersion::V4, ZodVersion::V3] {
        let issues = on_worker_stack(move || {
            let err = tree_schema().parse_with(&tree(300), &ParseOptions::new().version(version)).unwrap_err();
            err.issues.iter().map(|i| (i.message.clone(), i.path.len())).collect::<Vec<_>>()
        });
        // The root object is not behind a lazy: 128 nested `children[0]` are allowed and the
        // 129th is reported.
        assert_eq!(issues, vec![(too_deep(128), 2 * 129)], "{version:?}");
    }
}

#[test]
fn inputs_under_the_limit_still_parse() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    on_worker_stack(|| {
        let v = nested_arrays(120);
        assert_eq!(z::json().parse(&v).unwrap(), v);
        assert!(tree_schema().parse(&tree(120)).is_ok());
        assert!(tree_schema().parse_with(&tree(120), &ParseOptions::v3()).is_ok());
    });
}

#[test]
fn max_depth_is_configurable() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let schema = z::json();
    // Per parse.
    assert!(schema.parse_with(&nested_arrays(3), &ParseOptions::new().max_depth(4)).is_ok());
    let err = schema.parse_with(&nested_arrays(4), &ParseOptions::new().max_depth(4)).unwrap_err();
    assert_eq!(err.issues[0].message, too_deep(4));
    assert_eq!(err.issues[0].path, vec![json!(0); 4]);
    // Globally (`z.config`).
    z::configure(|c| c.max_depth = 2);
    let small = schema.parse(&nested_arrays(2)).unwrap_err();
    let small_v3 = schema.parse_with(&nested_arrays(2), &ParseOptions::v3()).unwrap_err();
    z::configure(|c| c.max_depth = z::DEFAULT_MAX_DEPTH);
    assert_eq!(small.issues[0].message, too_deep(2));
    assert_eq!(small_v3.issues[0].message, too_deep(2));
    assert_eq!(z::get_config().max_depth, 128);
    assert!(schema.parse(&nested_arrays(2)).is_ok());
}

/// The nesting issue is reported once, with its path, through unions and wrappers.
#[test]
fn too_deep_issue_is_not_wrapped_in_invalid_union() {
    let _g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    fn node() -> z::Schema {
        z::union([z::string().into(), z::array(z::lazy(node)).into()])
    }
    let err = node().parse_with(&nested_arrays(10), &ParseOptions::new().max_depth(3)).unwrap_err();
    assert_eq!(err.to_json(), json!([{ "code": "custom", "message": too_deep(3), "path": [0, 0, 0, 0] }]));
}

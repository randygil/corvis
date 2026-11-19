//! Errors the original throws while parsing are returned as errors, without the panic hook; a panic of user code
//! inside `ZodValidationPipe` is answered as a thrown error and logged once a minute without its message.
//! Its own test binary: the panic hook and the logger are process-wide.

use std::cell::Cell;
use std::sync::{Arc, Mutex, Once};

use corvis::common::pipe::{ArgumentMetadata, Metatype, ParamKind, PipeTransform};
use corvis::z::{self, ParseOptions, ZodSchema};
use corvis::{LogLevel, LoggerService};
use serde_json::{Value, json};

thread_local! {
    /// Calls of the panic hook on this thread.
    static HOOK_CALLS: Cell<usize> = const { Cell::new(0) };
}

/// Wraps the panic hook in one that counts the calls on the panicking thread.
fn count_hook_calls() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            HOOK_CALLS.with(|c| c.set(c.get() + 1));
            previous(info);
        }));
    });
}

fn hook_calls() -> usize {
    HOOK_CALLS.with(Cell::get)
}

/// An intersection of two transforms that return different values: zod throws.
fn conflict() -> z::Schema {
    z::intersection(z::string().transform(|_| json!(1)), z::string().transform(|_| json!(2)))
}

#[test]
fn an_unmergeable_intersection_is_returned_without_the_panic_hook() {
    count_hook_calls();
    let before = hook_calls();

    let e = conflict().parse(&json!("x")).unwrap_err();
    assert!(e.issues.is_empty());
    assert_eq!(e.thrown(), Some("Unmergable intersection. Error path: []"));
    assert_eq!(e.to_string(), "Unmergable intersection. Error path: []");
    assert!(!conflict().safe_parse(&json!("x")).success);
    assert_eq!(z::ZodError::new(Vec::new()).thrown(), None);

    // A custom `createValidationException` receives the thrown error.
    let create: z::CreateValidationException = Arc::new(|e: z::ZodError| {
        corvis::http::BadRequestException::new(e.thrown().unwrap_or("no thrown error").to_string())
    });
    let ex = z::validate(&json!("x"), &conflict(), Some(&create)).unwrap_err();
    assert_eq!(ex.get_response()["message"], "Unmergable intersection. Error path: []");

    assert_eq!(hook_calls(), before, "the panic hook was called");
}

/// The serializer answers a thrown error with the recorded 500 (`serializationException` in `pipe.json`) and keeps
/// the thrown error as the cause.
#[test]
fn the_serializer_answers_a_thrown_error_with_its_500() {
    let meta = z::ZodSerializerDto::schema(conflict());
    let ex = z::ZodSerializerDto::serialize(&meta, &json!("x"), None).unwrap_err();
    assert_eq!(ex.get_status(), 500);
    assert_eq!(*ex.get_response(), json!({ "message": "Internal Server Error", "statusCode": 500 }));
    let cause = z::ZodSerializationException::get_zod_error(&ex).expect("the thrown error is the cause");
    assert_eq!(cause.thrown(), Some("Unmergable intersection. Error path: []"));
}

#[test]
fn an_encode_through_a_transform_is_returned_without_the_panic_hook() {
    count_hook_calls();
    let before = hook_calls();
    // zod 4.4.3: `z.string().transform(s => s.length).encode("a")` throws this `$ZodEncodeError`.
    let length = z::string().transform(|v| json!(v.as_str().unwrap().len()));
    let e = length.encode(&json!("a")).unwrap_err();
    assert!(e.issues.is_empty(), "{:?}", e.to_json());
    assert_eq!(e.thrown(), Some("Encountered unidirectional transform during encode: ZodTransform"));
    assert_eq!(hook_calls(), before, "the panic hook was called");
}

fn nested(depth: usize, leaf: Value) -> Value {
    let mut v = leaf;
    for _ in 0..depth {
        v = Value::Array(vec![v]);
    }
    v
}

/// The deepest `[[...[]]]` the schema accepts.
fn deepest_ok(schema: &z::Schema, opts: &ParseOptions) -> usize {
    (0..64).take_while(|&d| schema.parse_with(&nested(d, json!([])), opts).is_ok()).last().expect("an empty array parses")
}

#[test]
fn a_thrown_error_deep_in_a_lazy_schema_leaves_the_full_nesting_limit() {
    fn node() -> z::Schema {
        z::union([conflict(), z::array(z::lazy(node)).into()])
    }
    let schema = node();
    let opts = ParseOptions::new().max_depth(8);
    let limit = deepest_ok(&schema, &opts);
    assert!(limit >= 6, "the limit is reachable: {limit}");
    // The intersection throws at the bottom of a nesting just under the limit, unwinding through every level.
    for _ in 0..2 {
        let e = schema.parse_with(&nested(limit - 1, json!("x")), &opts).unwrap_err();
        assert_eq!(e.thrown(), Some("Unmergable intersection. Error path: []"));
    }
    assert_eq!(deepest_ok(&schema, &opts), limit, "the depth was restored after the thrown errors");
}

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

#[tokio::test]
async fn a_panicking_transform_in_the_pipe_is_a_thrown_error_logged_once_a_minute_without_its_message() {
    count_hook_calls();
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());

    let pipe = z::ZodValidationPipe::new(z::string().transform(|v| -> Value { panic!("secret-marker {v}") }));
    let meta = ArgumentMetadata { kind: ParamKind::Body, data: None, metatype: Metatype { name: "Dto", validator: None, zod: None } };
    for _ in 0..2 {
        let e = pipe.transform(json!("request data"), &meta).await.unwrap_err();
        assert_eq!(e.get_status(), 400);
        assert_eq!(*e.get_response(), json!({ "statusCode": 400, "message": "Validation failed" }));
    }

    let lines: Vec<_> = captured.lock().unwrap().iter().filter(|(_, _, c)| c.as_deref() == Some("ZodValidationPipe")).cloned().collect();
    assert_eq!(lines.len(), 1, "one line for two panics within the minute: {lines:?}");
    let (level, message, _) = &lines[0];
    assert_eq!(*level, LogLevel::Error);
    assert!(message.contains("body"), "names the parameter kind: {message}");
    assert!(!message.contains("secret-marker") && !message.contains("request data"), "no panic text: {message}");
}

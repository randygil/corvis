//! Own test binary: it installs a global logger, which must not leak into other tests.

use std::sync::{Arc, Mutex};

use corvis::{HealthCheckService, HealthIndicator, LogLevel, LoggerService};
use serde_json::json;

type Line = (LogLevel, String, Option<String>);

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<Line>>>);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

#[tokio::test]
async fn panic_is_logged_once_with_key_and_payload_and_not_returned() {
    let capture = Capture::default();
    corvis::logger::set_logger(capture.clone());
    let e = HealthCheckService
        .check(vec![HealthIndicator::new("primary_db", || async {
            if true {
                panic!("postgres://user:secret@db/app");
            }
            Ok(json!({}))
        })])
        .await
        .unwrap_err();
    let body = e.get_response().to_string();
    assert!(!body.contains("secret"), "{body}");

    let lines = capture.0.lock().unwrap();
    let mine: Vec<_> = lines.iter().filter(|(_, m, _)| m.contains("primary_db")).collect();
    assert_eq!(mine.len(), 1, "{lines:?}");
    let (level, message, context) = mine[0];
    assert_eq!(*level, LogLevel::Error);
    assert_eq!(context.as_deref(), Some("HealthCheckService"));
    assert!(message.contains("postgres://user:secret@db/app"), "{message}");
}

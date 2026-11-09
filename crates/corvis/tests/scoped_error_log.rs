//! `Scoped<T>` hides resolution errors from clients but logs them. Its own test binary: the logger is process-wide, so
//! the test installs a capturing sink.

use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{LogLevel, LoggerService, Scoped};

#[injectable(scope = "request")]
pub struct Secret;

#[controller("log")]
pub struct LogController;

#[routes]
impl LogController {
    #[get("secret")]
    async fn secret(&self, _secret: Scoped<Secret>) -> &'static str {
        "never"
    }
}

#[module(controllers = [LogController])]
pub struct LogModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

#[tokio::test]
async fn the_di_error_is_logged_and_the_client_gets_a_generic_500() {
    // `Secret` lives in the root module, which the controller's module cannot see.
    let module = Test::create_testing_module().import::<LogModule>().provider::<Secret>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    let captured: Captured = Arc::default();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    app.init().await.unwrap();

    let response = request(&app).get("/log/secret").send().await;
    response.expect_status(500);
    assert!(!response.text().contains("Secret"), "{}", response.text());

    let logged: Vec<String> = captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Error && ctx.as_deref() == Some("Scoped"))
        .map(|(_, message, _)| message.clone())
        .collect();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].contains("Scoped<Secret>") && logged[0].contains("Secret"), "{}", logged[0]);
    assert!(logged[0].contains("could not find"), "the DiError message is logged: {}", logged[0]);
}

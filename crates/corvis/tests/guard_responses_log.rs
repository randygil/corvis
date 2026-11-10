//! A guard result discarded because the guard stored a response is logged. Its own test binary: the logger is
//! process-wide, so the test installs a capturing sink and nothing else may log meanwhile.

use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{LogLevel, LoggerService};

fn redirect() -> Response {
    Response::new().with_status(302u16).with_header("Location", "/login")
}

#[injectable]
pub struct StoreGuard;

#[async_trait]
impl CanActivate for StoreGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().respond_with(redirect());
        Ok(true)
    }
}

#[injectable]
pub struct StoreThenErrorGuard;

#[async_trait]
impl CanActivate for StoreThenErrorGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().respond_with(redirect());
        Err(UnauthorizedException::empty())
    }
}

#[injectable]
pub struct StoreThenFalseGuard;

#[async_trait]
impl CanActivate for StoreThenFalseGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().respond_with(redirect());
        Ok(false)
    }
}

#[controller("log")]
pub struct LogController;

#[routes]
impl LogController {
    #[get("ok")]
    #[use_guards(StoreGuard)]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("error")]
    #[use_guards(StoreThenErrorGuard)]
    fn error(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("false")]
    #[use_guards(StoreThenFalseGuard)]
    fn false_(&self) -> Value {
        json!({ "ok": true })
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

#[corvis::test]
async fn a_discarded_guard_result_is_logged_once() {
    let mut app = CorvisFactory::create::<LogModule>().await.unwrap();
    let captured: Captured = Arc::default();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    app.init().await.unwrap();
    let errors = || {
        captured.lock().unwrap().iter().filter(|(l, _, ctx)| *l == LogLevel::Error && ctx.as_deref() == Some("ExceptionsHandler")).count()
    };

    request(&app).get("/log/ok").send().await.expect_status(302);
    assert_eq!(errors(), 0, "an `Ok(true)` result is not discarded");
    request(&app).get("/log/error").send().await.expect_status(302);
    assert_eq!(errors(), 1);
    request(&app).get("/log/false").send().await.expect_status(302);
    assert_eq!(errors(), 2);
}

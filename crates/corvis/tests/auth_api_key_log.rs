//! What the API key strategy logs: one startup warning per query source, nothing that holds a key, the redacted URL
//! in the request log, and rate-limited warnings for what a client can trigger. Its own test binary: the logger is
//! process-wide, so the tests share one capturing sink and are serialized.
#![cfg(feature = "auth-api-key")]

use std::sync::{Arc, Mutex};

use corvis::auth::api_key::{ApiKey, ApiKeySource, ApiKeyStrategy, ApiKeyStrategyOptions, ApiKeyValidator};
use corvis::auth::{AuthGuard, User};
use corvis::prelude::*;
use corvis::testing::request;
use corvis::{LogLevel, LoggerService, LoggingInterceptor};
use serde::Serialize;

const KEY: &str = "test-key-secretmarker-000000000000001";

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    name: &'static str,
}

async fn behave(key: &ApiKey) -> Result<Option<Service>, HttpException> {
    if key.expose().starts_with("test-key-fail") {
        return Err(InternalServerErrorException::new(format!("database error for {}", key.expose())));
    }
    if key.expose().starts_with("test-key-panic") {
        panic!("the validator broke on {}", key.expose());
    }
    Ok(key.expose().starts_with("test-key-").then_some(Service { name: "billing" }))
}

macro_rules! validator {
    ($name:ident, $strategy:literal, $options:expr) => {
        #[injectable]
        pub struct $name;

        #[async_trait]
        impl ApiKeyValidator for $name {
            const NAME: &'static str = $strategy;
            type User = Service;
            fn options(&self) -> ApiKeyStrategyOptions {
                $options
            }
            async fn validate(&self, _req: &Request, key: &ApiKey) -> Result<Option<Service>, HttpException> {
                behave(key).await
            }
        }
    };
}

validator!(HeaderOnly, "header-only", ApiKeyStrategyOptions::default());
validator!(InQuery, "in-query", ApiKeyStrategyOptions::new().source(ApiKeySource::query("api_key")));
validator!(
    HeaderAndQuery,
    "both",
    ApiKeyStrategyOptions::new()
        .source(ApiKeySource::header("X-Api-Key"))
        .source(ApiKeySource::query("token"))
        .source(ApiKeySource::bearer())
);

#[controller("")]
pub struct Routes;

#[routes]
impl Routes {
    #[get("data")]
    #[use_guards(AuthGuard::chain(["in-query", "header-only"]))]
    fn data(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }

    #[get("both")]
    #[use_guards(AuthGuard::new("both"))]
    fn both(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }

    /// The header strategy succeeds first, so the query strategy never runs.
    #[get("header-first")]
    #[use_guards(AuthGuard::chain(["header-only", "in-query"]))]
    fn header_first(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }

    /// No guard at all: no strategy runs.
    #[get("open")]
    fn open(&self) -> Value {
        json!({ "ok": true })
    }

    /// No guard: what `{:?}` of the request prints.
    #[get("debug")]
    fn debug(&self, req: Request) -> String {
        format!("{req:?}")
    }

    /// A guard whose strategy has no query source.
    #[get("header-only")]
    #[use_guards(AuthGuard::new("header-only"))]
    fn header_only(&self, service: User<Service>) -> Value {
        json!({ "name": service.name })
    }
}

#[module(
    providers = [
        Provider::strategy::<ApiKeyStrategy<HeaderOnly>>(),
        Provider::strategy::<ApiKeyStrategy<InQuery>>(),
        Provider::strategy::<ApiKeyStrategy<HeaderAndQuery>>(),
    ],
    controllers = [Routes],
)]
pub struct AppModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn capture() -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logger(Capture(captured.clone()));
    corvis::logger::set_log_levels(&LogLevel::all());
    captured
}

fn lines(captured: &Captured, context: &str) -> Vec<String> {
    captured.lock().unwrap().iter().filter(|(_, _, c)| c.as_deref() == Some(context)).map(|(_, m, _)| m.clone()).collect()
}

async fn start() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.use_global_interceptors(LoggingInterceptor);
    app.init().await.unwrap();
    app
}

#[tokio::test]
async fn each_query_source_logs_one_startup_warning_that_names_the_strategy_and_the_parameter() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let _app = start().await;
    let warnings: Vec<String> = captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Warn && ctx.as_deref() == Some("ApiKeyStrategy"))
        .map(|(_, m, _)| m.clone())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("\"in-query\"") && w.contains("\"api_key\"") && w.contains("access logs")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("\"both\"") && w.contains("\"token\"")), "{warnings:?}");
    assert!(!warnings.iter().any(|w| w.contains("header-only")), "{warnings:?}");
}

#[tokio::test]
async fn the_request_log_hides_the_key_of_a_query_source() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    let r = request(&app).get(&format!("/data?page=2&api_key={KEY}")).send().await;
    assert_eq!(r.status, 200);
    let logged = lines(&captured, "LoggingInterceptor");
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].contains("GET /data?page=2&api_key=REDACTED 200"), "{logged:?}");
    assert!(!logged[0].contains("secretmarker"), "{logged:?}");
    // A request that carries no marked parameter logs the line it always did.
    let r = request(&app).get("/data?page=3").header("x-api-key", KEY).send().await;
    assert_eq!(r.status, 200);
    let logged = lines(&captured, "LoggingInterceptor");
    assert!(logged[1].contains("GET /data?page=3 200"), "{logged:?}");
}

#[tokio::test]
async fn query_key_logged_when_the_query_strategy_does_not_run() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    // An earlier strategy of the chain succeeds, the route has a guard without the query strategy, or no guard.
    let calls = [
        request(&app).get(&format!("/header-first?api_key={KEY}")).header("x-api-key", KEY),
        request(&app).get(&format!("/header-only?api_key={KEY}")).header("x-api-key", KEY),
        request(&app).get(&format!("/open?api_key={KEY}")),
        request(&app).get(&format!("/open?x=1&api%5Fkey={KEY}&api_key=other")),
        request(&app).get(&format!("/nowhere?api_key={KEY}")),
    ];
    for call in calls {
        let _ = call.send().await;
    }
    let logged = lines(&captured, "LoggingInterceptor");
    assert!(logged.iter().any(|l| l.contains("GET /header-first?api_key=REDACTED 200")), "{logged:?}");
    assert!(logged.iter().any(|l| l.contains("GET /header-only?api_key=REDACTED 200")), "{logged:?}");
    assert!(logged.iter().any(|l| l.contains("GET /open?api_key=REDACTED 200")), "{logged:?}");
    assert!(logged.iter().any(|l| l.contains("GET /open?x=1&api%5Fkey=REDACTED&api_key=REDACTED 200")), "{logged:?}");
    assert!(!logged.join("\n").contains("secretmarker"), "{logged:?}");
}

#[tokio::test]
async fn an_application_without_a_query_source_logs_the_url_untouched() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let mut app = CorvisFactory::create::<NoQueryModule>().await.unwrap();
    app.silence_route_logs();
    app.use_global_interceptors(LoggingInterceptor);
    app.init().await.unwrap();
    let r = request(&app).get("/plain?api_key=visible&x=1").header("x-api-key", KEY).send().await;
    assert_eq!(r.status, 200);
    let logged = lines(&captured, "LoggingInterceptor");
    assert!(logged[0].contains("GET /plain?api_key=visible&x=1 200"), "{logged:?}");
}

#[controller("")]
pub struct PlainRoutes;

#[routes]
impl PlainRoutes {
    #[get("plain")]
    #[use_guards(AuthGuard::new("header-only"))]
    fn plain(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(providers = [Provider::strategy::<ApiKeyStrategy<HeaderOnly>>()], controllers = [PlainRoutes])]
pub struct NoQueryModule;

#[tokio::test]
async fn the_request_line_is_not_built_when_the_level_is_off() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    corvis::logger::set_log_levels(&[LogLevel::Error]);
    let r = request(&app).get(&format!("/open?api_key={KEY}")).send().await;
    assert_eq!(r.status, 200);
    corvis::logger::set_log_levels(&LogLevel::all());
    assert!(lines(&captured, "LoggingInterceptor").is_empty());
}

#[tokio::test]
async fn no_log_line_holds_a_key_whatever_the_outcome() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    let long = format!("test-key-secretmarker-{}", "k".repeat(600));
    let calls = [
        request(&app).get("/data").header("x-api-key", KEY),
        request(&app).get(&format!("/data?api_key={KEY}")),
        request(&app).get(&format!("/both?token={KEY}x")).header("x-api-key", KEY),
        request(&app).get("/both").header("x-api-key", &long),
        request(&app).get("/both").bearer(KEY),
        request(&app).get("/data").header("x-api-key", "test-key-fail-secretmarker-00000000001"),
        request(&app).get("/data?api_key=wrong-secretmarker-%00-0000000000000000"),
        request(&app).get("/data?api_key=secretmarker&api_key=secretmarker2"),
    ];
    for call in calls {
        let _ = call.send().await;
    }
    let all = captured.lock().unwrap().iter().map(|(_, m, _)| m.clone()).collect::<Vec<_>>().join("\n");
    assert!(!all.contains("secretmarker"), "a log line holds a key:\n{all}");
}

#[tokio::test]
async fn a_failing_validator_logs_at_most_one_warning_a_minute() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    for i in 0..40 {
        let r = request(&app).get("/data").header("x-api-key", &format!("test-key-fail-{i:030}")).send().await;
        assert_eq!(r.status, 500);
    }
    let warnings: Vec<String> = lines(&captured, "ApiKeyStrategy").into_iter().filter(|w| w.contains("validator failed")).collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("\"header-only\"") && warnings[0].contains("status 500"), "{warnings:?}");
}

#[tokio::test]
async fn a_panicking_validator_logs_at_most_one_warning_a_minute_and_never_its_message() {
    let _g = SERIAL.lock().await;
    let captured = capture();
    let app = start().await;
    for i in 0..40 {
        let r = request(&app).get("/data").header("x-api-key", &format!("test-key-panic-secretmarker-{i:020}")).send().await;
        assert_eq!(r.status, 500);
    }
    let warnings: Vec<String> = lines(&captured, "Authenticator");
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("\"header-only\"") && warnings[0].contains("panicked"), "{warnings:?}");
    // The default exception filter does not write one more line per request.
    assert!(lines(&captured, "ExceptionsHandler").is_empty(), "{:?}", lines(&captured, "ExceptionsHandler"));
    let all = captured.lock().unwrap().iter().map(|(_, m, _)| m.clone()).collect::<Vec<_>>().join("\n");
    assert!(!all.contains("secretmarker"), "a log line holds a key:\n{all}");
}

validator!(PercentName, "percent-name", ApiKeyStrategyOptions::new().source(ApiKeySource::query("k%FF")));

#[module(providers = [Provider::strategy::<ApiKeyStrategy<PercentName>>()])]
pub struct PercentNameModule;

/// The parsers and the log read a name that is not valid percent-encoding differently, so such a name cannot be
/// hidden reliably: the application does not start with it.
#[tokio::test]
async fn a_query_source_whose_name_holds_a_percent_sign_stops_the_start() {
    let _g = SERIAL.lock().await;
    let _captured = capture();
    let mut app = CorvisFactory::create::<PercentNameModule>().await.unwrap();
    app.silence_route_logs();
    let error = app.init().await.expect_err("the start is refused").to_string();
    assert!(error.contains("percent-name") && error.contains("k%FF") && error.contains('%'), "{error}");
}

/// `{:?}` of a request ends up in error reports and tracing spans: it prints the redacted URL.
#[tokio::test]
async fn the_debug_form_of_a_request_hides_the_key_of_a_query_source() {
    let _g = SERIAL.lock().await;
    let _captured = capture();
    let app = start().await;
    let r = request(&app).get(&format!("/debug?page=2&api_key={KEY}")).send().await;
    assert_eq!(r.status, 200);
    let shown = r.text();
    assert!(shown.contains("api_key=REDACTED") && shown.contains("page=2"), "{shown}");
    assert!(!shown.contains("secretmarker"), "{shown}");
}

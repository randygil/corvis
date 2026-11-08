//! The startup warning for CORS options that reflect the request origin with credentials.
//! Its own test binary: the logger is process-wide, so the tests share one capturing sink and are
//! serialized.

use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{CorsOptions, CorsOrigin, CorvisApplicationOptions, LogLevel, LoggerService};

#[controller("health")]
pub struct HealthController;

#[routes]
impl HealthController {
    #[get]
    fn health(&self) -> Value {
        json!({ "status": "ok" })
    }
}

#[module(controllers = [HealthController])]
pub struct AppModule;

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn install(app: &mut CorvisApplication) -> Captured {
    let captured: Captured = Arc::default();
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    captured
}

fn cors_warnings(captured: &Captured) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(level, _, ctx)| *level == LogLevel::Warn && ctx.as_deref() == Some("CorsOptions"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

async fn app_with_cors(cors: CorsOptions) -> (CorvisApplication, Captured) {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    let captured = install(&mut app);
    app.enable_cors(cors);
    (app, captured)
}

#[tokio::test]
async fn warns_once_for_reflection_with_credentials_before_listen_returns() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_with_cors(CorsOptions::new().reflect_origin().credentials(true)).await;
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(addr.ip().is_loopback());
    let warnings = cors_warnings(&captured);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("reflect_origin()") && warnings[0].contains("credentials(true)"));
    assert!(warnings[0].contains("https://corvis.randygil.com/security/cors#credentials-and-open-origins"));
}

#[tokio::test]
async fn warns_for_options_given_at_creation() {
    let _g = SERIAL.lock().await;
    let options = CorvisApplicationOptions {
        cors: Some(CorsOptions::new().origin_value(CorsOrigin::Reflect).credentials(true)),
        ..Default::default()
    };
    let mut app = CorvisFactory::create_with_options::<AppModule>(options).await.unwrap();
    let captured = install(&mut app);
    app.init().await.unwrap();
    let warnings = cors_warnings(&captured);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("origins([...])"));
}

#[tokio::test]
async fn second_init_and_requests_do_not_warn_again() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_with_cors(CorsOptions::new().reflect_origin().credentials(true)).await;
    app.init().await.unwrap();
    app.init().await.unwrap();
    let c = request(&app);
    c.get("/health").header("origin", "http://z.io").send().await;
    c.get("/health").header("origin", "http://y.io").send().await;
    assert_eq!(cors_warnings(&captured).len(), 1);
}

#[tokio::test]
async fn no_warning_for_safe_or_undecidable_configurations() {
    let _g = SERIAL.lock().await;
    let cases = [
        CorsOptions::new().origins(["https://app.example.com"]).credentials(true),
        CorsOptions::new().reflect_origin(),
        CorsOptions::new().origin_fn(|_| CorsOrigin::Reflect).credentials(true),
        CorsOptions::new().credentials(true),
    ];
    for cors in cases {
        let (mut app, captured) = app_with_cors(cors).await;
        app.init().await.unwrap();
        assert!(cors_warnings(&captured).is_empty());
    }
}

#[tokio::test]
async fn reflected_response_is_unchanged() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_with_cors(CorsOptions::new().reflect_origin().credentials(true)).await;
    app.init().await.unwrap();
    assert_eq!(cors_warnings(&captured).len(), 1);
    let r = request(&app).get("/health").header("origin", "http://z.io").send().await;
    assert_eq!(r.header("access-control-allow-origin"), Some("http://z.io"));
    assert_eq!(r.header("vary"), Some("Origin"));
    assert_eq!(r.header("access-control-allow-credentials"), Some("true"));
}

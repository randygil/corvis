//! The startup warning for a route whose guards run an `AuthGuard` before a `ThrottlerGuard` (`auth/login-rate-limiting`):
//! the rate limit is then checked after the password hash, the store read or the provider request. A warning, not an
//! error, and never for a throttler that runs first. Its own test binary: the logger is process-wide, so every test
//! serializes on one mutex and installs a capturing sink.

use std::sync::{Arc, Mutex};

use corvis::auth::{AuthGuard, AuthenticateOptions, PassportModule, PassportModuleOptions, Strategy, StrategyOutcome};
use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{LogLevel, LoggerService, ThrottlerGuard, ThrottlerModule, ThrottlerOptions};

#[injectable]
pub struct Open;

#[async_trait]
impl Strategy for Open {
    fn name(&self) -> &str {
        "open"
    }

    async fn authenticate(&self, _: &ExecutionContext, _: &AuthenticateOptions) -> StrategyOutcome {
        StrategyOutcome::Fail { challenge: None, status: None }
    }
}

/// A route-level `ThrottlerGuard` after a global `AuthGuard` (added with `use_global_guards`).
#[controller("a")]
#[use_guards(ThrottlerGuard)]
pub struct ControllerLevel;

#[routes]
impl ControllerLevel {
    #[post("login")]
    fn login(&self) -> &'static str {
        "ok"
    }

    #[get("status")]
    fn status(&self) -> &'static str {
        "ok"
    }
}

/// Both guards on the route, the `AuthGuard` first.
#[controller("b")]
pub struct RouteLevel;

#[routes]
impl RouteLevel {
    #[post("login")]
    #[use_guards(AuthGuard::new("open").optional(), ThrottlerGuard)]
    fn login(&self) -> &'static str {
        "ok"
    }

    /// The `ThrottlerGuard` first: no warning.
    #[post("safe")]
    #[use_guards(ThrottlerGuard, AuthGuard::new("open").optional())]
    fn safe(&self) -> &'static str {
        "ok"
    }

    /// No `AuthGuard` at all.
    #[get("free")]
    #[use_guards(ThrottlerGuard)]
    fn free(&self) -> &'static str {
        "ok"
    }
}

/// A guard that wraps both and shows neither (no `as_any`, no `inner_guards`): the warning cannot see through it.
pub struct Opaque(AuthGuard);

#[async_trait]
impl corvis::CanActivate for Opaque {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        self.0.can_activate(ctx).await
    }
}

#[controller("c")]
pub struct WrappedLevel;

#[routes]
impl WrappedLevel {
    #[post("login")]
    #[use_guards(Opaque(AuthGuard::new("open").optional()), ThrottlerGuard)]
    fn login(&self) -> &'static str {
        "ok"
    }
}

type Captured = Arc<Mutex<Vec<(LogLevel, String, Option<String>)>>>;

struct Capture(Captured);

impl LoggerService for Capture {
    fn log(&self, level: LogLevel, message: &str, context: Option<&str>) {
        self.0.lock().unwrap().push((level, message.to_string(), context.map(str::to_string)));
    }
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn warnings(captured: &Captured) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, m, _)| *l == LogLevel::Warn && m.contains("runs before ThrottlerGuard"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

async fn start(global_auth: bool, global_throttler: bool) -> Captured {
    let captured: Captured = Arc::default();
    let mut providers = vec![Provider::strategy::<Open>()];
    if global_throttler {
        providers.push(Provider::app_guard::<ThrottlerGuard>());
    }
    let module = Test::create_testing_module()
        .with_logger(true)
        .import_dynamic(PassportModule::register(PassportModuleOptions::new().default_strategy("open")))
        .import_dynamic(ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]))
        .providers(providers)
        .controller::<ControllerLevel>()
        .controller::<RouteLevel>()
        .controller::<WrappedLevel>()
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    corvis::logger::set_logging_enabled(true);
    corvis::logger::set_log_levels(&[LogLevel::Log, LogLevel::Error, LogLevel::Warn, LogLevel::Debug, LogLevel::Verbose, LogLevel::Fatal]);
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    if global_auth {
        app.use_global_guards(AuthGuard::new("open").optional());
    }
    app.init().await.unwrap();
    // The application still serves: a warning does not stop it.
    assert_eq!(request(&app).get("/a/status").send().await.status, 200);
    captured
}

#[tokio::test]
async fn global_auth_guard_route_level_throttler() {
    let _s = SERIAL.lock().await;
    let captured = start(true, false).await;
    let lines = warnings(&captured);
    // The controller-level throttler comes after the global AuthGuard on both of its routes.
    assert!(lines.iter().any(|l| l.contains("POST /a/login")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("GET /a/status")), "{lines:?}");
    // And the route that lists the AuthGuard first (`/b/login`), whatever the global one does.
    assert!(lines.iter().any(|l| l.contains("POST /b/login")), "{lines:?}");
    let line = lines.iter().find(|l| l.contains("POST /a/login")).unwrap();
    assert!(line.contains("the rate limit is checked after authentication") && line.contains("Provider::app_guard"), "{line}");
    // Every route with a throttler of its own runs the global AuthGuard first, `/b/safe` and `/b/free` included.
    assert!(lines.iter().any(|l| l.contains("POST /b/safe")), "the global AuthGuard still runs first: {lines:?}");
    assert!(lines.iter().any(|l| l.contains("GET /b/free")), "{lines:?}");
    assert_eq!(lines.len(), 6, "five routes with a route-level throttler plus the wrapped one: {lines:?}");
    // One warning per route, not per request.
    assert_eq!(lines.len(), lines.iter().collect::<std::collections::BTreeSet<_>>().len(), "{lines:?}");
}

#[tokio::test]
async fn throttler_first() {
    let _s = SERIAL.lock().await;
    // The throttler is a global guard (global guards run first) and the only AuthGuards are route-level, after it.
    let captured = start(false, true).await;
    let lines = warnings(&captured);
    assert!(!lines.iter().any(|l| l.contains("/a/") || l.contains("/b/safe") || l.contains("/b/free")), "{lines:?}");
    // `/b/login` lists its own AuthGuard before the route's own ThrottlerGuard: the warning is about the order the
    // route's guards run in, not about which throttler is global.
    assert!(lines.iter().any(|l| l.contains("POST /b/login")), "{lines:?}");
    // A wrapper that exposes neither guard hides the problem from the check (documented limit).
    assert!(!lines.iter().any(|l| l.contains("/c/login")), "{lines:?}");
}

//! Route posture: `#[public]`, the opt-in startup check and the posture report.
//! Its own test binary: the logger is process-wide, so every test serializes on one mutex and installs a
//! capturing sink.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{
    ControllerBase, LogLevel, LoggerService, PostureStatus, RoutePostureError, RoutePostureMode, RoutePostureOptions, RoutePostureReport,
    ThrottlerOptions,
};

// ---------------------------------------------------------------------------
// Guards and fixtures.
// ---------------------------------------------------------------------------

#[injectable]
pub struct AuthGuard;

#[async_trait]
impl CanActivate for AuthGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let targets = [ctx.get_handler(), ctx.get_class()];
        if Reflector.get_all_and_override::<bool>("isPublic", &targets).unwrap_or(false) {
            return Ok(true);
        }
        Ok(ctx.switch_to_http().get_request().header("authorization").is_some())
    }
}

#[injectable]
pub struct AllowGuard;

#[async_trait]
impl CanActivate for AllowGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }
}

/// A pass-through guard that says so.
#[injectable]
pub struct RoleGuard;

#[async_trait]
impl CanActivate for RoleGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }
    fn controls_access(&self) -> bool {
        false
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

fn install(app: &mut CorvisApplication) -> Captured {
    let captured: Captured = Arc::default();
    corvis::logger::set_logging_enabled(true);
    corvis::logger::set_log_levels(&[LogLevel::Log, LogLevel::Error, LogLevel::Warn, LogLevel::Debug, LogLevel::Verbose, LogLevel::Fatal]);
    app.use_logger(Capture(captured.clone()));
    app.silence_route_logs();
    captured
}

fn posture_logs(captured: &Captured) -> Vec<(LogLevel, String)> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, _, ctx)| *l != LogLevel::Log && ctx.as_deref() == Some("RoutePosture"))
        .map(|(l, m, _)| (*l, m.clone()))
        .collect()
}

/// The info-level lines of the posture check (the access guards summary).
fn summary_logs(captured: &Captured) -> Vec<String> {
    captured
        .lock()
        .unwrap()
        .iter()
        .filter(|(l, _, ctx)| *l == LogLevel::Log && ctx.as_deref() == Some("RoutePosture"))
        .map(|(_, m, _)| m.clone())
        .collect()
}

async fn app_of<M: corvis::Module>() -> (CorvisApplication, Captured) {
    let mut app = CorvisFactory::create::<M>().await.unwrap();
    let captured = install(&mut app);
    (app, captured)
}

fn status_of(report: &RoutePostureReport, method: &str, path: &str) -> PostureStatus {
    let found: Vec<_> = report.entries().iter().filter(|e| e.path == path && format!("{:?}", e.kind).contains(method)).collect();
    assert_eq!(found.len(), 1, "{method} {path} in\n{report}");
    found[0].status.clone()
}

fn unprotected_of(err: &anyhow::Error) -> Vec<String> {
    let err = err.downcast_ref::<RoutePostureError>().expect("a RoutePostureError");
    err.unprotected().iter().map(|e| format!("{} {}::{}", e.path, e.owner, e.handler)).collect()
}

// ---------------------------------------------------------------------------
// An application with nothing guarded: two routes, one of them in a second controller.
// ---------------------------------------------------------------------------

static MODULE_INIT_RAN: AtomicBool = AtomicBool::new(false);
static CONFIGURE_RAN: AtomicBool = AtomicBool::new(false);

#[injectable]
pub struct Flag;

#[async_trait]
impl OnModuleInit for Flag {
    async fn on_module_init(&self) -> corvis::Result<()> {
        MODULE_INIT_RAN.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[controller("rooms")]
pub struct RoomsController;

#[routes]
impl RoomsController {
    #[get]
    fn list(&self) -> Value {
        json!([])
    }

    #[post(":room/messages")]
    fn post(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [RoomsController], providers = [Flag])]
pub struct OpenModule;

impl CorvisModule for OpenModule {
    fn configure(&self, _consumer: &mut MiddlewareConsumer) {
        CONFIGURE_RAN.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn off_mode_starts_with_unguarded_routes_and_logs_nothing_about_posture() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<OpenModule>().await;
    assert!(app.route_posture().is_none(), "no report before init()");
    app.init().await.unwrap();
    assert!(posture_logs(&captured).is_empty(), "{:?}", posture_logs(&captured));
    assert!(summary_logs(&captured).is_empty(), "off mode logs no summary");
    // The report exists in off mode and says what is open.
    let report = app.route_posture().unwrap();
    assert_eq!(report.entries().len(), 2);
    assert_eq!(report.unprotected().len(), 2);
    app.init().await.unwrap();
    let res = request(&app).get("/rooms").send().await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn off_mode_ignores_a_blank_exemption_reason() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<OpenModule>().await;
    app.set_route_posture(RoutePostureOptions::off().exempt(RequestMethod::Get, "/x", ""));
    app.init().await.unwrap();
    assert!(posture_logs(&captured).is_empty());
}

#[tokio::test]
async fn enforce_fails_before_any_hook_or_socket_and_lists_every_unprotected_route() {
    let _g = SERIAL.lock().await;
    MODULE_INIT_RAN.store(false, Ordering::SeqCst);
    CONFIGURE_RAN.store(false, Ordering::SeqCst);
    let (mut app, captured) = app_of::<OpenModule>().await;
    app.require_route_posture();
    let err = app.listen_in_background("127.0.0.1:0").await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/rooms RoomsController::list", "/rooms/:room/messages RoomsController::post"]);
    let text = err.to_string();
    assert!(text.starts_with("Route posture check failed: 2 routes have no guard and are not marked #[public]."), "{text}");
    assert!(text.contains("GET") && text.contains("/rooms/:room/messages") && text.contains("RoomsController::post"), "{text}");
    assert!(text.contains("#[public]") && text.contains("exemption with a reason"), "{text}");
    assert!(app.get_url().is_none(), "no socket was bound");
    assert!(!MODULE_INIT_RAN.load(Ordering::SeqCst), "OnModuleInit must not run");
    assert!(!CONFIGURE_RAN.load(Ordering::SeqCst), "module configure must not run");
    let logs = posture_logs(&captured);
    assert_eq!(logs.len(), 1, "logged once: {logs:?}");
    assert_eq!(logs[0].0, LogLevel::Error);
}

#[tokio::test]
async fn warn_mode_logs_once_and_serves_requests() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<OpenModule>().await;
    app.set_route_posture(RoutePostureOptions::warn());
    app.init().await.unwrap();
    let res = request(&app).get("/rooms").send().await;
    assert_eq!(res.status(), 200);
    let logs = posture_logs(&captured);
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert_eq!(logs[0].0, LogLevel::Warn);
    assert!(logs[0].1.contains("2 routes") && logs[0].1.contains("RoomsController::list"), "{}", logs[0].1);
    request(&app).get("/rooms").send().await.expect_status(200);
    assert_eq!(posture_logs(&captured).len(), 1, "later requests do not log again");
}

#[tokio::test]
async fn a_blank_exemption_reason_fails_init_naming_the_exemption() {
    let _g = SERIAL.lock().await;
    let (mut app, _captured) = app_of::<OpenModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/rooms", "   "));
    let err = app.init().await.unwrap_err().to_string();
    assert!(err.contains("GET /rooms"), "{err}");
    assert!(app.route_posture().is_none());
}

// ---------------------------------------------------------------------------
// Guard levels.
// ---------------------------------------------------------------------------

#[controller("route-level")]
pub struct RouteLevelController;

#[routes]
impl RouteLevelController {
    #[get("guarded")]
    #[use_guards(AuthGuard)]
    fn guarded(&self) -> Value {
        json!({})
    }

    #[get("bare")]
    fn bare(&self) -> Value {
        json!({})
    }
}

#[controller("class-level")]
#[use_guards(AuthGuard)]
pub struct ClassLevelController;

#[routes]
impl ClassLevelController {
    #[get]
    fn any(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [RouteLevelController, ClassLevelController])]
pub struct LevelsModule;

#[tokio::test]
async fn route_and_controller_guards_count() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<LevelsModule>().await;
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(status_of(&report, "Get", "/route-level/guarded"), PostureStatus::Protected);
    assert_eq!(status_of(&report, "Get", "/route-level/bare"), PostureStatus::Unprotected);
    assert_eq!(status_of(&report, "Get", "/class-level"), PostureStatus::Protected);
    let guarded = report.entries().iter().find(|e| e.path == "/route-level/guarded").unwrap();
    assert_eq!(guarded.guards.len(), 1);
    assert_eq!(guarded.guards[0].name, "AuthGuard");
    assert!(guarded.guards[0].controls_access);
}

#[tokio::test]
async fn use_global_guards_cover_every_route() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<LevelsModule>().await;
    app.use_global_guards(AuthGuard);
    app.require_route_posture();
    app.init().await.unwrap();
    assert!(app.route_posture().unwrap().unprotected().is_empty());
}

#[controller("plain")]
pub struct PlainController;

#[routes]
impl PlainController {
    #[get]
    fn plain(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [PlainController], providers = [Provider::app_guard::<AuthGuard>()])]
pub struct ProviderGlobalModule;

#[module(controllers = [PlainController])]
pub struct PlainModule;

#[tokio::test]
async fn provider_global_guards_count() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<ProviderGlobalModule>().await;
    app.require_route_posture();
    app.init().await.unwrap();
    assert_eq!(status_of(&app.route_posture().unwrap(), "Get", "/plain"), PostureStatus::Protected);
}

#[controller("limited")]
#[use_guards(ThrottlerGuard)]
pub struct LimitedController;

#[routes]
impl LimitedController {
    #[get]
    fn limited(&self) -> Value {
        json!({})
    }
}

#[module(imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)])], controllers = [LimitedController])]
pub struct ThrottledModule;

#[controller("validated")]
pub struct ValidatedController;

#[routes]
impl ValidatedController {
    #[post]
    #[use_guards(z::ZodGuard::new(z::ZodGuardSource::Body, z::string()))]
    fn validated(&self, #[body] body: Value) -> Value {
        body
    }
}

#[module(controllers = [ValidatedController])]
pub struct ValidatedModule;

#[controller("roles")]
#[use_guards(RoleGuard)]
pub struct RolesController;

#[routes]
impl RolesController {
    #[get]
    fn roles(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [RolesController])]
pub struct RolesModule;

#[tokio::test]
async fn guards_that_do_not_control_access_do_not_count() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<ThrottledModule>().await;
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(status_of(&report, "Get", "/limited"), PostureStatus::Unprotected);
    let g = &report.entries()[0].guards;
    assert_eq!((g[0].name.as_str(), g[0].controls_access), ("ThrottlerGuard", false));

    let (mut app, _) = app_of::<ValidatedModule>().await;
    app.init().await.unwrap();
    assert_eq!(status_of(&app.route_posture().unwrap(), "Post", "/validated"), PostureStatus::Unprotected);

    let (mut app, _) = app_of::<RolesModule>().await;
    app.require_route_posture();
    let err = app.init().await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/roles RolesController::roles"]);
    assert!(err.to_string().contains("1 route has no guard"), "{err}");
}

#[tokio::test]
async fn a_guard_that_does_not_control_access_does_not_change_how_it_runs() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<ThrottledModule>().await;
    app.init().await.unwrap();
    let res = request(&app).get("/limited").send().await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn an_override_guard_double_keeps_the_route_protected() {
    let _g = SERIAL.lock().await;
    let mut app = Test::create_testing_module()
        .import::<LevelsModule>()
        .override_guard::<AuthGuard>(AllowGuard)
        .compile()
        .await
        .unwrap()
        .create_corvis_application();
    install(&mut app);
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    let entry = report.entries().iter().find(|e| e.path == "/route-level/guarded").unwrap();
    assert_eq!(entry.status, PostureStatus::Protected);
    assert_eq!(entry.guards[0].name, "AllowGuard");
}

// ---------------------------------------------------------------------------
// The public marker.
// ---------------------------------------------------------------------------

const IS_PUBLIC_KEY: &str = "isPublic";

#[controller("open")]
#[public]
pub struct OpenController;

#[routes]
impl OpenController {
    #[get]
    fn open(&self) -> Value {
        json!({})
    }

    #[get("closed")]
    #[set_metadata(PUBLIC_METADATA, false)]
    fn closed(&self) -> Value {
        json!({})
    }
}

#[controller("marked")]
pub struct MarkedController;

#[routes]
impl MarkedController {
    #[get("public")]
    #[public]
    fn public_route(&self) -> Value {
        json!({ "public": true })
    }

    #[get("literal")]
    #[set_metadata(IS_PUBLIC_KEY, true)]
    fn literal(&self) -> Value {
        json!({})
    }

    #[get("secret")]
    fn secret(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [OpenController, MarkedController], providers = [Provider::app_guard::<AuthGuard>()])]
pub struct PublicModule;

#[module(controllers = [OpenController, MarkedController])]
pub struct PublicUnguardedModule;

#[tokio::test]
async fn public_resolution_is_handler_first_then_controller() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<PublicUnguardedModule>().await;
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(status_of(&report, "Get", "/open"), PostureStatus::Public);
    assert_eq!(status_of(&report, "Get", "/open/closed"), PostureStatus::Unprotected, "the handler overrides the controller");
    assert_eq!(status_of(&report, "Get", "/marked/public"), PostureStatus::Public);
    assert_eq!(status_of(&report, "Get", "/marked/literal"), PostureStatus::Public, "the literal key is the same marker");
    assert_eq!(status_of(&report, "Get", "/marked/secret"), PostureStatus::Unprotected);

    let (mut app, _) = app_of::<PublicUnguardedModule>().await;
    app.require_route_posture();
    let err = app.init().await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/marked/secret MarkedController::secret", "/open/closed OpenController::closed"]);
}

#[tokio::test]
async fn the_marker_works_with_a_global_guard_that_skips_public_routes() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<PublicModule>().await;
    app.require_route_posture();
    app.init().await.unwrap();
    // Public wins over the guard in the report, and the guards stay listed.
    let report = app.route_posture().unwrap();
    let entry = report.entries().iter().find(|e| e.path == "/marked/public").unwrap();
    assert_eq!(entry.status, PostureStatus::Public);
    assert_eq!(entry.guards.len(), 1);
    // The guard honors the marker unchanged, and still blocks the others.
    assert_eq!(request(&app).get("/marked/public").send().await.status(), 200);
    assert_eq!(request(&app).get("/marked/secret").send().await.status(), 403);
}

#[test]
fn public_metadata_is_the_conventional_key_and_is_stored_on_controller_and_handler() {
    assert_eq!(PUBLIC_METADATA, "isPublic");
    let class = <OpenController as ControllerBase>::controller_metadata();
    assert!(class.metadata.iter().any(|(k, v)| k == "isPublic" && v == &json!(true)));
    let routes = <MarkedController as ControllerBase>::controller_metadata();
    assert!(!routes.metadata.iter().any(|(k, _)| k == "isPublic"));
}

// ---------------------------------------------------------------------------
// Exemptions, paths, versions and aliases.
// ---------------------------------------------------------------------------

#[controller("metrics")]
pub struct MetricsController;

#[routes]
impl MetricsController {
    #[get]
    fn scrape(&self) -> Value {
        json!({})
    }

    #[post]
    fn push(&self) -> Value {
        json!({})
    }

    #[all("any")]
    fn any(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [MetricsController])]
pub struct MetricsModule;

#[tokio::test]
async fn exemptions_match_by_method_wildcard_and_all() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<MetricsModule>().await;
    app.set_route_posture(
        RoutePostureOptions::warn()
            .exempt(RequestMethod::Get, "/metrics", "scraped from the private network only")
            .exempt(RequestMethod::All, "/metrics/any", "internal")
            .exempt(RequestMethod::Delete, "/nothing", "stale"),
    );
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(status_of(&report, "Get", "/metrics"), PostureStatus::Exempt { reason: "scraped from the private network only".into() });
    assert_eq!(status_of(&report, "All", "/metrics/any"), PostureStatus::Exempt { reason: "internal".into() });
    assert_eq!(status_of(&report, "Post", "/metrics"), PostureStatus::Unprotected);
    assert_eq!(report.stale_exemptions().len(), 1);
    // The same options in enforce mode refuse to start because of the POST route and the `#[all]` route.
    let (mut strict, _) = app_of::<MetricsModule>().await;
    strict.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/metrics", "private"));
    let err = strict.init().await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/metrics MetricsController::push", "/metrics/any MetricsController::any"]);
}

#[tokio::test]
async fn a_get_exemption_does_not_cover_an_all_route() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<MetricsModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/metrics/any", "r"));
    app.init().await.unwrap();
    assert_eq!(status_of(&app.route_posture().unwrap(), "All", "/metrics/any"), PostureStatus::Unprotected);
}

#[tokio::test]
async fn a_stale_exemption_warns_and_does_not_fail() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<PlainModule>().await;
    app.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/plain", "open on purpose").exempt(
        RequestMethod::Get,
        "/gone",
        "removed endpoint",
    ));
    app.init().await.unwrap();
    let logs = posture_logs(&captured);
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert_eq!(logs[0].0, LogLevel::Warn);
    assert!(logs[0].1.contains("GET /gone") && logs[0].1.contains("matches no route"), "{}", logs[0].1);
}

#[controller(path = ["items", "things"], version = ["1", "2"])]
pub struct VersionedController;

#[routes]
impl VersionedController {
    #[get("list")]
    fn list(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [VersionedController])]
pub struct VersionedModule;

#[tokio::test]
async fn prefix_versions_and_aliases_are_reported_one_by_one() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<VersionedModule>().await;
    app.set_global_prefix("api");
    app.enable_versioning(corvis::VersioningOptions::uri());
    app.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/api/v1/items/list", "only this one"));
    let err = app.init().await.unwrap_err();
    assert_eq!(
        unprotected_of(&err),
        [
            "/api/v1/things/list VersionedController::list",
            "/api/v2/items/list VersionedController::list",
            "/api/v2/things/list VersionedController::list",
        ]
    );

    let (mut app, _) = app_of::<VersionedModule>().await;
    app.set_global_prefix("api");
    app.enable_versioning(corvis::VersioningOptions::uri());
    app.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::All, "api/*/things/*", "aliases are internal").exempt(
        RequestMethod::Get,
        "/api/*/items/list",
        "r",
    ));
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(report.entries().len(), 4);
    assert!(report.unprotected().is_empty(), "{report}");
}

// ---------------------------------------------------------------------------
// The report.
// ---------------------------------------------------------------------------

#[controller("mixed")]
pub struct MixedController;

#[routes]
impl MixedController {
    #[get("guarded")]
    #[use_guards(AuthGuard)]
    fn guarded(&self) -> Value {
        json!({})
    }

    #[get("public")]
    #[public]
    fn public_route(&self) -> Value {
        json!({})
    }

    #[get("exempt")]
    fn exempt(&self) -> Value {
        json!({})
    }

    #[get("open")]
    fn open(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [MixedController])]
pub struct MixedModule;

#[tokio::test]
async fn the_report_of_a_mixed_application_has_every_status_and_serializes_to_json() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<MixedModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/mixed/exempt", "behind the gateway"));
    assert_eq!(app.route_posture(), None);
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(status_of(&report, "Get", "/mixed/guarded"), PostureStatus::Protected);
    assert_eq!(status_of(&report, "Get", "/mixed/public"), PostureStatus::Public);
    assert_eq!(status_of(&report, "Get", "/mixed/exempt"), PostureStatus::Exempt { reason: "behind the gateway".into() });
    assert_eq!(status_of(&report, "Get", "/mixed/open"), PostureStatus::Unprotected);

    let table = report.to_string();
    assert!(table.contains("MixedController::open") && table.contains("UNPROTECTED"), "{table}");
    assert!(table.contains("exempt (behind the gateway)"), "{table}");

    let json = serde_json::to_value(&report).unwrap();
    let entries = json["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    let exempt = entries.iter().find(|e| e["path"] == "/mixed/exempt").unwrap();
    assert_eq!(exempt["status"], serde_json::json!({ "state": "exempt", "reason": "behind the gateway" }));
    assert_eq!(exempt["kind"], serde_json::json!({ "type": "http", "method": "GET" }));
    assert_eq!(exempt["owner"], "MixedController");
    assert_eq!(exempt["handler"], "exempt");
    let guarded = entries.iter().find(|e| e["path"] == "/mixed/guarded").unwrap();
    assert_eq!(guarded["guards"], serde_json::json!([{ "name": "AuthGuard", "controls_access": true }]));
}

#[test]
fn the_default_mode_is_off() {
    assert_eq!(RoutePostureMode::default(), RoutePostureMode::Off);
    assert_eq!(RoutePostureOptions::default().mode(), RoutePostureMode::Off);
}

// ---------------------------------------------------------------------------
// Security review: visible pass-through guards, literal exemptions, public source.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_access_guards_summary_names_each_guard_scope_and_route_count() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<ProviderGlobalModule>().await;
    app.set_route_posture(RoutePostureOptions::warn());
    app.init().await.unwrap();
    assert_eq!(summary_logs(&captured), ["Route posture: access guards counted: AuthGuard (global, 1 route)"]);
    let report = app.route_posture().unwrap();
    assert_eq!(report.access_guards().len(), 1);
    assert_eq!(report.access_guards()[0].scope, corvis::GuardScope::Global);
    assert_eq!(report.access_guards()[0].routes, 1);

    let (mut app, captured) = app_of::<LevelsModule>().await;
    app.use_global_guards(AllowGuard);
    app.set_route_posture(RoutePostureOptions::warn());
    app.init().await.unwrap();
    assert_eq!(
        summary_logs(&captured),
        ["Route posture: access guards counted: AllowGuard (global, 3 routes), AuthGuard (controller ClassLevelController, 1 route), \
             AuthGuard (route RouteLevelController::guarded, 1 route)"]
    );
    let json = serde_json::to_value(app.route_posture().unwrap()).unwrap();
    assert_eq!(json["access_guards"][0], serde_json::json!({ "name": "AllowGuard", "scope": { "level": "global" }, "routes": 3 }));
}

#[tokio::test]
async fn the_summary_is_logged_in_enforce_mode_even_when_init_fails_and_lists_none_without_guards() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<OpenModule>().await;
    app.require_route_posture();
    app.init().await.unwrap_err();
    assert_eq!(summary_logs(&captured), ["Route posture: access guards counted: none"]);
}

#[tokio::test]
async fn guards_that_do_not_control_access_are_not_in_the_summary() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<RolesModule>().await;
    app.set_route_posture(RoutePostureOptions::warn());
    app.init().await.unwrap();
    assert_eq!(summary_logs(&captured), ["Route posture: access guards counted: none"]);
}

#[controller("files")]
pub struct FilesController;

#[routes]
impl FilesController {
    #[get(":id")]
    fn one(&self) -> Value {
        json!({})
    }

    #[get("export-all")]
    fn export_all(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [FilesController])]
pub struct FilesModule;

#[tokio::test]
async fn a_param_exemption_does_not_cover_a_literal_sibling_route() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<FilesModule>().await;
    app.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::Get, "/files/:id", "public downloads"));
    let err = app.init().await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/files/export-all FilesController::export_all"]);

    let (mut app, _) = app_of::<FilesModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/files/*", "both"));
    app.init().await.unwrap();
    assert!(app.route_posture().unwrap().unprotected().is_empty());
}

#[tokio::test]
async fn a_glob_exemption_does_not_cross_segments() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<FilesModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/fil*", "r"));
    app.init().await.unwrap();
    // `/fil*` is one segment: it matches no `/files/...` route and is stale.
    let report = app.route_posture().unwrap();
    assert_eq!(report.unprotected().len(), 2);
    assert_eq!(report.stale_exemptions().len(), 1);
}

#[tokio::test]
async fn an_exemption_that_matches_only_guarded_or_public_routes_warns() {
    let _g = SERIAL.lock().await;
    let (mut app, captured) = app_of::<LevelsModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/class-level", "already guarded").exempt(
        RequestMethod::Get,
        "/route-level/*",
        "covers one guarded and one open route",
    ));
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    assert_eq!(report.redundant_exemptions().len(), 1);
    assert_eq!(report.redundant_exemptions()[0].pattern(), "/class-level");
    let logs = posture_logs(&captured);
    let redundant: Vec<_> =
        logs.iter().filter(|(l, m)| *l == LogLevel::Warn && m.contains("matches only guarded or public routes")).collect();
    assert_eq!(redundant.len(), 1, "{logs:?}");
    assert!(redundant[0].1.contains("GET /class-level"));
}

#[tokio::test]
async fn the_report_says_where_a_public_status_comes_from() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<PublicUnguardedModule>().await;
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    let source = |path: &str| report.entries().iter().find(|e| e.path == path).unwrap().public_source;
    assert_eq!(source("/open"), Some(corvis::PublicSource::Controller));
    assert_eq!(source("/marked/public"), Some(corvis::PublicSource::Handler));
    assert_eq!(source("/marked/secret"), None);
    let table = report.to_string();
    assert!(table.contains("public (controller)") && table.contains("public (handler)"), "{table}");
    let json = serde_json::to_value(&report).unwrap();
    let e = json["entries"].as_array().unwrap().iter().find(|e| e["path"] == "/marked/public").unwrap();
    assert_eq!(e["public_source"], "handler");
}

// ---------------------------------------------------------------------------
// SSE streams.
// ---------------------------------------------------------------------------

fn stream() -> Sse {
    Sse::new(futures::stream::iter((1..=2).map(|i| MessageEvent::new(json!({ "n": i })))))
}

#[controller("feeds")]
pub struct FeedController;

#[routes]
impl FeedController {
    #[sse("stream")]
    fn stream(&self) -> Sse {
        stream()
    }

    #[sse("open")]
    #[public]
    fn open(&self) -> Sse {
        stream()
    }

    #[sse("guarded")]
    #[use_guards(AuthGuard)]
    fn guarded(&self) -> Sse {
        stream()
    }

    #[get("plain")]
    #[public]
    fn plain(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [FeedController])]
pub struct FeedModule;

#[tokio::test]
async fn an_unguarded_sse_stream_fails_enforce_mode_with_an_sse_line() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<FeedModule>().await;
    app.require_route_posture();
    let err = app.init().await.unwrap_err();
    assert_eq!(unprotected_of(&err), ["/feeds/stream FeedController::stream"]);
    let text = err.to_string();
    assert!(text.contains("  SSE  /feeds/stream  FeedController::stream"), "{text}");
    assert!(text.starts_with("Route posture check failed: 1 route has no guard"), "{text}");
}

#[tokio::test]
async fn sse_routes_are_reported_with_their_kind_and_public_or_guarded_ones_pass() {
    let _g = SERIAL.lock().await;
    let (mut app, _) = app_of::<FeedModule>().await;
    app.set_route_posture(RoutePostureOptions::warn().exempt(RequestMethod::Get, "/feeds/stream", "fan-out proxy authenticates"));
    app.init().await.unwrap();
    let report = app.route_posture().unwrap();
    let find = |path: &str| report.entries().iter().find(|e| e.path == path).unwrap().clone();
    assert_eq!(find("/feeds/open").status, PostureStatus::Public);
    assert_eq!(find("/feeds/open").kind, corvis::PostureKind::Sse);
    assert_eq!(find("/feeds/guarded").status, PostureStatus::Protected);
    assert_eq!(find("/feeds/stream").status, PostureStatus::Exempt { reason: "fan-out proxy authenticates".into() });
    assert_eq!(find("/feeds/plain").kind, corvis::PostureKind::Http { method: RequestMethod::Get });
    let json = serde_json::to_value(&report).unwrap();
    let open = json["entries"].as_array().unwrap().iter().find(|e| e["path"] == "/feeds/open").unwrap();
    assert_eq!(open["kind"], serde_json::json!({ "type": "sse" }));
    assert!(report.to_string().contains("SSE  /feeds/open"), "{report}");
}

// ---------------------------------------------------------------------------
// WebSocket gateways.
// ---------------------------------------------------------------------------

#[cfg(feature = "websockets")]
mod gateways {
    use super::*;
    use corvis::{OnGatewayInit, PostureKind, UncheckedSurface, WsServer, websocket_gateway};

    static AFTER_INIT_RAN: AtomicBool = AtomicBool::new(false);

    macro_rules! gateway {
        ($name:ident, $path:tt, [$($attr:meta),*], { $($body:tt)* }) => {
            #[websocket_gateway(path = $path)]
            $(#[$attr])*
            pub struct $name {
                #[websocket_server]
                server: WsServer,
            }

            #[handlers]
            impl $name {
                $($body)*
            }
        };
    }

    gateway!(GuardedGateway, "/guarded", [use_guards(AuthGuard)], {
        #[subscribe_message("a")]
        fn a(&self) {}
        #[subscribe_message("b")]
        fn b(&self) {}
    });

    gateway!(HandlerGuardGateway, "/handler-guard", [], {
        #[subscribe_message("secure")]
        #[use_guards(AuthGuard)]
        fn secure(&self) {}
        #[subscribe_message("open")]
        fn open(&self) {}
    });

    gateway!(PublicGateway, "/public", [public], {
        #[subscribe_message("a")]
        fn a(&self) {}
        #[subscribe_message("b")]
        fn b(&self) {}
    });

    gateway!(PublicHandlerGateway, "/public-handler", [], {
        #[subscribe_message("ping")]
        #[public]
        fn ping(&self) {}
        #[subscribe_message("other")]
        fn other(&self) {}
    });

    gateway!(ChatGateway, "/chat", [], {
        #[subscribe_message("ping")]
        fn ping(&self) {}
        #[subscribe_message("rooms")]
        fn rooms(&self) {}
    });

    #[websocket_gateway(path = "/hooked")]
    pub struct HookedGateway {
        #[websocket_server]
        server: WsServer,
    }

    #[handlers]
    impl HookedGateway {
        #[subscribe_message("ping")]
        fn ping(&self) {}
    }

    #[async_trait]
    impl OnGatewayInit for HookedGateway {
        async fn after_init(&self, _server: WsServer) {
            AFTER_INIT_RAN.store(true, Ordering::SeqCst);
        }
    }

    #[module(providers = [GuardedGateway])]
    pub struct GuardedModule;

    #[module(providers = [HandlerGuardGateway])]
    pub struct HandlerGuardModule;

    #[module(providers = [PublicGateway])]
    pub struct PublicModule;

    #[module(providers = [PublicHandlerGateway])]
    pub struct PublicHandlerModule;

    #[module(providers = [ChatGateway])]
    pub struct ChatModule;

    #[module(providers = [HookedGateway])]
    pub struct HookedModule;

    #[module(controllers = [RoomsController], providers = [HookedGateway])]
    pub struct MixedGatewayModule;

    #[module(providers = [ChatGateway, Provider::app_guard::<AuthGuard>()])]
    pub struct GlobalGuardModule;

    #[module(
        imports = [ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)])],
        providers = [ChatGateway, Provider::app_guard::<ThrottlerGuard>()]
    )]
    pub struct GlobalThrottlerModule;

    fn unprotected_events(err: &anyhow::Error) -> Vec<String> {
        let err = err.downcast_ref::<RoutePostureError>().expect("a RoutePostureError");
        err.unprotected()
            .iter()
            .map(|e| match &e.kind {
                PostureKind::WsMessage { event } => format!("{} {event} {}::{}", e.path, e.owner, e.handler),
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    async fn enforced<M: corvis::Module>() -> Result<RoutePostureReport, anyhow::Error> {
        let (mut app, _) = app_of::<M>().await;
        app.require_route_posture();
        app.init().await?;
        Ok(app.route_posture().unwrap())
    }

    #[tokio::test]
    async fn a_gateway_guard_covers_every_handler() {
        let _g = SERIAL.lock().await;
        let report = enforced::<GuardedModule>().await.unwrap();
        assert_eq!(report.entries().len(), 2);
        assert!(report.entries().iter().all(|e| e.status == PostureStatus::Protected), "{report}");
    }

    #[tokio::test]
    async fn a_handler_guard_covers_only_its_handler() {
        let _g = SERIAL.lock().await;
        let err = enforced::<HandlerGuardModule>().await.unwrap_err();
        assert_eq!(unprotected_events(&err), ["/handler-guard open HandlerGuardGateway::open"]);
        let text = err.to_string();
        assert!(text.starts_with("Route posture check failed: 1 message handler has no guard and is not marked #[public]."), "{text}");
        assert!(text.contains("  WS  /handler-guard  event \"open\"  HandlerGuardGateway::open"), "{text}");
        assert!(text.contains("the message handler") && text.contains("the gateway"), "{text}");
    }

    #[tokio::test]
    async fn public_on_the_gateway_or_on_a_handler() {
        let _g = SERIAL.lock().await;
        let report = enforced::<PublicModule>().await.unwrap();
        assert!(report.entries().iter().all(|e| e.status == PostureStatus::Public), "{report}");

        let err = enforced::<PublicHandlerModule>().await.unwrap_err();
        assert_eq!(unprotected_events(&err), ["/public-handler other PublicHandlerGateway::other"]);
        let (mut app, _) = app_of::<PublicHandlerModule>().await;
        app.set_route_posture(RoutePostureOptions::warn());
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        let ping = report.entries().iter().find(|e| e.kind == PostureKind::WsMessage { event: "ping".into() }).unwrap();
        assert_eq!(ping.status, PostureStatus::Public);
    }

    #[tokio::test]
    async fn an_unguarded_handler_stops_init_before_after_init_runs() {
        let _g = SERIAL.lock().await;
        AFTER_INIT_RAN.store(false, Ordering::SeqCst);
        let err = enforced::<HookedModule>().await.unwrap_err();
        assert_eq!(unprotected_events(&err), ["/hooked ping HookedGateway::ping"]);
        assert!(!AFTER_INIT_RAN.load(Ordering::SeqCst), "after_init must not run when the check fails");

        let (mut app, _) = app_of::<HookedModule>().await;
        app.set_route_posture(RoutePostureOptions::warn());
        app.init().await.unwrap();
        assert!(AFTER_INIT_RAN.load(Ordering::SeqCst), "after_init runs once the check passes");
    }

    #[tokio::test]
    async fn routes_and_message_handlers_share_one_error() {
        let _g = SERIAL.lock().await;
        let (mut app, _) = app_of::<MixedGatewayModule>().await;
        app.require_route_posture();
        let err = app.init().await.unwrap_err();
        let text = err.to_string();
        assert!(text.starts_with("Route posture check failed: 3 routes or message handlers have no guard"), "{text}");
        assert!(text.contains("RoomsController::list"), "{text}");
        assert!(text.contains("event \"ping\"  HookedGateway::ping"), "{text}");
    }

    #[tokio::test]
    async fn global_guards_count_for_gateways_unless_they_are_kept_http_only() {
        let _g = SERIAL.lock().await;
        let report = enforced::<GlobalGuardModule>().await.unwrap();
        assert!(report.entries().iter().all(|e| e.status == PostureStatus::Protected), "{report}");
        let guards = &report.entries()[0].guards;
        assert_eq!((guards[0].name.as_str(), guards[0].controls_access), ("AuthGuard", true));

        let (mut app, _) = app_of::<GlobalGuardModule>().await;
        app.apply_global_enhancers_to_gateways(false);
        app.require_route_posture();
        let err = app.init().await.unwrap_err();
        assert_eq!(unprotected_events(&err), ["/chat ping ChatGateway::ping", "/chat rooms ChatGateway::rooms"]);
    }

    #[tokio::test]
    async fn a_global_rate_limiter_alone_leaves_handlers_unprotected() {
        let _g = SERIAL.lock().await;
        let err = enforced::<GlobalThrottlerModule>().await.unwrap_err();
        assert_eq!(unprotected_events(&err).len(), 2);
        let (mut app, _) = app_of::<GlobalThrottlerModule>().await;
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        let guards = &report.entries()[0].guards;
        assert_eq!((guards[0].name.as_str(), guards[0].controls_access), ("ThrottlerGuard", false));
    }

    #[tokio::test]
    async fn message_exemptions_match_by_path_and_event() {
        let _g = SERIAL.lock().await;
        let (mut app, captured) = app_of::<ChatModule>().await;
        app.set_route_posture(
            RoutePostureOptions::enforce()
                .exempt_message("/chat", "ping", "health probe, no data")
                .exempt_message("/chat", "rooms", "public listing")
                .exempt_message("/chat", "gone", "removed"),
        );
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        assert_eq!(report.entries().len(), 2);
        let ping = report.entries().iter().find(|e| e.handler == "ping").unwrap();
        assert_eq!(ping.status, PostureStatus::Exempt { reason: "health probe, no data".into() });
        let warns: Vec<_> = posture_logs(&captured).into_iter().filter(|(l, _)| *l == LogLevel::Warn).collect();
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert!(warns[0].1.contains("WS /chat event \"gone\" matches no handler"), "{}", warns[0].1);
        assert_eq!(report.stale_exemptions().len(), 1);
        assert_eq!(report.stale_exemptions()[0].event(), Some("gone"));

        let (mut app, _) = app_of::<ChatModule>().await;
        app.set_route_posture(RoutePostureOptions::enforce().exempt_message("/chat", "*", "private network only"));
        app.init().await.unwrap();
        assert!(app.route_posture().unwrap().entries().iter().all(|e| matches!(e.status, PostureStatus::Exempt { .. })));

        let (mut app, _) = app_of::<ChatModule>().await;
        app.set_route_posture(RoutePostureOptions::enforce().exempt_message("/chat", "ping", "only ping"));
        let err = app.init().await.unwrap_err();
        assert_eq!(unprotected_events(&err), ["/chat rooms ChatGateway::rooms"]);
    }

    #[tokio::test]
    async fn exemption_kinds_do_not_cross() {
        let _g = SERIAL.lock().await;
        let (mut app, _) = app_of::<ChatModule>().await;
        app.set_route_posture(RoutePostureOptions::enforce().exempt(RequestMethod::All, "/chat", "an http exemption"));
        let err = app.init().await.unwrap_err();
        assert_eq!(unprotected_events(&err).len(), 2);

        let (mut app, _) = app_of::<OpenModule>().await;
        app.set_route_posture(RoutePostureOptions::enforce().exempt_message("/rooms", "*", "not a route"));
        let err = app.init().await.unwrap_err();
        assert_eq!(unprotected_of(&err).len(), 2);
    }

    #[tokio::test]
    async fn a_blank_message_exemption_reason_names_path_and_event() {
        let _g = SERIAL.lock().await;
        let (mut app, _) = app_of::<ChatModule>().await;
        app.set_route_posture(RoutePostureOptions::warn().exempt_message("/chat", "ping", " "));
        let err = app.init().await.unwrap_err().to_string();
        assert!(err.contains("WS /chat event \"ping\""), "{err}");
    }

    // -----------------------------------------------------------------------
    // Not-checked surfaces.
    // -----------------------------------------------------------------------

    #[websocket_gateway(path = "/secured")]
    #[public]
    pub struct SecuredGateway {
        #[websocket_server]
        server: WsServer,
    }

    #[handlers]
    impl SecuredGateway {
        #[subscribe_message("ping")]
        fn ping(&self) {}
    }

    #[async_trait]
    impl OnGatewayInit for SecuredGateway {
        async fn after_init(&self, server: WsServer) {
            server.set_options(corvis::websockets::GatewayOptions::default().verify_client(|_| true));
        }
    }

    #[injectable]
    pub struct UpgradeProbe {
        host: Arc<corvis::HttpAdapterHost>,
    }

    #[async_trait]
    impl OnApplicationBootstrap for UpgradeProbe {
        async fn on_application_bootstrap(&self) -> corvis::Result<()> {
            self.host.http_adapter().get_http_server().on_upgrade(|req| req.destroy());
            Ok(())
        }
    }

    #[module(
        imports = [ServeStaticModule::for_root(vec![ServeStaticOptions::new(env!("CARGO_MANIFEST_DIR")).serve_root("/assets")])],
        providers = [SecuredGateway, UpgradeProbe],
    )]
    pub struct SurfacesModule;

    #[tokio::test]
    async fn unchecked_surfaces_are_listed_and_never_fail_init() {
        let _g = SERIAL.lock().await;
        let (mut app, _) = app_of::<SurfacesModule>().await;
        app.require_route_posture();
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        let unchecked = report.unchecked();
        assert!(unchecked.contains(&UncheckedSurface::StaticFiles { serve_root: "/assets".into() }), "{unchecked:?}");
        assert!(
            unchecked.contains(&UncheckedSurface::WsHandshake {
                path: "/secured".into(),
                gateway: "SecuredGateway".into(),
                verify_client: true
            }),
            "{unchecked:?}"
        );
        assert!(unchecked.contains(&UncheckedSurface::UpgradeListeners { count: 1 }), "{unchecked:?}");
        let table = report.to_string();
        assert!(table.contains("Not checked:") && table.contains("static files served at /assets"), "{table}");
        assert!(table.contains("verify_client set") && table.contains("1 raw upgrade listener"), "{table}");
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["unchecked"][0], serde_json::json!({ "type": "static_files", "serve_root": "/assets" }));
    }

    gateway!(ShapeGateway, "/shape", [use_guards(AuthGuard)], {
        #[subscribe_message("rooms")]
        fn rooms(&self) {}
    });

    #[module(controllers = [FeedController], providers = [ShapeGateway])]
    pub struct ShapeModule;

    #[tokio::test]
    async fn the_report_lists_handlers_sse_routes_and_handshakes_in_off_mode() {
        let _g = SERIAL.lock().await;
        let (mut app, captured) = app_of::<ShapeModule>().await;
        app.init().await.unwrap();
        assert!(posture_logs(&captured).is_empty());
        let report = app.route_posture().unwrap();
        let json = serde_json::to_value(&report).unwrap();
        let entries = json["entries"].as_array().unwrap();
        let ws = entries.iter().find(|e| e["path"] == "/shape").unwrap();
        assert_eq!(ws["kind"], serde_json::json!({ "type": "ws_message", "event": "rooms" }));
        assert_eq!(ws["owner"], "ShapeGateway");
        assert_eq!(ws["handler"], "rooms");
        assert_eq!(ws["guards"], serde_json::json!([{ "name": "AuthGuard", "controls_access": true }]));
        assert_eq!(ws["status"], serde_json::json!({ "state": "protected" }));
        assert!(entries.iter().any(|e| e["kind"] == serde_json::json!({ "type": "sse" })));
        assert_eq!(
            json["unchecked"],
            serde_json::json!([{ "type": "ws_handshake", "path": "/shape", "gateway": "ShapeGateway", "verify_client": false }])
        );
    }
}

#[cfg(feature = "websockets")]
mod gateway_details {
    use super::gateways::{ChatModule, GuardedModule, PublicHandlerModule};
    use super::*;
    use corvis::{GuardScope, PublicSource};

    #[tokio::test]
    async fn the_summary_counts_gateway_handlers_with_their_scope() {
        let _g = SERIAL.lock().await;
        let (mut app, captured) = app_of::<ScopesModule>().await;
        app.set_route_posture(RoutePostureOptions::warn());
        app.init().await.unwrap();
        assert_eq!(
            summary_logs(&captured),
            ["Route posture: access guards counted: AllowGuard (global, 2 routes), AuthGuard (gateway ScopedGateway, 2 routes), \
              AuthGuard (message handler ScopedGateway::b, 1 route)"]
        );
        let report = app.route_posture().unwrap();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["access_guards"][1]["scope"], serde_json::json!({ "level": "gateway", "gateway": "ScopedGateway" }));
        assert!(report.access_guards().iter().any(|g| matches!(&g.scope, GuardScope::Message { handler, .. } if handler == "b")));

        let (mut app, captured) = app_of::<ScopesModule>().await;
        app.apply_global_enhancers_to_gateways(false);
        app.set_route_posture(RoutePostureOptions::warn());
        app.init().await.unwrap();
        assert_eq!(
            summary_logs(&captured),
            ["Route posture: access guards counted: AuthGuard (gateway ScopedGateway, 2 routes), \
              AuthGuard (message handler ScopedGateway::b, 1 route)"]
        );
    }

    #[tokio::test]
    async fn public_source_names_the_gateway_or_the_handler() {
        let _g = SERIAL.lock().await;
        let (mut app, _) = app_of::<super::gateways::PublicModule>().await;
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        assert!(report.entries().iter().all(|e| e.public_source == Some(PublicSource::Gateway)), "{report}");
        assert!(report.to_string().contains("public (gateway)"), "{report}");

        let (mut app, _) = app_of::<PublicHandlerModule>().await;
        app.init().await.unwrap();
        let report = app.route_posture().unwrap();
        let ping = report.entries().iter().find(|e| e.handler == "ping").unwrap();
        assert_eq!(ping.public_source, Some(PublicSource::Handler));
        let json = serde_json::to_value(&report).unwrap();
        assert!(json["entries"].as_array().unwrap().iter().any(|e| e["public_source"] == "handler"));
    }

    #[tokio::test]
    async fn message_exemptions_use_the_literal_path_matcher_and_redundant_ones_warn() {
        let _g = SERIAL.lock().await;
        let (mut app, captured) = app_of::<ChatModule>().await;
        app.set_route_posture(
            RoutePostureOptions::enforce().exempt_message("/c*", "ping", "glob inside a segment").exempt_message("/chat", "rooms", "ok"),
        );
        app.init().await.unwrap();
        assert!(app.route_posture().unwrap().entries().iter().all(|e| matches!(e.status, PostureStatus::Exempt { .. })));

        let (mut app, _) = app_of::<ChatModule>().await;
        app.set_route_posture(RoutePostureOptions::enforce().exempt_message("/chat/*", "*", "needs one more segment"));
        app.init().await.unwrap_err();

        let (mut app, captured2) = app_of::<GuardedModule>().await;
        app.set_route_posture(RoutePostureOptions::warn().exempt_message("/guarded", "a", "already guarded"));
        app.init().await.unwrap();
        let warns: Vec<_> = posture_logs(&captured2).into_iter().filter(|(l, _)| *l == LogLevel::Warn).collect();
        assert!(
            warns.iter().any(|(_, m)| m.contains("WS /guarded event \"a\" matches only guarded or public message handlers")),
            "{warns:?}"
        );
        assert_eq!(app.route_posture().unwrap().redundant_exemptions().len(), 1);
        let _ = captured;
    }

    #[corvis::websocket_gateway(path = "/scoped")]
    #[use_guards(AuthGuard)]
    pub struct ScopedGateway {
        #[websocket_server]
        server: corvis::WsServer,
    }

    #[handlers]
    impl ScopedGateway {
        #[subscribe_message("a")]
        fn a(&self) {}
        #[subscribe_message("b")]
        #[use_guards(AuthGuard)]
        fn b(&self) {}
    }

    #[module(providers = [ScopedGateway, Provider::app_guard::<AllowGuard>()])]
    pub struct ScopesModule;
}

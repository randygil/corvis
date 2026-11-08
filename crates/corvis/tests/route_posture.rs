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
    captured.lock().unwrap().iter().filter(|(_, _, ctx)| ctx.as_deref() == Some("RoutePosture")).map(|(l, m, _)| (*l, m.clone())).collect()
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
// Report mode (CORVIS_ROUTE_POSTURE_REPORT): the test binary runs itself as a child process.
// ---------------------------------------------------------------------------

#[injectable]
pub struct HookProbe;

#[async_trait]
impl OnModuleInit for HookProbe {
    async fn on_module_init(&self) -> corvis::Result<()> {
        println!("HOOK RAN");
        Ok(())
    }
}

#[controller("report-open")]
pub struct ReportOpenController;

#[routes]
impl ReportOpenController {
    #[get]
    fn unguarded(&self) -> Value {
        json!({})
    }
}

#[controller("report-public")]
#[public]
pub struct ReportPublicController;

#[routes]
impl ReportPublicController {
    #[get]
    fn open_on_purpose(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [ReportOpenController], providers = [HookProbe])]
pub struct ReportOpenModule;

#[module(controllers = [ReportPublicController], providers = [HookProbe])]
pub struct ReportPublicModule;

const FIXTURE_VAR: &str = "CORVIS_TEST_POSTURE_FIXTURE";

/// The child of the `report_mode_*` tests: does nothing unless the parent selected a fixture.
#[test]
fn report_mode_child() {
    let Ok(fixture) = std::env::var(FIXTURE_VAR) else { return };
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap().block_on(async {
        let mut app = match fixture.as_str() {
            "open" => CorvisFactory::create::<ReportOpenModule>().await.unwrap(),
            _ => CorvisFactory::create::<ReportPublicModule>().await.unwrap(),
        };
        app.silence_route_logs();
        if fixture == "init" {
            match app.init().await {
                Ok(()) => println!("INIT OK"),
                Err(e) => println!("INIT ERROR: {e}"),
            }
            return;
        }
        let result = app.listen_in_background(0).await;
        println!("AFTER LISTEN {}", result.is_ok());
    });
}

struct Child {
    code: Option<i32>,
    stdout: String,
}

fn run_child(fixture: &str, report: Option<&str>) -> Child {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "report_mode_child", "--nocapture", "--test-threads=1"]).env(FIXTURE_VAR, fixture);
    match report {
        Some(v) => cmd.env("CORVIS_ROUTE_POSTURE_REPORT", v),
        None => cmd.env_remove("CORVIS_ROUTE_POSTURE_REPORT"),
    };
    let out = cmd.output().unwrap();
    Child { code: out.status.code(), stdout: String::from_utf8_lossy(&out.stdout).into_owned() }
}

fn between_markers(stdout: &str) -> Vec<&str> {
    let lines: Vec<&str> = stdout.lines().collect();
    let begin = lines.iter().position(|l| *l == corvis::posture::REPORT_BEGIN).expect("BEGIN marker");
    let end = lines.iter().position(|l| *l == corvis::posture::REPORT_END).expect("END marker");
    assert!(begin < end);
    lines[begin + 1..end].to_vec()
}

#[test]
fn report_mode_text_lists_the_unprotected_route_and_stops_before_hooks_and_listening() {
    let child = run_child("open", Some("text"));
    assert_eq!(child.code, Some(1), "{}", child.stdout);
    let body = between_markers(&child.stdout).join("\n");
    assert!(body.contains("/report-open") && body.contains("ReportOpenController::unguarded") && body.contains("UNPROTECTED"), "{body}");
    assert!(!child.stdout.contains("HOOK RAN") && !child.stdout.contains("AFTER LISTEN"), "{}", child.stdout);
}

#[test]
fn report_mode_json_with_only_public_routes_exits_zero() {
    let child = run_child("public", Some("json"));
    assert_eq!(child.code, Some(0), "{}", child.stdout);
    let json: serde_json::Value = serde_json::from_str(&between_markers(&child.stdout).join("\n")).unwrap();
    assert_eq!(json["entries"][0]["status"]["state"], "public");
    assert_eq!(json["entries"][0]["path"], "/report-public");
    assert!(!child.stdout.contains("HOOK RAN") && !child.stdout.contains("AFTER LISTEN"), "{}", child.stdout);
}

#[test]
fn report_mode_json_has_only_the_report_between_the_markers() {
    let child = run_child("open", Some(" json "));
    assert_eq!(child.code, Some(1), "{}", child.stdout);
    let lines = between_markers(&child.stdout);
    assert!(lines.first().is_some_and(|l| *l == "{"), "{lines:?}");
    assert!(lines.last().is_some_and(|l| *l == "}"), "{lines:?}");
}

#[test]
fn report_mode_with_an_unknown_value_makes_init_fail_without_ending_the_process() {
    let child = run_child("init", Some("yaml"));
    assert_eq!(child.code, Some(0), "{}", child.stdout);
    assert!(!child.stdout.contains(corvis::posture::REPORT_BEGIN), "{}", child.stdout);
    let line = child.stdout.lines().find(|l| l.starts_with("INIT ERROR")).expect("init failed");
    assert!(line.contains("CORVIS_ROUTE_POSTURE_REPORT") && line.contains("text") && line.contains("json"), "{line}");
}

#[test]
fn report_mode_unset_or_blank_leaves_init_unchanged() {
    for value in [None, Some(""), Some("  ")] {
        let child = run_child("init", value);
        assert_eq!(child.code, Some(0));
        assert!(child.stdout.contains("INIT OK") && child.stdout.contains("HOOK RAN"), "{value:?}: {}", child.stdout);
        assert!(!child.stdout.contains(corvis::posture::REPORT_BEGIN));
    }
}

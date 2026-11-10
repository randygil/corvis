//! Guard responses: `respond_with` and `append_response_header` on `ctx.switch_to_http()`, and how the router
//! merges them on every response path (success, exception, filter, `#[res]`, stored guard response).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use corvis::prelude::*;
use corvis::testing::request;
use corvis::{CookieOptions, Res};

static HANDLER_CALLS: AtomicUsize = AtomicUsize::new(0);
static SECOND_GUARD_CALLS: AtomicUsize = AtomicUsize::new(0);
static FILTER_CALLS: AtomicUsize = AtomicUsize::new(0);
static INJECTION_ACCEPTED: AtomicBool = AtomicBool::new(true);
static CAP_OK: AtomicBool = AtomicBool::new(false);
static CAP_129TH: AtomicBool = AtomicBool::new(true);

fn redirect_to(location: &str) -> Response {
    Response::new().with_status(302u16).with_header("Location", location)
}

// ---- guards ----

#[injectable]
pub struct CookieGuard;

#[async_trait]
impl CanActivate for CookieGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        assert!(http.append_response_header("Set-Cookie", "sid=1; Path=/; HttpOnly"));
        assert!(http.append_response_header("Set-Cookie", "theme=dark; Path=/"));
        Ok(true)
    }
}

#[injectable]
pub struct OneCookieGuard;

#[async_trait]
impl CanActivate for OneCookieGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        assert!(ctx.switch_to_http().append_response_header("Set-Cookie", "a=1"));
        Ok(true)
    }
}

#[injectable]
pub struct LinkGuard;

#[async_trait]
impl CanActivate for LinkGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        http.set_response_header("X-Mixed", "set");
        http.append_response_header("Link", "</a>; rel=preload");
        http.append_response_header("Link", "</b>; rel=preload");
        http.append_response_header("X-Trace", "guard");
        http.append_response_header("X-Mixed", "appended");
        Ok(true)
    }
}

#[injectable]
pub struct InjectionGuard;

#[async_trait]
impl CanActivate for InjectionGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        INJECTION_ACCEPTED.store(http.append_response_header("Set-Cookie", "a=1\r\nX-Evil: 1"), Ordering::SeqCst);
        assert!(!http.append_response_header("bad name", "v"));
        for reserved in ["Content-Length", "Transfer-Encoding", "Connection", "Keep-Alive", "Proxy-Connection", "TE", "Trailer", "Upgrade"]
        {
            assert!(!http.append_response_header(reserved, "5"), "{reserved}");
        }
        Ok(true)
    }
}

#[injectable]
pub struct CapGuard;

#[async_trait]
impl CanActivate for CapGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        CAP_OK.store((0..128).all(|_| http.append_response_header("X-Trace", "v")), Ordering::SeqCst);
        CAP_129TH.store(http.append_response_header("X-Trace", "v"), Ordering::SeqCst);
        Ok(true)
    }
}

#[injectable]
pub struct LimitGuard;

#[async_trait]
impl CanActivate for LimitGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().set_response_header("X-RateLimit-Limit", "10");
        Ok(true)
    }
}

#[injectable]
pub struct RedirectGuard;

#[async_trait]
impl CanActivate for RedirectGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        assert!(ctx.switch_to_http().respond_with(redirect_to("/login")));
        Ok(true)
    }
}

#[injectable]
pub struct RedirectThenDenyGuard;

#[async_trait]
impl CanActivate for RedirectThenDenyGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().respond_with(redirect_to("/denied"));
        Err(UnauthorizedException::empty())
    }
}

#[injectable]
pub struct RedirectThenFalseGuard;

#[async_trait]
impl CanActivate for RedirectThenFalseGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        ctx.switch_to_http().respond_with(redirect_to("/false"));
        Ok(false)
    }
}

#[injectable]
pub struct ReplacingGuard;

#[async_trait]
impl CanActivate for ReplacingGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        http.respond_with(redirect_to("/first"));
        http.respond_with(redirect_to("/second"));
        Ok(true)
    }
}

#[injectable]
pub struct SecondGuard;

#[async_trait]
impl CanActivate for SecondGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        SECOND_GUARD_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

#[injectable]
pub struct DenyGuard;

#[async_trait]
impl CanActivate for DenyGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(false)
    }
}

#[injectable]
pub struct ThrowGuard;

#[async_trait]
impl CanActivate for ThrowGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        Err(UnauthorizedException::empty())
    }
}

#[injectable]
pub struct RedirectWithCookieGuard;

#[async_trait]
impl CanActivate for RedirectWithCookieGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        http.append_response_header("Set-Cookie", "state=xyz; Path=/");
        http.respond_with(redirect_to("/idp").with_cookie(Cookie::new("own", "1")));
        Ok(true)
    }
}

#[injectable]
pub struct GlobalRedirectGuard;

#[async_trait]
impl CanActivate for GlobalRedirectGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        if ctx.request().path() == "/g/ok" {
            ctx.switch_to_http().respond_with(redirect_to("/global"));
        }
        Ok(true)
    }
}

// ---- filters ----

#[injectable]
pub struct CatchAll;

#[async_trait]
impl ExceptionFilter for CatchAll {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        FILTER_CALLS.fetch_add(1, Ordering::SeqCst);
        Response::json(json!({ "by": "filter" })).with_status(418)
    }
}

#[injectable]
pub struct PlainFilter;

#[async_trait]
impl ExceptionFilter for PlainFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "plain filter" })).with_status(418)
    }
}

#[injectable]
pub struct CookieFilter;

#[async_trait]
impl ExceptionFilter for CookieFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "filter" })).with_status(418).with_cookie(Cookie::new("filter", "1"))
    }
}

// ---- controllers ----

#[controller("c")]
#[use_guards(CookieGuard)]
pub struct CookieController;

#[routes]
impl CookieController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("handler-cookie")]
    fn handler_cookie(&self) -> Response {
        Response::json(json!({ "ok": true })).with_cookie(Cookie::new("mine", "3"))
    }

    #[get("err")]
    fn err(&self) -> HttpResult<()> {
        Err(UnauthorizedException::empty())
    }

    #[get("filtered")]
    #[use_filters(CookieFilter)]
    fn filtered(&self) -> HttpResult<()> {
        Err(NotFoundException::new("x"))
    }

    #[get("res")]
    fn res(&self, #[res] res: Res) {
        res.cookie("b", "2", CookieOptions::new());
        res.send("ok");
    }

    #[get("res-pass")]
    fn res_pass(&self, #[res(passthrough)] res: Res) -> Value {
        res.cookie("b", "2", CookieOptions::new());
        json!({ "ok": true })
    }
}

#[controller("one")]
#[use_guards(OneCookieGuard)]
pub struct OneCookieController;

#[routes]
impl OneCookieController {
    #[get("res")]
    fn res(&self, #[res] res: Res) {
        res.cookie("b", "2", CookieOptions::new());
        res.send("ok");
    }
}

#[controller("h")]
#[use_guards(LinkGuard)]
pub struct HeaderController;

#[routes]
impl HeaderController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("mine")]
    fn mine(&self) -> Response {
        Response::json(json!({ "ok": true })).with_header("X-Trace", "mine")
    }

    #[get("res")]
    fn res(&self, #[res(passthrough)] res: Res) -> Value {
        res.set("X-Own", "1");
        json!({ "ok": true })
    }

    #[get("res-mine")]
    fn res_mine(&self, #[res(passthrough)] res: Res) -> Value {
        res.set("X-Trace", "mine");
        json!({ "ok": true })
    }
}

#[controller("inj")]
#[use_guards(InjectionGuard)]
pub struct InjectionController;

#[routes]
impl InjectionController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("cap")]
#[use_guards(CapGuard)]
pub struct CapController;

#[routes]
impl CapController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("r")]
#[use_guards(RedirectGuard, SecondGuard)]
pub struct RedirectController;

#[routes]
impl RedirectController {
    #[get("ok")]
    fn ok(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("filtered")]
    #[use_filters(CatchAll)]
    fn filtered(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("route-redirect")]
    #[redirect("/elsewhere", 301)]
    #[header("X-Route", "1")]
    fn route_redirect(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("res-redirect")]
#[use_guards(RedirectGuard)]
pub struct ResRedirectController;

#[routes]
impl ResRedirectController {
    #[get("ok")]
    fn ok(&self, #[res] res: Res) {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        res.send("handler");
    }
}

#[controller("stored")]
#[use_guards(RedirectThenDenyGuard)]
pub struct StoredThenDenyController;

#[routes]
impl StoredThenDenyController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("stored-false")]
#[use_guards(RedirectThenFalseGuard)]
pub struct StoredThenFalseController;

#[routes]
impl StoredThenFalseController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("replace")]
#[use_guards(ReplacingGuard)]
pub struct ReplaceController;

#[routes]
impl ReplaceController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("limit")]
#[use_guards(LimitGuard, RedirectGuard)]
pub struct LimitController;

#[routes]
impl LimitController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("idp")]
#[use_guards(RedirectWithCookieGuard)]
pub struct IdpController;

#[routes]
impl IdpController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }
}

#[controller("g")]
pub struct GlobalController;

#[routes]
impl GlobalController {
    #[get("ok")]
    #[use_guards(SecondGuard)]
    fn ok(&self) -> Value {
        HANDLER_CALLS.fetch_add(1, Ordering::SeqCst);
        json!({ "ok": true })
    }

    #[get("other")]
    fn other(&self) -> Value {
        json!({ "other": true })
    }
}

#[controller("plain")]
pub struct PlainController;

#[routes]
impl PlainController {
    #[get("deny")]
    #[use_guards(DenyGuard)]
    fn deny(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("throw")]
    #[use_guards(ThrowGuard)]
    fn throw(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("filtered-deny")]
    #[use_guards(DenyGuard)]
    #[use_filters(PlainFilter)]
    fn filtered_deny(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [
    CookieController, OneCookieController, HeaderController, InjectionController, CapController, RedirectController,
    ResRedirectController, StoredThenDenyController, StoredThenFalseController, ReplaceController, LimitController,
    IdpController, GlobalController, PlainController
])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

async fn app_with_global_redirect() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.use_global_guards(GlobalRedirectGuard);
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

fn cookies(r: &corvis::testing::TestResponse) -> Vec<String> {
    r.headers.get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

fn values(r: &corvis::testing::TestResponse, name: &str) -> Vec<String> {
    r.headers.get_all(name).iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

// ---- append_response_header ----

#[corvis::test]
async fn cookie_set_by_a_guard_survives_an_exception() {
    let app = app().await;
    let r = request(&app).get("/c/err").send().await;
    r.expect_status(401).expect_json(json!({ "message": "Unauthorized", "statusCode": 401 }));
    assert_eq!(cookies(&r), ["sid=1; Path=/; HttpOnly", "theme=dark; Path=/"]);
}

#[corvis::test]
async fn guard_cookies_are_kept_on_success() {
    let app = app().await;
    let r = request(&app).get("/c/ok").send().await;
    r.expect_status(200);
    assert_eq!(cookies(&r), ["sid=1; Path=/; HttpOnly", "theme=dark; Path=/"]);
}

#[corvis::test]
async fn guard_cookies_and_handler_cookie() {
    let app = app().await;
    let r = request(&app).get("/c/handler-cookie").send().await;
    r.expect_status(200);
    assert_eq!(cookies(&r).len(), 3);
    assert_eq!(cookies(&r)[0], "sid=1; Path=/; HttpOnly");
    assert_eq!(cookies(&r)[1], "theme=dark; Path=/");
    assert!(cookies(&r)[2].starts_with("mine=3"), "{:?}", cookies(&r));
}

#[corvis::test]
async fn guard_cookies_and_filter_cookie() {
    let app = app().await;
    let r = request(&app).get("/c/filtered").send().await;
    r.expect_status(418);
    let all = cookies(&r);
    assert_eq!(all.len(), 3, "{all:?}");
    assert_eq!(&all[..2], ["sid=1; Path=/; HttpOnly", "theme=dark; Path=/"]);
    assert!(all[2].starts_with("filter=1"), "{all:?}");
}

#[corvis::test]
async fn res_handler_adds_a_cookie_after_the_guard_cookie() {
    let app = app().await;
    let r = request(&app).get("/one/res").send().await;
    r.expect_status(200);
    let all = cookies(&r);
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], "a=1");
    assert!(all[1].starts_with("b=2"), "{all:?}");
}

#[corvis::test]
async fn res_passthrough_keeps_guard_cookies_once() {
    let app = app().await;
    let r = request(&app).get("/c/res-pass").send().await;
    r.expect_status(200);
    let all = cookies(&r);
    assert_eq!(all.len(), 3, "{all:?}");
    assert_eq!(&all[..2], ["sid=1; Path=/; HttpOnly", "theme=dark; Path=/"]);
    assert!(all[2].starts_with("b=2"), "{all:?}");
    let r = request(&app).get("/c/res").send().await;
    assert_eq!(cookies(&r).len(), 3, "{:?}", cookies(&r));
}

#[corvis::test]
async fn two_appended_values_keep_their_order() {
    let app = app().await;
    let r = request(&app).get("/h/ok").send().await;
    r.expect_status(200);
    assert_eq!(values(&r, "link"), ["</a>; rel=preload", "</b>; rel=preload"]);
    assert_eq!(values(&r, "x-trace"), ["guard"]);
}

#[corvis::test]
async fn appended_values_follow_the_value_recorded_with_set() {
    let app = app().await;
    let r = request(&app).get("/h/ok").send().await;
    assert_eq!(values(&r, "x-mixed"), ["set", "appended"]);
    let r = request(&app).get("/h/res").send().await;
    assert_eq!(values(&r, "x-mixed"), ["set", "appended"]);
    assert_eq!(values(&r, "link").len(), 2);
    assert_eq!(values(&r, "x-own"), ["1"]);
}

#[corvis::test]
async fn handler_header_wins_over_an_appended_one() {
    let app = app().await;
    let r = request(&app).get("/h/mine").send().await;
    assert_eq!(values(&r, "x-trace"), ["mine"]);
    let r = request(&app).get("/h/res-mine").send().await;
    assert_eq!(values(&r, "x-trace"), ["mine"]);
}

#[corvis::test]
async fn header_injection_is_refused() {
    let app = app().await;
    let r = request(&app).get("/inj/ok").send().await;
    r.expect_status(200);
    assert!(!INJECTION_ACCEPTED.load(Ordering::SeqCst));
    assert!(r.header("x-evil").is_none());
    assert!(r.header("set-cookie").is_none());
}

#[corvis::test]
async fn at_most_128_values_are_recorded() {
    let app = app().await;
    let r = request(&app).get("/cap/ok").send().await;
    r.expect_status(200);
    assert!(CAP_OK.load(Ordering::SeqCst));
    assert!(!CAP_129TH.load(Ordering::SeqCst));
    assert_eq!(values(&r, "x-trace").len(), 128);
}

// ---- respond_with ----

#[corvis::test]
async fn guard_short_circuits_with_a_redirect() {
    let app = app().await;
    let before = (HANDLER_CALLS.load(Ordering::SeqCst), SECOND_GUARD_CALLS.load(Ordering::SeqCst));
    let r = request(&app).get("/r/ok").send().await;
    r.expect_status(302).expect_header("location", "/login");
    assert!(r.body.is_empty());
    assert_eq!(before, (HANDLER_CALLS.load(Ordering::SeqCst), SECOND_GUARD_CALLS.load(Ordering::SeqCst)));
}

#[corvis::test]
async fn exception_filters_are_not_involved() {
    let app = app().await;
    let before = FILTER_CALLS.load(Ordering::SeqCst);
    let r = request(&app).get("/r/filtered").send().await;
    r.expect_status(302).expect_header("location", "/login");
    assert_eq!(before, FILTER_CALLS.load(Ordering::SeqCst));
}

#[corvis::test]
async fn route_redirect_and_header_settings_do_not_touch_a_stored_response() {
    let app = app().await;
    let r = request(&app).get("/r/route-redirect").send().await;
    r.expect_status(302).expect_header("location", "/login");
    assert!(r.header("x-route").is_none());
}

#[corvis::test]
async fn stored_response_on_a_res_route() {
    let app = app().await;
    let r = request(&app).get("/res-redirect/ok").send().await;
    r.expect_status(302).expect_header("location", "/login");
    assert!(r.body.is_empty());
}

#[corvis::test]
async fn global_guard_stores_a_response() {
    let app = app_with_global_redirect().await;
    let before = SECOND_GUARD_CALLS.load(Ordering::SeqCst);
    let r = request(&app).get("/g/ok").send().await;
    r.expect_status(302).expect_header("location", "/global");
    assert_eq!(before, SECOND_GUARD_CALLS.load(Ordering::SeqCst));
    request(&app).get("/g/other").send().await.expect_status(200).expect_json(json!({ "other": true }));
}

#[corvis::test]
async fn a_second_respond_with_replaces_the_first() {
    let app = app().await;
    request(&app).get("/replace/ok").send().await.expect_status(302).expect_header("location", "/second");
}

#[corvis::test]
async fn stored_response_wins_over_an_error_or_a_denial() {
    let app = app().await;
    request(&app).get("/stored/ok").send().await.expect_status(302).expect_header("location", "/denied");
    request(&app).get("/stored-false/ok").send().await.expect_status(302).expect_header("location", "/false");
}

#[corvis::test]
async fn rate_limit_headers_are_kept_on_a_redirect() {
    let app = app().await;
    let r = request(&app).get("/limit/ok").send().await;
    r.expect_status(302).expect_header("location", "/login").expect_header("x-ratelimit-limit", "10");
}

#[corvis::test]
async fn appended_cookies_come_before_the_stored_response_cookies() {
    let app = app().await;
    let r = request(&app).get("/idp/ok").send().await;
    r.expect_status(302).expect_header("location", "/idp");
    let all = cookies(&r);
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0], "state=xyz; Path=/");
    assert!(all[1].starts_with("own=1"), "{all:?}");
}

// ---- unchanged behavior ----

#[corvis::test]
async fn plain_guard_denies_as_before() {
    let app = app().await;
    let r = request(&app).get("/plain/deny").send().await;
    r.expect_status(403).expect_json(json!({ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }));
    assert!(r.header("location").is_none() && r.header("set-cookie").is_none());
}

#[corvis::test]
async fn plain_guard_error_goes_to_the_filters() {
    let app = app().await;
    request(&app).get("/plain/throw").send().await.expect_status(401).expect_json(json!({ "message": "Unauthorized", "statusCode": 401 }));
    request(&app).get("/plain/filtered-deny").send().await.expect_status(418).expect_json(json!({ "by": "plain filter" }));
}

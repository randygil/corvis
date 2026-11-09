//! Response headers set by enhancers through `ctx.switch_to_http().set_response_header`, and how they are merged
//! into the response of every kind of route.

use std::sync::atomic::{AtomicBool, Ordering};

use corvis::Res;
use corvis::prelude::*;
use corvis::testing::request;

static INJECTION_ACCEPTED: AtomicBool = AtomicBool::new(true);
static RESERVED_ACCEPTED: AtomicBool = AtomicBool::new(true);
static EURO_ACCEPTED: AtomicBool = AtomicBool::new(true);
static CAP_FIRST_128_ACCEPTED: AtomicBool = AtomicBool::new(false);
static CAP_129TH_ACCEPTED: AtomicBool = AtomicBool::new(true);
static CAP_REPLACE_ACCEPTED: AtomicBool = AtomicBool::new(false);

#[injectable]
pub struct ManyGuard;

#[async_trait]
impl CanActivate for ManyGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        let all = (0..128).all(|i| http.set_response_header(&format!("X-Many-{i}"), "v"));
        CAP_FIRST_128_ACCEPTED.store(all, Ordering::SeqCst);
        CAP_129TH_ACCEPTED.store(http.set_response_header("X-Many-128", "v"), Ordering::SeqCst);
        CAP_REPLACE_ACCEPTED.store(http.set_response_header("X-Many-0", "replaced"), Ordering::SeqCst);
        Ok(true)
    }
}

#[injectable]
pub struct HeaderGuard;

#[async_trait]
impl CanActivate for HeaderGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let http = ctx.switch_to_http();
        http.set_response_header("X-Trace", "guard");
        http.set_response_header("X-Other", "1");
        let accepted = http.set_response_header("X-Injected", "a\r\nSet-Cookie: x=1");
        INJECTION_ACCEPTED.store(accepted, Ordering::SeqCst);
        let reserved = ["Content-Length", "Transfer-Encoding", "Connection", "Keep-Alive", "Proxy-Connection", "TE", "Trailer", "Upgrade"];
        let any_reserved = reserved.iter().any(|n| http.set_response_header(n, "5"));
        RESERVED_ACCEPTED.store(any_reserved, Ordering::SeqCst);
        http.set_response_header("Content-Type", "text/html");
        EURO_ACCEPTED.store(http.set_response_header("X-Euro", "\u{20ac}"), Ordering::SeqCst);
        http.set_response_header("X-Latin", "\u{e9}");
        // The last value recorded for a name wins.
        http.set_response_header("X-Replaced", "first");
        http.set_response_header("X-Replaced", "second");
        Ok(true)
    }
}

#[injectable]
pub struct ElapsedInterceptor;

#[async_trait]
impl CorvisInterceptor for ElapsedInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let r = next.handle().await;
        let http = ctx.switch_to_http();
        http.set_response_header("X-Elapsed", "5");
        // Written through to the `Res` of `#[res]` routes without panicking.
        let euro = http.set_response_header("X-Late-Euro", "\u{20ac}");
        assert!(!euro);
        http.set_response_header("X-Late-Latin", "\u{e9}");
        r
    }
}

#[injectable]
pub struct TraceFilter;

#[async_trait]
impl ExceptionFilter for TraceFilter {
    async fn catch(&self, _e: HttpException, _h: &ArgumentsHost) -> Response {
        Response::json(json!({ "by": "filter" })).with_status(418).with_header("X-Trace", "filter")
    }
}

#[controller("h")]
#[use_guards(HeaderGuard)]
pub struct HeadersController;

#[routes]
impl HeadersController {
    #[get("ok")]
    fn ok(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("bytes")]
    fn bytes(&self) -> Response {
        Response::bytes(vec![b'x'; 20], "image/png")
    }

    #[get("route")]
    #[header("X-Trace", "route")]
    fn route(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("handler")]
    fn handler(&self) -> Response {
        Response::json(json!({ "ok": true })).with_header("X-Trace", "handler")
    }

    #[get("err")]
    fn err(&self) -> HttpResult<()> {
        Err(NotFoundException::new("x"))
    }

    #[get("filtered")]
    #[use_filters(TraceFilter)]
    fn filtered(&self) -> HttpResult<()> {
        Err(NotFoundException::new("x"))
    }

    #[get("elapsed")]
    #[use_interceptors(ElapsedInterceptor)]
    fn elapsed(&self) -> Value {
        json!({ "ok": true })
    }

    #[get("res-elapsed/:value")]
    #[use_interceptors(ElapsedInterceptor)]
    fn res_elapsed(&self, #[res(passthrough)] _res: Res) -> Value {
        json!({ "ok": true })
    }

    #[get("pass")]
    fn pass(&self, #[res(passthrough)] _res: Res) -> Value {
        json!({ "ok": true })
    }

    #[get("pass-route")]
    #[header("X-Trace", "route")]
    fn pass_route(&self, #[res(passthrough)] _res: Res) -> Value {
        json!({ "ok": true })
    }

    #[get("handled")]
    fn handled(&self, #[res] res: Res) {
        res.send("ok");
    }

    #[get("handled-mine")]
    fn handled_mine(&self, #[res] res: Res) {
        res.set("X-Trace", "mine");
        res.send("ok");
    }
}

#[controller("m")]
#[use_guards(ManyGuard)]
pub struct ManyController;

#[routes]
impl ManyController {
    #[get("many")]
    fn many(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [HeadersController, ManyController])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let mut app = CorvisFactory::create::<AppModule>().await.unwrap();
    app.silence_route_logs();
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn guard_header_reaches_a_successful_response() {
    let app = app().await;
    let r = request(&app).get("/h/ok").send().await;
    r.expect_status(200).expect_header("x-trace", "guard").expect_header("x-other", "1");
    assert_eq!(r.header("x-replaced"), Some("second"));
}

#[corvis::test]
async fn invalid_values_are_rejected_and_nothing_is_injected() {
    let app = app().await;
    let r = request(&app).get("/h/ok").send().await;
    assert!(!INJECTION_ACCEPTED.load(Ordering::SeqCst));
    assert_eq!(r.header("x-injected"), None);
    assert_eq!(r.header("set-cookie"), None);
}

#[corvis::test]
async fn route_header_wins() {
    let app = app().await;
    request(&app).get("/h/route").send().await.expect_status(200).expect_header("x-trace", "route");
}

#[corvis::test]
async fn handler_header_wins() {
    let app = app().await;
    request(&app).get("/h/handler").send().await.expect_status(200).expect_header("x-trace", "handler");
}

#[corvis::test]
async fn headers_are_kept_on_an_error_without_filter() {
    let app = app().await;
    request(&app).get("/h/err").send().await.expect_status(404).expect_header("x-trace", "guard");
}

#[corvis::test]
async fn filter_headers_win_and_the_rest_are_kept() {
    let app = app().await;
    let r = request(&app).get("/h/filtered").send().await;
    r.expect_status(418).expect_header("x-trace", "filter").expect_header("x-other", "1");
}

#[corvis::test]
async fn interceptor_can_set_a_header_after_the_handler() {
    let app = app().await;
    let r = request(&app).get("/h/elapsed").send().await;
    r.expect_status(200).expect_header("x-elapsed", "5").expect_header("x-trace", "guard");
}

#[corvis::test]
async fn res_passthrough_receives_the_headers() {
    let app = app().await;
    request(&app).get("/h/pass").send().await.expect_status(200).expect_header("x-trace", "guard");
    request(&app).get("/h/pass-route").send().await.expect_header("x-trace", "route");
}

#[corvis::test]
async fn res_handled_receives_the_headers_and_the_handler_wins() {
    let app = app().await;
    let r = request(&app).get("/h/handled").send().await;
    r.expect_status(200).expect_header("x-trace", "guard").expect_header("x-other", "1");
    request(&app).get("/h/handled-mine").send().await.expect_header("x-trace", "mine");
}

#[corvis::test]
async fn framing_headers_are_refused() {
    let app = app().await;
    let r = request(&app).get("/h/bytes").send().await;
    r.expect_status(200);
    assert!(!RESERVED_ACCEPTED.load(Ordering::SeqCst));
    assert_ne!(r.header("content-length"), Some("5"));
    assert_eq!(r.header("transfer-encoding"), None);
    assert_eq!(r.header("connection"), None);
    assert_eq!(r.body.len(), 20);
}

#[corvis::test]
async fn an_enhancer_content_type_does_not_retype_a_body() {
    let app = app().await;
    request(&app).get("/h/bytes").send().await.expect_header("content-type", "image/png");
    request(&app).get("/h/ok").send().await.expect_header("content-type", "application/json; charset=utf-8");
}

#[corvis::test]
async fn non_latin1_values_return_false_and_latin1_values_reach_res_routes() {
    let app = app().await;
    let r = request(&app).get("/h/handled").send().await;
    r.expect_status(200);
    assert!(!EURO_ACCEPTED.load(Ordering::SeqCst));
    assert_eq!(r.headers.get("x-latin").unwrap().as_bytes(), "\u{e9}".as_bytes());
    assert!(r.header("x-euro").is_none());
    let r = request(&app).get("/h/pass").send().await;
    assert_eq!(r.headers.get("x-latin").unwrap().as_bytes(), "\u{e9}".as_bytes());
}

#[corvis::test]
async fn a_non_latin1_value_set_while_a_res_exists_does_not_panic() {
    let app = app().await;
    let r = request(&app).get("/h/res-elapsed/%E2%82%AC").send().await;
    r.expect_status(200);
    assert!(r.header("x-late-euro").is_none());
    assert_eq!(r.headers.get("x-late-latin").unwrap().as_bytes(), "\u{e9}".as_bytes());
}

#[corvis::test]
async fn at_most_128_distinct_names_are_recorded() {
    let app = app().await;
    let r = request(&app).get("/m/many").send().await;
    r.expect_status(200);
    assert!(CAP_FIRST_128_ACCEPTED.load(Ordering::SeqCst));
    assert!(!CAP_129TH_ACCEPTED.load(Ordering::SeqCst));
    assert!(CAP_REPLACE_ACCEPTED.load(Ordering::SeqCst));
    assert_eq!(r.header("x-many-0"), Some("replaced"));
    assert!(r.header("x-many-128").is_none());
}

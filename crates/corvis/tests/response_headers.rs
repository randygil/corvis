//! Response headers set by enhancers through `ctx.switch_to_http().set_response_header`, and how they are merged
//! into the response of every kind of route.

use std::sync::atomic::{AtomicBool, Ordering};

use corvis::Res;
use corvis::prelude::*;
use corvis::testing::request;

static INJECTION_ACCEPTED: AtomicBool = AtomicBool::new(true);

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
        ctx.switch_to_http().set_response_header("X-Elapsed", "5");
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

#[module(controllers = [HeadersController])]
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

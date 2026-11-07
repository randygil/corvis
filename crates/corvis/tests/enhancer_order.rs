//! Global enhancers registered as providers (`APP_INTERCEPTOR`, ...) are applied while
//! the application is created, before the `use_global_interceptors(...)` of the bootstrap code,
//! so they wrap the ones registered with `use_global_*`.

use std::sync::Mutex;

use corvis::prelude::*;
use corvis::testing::request;

static LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

#[injectable]
pub struct ProviderInterceptor;

#[async_trait]
impl CorvisInterceptor for ProviderInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        LOG.lock().unwrap().push("provider:before");
        let r = next.handle().await;
        LOG.lock().unwrap().push("provider:after");
        r
    }
}

pub struct BootstrapInterceptor;

#[async_trait]
impl CorvisInterceptor for BootstrapInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        LOG.lock().unwrap().push("bootstrap:before");
        let r = next.handle().await;
        LOG.lock().unwrap().push("bootstrap:after");
        r
    }
}

#[controller("x")]
pub struct XController;

#[routes]
impl XController {
    #[get]
    fn get(&self) -> Value {
        LOG.lock().unwrap().push("handler");
        json!({ "ok": true })
    }
}

#[module(controllers = [XController], providers = [Provider::app_interceptor::<ProviderInterceptor>()])]
pub struct AppModule;

#[corvis::test]
async fn app_interceptor_providers_wrap_use_global_interceptors() {
    let module = corvis::testing::Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_interceptors(BootstrapInterceptor);
    app.init().await.unwrap();
    request(&app).get("/x").send().await.expect_status(200);

    assert_eq!(
        *LOG.lock().unwrap(),
        ["provider:before", "bootstrap:before", "handler", "bootstrap:after", "provider:after"]
    );
}

// `res.statusCode` already holds the route status while interceptors run (it is set
// before guards), e.g. for an interceptor that caches `{ status, body }`.
static SEEN: Mutex<Vec<u16>> = Mutex::new(Vec::new());

pub struct StatusSpy;

#[async_trait]
impl CorvisInterceptor for StatusSpy {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        SEEN.lock().unwrap().push(ctx.switch_to_http().status_code());
        next.handle().await
    }
}

#[controller("s")]
pub struct StatusController;

#[routes]
impl StatusController {
    #[post]
    fn create(&self) -> Value {
        json!({})
    }
    #[post("accepted")]
    #[http_code(202)]
    fn accepted(&self) -> Value {
        json!({})
    }
    #[get]
    fn read(&self) -> Value {
        json!({})
    }
}

#[module(controllers = [StatusController])]
pub struct StatusModule;

#[corvis::test]
async fn interceptors_see_the_route_status() {
    let module = corvis::testing::Test::create_testing_module().import::<StatusModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_interceptors(StatusSpy);
    app.init().await.unwrap();
    request(&app).post("/s").send().await.expect_status(201);
    request(&app).post("/s/accepted").send().await.expect_status(202);
    request(&app).get("/s").send().await.expect_status(200);
    assert_eq!(*SEEN.lock().unwrap(), [201, 202, 200]);
}

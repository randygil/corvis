use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::prelude::*;
use corvis::testing::{Test, request};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- DTOs

#[derive(Debug, Clone, Serialize, Deserialize, corvis::Validate)]
#[serde(rename_all = "camelCase")]
pub struct CreateCatDto {
    #[is_string]
    #[is_not_empty]
    pub name: String,
    #[is_int]
    #[min(0)]
    #[max(30)]
    pub age: i32,
    #[is_optional]
    #[is_email]
    pub owner_email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, corvis::Validate)]
pub struct PaginationQuery {
    #[is_optional]
    #[is_int]
    #[min(1)]
    pub page: Option<u32>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cat {
    pub id: u32,
    pub name: String,
    pub age: i32,
}

// ---------------------------------------------------------------- Providers

pub static INIT_CALLS: AtomicUsize = AtomicUsize::new(0);
pub static DESTROY_CALLS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct CatsService {
    #[default]
    cats: Mutex<Vec<Cat>>,
    #[inject("PREFIX")]
    prefix: String,
}

impl CatsService {
    pub fn create(&self, dto: CreateCatDto) -> Cat {
        let mut cats = self.cats.lock().unwrap();
        let cat = Cat { id: cats.len() as u32 + 1, name: format!("{}{}", self.prefix, dto.name), age: dto.age };
        cats.push(cat.clone());
        cat
    }
    pub fn find_all(&self) -> Vec<Cat> {
        self.cats.lock().unwrap().clone()
    }
    pub fn find_one(&self, id: u32) -> Option<Cat> {
        self.cats.lock().unwrap().iter().find(|c| c.id == id).cloned()
    }
}

#[async_trait]
impl OnModuleInit for CatsService {
    async fn on_module_init(&self) -> corvis::Result<()> {
        INIT_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait]
impl OnModuleDestroy for CatsService {
    async fn on_module_destroy(&self) -> corvis::Result<()> {
        DESTROY_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------- Enhancers

pub const ROLES_KEY: &str = "roles";

#[injectable]
pub struct RolesGuard {
    reflector: Arc<Reflector>,
}

#[async_trait]
impl CanActivate for RolesGuard {
    async fn can_activate(&self, ctx: &ExecutionContext) -> Result<bool, HttpException> {
        let roles: Option<Vec<String>> = self.reflector.get_all_and_override(ROLES_KEY, &[ctx.get_handler(), ctx.get_class()]);
        let Some(roles) = roles else { return Ok(true) };
        let role = ctx.switch_to_http().get_request().header("x-role").unwrap_or("").to_string();
        Ok(roles.contains(&role))
    }
}

#[injectable]
pub struct TransformInterceptor;

#[async_trait]
impl CorvisInterceptor for TransformInterceptor {
    async fn intercept(&self, _ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        Ok(next.handle().await?.map_data(|data| json!({ "data": data })))
    }
}

#[injectable]
pub struct HttpExceptionFilter;

#[async_trait]
impl ExceptionFilter for HttpExceptionFilter {
    fn catches(&self, e: &HttpException) -> bool {
        e.get_status() == 404
    }
    async fn catch(&self, exception: HttpException, host: &ArgumentsHost) -> Response {
        Response::json(json!({
            "statusCode": exception.get_status(),
            "path": host.switch_to_http().get_request().path(),
            "custom": true,
        }))
    }
}

#[injectable]
pub struct HeaderMiddleware;

#[async_trait]
impl CorvisMiddleware for HeaderMiddleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        let mut res = next.run(req).await?;
        res.set_header("x-middleware", "yes");
        Ok(res)
    }
}

pub struct CurrentRole(pub String);

#[async_trait]
impl FromContext for CurrentRole {
    async fn from_context(ctx: &ExecutionContext) -> Result<Self, HttpException> {
        Ok(CurrentRole(ctx.request().header("x-role").unwrap_or("anonymous").to_string()))
    }
}

// ---------------------------------------------------------------- Controller

#[controller("cats")]
#[use_guards(RolesGuard)]
pub struct CatsController {
    cats_service: Arc<CatsService>,
}

#[routes]
impl CatsController {
    #[post]
    async fn create(&self, #[body] dto: CreateCatDto) -> Cat {
        self.cats_service.create(dto)
    }

    #[get]
    async fn find_all(&self, #[query] q: PaginationQuery) -> Value {
        json!({ "items": self.cats_service.find_all(), "page": q.page.unwrap_or(1) })
    }

    #[get(":id")]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<Cat> {
        self.cats_service.find_one(id).ok_or_else(|| NotFoundException::new(format!("Cat #{id} not found")))
    }

    #[get("admin/secret")]
    #[set_metadata(ROLES_KEY, ["admin"])]
    fn secret(&self) -> &'static str {
        "top secret"
    }

    #[get("wrapped/data")]
    #[use_interceptors(TransformInterceptor)]
    async fn wrapped(&self) -> Value {
        json!([1, 2, 3])
    }

    #[delete(":id")]
    #[http_code(204)]
    async fn remove(&self, #[param("id")] _id: u32) {}

    #[get("whoami/role")]
    #[header("Cache-Control", "none")]
    async fn whoami(&self, role: CurrentRole, #[headers("user-agent")] ua: Option<String>) -> String {
        format!("{}:{}", role.0, ua.unwrap_or_default())
    }

    #[get("filtered/missing")]
    #[use_filters(HttpExceptionFilter)]
    async fn filtered(&self) -> HttpResult<()> {
        Err(NotFoundException.into())
    }

    #[get("boom/io")]
    async fn boom(&self) -> HttpResult<String> {
        let r: Result<String, std::io::Error> = Err(std::io::Error::other("disk on fire"));
        Ok(r?)
    }

    #[get("legacy/redirect")]
    #[redirect("https://example.com", 301)]
    fn legacy(&self) {}
}

#[module(controllers = [CatsController], providers = [CatsService, Provider::use_value("PREFIX", "Sir ".to_string())], exports = [CatsService])]
pub struct CatsModule;

impl CorvisModule for CatsModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer) {
        consumer.apply::<HeaderMiddleware>().for_routes(RouteSpec::controller::<CatsController>());
    }
}

// A second module consuming the exported service.
#[injectable]
pub struct StatsService {
    cats: Arc<CatsService>,
}

#[controller("stats")]
pub struct StatsController {
    stats: Arc<StatsService>,
}

#[routes]
impl StatsController {
    #[get]
    async fn count(&self) -> Value {
        json!({ "count": self.stats.cats.find_all().len() })
    }
}

#[module(imports = [CatsModule], controllers = [StatsController], providers = [StatsService])]
pub struct AppModule;

async fn app() -> CorvisApplication {
    let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_pipes(ValidationPipe::new().whitelist(true));
    app.init().await.unwrap();
    app
}

#[corvis::test]
async fn crud_validation_and_errors() {
    let app = app().await;
    let client = request(&app);

    let res = client.post("/cats").json(&json!({ "name": "Tom", "age": 3, "extra": 1 })).send().await;
    res.expect_status(201);
    assert_eq!(res.json_value(), json!({ "id": 1, "name": "Sir Tom", "age": 3 }));
    assert_eq!(res.header("x-middleware"), Some("yes"));

    let res = client.post("/cats").json(&json!({ "name": "", "age": 99, "ownerEmail": "nope" })).send().await;
    res.expect_status(400);
    assert_eq!(
        res.json_value(),
        json!({
            "message": ["name should not be empty", "age must not be greater than 30", "ownerEmail must be an email"],
            "error": "Bad Request",
            "statusCode": 400
        })
    );

    client.get("/cats/1").send().await.expect_status(200).expect_json(json!({ "id": 1, "name": "Sir Tom", "age": 3 }));
    client
        .get("/cats/42")
        .send()
        .await
        .expect_status(404)
        .expect_json(json!({ "message": "Cat #42 not found", "error": "Not Found", "statusCode": 404 }));
    client
        .get("/cats/abc")
        .send()
        .await
        .expect_status(400)
        .expect_json(json!({ "message": "Validation failed (numeric string is expected)", "error": "Bad Request", "statusCode": 400 }));

    let res = client.get("/cats?page=2").send().await;
    res.expect_status(200);
    assert_eq!(res.json_value()["page"], json!(2));

    client.get("/cats?page=0").send().await.expect_status(400);

    client.get("/stats").send().await.expect_status(200).expect_json(json!({ "count": 1 }));

    client
        .get("/nope")
        .send()
        .await
        .expect_status(404)
        .expect_json(json!({ "message": "Cannot GET /nope", "error": "Not Found", "statusCode": 404 }));

    client
        .get("/cats/boom/io")
        .send()
        .await
        .expect_status(500)
        .expect_json(json!({ "statusCode": 500, "message": "Internal server error" }));

    let bad = client.post("/cats").header("content-type", "application/json").body("{bad").send().await;
    bad.expect_status(400);
}

#[corvis::test]
async fn guards_interceptors_filters_and_params() {
    let app = app().await;
    let client = request(&app);

    client
        .get("/cats/admin/secret")
        .send()
        .await
        .expect_status(403)
        .expect_json(json!({ "message": "Forbidden resource", "error": "Forbidden", "statusCode": 403 }));
    client.get("/cats/admin/secret").header("x-role", "admin").send().await.expect_status(200).expect_text("top secret");

    client.get("/cats/wrapped/data").send().await.expect_status(200).expect_json(json!({ "data": [1, 2, 3] }));
    client.delete("/cats/1").send().await.expect_status(204);

    let res = client.get("/cats/whoami/role").header("x-role", "dev").header("user-agent", "jest").send().await;
    res.expect_status(200).expect_text("dev:jest").expect_header("cache-control", "none");

    client
        .get("/cats/filtered/missing")
        .send()
        .await
        .expect_status(404)
        .expect_json(json!({ "statusCode": 404, "path": "/cats/filtered/missing", "custom": true }));

    let res = client.get("/cats/legacy/redirect").send().await;
    res.expect_status(301).expect_header("location", "https://example.com");
}

#[corvis::test]
async fn lifecycle_and_overrides() {
    let before = INIT_CALLS.load(Ordering::SeqCst);
    let mut app = app().await;
    assert!(INIT_CALLS.load(Ordering::SeqCst) >= before + 1);
    let svc = app.get::<CatsService>().unwrap();
    assert_eq!(svc.find_all().len(), 0);
    let d = DESTROY_CALLS.load(Ordering::SeqCst);
    app.close().await.unwrap();
    assert_eq!(DESTROY_CALLS.load(Ordering::SeqCst), d + 1);

    // Override a provider and a guard.
    struct AllowAll;
    #[async_trait]
    impl CanActivate for AllowAll {
        async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
            Ok(true)
        }
    }
    let module = Test::create_testing_module()
        .import::<AppModule>()
        .override_provider_token("PREFIX")
        .use_value("Lord ".to_string())
        .override_guard::<RolesGuard>(AllowAll)
        .compile()
        .await
        .unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let client = request(&app);
    client.get("/cats/admin/secret").send().await.expect_status(200);
    let res = client.post("/cats").json(&json!({ "name": "Felix", "age": 1 })).send().await;
    assert_eq!(res.json_value()["name"], json!("Lord Felix"));
}

#[corvis::test]
async fn missing_dependency_message() {
    #[injectable]
    pub struct NeedsCats {
        _cats: Arc<CatsService>,
    }
    #[module(providers = [NeedsCats])]
    pub struct BrokenModule;

    let err = Test::create_testing_module().import::<BrokenModule>().compile().await.err().unwrap();
    let msg = err.to_string();
    assert!(msg.contains("Corvis can't resolve dependencies of the NeedsCats (?)"), "{msg}");
    assert!(msg.contains("argument CatsService at index [0] is available in the BrokenModule context"), "{msg}");
}

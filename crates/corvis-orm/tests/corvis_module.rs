mod common;

use common::*;
use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis_orm::{OrmError, OrmExceptionFilter, OrmModule, OrmModuleOptions, OrmResultExt, OrmService};

#[injectable]
pub struct UsersService {
    db: Arc<OrmService>,
}

impl UsersService {
    pub async fn create(&self, data: Value) -> Result<User, OrmError> {
        self.db.user().create(json!({ "data": data })).await
    }

    pub async fn find_one(&self, id: i64) -> Result<User, OrmError> {
        self.db.user().find_unique_or_throw(json!({ "where": { "id": id }, "include": { "posts": true } })).await
    }
}

#[controller("users")]
pub struct UsersController {
    users: Arc<UsersService>,
}

#[routes]
impl UsersController {
    /// `HttpResult` + `.http()`: ORM errors become 409/404/400 without a filter.
    #[post]
    async fn create(&self, #[body] body: Value) -> HttpResult<User> {
        self.users.create(body).await.http()
    }

    /// `corvis::Result` (anyhow): the error reaches `OrmExceptionFilter`.
    #[get(":id")]
    async fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> corvis::Result<User> {
        Ok(self.users.find_one(id).await?)
    }

    /// `HttpResult` + `?` (blanket conversion to 500) – also rescued by the filter.
    #[get]
    async fn search(&self, #[query("field")] field: Option<String>) -> HttpResult<Vec<User>> {
        let field = field.unwrap_or_else(|| "email".into());
        let mut w = serde_json::Map::new();
        w.insert(field, json!({ "contains": "@" }));
        Ok(self.users.db.user().find_many(json!({ "where": w })).await?)
    }
}

#[module(
    imports = [OrmModule::for_root(OrmModuleOptions::new("sqlite::memory:").push::<(User, Post, Profile)>())],
    controllers = [UsersController],
    providers = [UsersService],
)]
pub struct UsersModule;

#[tokio::test]
async fn orm_module_end_to_end() {
    let module = Test::create_testing_module().import::<UsersModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.use_global_filters(OrmExceptionFilter::new());
    app.init().await.unwrap();

    let db = app.get::<OrmService>().unwrap();
    assert!(db.is_connected().await, "connected in on_module_init");

    let res = request(&app).post("/users").json(&json!({ "email": "ana@corvis.dev", "name": "Ana" })).send().await;
    res.expect_status(201);
    assert_eq!(res.json_value()["email"], "ana@corvis.dev");

    let res = request(&app).post("/users").json(&json!({ "email": "ana@corvis.dev" })).send().await;
    res.expect_status(409);
    assert_eq!(
        res.json_value(),
        json!({ "message": "Unique constraint failed on the fields: (`email`)", "error": "Conflict", "statusCode": 409 })
    );

    let res = request(&app).post("/users").json(&json!({ "emial": "x" })).send().await;
    res.expect_status(400);
    assert!(res.json_value()["message"].as_str().unwrap().contains("Did you mean `email`?"));

    let res = request(&app).get("/users/1").send().await;
    res.expect_status(200);
    assert_eq!(res.json_value()["posts"], json!([]));

    let res = request(&app).get("/users/99").send().await;
    res.expect_status(404);
    assert_eq!(res.json_value()["error"], "Not Found");

    request(&app).get("/users").send().await.expect_status(200);
    let res = request(&app).get("/users?field=nope").send().await;
    res.expect_status(400);

    app.close().await.unwrap();
    assert!(!db.is_connected().await, "disconnected in on_module_destroy");
}

#[tokio::test]
async fn for_root_async_reads_config() {
    let config = ConfigModule::for_root(ConfigModuleOptions {
        is_global: true,
        ignore_env_file: true,
        ignore_env_vars: true,
        load: vec![corvis::config::ConfigFactory::from(|| json!({ "DATABASE_URL": "sqlite::memory:" }))],
        ..Default::default()
    });
    let mut module =
        Test::create_testing_module().import_dynamic(config).import_dynamic(OrmModule::for_root_from_config(true)).compile().await.unwrap();
    module.init().await.unwrap();
    let db = module.get::<OrmService>().unwrap();
    db.db_push::<(User, Post, Profile)>().await.unwrap();
    assert_eq!(db.user().count(()).await.unwrap(), 0);
    assert!(db.is_connected().await);
    module.close().await.unwrap();
}

// `corvis::prelude::*` (`ResultHttpExt`) and `OrmResultExt` both define `.http()`; they apply to disjoint error types.
#[derive(Debug)]
struct DomainError;
impl std::fmt::Display for DomainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "domain failure")
    }
}
impl std::error::Error for DomainError {}
impl HttpError for DomainError {
    fn status(&self) -> corvis::StatusCode {
        corvis::StatusCode::CONFLICT
    }
}

#[test]
fn both_http_extensions_resolve_without_ambiguity() {
    let orm: Result<u8, OrmError> = Err(OrmError::validation("bad"));
    assert_eq!(orm.http().unwrap_err().get_status(), 400);
    let domain: Result<u8, DomainError> = Err(DomainError);
    assert_eq!(domain.http().unwrap_err().get_status(), 409);
}

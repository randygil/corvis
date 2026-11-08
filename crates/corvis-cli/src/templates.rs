//! File templates. Placeholders: `__Name__` (PascalCase), `__name__` (snake_case),
//! `__route__` (kebab-case), `__Entity__` / `__entity__` (singular forms).

use crate::names::Names;

pub fn render(template: &str, n: &Names) -> String {
    let s = n.singular();
    template
        .replace("__Name__", &n.pascal)
        .replace("__name__", &n.snake)
        .replace("__route__", &n.kebab)
        .replace("__Entity__", &s.pascal)
        .replace("__entity__", &s.snake)
}

// ------------------------------------------------------------------ new project

pub const CARGO_TOML: &str = r#"[package]
name = "__route__"
version = "0.1.0"
edition = "2024"

[dependencies]
corvis = { __CORVIS_DEP__ }
serde = { version = "1", features = ["derive"] }
"#;

pub const GITIGNORE: &str = "/target\n.env\n";

pub const ENV: &str = "PORT=3000\n";

pub const MAIN: &str = r#"mod app_controller;
mod app_module;
mod app_service;

use corvis::prelude::*;

use app_module::AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.require_route_posture();
    let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
    app.listen(port).await
}
"#;

pub const APP_MODULE: &str = r#"use corvis::prelude::*;

use crate::app_controller::AppController;
use crate::app_service::AppService;

#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    controllers = [AppController],
    providers = [AppService],
)]
pub struct AppModule;
"#;

pub const APP_CONTROLLER: &str = r#"use corvis::prelude::*;

use crate::app_service::AppService;

#[controller]
pub struct AppController {
    app_service: Arc<AppService>,
}

#[routes]
impl AppController {
    // Routes need a guard or #[public]: main enforces route posture (see require_route_posture).
    #[get]
    #[public]
    fn get_hello(&self) -> String {
        self.app_service.get_hello()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvis::testing::{Test, request};

    #[corvis::test]
    async fn should_return_hello_world() {
        let module = Test::create_testing_module()
            .controller::<AppController>()
            .provider::<AppService>()
            .compile()
            .await
            .unwrap();
        let controller = module.get::<AppController>().unwrap();
        assert_eq!(controller.get_hello(), "Hello World!");

        let mut app = module.create_corvis_application();
        app.init().await.unwrap();
        request(&app).get("/").send().await.expect_status(200).expect_text("Hello World!");
    }
}
"#;

pub const APP_SERVICE: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct AppService;

impl AppService {
    pub fn get_hello(&self) -> String {
        "Hello World!".to_string()
    }
}
"#;

// ------------------------------------------------------------------ schematics

pub const MODULE: &str = r#"use corvis::prelude::*;

#[module]
pub struct __Name__Module;
"#;

pub const CONTROLLER: &str = r#"use corvis::prelude::*;

#[controller("__route__")]
pub struct __Name__Controller {}

#[routes]
impl __Name__Controller {
    #[get]
    fn find_all(&self) -> &'static str {
        "This action returns all __name__"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvis::testing::Test;

    #[corvis::test]
    async fn should_be_defined() {
        let module = Test::create_testing_module().controller::<__Name__Controller>().compile().await.unwrap();
        assert!(module.get::<__Name__Controller>().is_ok());
    }
}
"#;

pub const SERVICE: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Service;

impl __Name__Service {}

#[cfg(test)]
mod tests {
    use super::*;
    use corvis::testing::Test;

    #[corvis::test]
    async fn should_be_defined() {
        let module = Test::create_testing_module().provider::<__Name__Service>().compile().await.unwrap();
        assert!(module.get::<__Name__Service>().is_ok());
    }
}
"#;

pub const GUARD: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Guard;

#[async_trait]
impl CanActivate for __Name__Guard {
    async fn can_activate(&self, _context: &ExecutionContext) -> Result<bool, HttpException> {
        Ok(true)
    }
}
"#;

pub const INTERCEPTOR: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Interceptor;

#[async_trait]
impl CorvisInterceptor for __Name__Interceptor {
    async fn intercept(&self, _context: &ExecutionContext, next: CallHandler) -> HandlerResult {
        next.handle().await
    }
}
"#;

pub const PIPE: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Pipe;

#[async_trait]
impl PipeTransform for __Name__Pipe {
    async fn transform(&self, value: Value, _metadata: &ArgumentMetadata) -> Result<Value, HttpException> {
        Ok(value)
    }
}
"#;

pub const FILTER: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Filter;

#[async_trait]
impl ExceptionFilter for __Name__Filter {
    async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
        BaseExceptionFilter::handle(&exception)
    }
}
"#;

pub const MIDDLEWARE: &str = r#"use corvis::prelude::*;

#[injectable]
pub struct __Name__Middleware;

#[async_trait]
impl CorvisMiddleware for __Name__Middleware {
    async fn use_(&self, req: Request, next: Next) -> HandlerResult {
        next.run(req).await
    }
}
"#;

pub const GATEWAY: &str = r#"use corvis::prelude::*;
use corvis::{WsServer, websocket_gateway};

#[websocket_gateway(path = "/__route__")]
pub struct __Name__Gateway {
    #[websocket_server]
    server: WsServer,
}

#[handlers]
impl __Name__Gateway {
    #[subscribe_message("message")]
    fn handle_message(&self, #[message_body] _payload: Value) -> &'static str {
        "Hello world!"
    }
}
"#;

// ------------------------------------------------------------------ resource (CRUD)

pub const RES_MODULE: &str = r#"use corvis::prelude::*;

use super::__name___controller::__Name__Controller;
use super::__name___service::__Name__Service;

#[module(controllers = [__Name__Controller], providers = [__Name__Service])]
pub struct __Name__Module;
"#;

pub const RES_CONTROLLER: &str = r#"use corvis::prelude::*;

use super::__name___service::__Name__Service;
use super::dto::{Create__Entity__Dto, Update__Entity__Dto};
use super::entities::__Entity__;

#[controller("__route__")]
pub struct __Name__Controller {
    __name___service: Arc<__Name__Service>,
}

#[routes]
impl __Name__Controller {
    #[post]
    fn create(&self, #[body] dto: Create__Entity__Dto) -> __Entity__ {
        self.__name___service.create(dto)
    }

    #[get]
    fn find_all(&self) -> Vec<__Entity__> {
        self.__name___service.find_all()
    }

    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<__Entity__> {
        self.__name___service.find_one(id)
    }

    #[patch(":id")]
    fn update(&self, #[param("id", ParseIntPipe)] id: u32, #[body] dto: Update__Entity__Dto) -> HttpResult<__Entity__> {
        self.__name___service.update(id, dto)
    }

    #[delete(":id")]
    fn remove(&self, #[param("id", ParseIntPipe)] id: u32) -> HttpResult<()> {
        self.__name___service.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvis::testing::{Test, request};

    #[corvis::test]
    async fn crud() {
        let module = Test::create_testing_module()
            .controller::<__Name__Controller>()
            .provider::<__Name__Service>()
            .compile()
            .await
            .unwrap();
        let mut app = module.create_corvis_application();
        app.use_global_pipes(ValidationPipe::new());
        app.init().await.unwrap();
        let client = request(&app);
        client.post("/__route__").json(&json!({ "name": "test" })).send().await.expect_status(201);
        client.get("/__route__/1").send().await.expect_status(200);
        client.get("/__route__/99").send().await.expect_status(404);
    }
}
"#;

pub const RES_SERVICE: &str = r#"use std::sync::RwLock;

use corvis::prelude::*;

use super::dto::{Create__Entity__Dto, Update__Entity__Dto};
use super::entities::__Entity__;

#[injectable]
pub struct __Name__Service {
    #[default]
    items: RwLock<Vec<__Entity__>>,
}

impl __Name__Service {
    pub fn create(&self, dto: Create__Entity__Dto) -> __Entity__ {
        let mut items = self.items.write().unwrap();
        let id = items.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let item = __Entity__ { id, name: dto.name };
        items.push(item.clone());
        item
    }

    pub fn find_all(&self) -> Vec<__Entity__> {
        self.items.read().unwrap().clone()
    }

    pub fn find_one(&self, id: u32) -> HttpResult<__Entity__> {
        self.items
            .read()
            .unwrap()
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or_else(|| NotFoundException::new(format!("__Entity__ #{id} not found")))
    }

    pub fn update(&self, id: u32, dto: Update__Entity__Dto) -> HttpResult<__Entity__> {
        let mut items = self.items.write().unwrap();
        let item = items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| NotFoundException::new(format!("__Entity__ #{id} not found")))?;
        if let Some(name) = dto.name {
            item.name = name;
        }
        Ok(item.clone())
    }

    pub fn remove(&self, id: u32) -> HttpResult<()> {
        let mut items = self.items.write().unwrap();
        let len = items.len();
        items.retain(|i| i.id != id);
        if items.len() == len {
            return Err(NotFoundException::new(format!("__Entity__ #{id} not found")));
        }
        Ok(())
    }
}
"#;

pub const RES_DTO: &str = r#"use corvis::Validate;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Create__Entity__Dto {
    #[is_string]
    #[is_not_empty]
    pub name: String,
}

/// `PartialType(Create__Entity__Dto)`.
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Update__Entity__Dto {
    #[is_optional]
    #[is_string]
    pub name: Option<String>,
}
"#;

pub const RES_ENTITY: &str = r#"use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct __Entity__ {
    pub id: u32,
    pub name: String,
}
"#;

pub const RES_MOD: &str = r#"pub mod dto;
pub mod entities;
pub mod __name___controller;
pub mod __name___module;
pub mod __name___service;

pub use __name___module::__Name__Module;
"#;

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn a_new_project_enforces_route_posture_and_marks_its_route_public() {
        assert!(MAIN.contains("let mut app = CorvisFactory::create::<AppModule>().await?;"));
        let enforce = MAIN.find("app.require_route_posture();").expect("main enables the check");
        assert!(enforce < MAIN.find("app.listen(port)").unwrap(), "the check is enabled before listen");
        assert!(APP_CONTROLLER.contains("    #[get]
    #[public]
    fn get_hello"), "get_hello is public");
    }
}

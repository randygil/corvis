//! App mínima de Corvis para el benchmark de throughput contra Node.js.
//! Expone exactamente los mismos endpoints que `bench/node-app`.

use corvis::Validate;
use corvis::prelude::*;
use serde::{Deserialize, Serialize};

// ---------- Servicio inyectado (como en `bench/node-app`) ----------

#[injectable]
pub struct AppService;

impl AppService {
    pub fn get_hello(&self) -> String {
        "Hello World!".to_string()
    }
}

// ---------- DTO validado (equivalente a class-validator) ----------

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateUserDto {
    #[is_string]
    #[is_not_empty]
    pub name: String,
    #[is_email]
    pub email: String,
    #[is_int]
    #[min(0)]
    pub age: i64,
}

// ---------- Controladores ----------

#[controller]
pub struct AppController {
    app_service: Arc<AppService>,
}

#[routes]
impl AppController {
    #[get]
    fn get_hello(&self) -> String {
        self.app_service.get_hello()
    }

    #[get("json")]
    fn get_json(&self) -> Value {
        json!({ "id": 1, "name": "Corvis", "tags": ["a", "b", "c"], "active": true })
    }
}

#[controller("users")]
pub struct UsersController;

#[routes]
impl UsersController {
    #[get(":id")]
    fn find_one(&self, #[param("id", ParseIntPipe)] id: i64) -> Value {
        json!({ "id": id })
    }

    #[post]
    fn create(&self, #[body] dto: CreateUserDto) -> CreateUserDto {
        dto
    }
}

#[module(controllers = [AppController, UsersController], providers = [AppService])]
pub struct AppModule;

#[corvis::main]
async fn main() -> corvis::Result<()> {
    // Corvis no registra logs por request; aun así silenciamos todo salvo errores
    // para que la salida estándar no interfiera con la medición.
    corvis::logger::set_log_levels(&[corvis::logger::LogLevel::Error, corvis::logger::LogLevel::Fatal]);

    let mut app = CorvisFactory::create::<AppModule>().await?;
    app.use_global_pipes(ValidationPipe::new());

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    // IMPORTANTE: escuchar solo en loopback (nunca 0.0.0.0) para no disparar el firewall de Windows.
    app.listen(format!("127.0.0.1:{port}")).await
}

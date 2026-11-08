use corvis::prelude::*;

use crate::app_service::AppService;

#[controller]
#[public]
pub struct AppController {
    app_service: Arc<AppService>,
    health: Arc<HealthCheckService>,
}

#[routes]
impl AppController {
    #[get]
    fn get_hello(&self) -> String {
        self.app_service.get_hello()
    }

    #[get("health")]
    #[version(VERSION_NEUTRAL)]
    async fn health(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                corvis::MemoryHealthIndicator::check_rss("memory_rss", 512 * 1024 * 1024),
                HealthIndicator::new("app", || async { Ok(json!({ "uptime": "ok" })) }),
            ])
            .await
    }
}

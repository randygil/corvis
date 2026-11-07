use corvis::prelude::*;

use super::entities::CatCreatedEvent;

#[injectable]
pub struct CatCreatedListener {
    #[init(Logger::new("CatCreatedListener"))]
    logger: Logger,
}

#[handlers]
impl CatCreatedListener {
    #[on_event("cat.created")]
    async fn handle(&self, event: CatCreatedEvent) {
        self.logger.log(format!("New cat registered: {} (#{})", event.cat.name, event.cat.id));
    }
}

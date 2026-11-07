use corvis::prelude::*;

/// Task scheduling (`@Cron`, `@Interval`, `@Timeout`).
#[injectable]
pub struct TasksService {
    #[init(Logger::new("TasksService"))]
    logger: Logger,
}

#[handlers]
impl TasksService {
    #[cron(CronExpression::EVERY_MINUTE, name = "heartbeat")]
    async fn heartbeat(&self) {
        self.logger.debug("Called every minute");
    }

    #[timeout(1_000)]
    async fn warmup(&self) {
        self.logger.log("Warm-up finished (1s after bootstrap)");
    }
}

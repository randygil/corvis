//! A standalone application context (`create_application_context`) runs event listeners and
//! scheduled jobs, in the same order as `init()`, and stops them on `close()`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use corvis::prelude::*;
use corvis::schedule::ScheduleModuleOptions;

#[derive(Clone, Debug)]
pub struct Ping;

#[injectable]
pub struct Worker {
    #[default]
    ticks: AtomicUsize,
    #[default]
    received: AtomicUsize,
}

#[handlers]
impl Worker {
    #[interval("tick", 20)]
    async fn tick(&self) {
        self.ticks.fetch_add(1, Ordering::SeqCst);
    }

    #[cron(CronExpression::EVERY_HOUR, name = "hourly")]
    async fn hourly(&self) {}

    #[on_event("order.created")]
    async fn on_created(&self, _e: Ping) {
        self.received.fetch_add(1, Ordering::SeqCst);
    }
}

#[module(
    imports = [EventEmitterModule::for_root(EventEmitterOptions::default()), ScheduleModule::for_root()],
    providers = [Worker],
)]
pub struct WorkerModule;

#[corvis::test]
async fn cron_and_interval_handlers_run_in_a_worker() {
    let mut app = CorvisFactory::create_application_context::<WorkerModule>().await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(app.get::<Worker>().unwrap().ticks.load(Ordering::SeqCst) >= 2);
    let registry = app.get::<SchedulerRegistry>().unwrap();
    assert!(registry.get_cron_job("hourly").is_ok());
    app.close().await.unwrap();
}

#[corvis::test]
async fn event_listeners_receive_events_in_a_worker() {
    let mut app = CorvisFactory::create_application_context::<WorkerModule>().await.unwrap();
    let emitter = app.get::<EventEmitter>().unwrap();
    let results = emitter.emit_async("order.created", Ping).await;
    assert_eq!(results.len(), 1);
    assert_eq!(app.get::<Worker>().unwrap().received.load(Ordering::SeqCst), 1);
    app.close().await.unwrap();
}

#[corvis::test]
async fn init_after_create_application_context_registers_once() {
    let mut app = CorvisFactory::create_application_context::<WorkerModule>().await.unwrap();
    app.init().await.unwrap();
    let emitter = app.get::<EventEmitter>().unwrap();
    let results = emitter.emit_async("order.created", Ping).await;
    assert_eq!(results.len(), 1);
    let worker = app.get::<Worker>().unwrap();
    assert_eq!(worker.received.load(Ordering::SeqCst), 1);
    let before = worker.ticks.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(worker.ticks.load(Ordering::SeqCst) >= before + 2);
    app.close().await.unwrap();
}

#[corvis::test]
async fn no_ticks_after_close() {
    let mut app = CorvisFactory::create_application_context::<WorkerModule>().await.unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    let worker = app.get::<Worker>().unwrap();
    let registry = app.get::<SchedulerRegistry>().unwrap();
    assert!(worker.ticks.load(Ordering::SeqCst) >= 1);
    app.close().await.unwrap();
    let at_close = worker.ticks.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(worker.ticks.load(Ordering::SeqCst), at_close);
    assert!(registry.get_intervals().is_empty());
}

#[injectable]
pub struct Partial {
    #[default]
    ticks: AtomicUsize,
}

#[handlers]
impl Partial {
    #[interval("partial-tick", 20)]
    async fn tick(&self) {
        self.ticks.fetch_add(1, Ordering::SeqCst);
    }

    #[cron(CronExpression::EVERY_HOUR, name = "partial-cron")]
    async fn cron(&self) {}
}

#[module(
    imports = [ScheduleModule::for_root_with(ScheduleModuleOptions { cron_jobs: false, ..Default::default() })],
    providers = [Partial],
)]
pub struct PartialModule;

#[corvis::test]
async fn a_disabled_kind_is_not_scheduled() {
    let mut app = CorvisFactory::create_application_context::<PartialModule>().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(app.get::<Partial>().unwrap().ticks.load(Ordering::SeqCst) >= 2);
    let registry = app.get::<SchedulerRegistry>().unwrap();
    assert!(registry.get_cron_jobs().is_empty());
    app.close().await.unwrap();
}

#[injectable]
pub struct Starter {
    emitter: Arc<EventEmitter>,
}

#[async_trait]
impl OnModuleInit for Starter {
    async fn on_module_init(&self) -> corvis::Result<()> {
        self.emitter.emit_async("ready", Ping).await;
        Ok(())
    }
}

#[injectable]
pub struct ReadyListener {
    #[default]
    seen: Mutex<usize>,
}

#[handlers]
impl ReadyListener {
    #[on_event("ready")]
    async fn on_ready(&self, _e: Ping) {
        *self.seen.lock().unwrap() += 1;
    }
}

#[module(
    imports = [EventEmitterModule::for_root(EventEmitterOptions::default())],
    providers = [Starter, ReadyListener],
)]
pub struct ReadyModule;

#[corvis::test]
async fn an_event_emitted_from_on_module_init_reaches_its_listener() {
    let mut app = CorvisFactory::create_application_context::<ReadyModule>().await.unwrap();
    assert_eq!(*app.get::<ReadyListener>().unwrap().seen.lock().unwrap(), 1);
    app.close().await.unwrap();
}

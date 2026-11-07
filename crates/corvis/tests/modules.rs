use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use corvis::prelude::*;
use corvis::testing::{Test, request};

#[derive(Clone, Debug)]
pub struct OrderCreated {
    pub id: u32,
}

#[injectable]
pub struct OrdersService {
    emitter: Arc<EventEmitter>,
}

#[injectable]
pub struct OrderListener {
    #[default]
    received: Mutex<Vec<u32>>,
}

#[handlers]
impl OrderListener {
    #[on_event("order.created")]
    async fn on_created(&self, e: OrderCreated) {
        self.received.lock().unwrap().push(e.id);
    }

    #[on_event("order.*")]
    fn on_any(&self, e: OrderCreated) -> corvis::Result<()> {
        self.received.lock().unwrap().push(e.id * 100);
        Ok(())
    }
}

pub static TICKS: AtomicUsize = AtomicUsize::new(0);
pub static TIMEOUTS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct TasksService;

#[handlers]
impl TasksService {
    #[interval("ticker", 20)]
    async fn tick(&self) {
        TICKS.fetch_add(1, Ordering::SeqCst);
    }

    #[timeout(10)]
    async fn once(&self) {
        TIMEOUTS.fetch_add(1, Ordering::SeqCst);
    }

    #[cron(CronExpression::EVERY_HOUR, name = "hourly")]
    async fn hourly(&self) {}
}

pub static HITS: AtomicUsize = AtomicUsize::new(0);

#[controller]
#[use_guards(ThrottlerGuard)]
pub struct MiscController {
    health: Arc<HealthCheckService>,
}

#[routes]
impl MiscController {
    #[get("cached")]
    #[use_interceptors(CacheInterceptor)]
    async fn cached(&self) -> Value {
        json!({ "hits": HITS.fetch_add(1, Ordering::SeqCst) + 1 })
    }

    #[get("limited")]
    #[set_metadata(THROTTLE_LIMIT, 2)]
    async fn limited(&self) -> &'static str {
        "ok"
    }

    #[sse("events")]
    fn events(&self) -> Sse {
        Sse::new(futures::stream::iter((1..=2).map(|i| MessageEvent::new(json!({ "n": i })).id(i.to_string()))))
    }

    #[get("health")]
    async fn check(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                HealthIndicator::new("db", || async { Ok(json!({})) }),
                HealthIndicator::new("redis", || async { Err(json!({ "message": "down" })) }),
            ])
            .await
    }
}

use corvis::modules::throttler::THROTTLE_LIMIT;

#[module(
    imports = [
        EventEmitterModule::for_root(EventEmitterOptions { wildcard: true, ..Default::default() }),
        ScheduleModule::for_root(),
        CacheModule::register(CacheOptions::default()),
        ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]),
        TerminusModule,
        ServeStaticModule::for_root(vec![ServeStaticOptions::new(env!("CARGO_MANIFEST_DIR")).serve_root("/static")]),
    ],
    controllers = [MiscController],
    providers = [OrdersService, OrderListener, TasksService],
)]
pub struct AppModule;

#[corvis::test]
async fn events_and_schedule() {
    let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();

    let orders = app.get::<OrdersService>().unwrap();
    let results = orders.emitter.emit_async("order.created", OrderCreated { id: 7 }).await;
    assert_eq!(results.len(), 2);
    let listener = app.get::<OrderListener>().unwrap();
    let mut got = listener.received.lock().unwrap().clone();
    got.sort();
    assert_eq!(got, vec![7, 700]);
    assert!(!orders.emitter.emit("unknown.event.deep", OrderCreated { id: 1 }));

    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(TICKS.load(Ordering::SeqCst) >= 2);
    assert_eq!(TIMEOUTS.load(Ordering::SeqCst), 1);
    let registry = app.get::<SchedulerRegistry>().unwrap();
    assert!(registry.get_cron_jobs().contains_key("hourly"));
    assert!(registry.does_exist("interval", "ticker"));
    app.close().await.unwrap();
    assert!(registry.get_intervals().is_empty());
}

#[corvis::test]
async fn cache_throttle_sse_health_static() {
    let module = Test::create_testing_module().import::<AppModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let c = request(&app);

    let a = c.get("/cached").send().await.json_value();
    let b = c.get("/cached").send().await.json_value();
    assert_eq!(a, b);

    c.get("/limited").send().await.expect_status(200);
    c.get("/limited").send().await.expect_status(200);
    c.get("/limited")
        .send()
        .await
        .expect_status(429)
        .expect_json(json!({ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }));

    let sse = c.get("/events").send().await;
    sse.expect_status(200).expect_header("content-type", "text/event-stream");
    assert_eq!(sse.text(), "id: 1\ndata: {\"n\":1}\n\nid: 2\ndata: {\"n\":2}\n\n");

    let h = c.get("/health").send().await;
    h.expect_status(503);
    assert_eq!(h.json_value()["status"], json!("error"));
    assert_eq!(h.json_value()["info"]["db"]["status"], json!("up"));

    let f = c.get("/static/Cargo.toml").send().await;
    f.expect_status(200);
    assert!(f.text().contains("name = \"corvis\""));
    c.get("/static/../Cargo.toml").send().await.expect_status(404);
}

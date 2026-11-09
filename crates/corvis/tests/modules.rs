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

// ---------------------------------------------------------------- async registration

mod async_registration {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use corvis::ResolveCtx;
    use corvis::modules::throttler::ThrottlerStorage;
    use corvis::prelude::*;
    use corvis::testing::{Test, request};

    /// Writes an env file to a fresh temp directory (tests never touch the process environment).
    fn env_file(contents: &str) -> String {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir: PathBuf =
            std::env::temp_dir().join(format!("corvis-async-reg-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn file_options(path: String) -> ConfigModuleOptions {
        ConfigModuleOptions { env_file_path: vec![path], ignore_env_vars: true, ..Default::default() }
    }

    /// A global module that exports the path of an env file.
    #[module(global)]
    pub struct PathHolder;

    impl PathHolder {
        fn with(path: String) -> DynamicModule {
            DynamicModule::new::<PathHolder>().provider(Provider::use_value("ENV_PATH", path)).export("ENV_PATH").global(true)
        }
    }

    /// A module that is not global and exports a number.
    #[module]
    pub struct SettingsHolder;

    impl SettingsHolder {
        fn with(ttl: u64) -> DynamicModule {
            DynamicModule::new::<SettingsHolder>().provider(Provider::use_value("TTL_SETTING", ttl)).export("TTL_SETTING")
        }
    }

    // ------------------------------------------------------------ ConfigModule

    #[corvis::test]
    async fn config_options_from_another_provider() {
        let path = env_file("GREETING=hello\n");
        let module = Test::create_testing_module()
            .import_dynamic(PathHolder::with(path))
            .import_dynamic(ConfigModule::for_root_async(true, |ctx: ResolveCtx| async move {
                let path = ctx.get_token::<String>("ENV_PATH").await?;
                Ok(file_options((*path).clone()))
            }))
            .compile()
            .await
            .unwrap();
        let config = module.get::<ConfigService>().unwrap();
        assert_eq!(config.get::<String>("GREETING").as_deref(), Some("hello"));
    }

    #[corvis::test]
    async fn config_factory_asking_for_config_service_is_circular() {
        let err = Test::create_testing_module()
            .import_dynamic(ConfigModule::for_root_async(true, |ctx: ResolveCtx| async move {
                let _ = ctx.get::<ConfigService>().await?;
                Ok(ConfigModuleOptions::default())
            }))
            .compile()
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("ircular") && err.contains("ConfigService"), "{err}");
    }

    #[injectable]
    pub struct NeedsConfig {
        pub config: Arc<ConfigService>,
    }

    #[module(providers = [NeedsConfig])]
    pub struct ConfigUser;

    #[corvis::test]
    async fn config_async_global_flag_is_the_argument() {
        // The `is_global` of the returned options is ignored: the argument decides (here `true`).
        let path = env_file("A=1\n");
        let module = Test::create_testing_module()
            .import_dynamic(ConfigModule::for_root_async(true, move |_| {
                let path = path.clone();
                async move { Ok(ConfigModuleOptions { is_global: false, ..file_options(path) }) }
            }))
            .import::<ConfigUser>()
            .compile()
            .await
            .unwrap();
        let user = module.get::<NeedsConfig>().unwrap();
        assert_eq!(user.config.get::<String>("A").as_deref(), Some("1"));
    }

    #[corvis::test]
    async fn config_factory_error_aborts_bootstrap() {
        let err = Test::create_testing_module()
            .import_dynamic(ConfigModule::for_root_async(true, |_| async { Err(anyhow::anyhow!("boom")) }))
            .compile()
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("Error while instantiating") && err.contains("ConfigService") && err.contains("boom"), "{err}");
    }

    // ------------------------------------------------------------ CacheModule

    #[corvis::test]
    async fn cache_ttl_from_config_service() {
        let path = env_file("CACHE_TTL=150\n");
        let module = Test::create_testing_module()
            .import_dynamic(ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..file_options(path) }))
            .import_dynamic(CacheModule::register_async(true, |ctx: ResolveCtx| async move {
                let config = ctx.get::<ConfigService>().await?;
                let ttl = config.get::<u64>("CACHE_TTL").ok_or_else(|| anyhow::anyhow!("CACHE_TTL is not set"))?;
                Ok(CacheOptions { ttl, ..Default::default() })
            }))
            .compile()
            .await
            .unwrap();
        let cache = module.get::<CacheManager>().unwrap();
        cache.set("k", 1, None).await;
        assert_eq!(cache.get::<i32>("k").await, Some(1));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(cache.get::<i32>("k").await, None);
    }

    #[injectable]
    pub struct ByType {
        pub cache: Arc<CacheManager>,
    }

    #[injectable]
    pub struct ByToken {
        #[inject(corvis::modules::cache::CACHE_MANAGER)]
        pub cache: Arc<CacheManager>,
    }

    #[module(providers = [ByType, ByToken])]
    pub struct CacheUsers;

    #[corvis::test]
    async fn cache_tokens_resolve_to_one_manager() {
        let module = Test::create_testing_module()
            .import_dynamic(CacheModule::register_async(true, |_| async { Ok(CacheOptions::default()) }))
            .import::<CacheUsers>()
            .compile()
            .await
            .unwrap();
        let a = module.get::<ByType>().unwrap();
        let b = module.get::<ByToken>().unwrap();
        assert!(Arc::ptr_eq(&a.cache, &b.cache));
        a.cache.set("shared", "v", None).await;
        assert_eq!(b.cache.get::<String>("shared").await.as_deref(), Some("v"));
    }

    #[corvis::test]
    async fn cache_factory_error_aborts_bootstrap() {
        let err = Test::create_testing_module()
            .import_dynamic(CacheModule::register_async(true, |_| async { Err(anyhow::anyhow!("CACHE_TTL is not set")) }))
            .compile()
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("Error while instantiating") && err.contains("CACHE_TTL is not set"), "{err}");
    }

    #[corvis::test]
    async fn cache_factory_resolves_a_non_global_import() {
        let module = Test::create_testing_module()
            .import_dynamic(
                CacheModule::register_async(true, |ctx: ResolveCtx| async move {
                    let ttl = ctx.get_token::<u64>("TTL_SETTING").await?;
                    Ok(CacheOptions { ttl: *ttl, ..Default::default() })
                })
                .import(SettingsHolder::with(150)),
            )
            .compile()
            .await
            .unwrap();
        let cache = module.get::<CacheManager>().unwrap();
        cache.set("k", 1, None).await;
        assert_eq!(cache.get::<i32>("k").await, Some(1));
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(cache.get::<i32>("k").await, None);
    }

    // ------------------------------------------------------------ ThrottlerModule

    #[controller("async-throttled")]
    #[use_guards(ThrottlerGuard)]
    pub struct AsyncThrottledController;

    #[routes]
    impl AsyncThrottledController {
        #[get]
        async fn hit(&self) -> &'static str {
            "ok"
        }
    }

    #[corvis::test]
    async fn throttler_limit_from_config_service() {
        let path = env_file("THROTTLE_LIMIT=2\n");
        let module = Test::create_testing_module()
            .import_dynamic(ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..file_options(path) }))
            .import_dynamic(ThrottlerModule::for_root_async(|ctx: ResolveCtx| async move {
                let config = ctx.get::<ConfigService>().await?;
                let limit = config.get::<u32>("THROTTLE_LIMIT").ok_or_else(|| anyhow::anyhow!("THROTTLE_LIMIT is not set"))?;
                Ok(vec![ThrottlerOptions::new(60_000, limit)])
            }))
            .controller::<AsyncThrottledController>()
            .compile()
            .await
            .unwrap();
        let mut app = module.create_corvis_application();
        app.init().await.unwrap();
        let c = request(&app);
        c.get("/async-throttled").send().await.expect_status(200);
        c.get("/async-throttled").send().await.expect_status(200);
        c.get("/async-throttled").send().await.expect_status(429);
    }

    #[corvis::test]
    async fn throttler_factory_can_return_a_storage() {
        let module = Test::create_testing_module()
            .import_dynamic(ThrottlerModule::for_root_async(|_| async {
                Ok(ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 1)]).with_max_entries(10))
            }))
            .controller::<AsyncThrottledController>()
            .compile()
            .await
            .unwrap();
        assert!(module.get::<ThrottlerStorage>().is_ok());
        let mut app = module.create_corvis_application();
        app.init().await.unwrap();
        let c = request(&app);
        c.get("/async-throttled").send().await.expect_status(200);
        c.get("/async-throttled").send().await.expect_status(429);
    }

    #[corvis::test]
    async fn throttler_factory_error_aborts_bootstrap() {
        let err = Test::create_testing_module()
            .import_dynamic(ThrottlerModule::for_root_async(|_| async { Err::<Vec<ThrottlerOptions>, _>(anyhow::anyhow!("no limit")) }))
            .compile()
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("Error while instantiating") && err.contains("no limit"), "{err}");
    }
}

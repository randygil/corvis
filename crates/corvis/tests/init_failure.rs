//! `init()` is single-use after a failure: later calls return `InitFailedError` and run nothing.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use corvis::InitFailedError;
use corvis::prelude::*;
use corvis::testing::{Test, request};

static FAIL_ONCE: AtomicBool = AtomicBool::new(true);
static GUARD_CALLS: AtomicUsize = AtomicUsize::new(0);
static MODULE_INITS: AtomicUsize = AtomicUsize::new(0);
static BOOTSTRAPS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct CountingGuard;

#[async_trait]
impl CanActivate for CountingGuard {
    async fn can_activate(&self, _ctx: &ExecutionContext) -> Result<bool, HttpException> {
        GUARD_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }
}

/// Fails its `on_module_init` the first time only.
#[injectable]
pub struct Flaky;

#[async_trait]
impl OnModuleInit for Flaky {
    async fn on_module_init(&self) -> corvis::Result<()> {
        MODULE_INITS.fetch_add(1, Ordering::SeqCst);
        if FAIL_ONCE.swap(false, Ordering::SeqCst) {
            anyhow::bail!("database unavailable");
        }
        Ok(())
    }
}

#[async_trait]
impl OnApplicationBootstrap for Flaky {
    async fn on_application_bootstrap(&self) -> corvis::Result<()> {
        BOOTSTRAPS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[controller("x")]
pub struct XController;

#[routes]
impl XController {
    #[get]
    fn get(&self) -> Value {
        json!({ "ok": true })
    }
}

#[module(controllers = [XController], providers = [Flaky, Provider::app_guard::<CountingGuard>()])]
pub struct AppModule;

async fn new_app() -> CorvisApplication {
    Test::create_testing_module().import::<AppModule>().compile().await.unwrap().create_corvis_application()
}

fn arm() {
    FAIL_ONCE.store(true, Ordering::SeqCst);
    MODULE_INITS.store(0, Ordering::SeqCst);
    BOOTSTRAPS.store(0, Ordering::SeqCst);
    GUARD_CALLS.store(0, Ordering::SeqCst);
}

// These tests share the statics above, so they run as one sequence.
#[corvis::test]
async fn failed_init_is_single_use() {
    arm();

    // The failed application: first error as is, then InitFailedError, nothing re-run.
    let mut failed = new_app().await;
    let first = failed.init().await.unwrap_err();
    assert!(first.downcast_ref::<InitFailedError>().is_none());
    assert!(first.to_string().contains("database unavailable"), "{first}");

    let second = failed.init().await.unwrap_err();
    let err = second.downcast_ref::<InitFailedError>().expect("InitFailedError");
    assert_eq!(err.first_error(), "database unavailable");
    let msg = second.to_string();
    assert!(msg.contains("failed to initialize"), "{msg}");
    assert!(msg.contains("database unavailable"), "{msg}");
    assert!(msg.contains("create a new application"), "{msg}");
    assert_eq!(MODULE_INITS.load(Ordering::SeqCst), 1, "the hook must not be re-run");
    assert_eq!(BOOTSTRAPS.load(Ordering::SeqCst), 0);

    // Implicit init through handle() and listen_in_background().
    let req = corvis::Request::new(http::Method::GET, "/x".parse().unwrap(), http::HeaderMap::new(), bytes::Bytes::new()).unwrap();
    let e = failed.handle(req).await.unwrap_err();
    assert!(e.downcast_ref::<InitFailedError>().is_some());
    let e = failed.listen_in_background("127.0.0.1:0").await.unwrap_err();
    assert!(e.downcast_ref::<InitFailedError>().is_some());
    assert_eq!(MODULE_INITS.load(Ordering::SeqCst), 1);

    // A fresh application from the same module works, and the global guard runs once per request.
    let mut fresh = new_app().await;
    fresh.init().await.unwrap();
    request(&fresh).get("/x").send().await.expect_status(200);
    assert_eq!(GUARD_CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(BOOTSTRAPS.load(Ordering::SeqCst), 1);

    // Two successful init() calls run the hooks once.
    fresh.init().await.unwrap();
    assert_eq!(MODULE_INITS.load(Ordering::SeqCst), 2, "one failed run and one successful run");
    assert_eq!(BOOTSTRAPS.load(Ordering::SeqCst), 1);
    fresh.close().await.unwrap();
}

#[corvis::test]
#[should_panic(expected = "init() failed")]
async fn test_client_of_a_failed_application_panics() {
    // A module whose hook always fails, so this test does not depend on the others.
    #[injectable]
    pub struct AlwaysFails;

    #[async_trait]
    impl OnModuleInit for AlwaysFails {
        async fn on_module_init(&self) -> corvis::Result<()> {
            anyhow::bail!("nope")
        }
    }

    #[module(providers = [AlwaysFails])]
    pub struct BadModule;

    let mut app = Test::create_testing_module().import::<BadModule>().compile().await.unwrap().create_corvis_application();
    assert!(app.init().await.is_err());
    let _ = request(&app);
}

#[injectable]
pub struct Counted;

static COUNTED_INITS: AtomicUsize = AtomicUsize::new(0);

#[async_trait]
impl OnModuleInit for Counted {
    async fn on_module_init(&self) -> corvis::Result<()> {
        COUNTED_INITS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[module(providers = [Counted])]
pub struct CountedModule;

#[corvis::test]
async fn bind_failure_after_a_successful_init_can_be_retried() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = taken.local_addr().unwrap();
    let mut app = Test::create_testing_module().import::<CountedModule>().compile().await.unwrap().create_corvis_application();

    let err = app.listen_in_background(busy).await.unwrap_err();
    assert!(err.downcast_ref::<InitFailedError>().is_none(), "{err}");
    let addr = app.listen_in_background("127.0.0.1:0").await.unwrap();
    assert!(addr.port() != 0);
    assert_eq!(COUNTED_INITS.load(Ordering::SeqCst), 1);
    app.close().await.unwrap();
}

//! `Provider::use_class_as` keeps the hooks of the concrete class.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::prelude::*;
use corvis::testing::Test;

pub trait Repo: Send + Sync {
    fn state(&self) -> String;
    fn id(&self) -> usize;
}

static INITS: AtomicUsize = AtomicUsize::new(0);
static SHUTDOWNS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct MemoryRepo {
    #[default]
    state: Mutex<String>,
}

impl Repo for MemoryRepo {
    fn state(&self) -> String {
        self.state.lock().unwrap().clone()
    }
    fn id(&self) -> usize {
        0
    }
}

#[async_trait]
impl OnModuleInit for MemoryRepo {
    async fn on_module_init(&self) -> corvis::Result<()> {
        INITS.fetch_add(1, Ordering::SeqCst);
        *self.state.lock().unwrap() = "ready".into();
        Ok(())
    }
}

#[async_trait]
impl OnApplicationShutdown for MemoryRepo {
    async fn on_application_shutdown(&self, _signal: Option<String>) -> corvis::Result<()> {
        SHUTDOWNS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[module(providers = [Provider::use_class_as::<MemoryRepo, dyn Repo>("REPO", |r| r)])]
pub struct LifecycleModule;

#[corvis::test]
async fn lifecycle_hooks_run_once_on_the_concrete_instance() {
    let module = Test::create_testing_module().import::<LifecycleModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let repo = app.get_token::<dyn Repo>("REPO").unwrap();
    assert_eq!(repo.state(), "ready");
    assert_eq!(INITS.load(Ordering::SeqCst), 1);
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), 0);
    app.close().await.unwrap();
    assert_eq!(INITS.load(Ordering::SeqCst), 1);
    assert_eq!(SHUTDOWNS.load(Ordering::SeqCst), 1);
}

static EVENTS: AtomicUsize = AtomicUsize::new(0);

pub trait Listener: Send + Sync {}

#[injectable]
pub struct EventListener;

impl Listener for EventListener {}

#[handlers]
impl EventListener {
    #[on_event("x")]
    async fn on_x(&self, _e: u32) {
        EVENTS.fetch_add(1, Ordering::SeqCst);
    }
}

#[module(
    imports = [EventEmitterModule::for_root(EventEmitterOptions::default())],
    providers = [Provider::use_class_as::<EventListener, dyn Listener>("LISTENER", |l| l)],
)]
pub struct EventsModule;

#[corvis::test]
async fn event_handlers_of_a_class_behind_a_trait_run_once() {
    let module = Test::create_testing_module().import::<EventsModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let emitter = app.get::<EventEmitter>().unwrap();
    emitter.emit_async("x", 1u32).await;
    assert_eq!(EVENTS.load(Ordering::SeqCst), 1);
    app.close().await.unwrap();
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);
static TRANSIENT_INITS: AtomicUsize = AtomicUsize::new(0);

#[injectable(scope = "transient")]
pub struct TransientRepo {
    #[init(NEXT_ID.fetch_add(1, Ordering::SeqCst))]
    id: usize,
}

impl Repo for TransientRepo {
    fn state(&self) -> String {
        String::new()
    }
    fn id(&self) -> usize {
        self.id
    }
}

#[async_trait]
impl OnModuleInit for TransientRepo {
    async fn on_module_init(&self) -> corvis::Result<()> {
        TRANSIENT_INITS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[injectable]
pub struct ConsumerA {
    #[inject("TREPO")]
    repo: Arc<dyn Repo>,
}

#[injectable]
pub struct ConsumerB {
    #[inject("TREPO")]
    repo: Arc<dyn Repo>,
}

#[module(providers = [Provider::use_class_as::<TransientRepo, dyn Repo>("TREPO", |r| r), ConsumerA, ConsumerB])]
pub struct TransientModule;

#[corvis::test]
async fn transient_class_behind_a_trait_gets_distinct_instances_and_no_hooks() {
    let module = Test::create_testing_module().import::<TransientModule>().compile().await.unwrap();
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let a = app.get::<ConsumerA>().unwrap();
    let b = app.get::<ConsumerB>().unwrap();
    assert_ne!(a.repo.id(), b.repo.id());
    assert_eq!(TRANSIENT_INITS.load(Ordering::SeqCst), 0);
    app.close().await.unwrap();
}

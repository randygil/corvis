//! `core/injection-scopes`: singleton and transient instance rules.

use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{compile_ok, next_id};

#[injectable]
pub struct Counter;

#[injectable]
pub struct CounterUserA {
    counter: Arc<Counter>,
}

#[injectable]
pub struct CounterUserB {
    counter: Arc<Counter>,
}

#[module(providers = [Counter, CounterUserA, CounterUserB])]
pub struct SingletonModule;

#[corvis::test]
async fn two_consumers_of_a_singleton_share_it() {
    let module = compile_ok(Test::create_testing_module().import::<SingletonModule>()).await;
    let a = module.get::<CounterUserA>().unwrap();
    let b = module.get::<CounterUserB>().unwrap();
    assert!(Arc::ptr_eq(&a.counter, &b.counter));
    assert!(Arc::ptr_eq(&a.counter, &module.get::<Counter>().unwrap()));
}

#[injectable(scope = "transient")]
pub struct OperationTimer {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct TimerUserA {
    timer: Arc<OperationTimer>,
}

#[injectable]
pub struct TimerUserB {
    timer: Arc<OperationTimer>,
}

#[module(providers = [OperationTimer, TimerUserA, TimerUserB])]
pub struct AttributeModule;

#[corvis::test]
async fn transient_attribute_gives_each_consumer_its_own_instance() {
    let module = compile_ok(Test::create_testing_module().import::<AttributeModule>()).await;
    let a = module.get::<TimerUserA>().unwrap();
    let b = module.get::<TimerUserB>().unwrap();
    assert!(!Arc::ptr_eq(&a.timer, &b.timer));
    assert_ne!(a.timer.id, b.timer.id);
}

#[injectable]
pub struct PlainTimer {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct PlainTimerUserA {
    #[inject("TIMER")]
    timer: Arc<PlainTimer>,
}

#[injectable]
pub struct PlainTimerUserB {
    #[inject("TIMER")]
    timer: Arc<PlainTimer>,
}

#[module(providers = [Provider::use_class::<PlainTimer>("TIMER").scope(Scope::Transient), PlainTimerUserA, PlainTimerUserB])]
pub struct ScopeOnProviderModule;

#[corvis::test]
async fn transient_scope_set_on_the_provider() {
    let module = compile_ok(Test::create_testing_module().import::<ScopeOnProviderModule>()).await;
    let a = module.get::<PlainTimerUserA>().unwrap();
    let b = module.get::<PlainTimerUserB>().unwrap();
    assert!(!Arc::ptr_eq(&a.timer, &b.timer));
    assert_ne!(a.timer.id, b.timer.id);
}

#[injectable]
pub struct TimerResolver {
    module_ref: ModuleRef,
}

#[module(providers = [OperationTimer, TimerResolver])]
pub struct ResolveModule;

#[corvis::test]
async fn module_ref_resolve_returns_a_new_transient_each_time() {
    let module = compile_ok(Test::create_testing_module().import::<ResolveModule>()).await;
    let resolver = module.get::<TimerResolver>().unwrap();
    let first = resolver.module_ref.resolve::<OperationTimer>().await.unwrap();
    let second = resolver.module_ref.resolve::<OperationTimer>().await.unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_ne!(first.id, second.id);
}

#[injectable(scope = "transient")]
pub struct Probe;

#[injectable]
pub struct TwoProbes {
    first: Arc<Probe>,
    second: Arc<Probe>,
}

#[module(providers = [Probe, TwoProbes])]
pub struct TwoFieldsModule;

#[corvis::test]
async fn every_injection_point_of_a_transient_gets_its_own_instance() {
    let module = compile_ok(Test::create_testing_module().import::<TwoFieldsModule>()).await;
    let consumer = module.get::<TwoProbes>().unwrap();
    assert!(!Arc::ptr_eq(&consumer.first, &consumer.second));
}

#[corvis::test]
async fn a_transient_is_not_created_at_startup() {
    static CREATED: AtomicUsize = AtomicUsize::new(0);

    #[injectable(scope = "transient")]
    pub struct Lazy {
        #[init(CREATED.fetch_add(1, Ordering::SeqCst))]
        _n: usize,
    }
    let module = compile_ok(Test::create_testing_module().provider::<Lazy>()).await;
    assert_eq!(CREATED.load(Ordering::SeqCst), 0);
    module.resolve::<Lazy>().await.unwrap();
    assert_eq!(CREATED.load(Ordering::SeqCst), 1);
}

#[corvis::test]
async fn a_transient_instance_is_not_stored() {
    let module = compile_ok(Test::create_testing_module().provider::<OperationTimer>()).await;
    assert!(matches!(module.get::<OperationTimer>(), Err(DiError::NotFound { .. })));
    // Resolving it does not make it findable either.
    module.resolve::<OperationTimer>().await.unwrap();
    assert!(matches!(module.get::<OperationTimer>(), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn lifecycle_hooks_are_not_called_on_transient_instances() {
    static HOOKS: AtomicUsize = AtomicUsize::new(0);

    #[injectable(scope = "transient")]
    pub struct Hooked;

    #[async_trait]
    impl OnModuleInit for Hooked {
        async fn on_module_init(&self) -> corvis::Result<()> {
            HOOKS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[injectable]
    pub struct HookedUser {
        _hooked: Arc<Hooked>,
    }
    #[module(providers = [Hooked, HookedUser])]
    pub struct HookedModule;

    let module = compile_ok(Test::create_testing_module().import::<HookedModule>()).await;
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    assert_eq!(HOOKS.load(Ordering::SeqCst), 0);
    app.close().await.unwrap();
}

#[injectable]
pub struct Report {
    timer: Arc<OperationTimer>,
}

#[corvis::test]
async fn a_singleton_keeps_its_transient_dependency_and_stays_a_singleton() {
    let module = compile_ok(Test::create_testing_module().provider::<OperationTimer>().provider::<Report>()).await;
    let first = module.get::<Report>().unwrap();
    let second = module.resolve::<Report>().await.unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(&first.timer, &second.timer));
}

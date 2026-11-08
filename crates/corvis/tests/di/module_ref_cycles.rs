//! `core/circular-dependencies`: cycles that pass through `ModuleRef` while a provider is being constructed are
//! reported, and a `ModuleRef` used after construction starts a new resolution.
//!
//! Every `compile()` goes through the helpers of `main.rs`, which fail after 20 seconds instead of hanging.

use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{assert_cycle, chain_names, compile_err, compile_ok, next_id};

// --- A factory resolves its own consumer through ModuleRef ---

pub struct Orders;

#[injectable]
pub struct Invoices {
    _orders: Arc<Orders>,
}

#[module(providers = [
    Invoices,
    Provider::use_factory(Token::of::<Orders>(), |ctx| async move {
        ctx.module_ref().resolve::<Invoices>().await?;
        Ok(Orders)
    }),
])]
pub struct FactoryConsumerModule;

#[corvis::test]
async fn a_factory_resolving_its_consumer_through_module_ref() {
    let err = compile_err(Test::create_testing_module().import::<FactoryConsumerModule>()).await;
    assert_cycle(&err, &["Orders", "Invoices"]);
}

// --- A factory resolves itself ---

static LOOP_BUILDS: AtomicUsize = AtomicUsize::new(0);

pub struct Loop;

#[module(providers = [
    Provider::use_factory(Token::of::<Loop>(), |ctx| async move {
        LOOP_BUILDS.fetch_add(1, Ordering::SeqCst);
        ctx.module_ref().resolve::<Loop>().await?;
        Ok(Loop)
    }),
])]
pub struct SelfResolvingModule;

#[corvis::test]
async fn a_factory_resolving_itself_through_module_ref() {
    let err = compile_err(Test::create_testing_module().import::<SelfResolvingModule>()).await;
    assert_eq!(chain_names(&err), ["Loop", "Loop"]);
    assert_eq!(LOOP_BUILDS.load(Ordering::SeqCst), 1, "the factory must run once");
}

// --- A stored ModuleRef of another provider ---

#[injectable]
pub struct Registry {
    module_ref: ModuleRef,
}

impl Registry {
    pub async fn find<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.module_ref.resolve::<T>().await
    }
}

pub struct StoredOrders;

#[injectable]
pub struct StoredInvoices {
    _orders: Arc<StoredOrders>,
}

#[module(providers = [
    Registry,
    StoredInvoices,
    Provider::use_factory(Token::of::<StoredOrders>(), |ctx| async move {
        let registry = ctx.get::<Registry>().await?;
        registry.find::<StoredInvoices>().await?;
        Ok(StoredOrders)
    }),
])]
pub struct StoredRefModule;

#[corvis::test]
async fn a_stored_module_ref_of_another_provider() {
    let err = compile_err(Test::create_testing_module().import::<StoredRefModule>()).await;
    assert_cycle(&err, &["StoredOrders", "StoredInvoices"]);
}

// --- ModuleRef::create during construction ---

pub struct CreateOrders;

#[injectable]
pub struct Report {
    _orders: Arc<CreateOrders>,
}

#[module(providers = [
    Provider::use_factory(Token::of::<CreateOrders>(), |ctx| async move {
        ctx.module_ref().create::<Report>().await?;
        Ok(CreateOrders)
    }),
])]
pub struct CreateModule;

#[corvis::test]
async fn module_ref_create_during_construction() {
    let err = compile_err(Test::create_testing_module().import::<CreateModule>()).await;
    assert_cycle(&err, &["CreateOrders"]);
}

// --- A guard or pipe built by type inside a factory ---

pub struct ByTypeOwner;

#[injectable]
pub struct NeedsOwner {
    _owner: Arc<ByTypeOwner>,
}

#[module(providers = [
    Provider::use_factory(Token::of::<ByTypeOwner>(), |ctx| async move {
        ctx.get_or_construct::<NeedsOwner>().await?;
        Ok(ByTypeOwner)
    }),
])]
pub struct ByTypeModule;

#[corvis::test]
async fn a_class_constructed_by_type_inside_a_factory_keeps_the_path() {
    let err = compile_err(Test::create_testing_module().import::<ByTypeModule>()).await;
    assert_cycle(&err, &["ByTypeOwner"]);
}

// --- A chain that never repeats a provider is cut by the depth limit ---

/// Deep chains need more stack than a default test thread has in a debug build, so this runs on its own thread.
#[test]
fn a_chain_deeper_than_the_limit_fails_with_the_circular_error() {
    const LINKS: usize = 300;
    let outcome = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let runtime = corvis::tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            runtime.block_on(async {
                let mut providers = Vec::new();
                for i in 0..LINKS {
                    providers.push(Provider::use_factory(format!("LINK_{i}"), move |ctx| async move {
                        if i + 1 < LINKS {
                            ctx.get_token::<usize>(format!("LINK_{}", i + 1)).await?;
                        }
                        Ok(i)
                    }));
                }
                compile_err(Test::create_testing_module().providers(providers)).await
            })
        })
        .unwrap()
        .join()
        .unwrap();
    match outcome {
        DiError::Circular { chain } => assert!(chain.starts_with("... -> LINK_"), "{chain}"),
        other => panic!("expected DiError::Circular, got {other:?}"),
    }
}

// --- Run-time use of a ModuleRef: no false cycles ---

#[injectable(scope = "transient")]
pub struct Job {
    module_ref: ModuleRef,
    #[init(next_id())]
    id: usize,
}

#[corvis::test]
async fn a_transient_provider_resolves_a_fresh_copy_of_itself_after_construction() {
    let module = compile_ok(Test::create_testing_module().provider::<Job>()).await;
    let job = module.resolve::<Job>().await.unwrap();
    let copy = job.module_ref.resolve::<Job>().await.unwrap();
    assert!(!Arc::ptr_eq(&job, &copy));
    assert_ne!(job.id, copy.id);
}

#[injectable(scope = "transient")]
pub struct Dependent {
    orders: Arc<RuntimeOrders>,
}

#[injectable]
pub struct RuntimeOrders {
    module_ref: ModuleRef,
}

#[corvis::test]
async fn a_singleton_resolves_a_transient_dependent_of_itself_at_run_time() {
    let module = compile_ok(Test::create_testing_module().provider::<RuntimeOrders>().provider::<Dependent>()).await;
    let orders = module.get::<RuntimeOrders>().unwrap();
    let dependent = orders.module_ref.resolve::<Dependent>().await.unwrap();
    assert!(Arc::ptr_eq(&dependent.orders, &orders));
}

#[corvis::test]
async fn concurrent_resolutions_return_distinct_instances() {
    let module = compile_ok(Test::create_testing_module().provider::<Job>()).await;
    let job = module.resolve::<Job>().await.unwrap();
    let mut tasks = Vec::new();
    for _ in 0..10 {
        let module_ref = job.module_ref.clone();
        tasks.push(corvis::tokio::spawn(async move { module_ref.resolve::<Job>().await }));
    }
    let mut jobs = Vec::new();
    for task in tasks {
        jobs.push(task.await.unwrap().unwrap());
    }
    for (i, a) in jobs.iter().enumerate() {
        for b in &jobs[i + 1..] {
            assert!(!Arc::ptr_eq(a, b));
        }
    }
}

// --- Two resolutions of one factory in parallel keep their own paths ---

pub struct Left;
pub struct Right;
pub struct Both;

#[module(providers = [
    Provider::use_factory(Token::of::<Left>(), |_ctx| async move { Ok(Left) }),
    Provider::use_factory(Token::of::<Right>(), |_ctx| async move { Ok(Right) }),
    Provider::use_factory(Token::of::<Both>(), |ctx| async move {
        let module_ref = ctx.module_ref();
        let (left, right) = corvis::futures::join!(ctx.get::<Left>(), module_ref.resolve::<Right>());
        left?;
        right?;
        Ok(Both)
    }),
])]
pub struct JoinModule;

#[corvis::test]
async fn parallel_resolutions_inside_one_factory_do_not_see_a_cycle() {
    let module = compile_ok(Test::create_testing_module().import::<JoinModule>()).await;
    assert!(module.get::<Both>().is_ok());
}

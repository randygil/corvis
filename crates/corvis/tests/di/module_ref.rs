//! `core/module-ref`: programmatic access to the container, anchored at one module.

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::{Test, TestingModule};

use crate::{compile_ok, next_id};

#[injectable]
pub struct CatsService {
    module_ref: ModuleRef,
}

#[module(providers = [CatsService], exports = [CatsService])]
pub struct CatsModule;

#[injectable]
pub struct Clock;

#[module(global, providers = [Clock], exports = [Clock])]
pub struct ClockModule;

#[injectable]
pub struct Counter {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct Secret {
    #[init(next_id())]
    id: usize,
}

/// Lives in module `AModule`; its `module_ref` is located there.
#[injectable]
pub struct ProbeA {
    module_ref: ModuleRef,
    counter: Arc<Counter>,
}

/// Lives in the unrelated module `BModule`.
#[injectable]
pub struct ProbeB {
    counter: Arc<Counter>,
    secret: Arc<Secret>,
}

/// A class that is never registered as a provider.
#[injectable]
pub struct ReportBuilder {
    cats: Arc<CatsService>,
}

#[injectable]
pub struct NeedsSecret {
    _secret: Arc<Secret>,
}

#[module(imports = [CatsModule], providers = [Counter, ProbeA, Provider::use_value("PORT", 8080u32)])]
pub struct AModule;

#[module(providers = [Counter, Secret, ProbeB])]
pub struct BModule;

async fn graph() -> TestingModule {
    compile_ok(Test::create_testing_module().import::<AModule>().import::<BModule>().import::<CatsModule>().import::<ClockModule>()).await
}

#[corvis::test]
async fn a_module_ref_field_is_located_in_the_owning_module() {
    let module = graph().await;
    let cats = module.get::<CatsService>().unwrap();
    assert_eq!(cats.module_ref.module_name(), "CatsModule");
    let probe = module.get::<ProbeA>().unwrap();
    assert_eq!(probe.module_ref.module_name(), "AModule");
}

#[corvis::test]
async fn the_application_module_ref_is_located_in_the_root_module() {
    let module = graph().await;
    let app = module.create_corvis_application();
    assert_eq!(app.module_ref().module_name(), "RootTestModule");
}

#[corvis::test]
async fn the_module_ref_of_a_factory_is_located_in_the_module_that_registers_it() {
    #[module(providers = [Provider::use_factory("WHERE", |ctx| async move { Ok(ctx.module_ref().module_name()) })])]
    pub struct FactoryModule;

    let module = compile_ok(Test::create_testing_module().import::<FactoryModule>()).await;
    assert_eq!(*module.get_token::<String>("WHERE").unwrap(), "FactoryModule");
}

#[corvis::test]
async fn get_prefers_the_provider_visible_from_its_module() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    let b = module.get::<ProbeB>().unwrap();
    assert!(!Arc::ptr_eq(&a.counter, &b.counter));
    assert_ne!(a.counter.id, b.counter.id);
    let from_a = a.module_ref.get::<Counter>().unwrap();
    assert!(Arc::ptr_eq(&from_a, &a.counter));
    // get_token behaves the same as get.
    assert!(Arc::ptr_eq(&a.module_ref.get_token::<Counter>(Token::of::<Counter>()).unwrap(), &a.counter));
    // The application-wide lookup of the testing module still finds one of them.
    assert!(module.get::<Counter>().is_ok());
}

#[corvis::test]
async fn get_falls_back_to_the_whole_application() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    let b = module.get::<ProbeB>().unwrap();
    // `Secret` is a private provider of BModule, which AModule does not import.
    let secret = a.module_ref.get::<Secret>().unwrap();
    assert!(Arc::ptr_eq(&secret, &b.secret));
    assert_eq!(secret.id, b.secret.id);
}

#[corvis::test]
async fn get_token_with_an_unknown_token_is_not_found() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    assert!(matches!(a.module_ref.get_token::<String>("NOPE"), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn get_token_with_the_wrong_type_is_a_type_mismatch() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    assert!(matches!(a.module_ref.get_token::<String>("PORT"), Err(DiError::TypeMismatch { .. })));
    assert_eq!(*a.module_ref.get_token::<u32>("PORT").unwrap(), 8080);
}

#[corvis::test]
async fn get_does_not_create_instances() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    assert!(matches!(a.module_ref.get::<ReportBuilder>(), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn get_token_in_context_sees_what_injection_sees() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    // Exported by an imported module.
    let cats = a.module_ref.get_token_in_context::<CatsService>(Token::of::<CatsService>()).unwrap();
    assert!(Arc::ptr_eq(&cats, &module.get::<CatsService>().unwrap()));
    // Exported by a global module.
    assert!(a.module_ref.get_token_in_context::<Clock>(Token::of::<Clock>()).is_ok());
    // Private provider of a module that is not imported.
    assert!(matches!(a.module_ref.get_token_in_context::<Secret>(Token::of::<Secret>()), Err(DiError::NotFound { .. })));
    // `get` would have found it through the application-wide fallback.
    assert!(a.module_ref.get::<Secret>().is_ok());
}

#[corvis::test]
async fn get_strict_only_sees_the_module_own_providers() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    let own = a.module_ref.get_strict::<Counter>().unwrap();
    assert!(Arc::ptr_eq(&own, &a.counter));
    // Exported by an import, and global: both are invisible to `get_strict`.
    assert!(matches!(a.module_ref.get_strict::<CatsService>(), Err(DiError::NotFound { .. })));
    assert!(matches!(a.module_ref.get_strict::<Clock>(), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn resolve_returns_the_existing_singleton() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    let resolved = a.module_ref.resolve::<CatsService>().await.unwrap();
    assert!(Arc::ptr_eq(&resolved, &module.get::<CatsService>().unwrap()));
    assert!(matches!(a.module_ref.resolve::<Secret>().await, Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn create_builds_unregistered_classes_without_storing_them() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    let first = a.module_ref.create::<ReportBuilder>().await.unwrap();
    let second = a.module_ref.create::<ReportBuilder>().await.unwrap();
    assert!(Arc::ptr_eq(&first.cats, &second.cats));
    assert!(Arc::ptr_eq(&first.cats, &module.get::<CatsService>().unwrap()));
    assert!(matches!(module.get::<ReportBuilder>(), Err(DiError::NotFound { .. })));
    assert!(matches!(a.module_ref.get::<ReportBuilder>(), Err(DiError::NotFound { .. })));
}

#[corvis::test]
async fn create_with_an_invisible_dependency_is_an_error_naming_the_module() {
    let module = graph().await;
    let a = module.get::<ProbeA>().unwrap();
    match a.module_ref.create::<NeedsSecret>().await {
        Err(DiError::UnknownDependencies { module, token, .. }) => {
            assert_eq!(module, "AModule");
            assert_eq!(token, "Secret");
        }
        Err(other) => panic!("expected UnknownDependencies, got {other:?}"),
        Ok(_) => panic!("create should have failed"),
    }
}

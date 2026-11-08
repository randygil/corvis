//! `core/custom-providers`: aliases (`use_existing`) and classes behind a trait (`use_class_as`).
//!
//! Lifecycle hooks of `use_class_as` classes and a transient class behind a trait are covered by
//! `tests/di_use_class_as.rs`.

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{compile_err, compile_ok, next_id};

#[injectable]
pub struct CatsService;

#[injectable]
pub struct CatsUser {
    #[inject("CATS")]
    cats: Arc<CatsService>,
}

#[module(providers = [CatsService, Provider::use_existing("CATS", Token::of::<CatsService>()), CatsUser])]
pub struct AliasModule;

#[corvis::test]
async fn an_alias_hands_over_the_same_arc() {
    let module = compile_ok(Test::create_testing_module().import::<AliasModule>()).await;
    let user = module.get::<CatsUser>().unwrap();
    assert!(Arc::ptr_eq(&user.cats, &module.get::<CatsService>().unwrap()));
    assert!(Arc::ptr_eq(&module.get_token::<CatsService>("CATS").unwrap(), &user.cats));
}

#[module(providers = [
    CatsService,
    Provider::use_existing("B", Token::of::<CatsService>()),
    Provider::use_existing("A", "B"),
])]
pub struct ChainModule;

#[corvis::test]
async fn aliases_can_point_to_other_aliases() {
    let module = compile_ok(Test::create_testing_module().import::<ChainModule>()).await;
    let cats = module.get::<CatsService>().unwrap();
    assert!(Arc::ptr_eq(&module.get_token::<CatsService>("A").unwrap(), &cats));
    assert!(Arc::ptr_eq(&module.get_token::<CatsService>("B").unwrap(), &cats));
}

#[injectable(scope = "transient")]
pub struct OperationTimer {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct AliasedTimerUserA {
    #[inject("TIMER")]
    timer: Arc<OperationTimer>,
}

#[injectable]
pub struct AliasedTimerUserB {
    #[inject("TIMER")]
    timer: Arc<OperationTimer>,
}

#[injectable]
pub struct DirectTimerUserA {
    timer: Arc<OperationTimer>,
}

#[injectable]
pub struct DirectTimerUserB {
    timer: Arc<OperationTimer>,
}

#[module(providers = [
    OperationTimer,
    Provider::use_existing("TIMER", Token::of::<OperationTimer>()),
    AliasedTimerUserA,
    AliasedTimerUserB,
    DirectTimerUserA,
    DirectTimerUserB,
])]
pub struct TransientAliasModule;

#[corvis::test]
async fn an_alias_of_a_transient_provider_is_transient_too() {
    let module = compile_ok(Test::create_testing_module().import::<TransientAliasModule>()).await;
    let a = module.get::<AliasedTimerUserA>().unwrap();
    let b = module.get::<AliasedTimerUserB>().unwrap();
    assert!(!Arc::ptr_eq(&a.timer, &b.timer));
    let c = module.get::<DirectTimerUserA>().unwrap();
    let d = module.get::<DirectTimerUserB>().unwrap();
    assert!(!Arc::ptr_eq(&c.timer, &d.timer));
    // Like the provider it points to, the alias is not stored.
    assert!(matches!(module.get_token::<OperationTimer>("TIMER"), Err(DiError::NotFound { .. })));
}

#[module(providers = [
    OperationTimer,
    Provider::use_existing("C", Token::of::<OperationTimer>()),
    Provider::use_existing("B", "C"),
    Provider::use_existing("TIMER", "B"),
    AliasedTimerUserA,
    AliasedTimerUserB,
])]
pub struct TransientChainModule;

#[corvis::test]
async fn a_chain_of_aliases_ending_in_a_transient_provider_gives_new_instances() {
    let module = compile_ok(Test::create_testing_module().import::<TransientChainModule>()).await;
    let a = module.get::<AliasedTimerUserA>().unwrap();
    let b = module.get::<AliasedTimerUserB>().unwrap();
    assert!(!Arc::ptr_eq(&a.timer, &b.timer));
    assert_ne!(a.timer.id, b.timer.id);
}

#[injectable]
pub struct AImportsCats {
    #[inject("CATS")]
    cats: Arc<CatsService>,
}

#[module(providers = [CatsService], exports = [CatsService])]
pub struct CatsModule;

#[module(imports = [CatsModule], providers = [Provider::use_existing("CATS", Token::of::<CatsService>()), AImportsCats])]
pub struct AliasOfImportModule;

#[corvis::test]
async fn an_alias_resolves_its_target_from_an_imported_module() {
    let module = compile_ok(Test::create_testing_module().import::<AliasOfImportModule>()).await;
    let user = module.get::<AImportsCats>().unwrap();
    assert!(Arc::ptr_eq(&user.cats, &module.get::<CatsService>().unwrap()));
}

#[injectable]
pub struct PrivateThing;

#[module(providers = [PrivateThing])]
pub struct OwnerModule;

#[module(providers = [Provider::use_existing("ALIAS", Token::of::<PrivateThing>())])]
pub struct InvisibleTargetModule;

#[corvis::test]
async fn an_alias_to_a_target_that_is_not_visible_fails_at_startup() {
    let err = compile_err(Test::create_testing_module().import::<OwnerModule>().import::<InvisibleTargetModule>()).await;
    match err {
        DiError::NotFound { token } => assert_eq!(token, "PrivateThing"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[injectable]
pub struct ExportedAliasUser {
    #[inject("CATS")]
    cats: Arc<CatsService>,
}

#[module(providers = [CatsService, Provider::use_existing("CATS", Token::of::<CatsService>())], exports = ["CATS"])]
pub struct ExportingAliasModule;

#[module(imports = [ExportingAliasModule], providers = [ExportedAliasUser])]
pub struct ConsumerModule;

#[corvis::test]
async fn an_alias_can_be_exported() {
    let module = compile_ok(Test::create_testing_module().import::<ConsumerModule>()).await;
    let user = module.get::<ExportedAliasUser>().unwrap();
    assert!(Arc::ptr_eq(&user.cats, &module.get::<CatsService>().unwrap()));
}

// ------------------------------------------------------------------ use_class_as

pub trait UserRepo: Send + Sync {
    fn name(&self) -> String;
    fn seed(&self) -> usize;
}

#[injectable]
pub struct Seed {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct MemoryUserRepo {
    seed: Arc<Seed>,
}

impl UserRepo for MemoryUserRepo {
    fn name(&self) -> String {
        "memory".into()
    }
    fn seed(&self) -> usize {
        self.seed.id
    }
}

#[injectable]
pub struct UsersService {
    #[inject("USER_REPO")]
    repo: Arc<dyn UserRepo>,
}

#[module(providers = [Seed, Provider::use_class_as::<MemoryUserRepo, dyn UserRepo>("USER_REPO", |r| r), UsersService])]
pub struct UsersModule;

#[corvis::test]
async fn use_class_as_injects_the_trait_object_and_the_class_dependencies() {
    let module = compile_ok(Test::create_testing_module().import::<UsersModule>()).await;
    let users = module.get::<UsersService>().unwrap();
    assert_eq!(users.repo.name(), "memory");
    assert_eq!(users.repo.seed(), module.get::<Seed>().unwrap().id);
}

#[corvis::test]
async fn use_class_as_is_not_available_as_the_concrete_type() {
    let module = compile_ok(Test::create_testing_module().import::<UsersModule>()).await;
    assert!(matches!(module.get_token::<MemoryUserRepo>("USER_REPO"), Err(DiError::TypeMismatch { .. })));
    assert!(module.get_token::<dyn UserRepo>("USER_REPO").is_ok());
}

pub trait Exporter: Send + Sync {
    fn export(&self) -> String;
}

#[injectable]
pub struct CsvExporter;
impl Exporter for CsvExporter {
    fn export(&self) -> String {
        "csv".into()
    }
}

#[injectable]
pub struct JsonExporter;
impl Exporter for JsonExporter {
    fn export(&self) -> String {
        "json".into()
    }
}

#[injectable]
pub struct ReportService {
    module_ref: ModuleRef,
}

impl ReportService {
    fn exporter(&self, format: &str) -> Result<Arc<dyn Exporter>, DiError> {
        self.module_ref.get_token_in_context::<dyn Exporter>(format!("EXPORTER:{format}"))
    }
}

#[module(providers = [
    Provider::use_class_as::<CsvExporter, dyn Exporter>("EXPORTER:csv", |e| e),
    Provider::use_class_as::<JsonExporter, dyn Exporter>("EXPORTER:json", |e| e),
    ReportService,
])]
pub struct ExportersModule;

#[corvis::test]
async fn the_implementation_can_be_chosen_by_token_at_runtime() {
    let module = compile_ok(Test::create_testing_module().import::<ExportersModule>()).await;
    let service = module.get::<ReportService>().unwrap();
    assert_eq!(service.exporter("csv").unwrap().export(), "csv");
    assert_eq!(service.exporter("json").unwrap().export(), "json");
    assert!(matches!(service.exporter("xml"), Err(DiError::NotFound { .. })));
}

#[injectable]
pub struct SharedImpl {
    #[init(next_id())]
    id: usize,
}
impl UserRepo for SharedImpl {
    fn name(&self) -> String {
        "shared".into()
    }
    fn seed(&self) -> usize {
        self.id
    }
}

#[injectable]
pub struct RepoUserA {
    #[inject("REPO")]
    repo: Arc<dyn UserRepo>,
}

#[injectable]
pub struct RepoUserB {
    #[inject("REPO")]
    repo: Arc<dyn UserRepo>,
}

#[module(providers = [
    Provider::use_class_as::<SharedImpl, dyn UserRepo>("REPO", |r| r).scope(Scope::Transient),
    RepoUserA,
    RepoUserB,
])]
pub struct ScopeOverrideModule;

#[module(providers = [Provider::use_class_as::<SharedImpl, dyn UserRepo>("REPO", |r| r), RepoUserA, RepoUserB])]
pub struct DefaultScopeModule;

#[corvis::test]
async fn provider_scope_overrides_the_scope_of_use_class_as() {
    let module = compile_ok(Test::create_testing_module().import::<ScopeOverrideModule>()).await;
    let a = module.get::<RepoUserA>().unwrap();
    let b = module.get::<RepoUserB>().unwrap();
    assert_ne!(a.repo.seed(), b.repo.seed());

    let module = compile_ok(Test::create_testing_module().import::<DefaultScopeModule>()).await;
    let a = module.get::<RepoUserA>().unwrap();
    let b = module.get::<RepoUserB>().unwrap();
    assert_eq!(a.repo.seed(), b.repo.seed());
}

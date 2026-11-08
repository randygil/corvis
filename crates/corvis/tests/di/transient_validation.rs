//! `core/injection-scopes`: the dependencies of transient providers are checked at startup, without creating them.

use std::sync::atomic::{AtomicUsize, Ordering};

use corvis::DiError;
use corvis::ResolveCtx;
use corvis::di::Dependencies;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{assert_cycle, compile_err, compile_ok};

pub struct Clock;

fn expect_unknown(err: DiError, requester: &str, token: &str, module: &str) {
    match err {
        DiError::UnknownDependencies { deps, token: missing, index, module: in_module, .. } => {
            assert_eq!(missing, token);
            assert_eq!(index, 0);
            assert_eq!(in_module, module);
            assert!(deps.contains('?'), "{deps}");
        }
        other => panic!("expected UnknownDependencies for {requester}, got {other:?}"),
    }
}

// --- Missing dependency of an unused transient provider ---

#[injectable(scope = "transient")]
pub struct OperationTimer {
    _clock: Arc<Clock>,
}

#[module(providers = [OperationTimer], exports = [OperationTimer])]
pub struct UnusedTimerModule;

#[corvis::test]
async fn a_missing_dependency_of_an_unused_transient_provider_fails_startup() {
    let err = compile_err(Test::create_testing_module().import::<UnusedTimerModule>()).await;
    let text = err.to_string();
    assert!(text.contains("Corvis can't resolve dependencies of the OperationTimer (?)"), "{text}");
    assert!(text.contains("argument Clock at index [0] is available in the UnusedTimerModule context"), "{text}");
    expect_unknown(err, "OperationTimer", "Clock", "UnusedTimerModule");
}

#[corvis::test]
async fn the_startup_error_is_the_one_of_the_first_resolution() {
    // The same class registered as a singleton fails during instantiation with the same message.
    #[injectable]
    pub struct SingletonTimer {
        _clock: Arc<Clock>,
    }
    #[module(providers = [SingletonTimer])]
    pub struct SingletonTimerModule;

    let singleton = compile_err(Test::create_testing_module().import::<SingletonTimerModule>()).await.to_string();
    let transient = compile_err(Test::create_testing_module().import::<UnusedTimerModule>()).await.to_string();
    assert_eq!(singleton.replace("SingletonTimerModule", "UnusedTimerModule").replace("SingletonTimer", "OperationTimer"), transient);
}

// --- Not instantiated by the check ---

static CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);

fn construction() -> usize {
    CONSTRUCTED.fetch_add(1, Ordering::SeqCst)
}

#[injectable]
pub struct Available;

#[injectable(scope = "transient")]
pub struct CountedTimer {
    _available: Arc<Available>,
    #[init(construction())]
    _built: usize,
}

#[module(providers = [Available, CountedTimer], exports = [CountedTimer])]
pub struct CountedModule;

#[corvis::test]
async fn the_check_does_not_instantiate_transient_providers() {
    let module = compile_ok(Test::create_testing_module().import::<CountedModule>()).await;
    assert_eq!(CONSTRUCTED.load(Ordering::SeqCst), 0);
    module.resolve::<CountedTimer>().await.unwrap();
    assert_eq!(CONSTRUCTED.load(Ordering::SeqCst), 1);
}

// --- Alias with a missing target ---

#[module(providers = [Provider::use_existing("TIMER", "MISSING").scope(Scope::Transient)])]
pub struct AliasModule;

#[corvis::test]
async fn an_alias_with_a_missing_target_fails_startup() {
    let err = compile_err(Test::create_testing_module().import::<AliasModule>()).await;
    assert!(matches!(&err, DiError::NotFound { token } if token == "MISSING"), "{err:?}");
}

#[module(providers = [OperationTimer, Provider::use_existing("TIMER_ALIAS", Token::of::<OperationTimer>())])]
pub struct AliasOfTransientModule;

#[corvis::test]
async fn an_alias_of_a_transient_provider_checks_the_target() {
    let err = compile_err(Test::create_testing_module().import::<AliasOfTransientModule>()).await;
    expect_unknown(err, "OperationTimer", "Clock", "AliasOfTransientModule");
}

// --- Transitive transient dependency ---

#[injectable(scope = "transient")]
pub struct Report {
    _timer: Arc<OperationTimer>,
}

#[module(providers = [OperationTimer, Report])]
pub struct ReportModule;

#[corvis::test]
async fn a_transitive_transient_dependency_is_checked() {
    let err = compile_err(Test::create_testing_module().import::<ReportModule>()).await;
    let text = err.to_string();
    assert!(text.contains("OperationTimer"), "{text}");
    expect_unknown(err, "OperationTimer", "Clock", "ReportModule");
}

// --- A dependency of another module is checked in its own module ---

#[injectable]
pub struct Secret;

#[injectable(scope = "transient")]
pub struct Vault {
    _secret: Arc<Secret>,
}

#[module(providers = [Secret, Vault], exports = [Vault])]
pub struct VaultModule;

#[injectable(scope = "transient")]
pub struct VaultUser {
    _vault: Arc<Vault>,
}

#[module(imports = [VaultModule], providers = [VaultUser])]
pub struct VaultUserModule;

#[corvis::test]
async fn a_transient_dependency_is_checked_in_its_own_module() {
    // `Secret` is private to VaultModule: VaultUserModule cannot see it, and does not need to.
    compile_ok(Test::create_testing_module().import::<VaultUserModule>()).await;
}

// --- Optional dependencies ---

#[injectable(scope = "transient")]
pub struct OptionalTimer {
    _clock: Option<Arc<Clock>>,
}

#[module(providers = [OptionalTimer])]
pub struct OptionalModule;

#[corvis::test]
async fn an_absent_optional_dependency_is_not_an_error() {
    compile_ok(Test::create_testing_module().import::<OptionalModule>()).await;
}

// --- Cycle of transient providers ---

#[injectable(scope = "transient")]
pub struct Ping {
    _pong: Arc<Pong>,
}

#[injectable(scope = "transient")]
pub struct Pong {
    _ping: Arc<Ping>,
}

#[module(providers = [Ping, Pong])]
pub struct PingPongModule;

#[corvis::test]
async fn a_cycle_of_transient_providers_fails_startup() {
    let err = compile_err(Test::create_testing_module().import::<PingPongModule>()).await;
    assert_cycle(&err, &["Ping", "Pong"]);
}

// --- Factory providers are not checked ---

pub struct Client;

#[module(providers = [
    Provider::use_factory(Token::of::<Client>(), |ctx| async move {
        ctx.get_token::<usize>("MISSING").await?;
        Ok(Client)
    })
    .scope(Scope::Transient),
])]
pub struct FactoryModule;

#[corvis::test]
async fn a_transient_factory_provider_is_not_checked() {
    let module = compile_ok(Test::create_testing_module().import::<FactoryModule>()).await;
    let err = module.resolve::<Client>().await.err().expect("the first resolution fails");
    assert!(matches!(err, DiError::NotFound { .. }), "{err:?}");
}

// --- Overrides ---

#[corvis::test]
async fn a_transient_provider_replaced_by_an_override_is_not_checked() {
    let module = compile_ok(
        Test::create_testing_module()
            .import::<UnusedTimerModule>()
            .override_provider::<OperationTimer>()
            .use_value(OperationTimer { _clock: Arc::new(Clock) }),
    )
    .await;
    assert!(module.resolve::<OperationTimer>().await.is_ok());
}

// --- The description generated by the macros ---

#[injectable]
pub struct A;
#[injectable]
pub struct B;
#[injectable]
pub struct C;

#[injectable]
pub struct Described {
    _a: Arc<A>,
    #[inject("T")]
    _b: Arc<B>,
    #[inject("PORT")]
    _port: u16,
    _c: Option<Arc<C>>,
    _module_ref: ModuleRef,
    #[init(7usize)]
    _seven: usize,
    _fallback: String,
}

#[test]
fn the_macro_describes_the_injected_fields() {
    let Dependencies { requester, names, items } = <Described as Injectable>::dependencies().expect("declared by the macro");
    assert_eq!(requester, "Described");
    assert_eq!(names.len(), 7);
    let seen: Vec<(String, usize, bool)> = items.iter().map(|d| (d.token.display_name(), d.index, d.optional)).collect();
    assert_eq!(
        seen,
        [("A".to_string(), 0, false), ("T".to_string(), 1, false), ("PORT".to_string(), 2, false), ("C".to_string(), 3, true)]
    );
}

#[test]
fn providers_without_a_declaration_are_not_validated() {
    struct Manual;
    impl Injectable for Manual {
        fn construct(_ctx: ResolveCtx) -> corvis::__private::BoxFuture<'static, Result<Self, DiError>> {
            Box::pin(async { Ok(Manual) })
        }
    }
    assert!(Manual::dependencies().is_none());
}

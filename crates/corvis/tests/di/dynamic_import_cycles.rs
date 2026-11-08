//! `core/circular-dependencies`: dynamic modules that import each other are reported with `DiError::CircularImport`
//! instead of overflowing the stack, and graphs that do terminate keep starting.

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{compile_err, compile_ok};

fn circular_chain(err: &DiError) -> Vec<String> {
    match err {
        DiError::CircularImport { chain } => chain.split(" -> ").map(str::to_string).collect(),
        other => panic!("expected DiError::CircularImport, got {other:?}"),
    }
}

// --- Two dynamic modules importing each other ---

#[module(imports = [BModule::for_root()])]
pub struct AModule;

#[module(imports = [AModule::for_root()])]
pub struct BModule;

impl AModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

impl BModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

#[corvis::test]
async fn two_dynamic_modules_importing_each_other() {
    let err = compile_err(Test::create_testing_module().import::<AModule>()).await;
    assert_eq!(circular_chain(&err), ["AModule", "BModule", "AModule"]);
}

#[corvis::test]
async fn the_root_imported_in_its_dynamic_form() {
    let err = compile_err(Test::create_testing_module().import_dynamic(AModule::for_root())).await;
    assert_eq!(circular_chain(&err), ["AModule", "BModule", "AModule"]);
}

#[corvis::test]
async fn the_message_has_the_chain_and_explains_the_fix() {
    let err = compile_err(Test::create_testing_module().import::<AModule>()).await;
    let text = err.to_string();
    assert!(text.contains("AModule -> BModule -> AModule"), "{text}");
    assert!(text.contains("dynamic import"), "{text}");
    assert!(text.contains("separate module"), "{text}");
    assert!(!text.contains("forwardRef"), "{text}");
}

// --- Three dynamic modules in a cycle ---

#[module(imports = [TwoModule::for_root()])]
pub struct OneModule;

#[module(imports = [ThreeModule::for_root()])]
pub struct TwoModule;

#[module(imports = [OneModule::for_root()])]
pub struct ThreeModule;

impl OneModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

impl TwoModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

impl ThreeModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

#[corvis::test]
async fn three_dynamic_modules_in_a_cycle() {
    let err = compile_err(Test::create_testing_module().import::<OneModule>()).await;
    assert_eq!(circular_chain(&err), ["OneModule", "TwoModule", "ThreeModule", "OneModule"]);
}

// --- A module outside the cycle leads into it ---

#[module(imports = [AModule::for_root()])]
pub struct EntryModule;

#[corvis::test]
async fn a_cycle_entered_from_outside_names_only_the_cycle() {
    let err = compile_err(Test::create_testing_module().import::<EntryModule>()).await;
    assert_eq!(circular_chain(&err), ["AModule", "BModule", "AModule"]);
}

// --- Graphs that terminate keep starting ---

#[injectable]
pub struct OrdersService;

#[injectable]
pub struct InvoicesService;

#[module(imports = [BillingModule], providers = [OrdersService], exports = [OrdersService])]
pub struct OrdersModule;

#[module(imports = [OrdersModule], providers = [InvoicesService], exports = [InvoicesService])]
pub struct BillingModule;

#[corvis::test]
async fn static_modules_importing_each_other_still_start() {
    let module = compile_ok(Test::create_testing_module().import::<OrdersModule>()).await;
    assert!(module.get::<OrdersService>().is_ok());
}

#[module(imports = [StaticBridgeModule])]
pub struct LoopbackModule;

#[module(imports = [LoopbackModule::for_root()])]
pub struct StaticBridgeModule;

impl LoopbackModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

#[corvis::test]
async fn a_dynamic_module_reached_again_through_a_static_one_still_starts() {
    compile_ok(Test::create_testing_module().import_dynamic(LoopbackModule::for_root())).await;
}

#[injectable]
pub struct Mailer;

#[module(providers = [Mailer], exports = [Mailer])]
pub struct MailerModule;

impl MailerModule {
    pub fn for_root() -> DynamicModule {
        DynamicModule::new::<Self>()
    }
}

#[module(imports = [MailerModule::for_root()])]
pub struct UsersModule;

#[module(imports = [MailerModule::for_root()])]
pub struct BillingMailModule;

#[corvis::test]
async fn the_same_dynamic_module_in_two_branches_still_starts() {
    compile_ok(Test::create_testing_module().import::<UsersModule>().import::<BillingMailModule>()).await;
}

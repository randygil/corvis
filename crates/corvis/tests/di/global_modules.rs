//! `core/global-modules`: visibility of the exports of global modules.

use corvis::DiError;
use corvis::prelude::*;
use corvis::testing::Test;

use crate::{compile_err, compile_ok, next_id};

// ---------------------------------------------------------------- declaration forms

#[injectable]
pub struct FlagService;
#[injectable]
pub struct ValueService;
#[injectable]
pub struct AttributeService;

#[module(global, providers = [FlagService], exports = [FlagService])]
pub struct FlagModule;

#[module(global = true, providers = [ValueService], exports = [ValueService])]
pub struct ValueModule;

#[module(providers = [AttributeService], exports = [AttributeService])]
#[global]
pub struct AttributeModule;

#[injectable]
pub struct AllThree {
    flag: Arc<FlagService>,
    value: Arc<ValueService>,
    attribute: Arc<AttributeService>,
}

#[module(providers = [AllThree])]
pub struct SiblingModule;

#[corvis::test]
async fn the_three_declaration_forms_declare_a_global_module() {
    let module = compile_ok(
        Test::create_testing_module().import::<FlagModule>().import::<ValueModule>().import::<AttributeModule>().import::<SiblingModule>(),
    )
    .await;
    let all = module.get::<AllThree>().unwrap();
    assert!(Arc::ptr_eq(&all.flag, &module.get::<FlagService>().unwrap()));
    assert!(Arc::ptr_eq(&all.value, &module.get::<ValueService>().unwrap()));
    assert!(Arc::ptr_eq(&all.attribute, &module.get::<AttributeService>().unwrap()));
}

// ---------------------------------------------------------------- visibility

#[injectable]
pub struct AuditService {
    #[init(next_id())]
    id: usize,
}

#[injectable]
pub struct AuditStore;

#[module(global, providers = [AuditService, AuditStore], exports = [AuditService])]
pub struct AuditModule;

#[injectable]
pub struct ChatService {
    audit: Arc<AuditService>,
}

#[module(providers = [ChatService])]
pub struct ChatModule;

#[corvis::test]
async fn global_exports_are_visible_without_importing_the_module() {
    let module = compile_ok(Test::create_testing_module().import::<AuditModule>().import::<ChatModule>()).await;
    let chat = module.get::<ChatService>().unwrap();
    assert!(Arc::ptr_eq(&chat.audit, &module.get::<AuditService>().unwrap()));
}

#[corvis::test]
async fn a_global_module_that_no_module_imports_contributes_nothing() {
    let err = compile_err(Test::create_testing_module().import::<ChatModule>()).await;
    match err {
        DiError::UnknownDependencies { requester, token, .. } => {
            assert_eq!(requester, "ChatService");
            assert_eq!(token, "AuditService");
        }
        other => panic!("expected UnknownDependencies, got {other:?}"),
    }
}

#[injectable]
pub struct StoreUser {
    _store: Arc<AuditStore>,
}

#[module(providers = [StoreUser])]
pub struct StoreUserModule;

#[corvis::test]
async fn only_exported_providers_of_a_global_module_are_shared() {
    let err = compile_err(Test::create_testing_module().import::<AuditModule>().import::<StoreUserModule>()).await;
    match err {
        DiError::UnknownDependencies { token, .. } => assert_eq!(token, "AuditStore"),
        other => panic!("expected UnknownDependencies, got {other:?}"),
    }
}

#[injectable]
pub struct Clock;

#[module(providers = [Clock], exports = [Clock])]
pub struct ClockModule;

#[module(global, imports = [ClockModule], exports = [ClockModule])]
pub struct CoreModule;

#[injectable]
pub struct ClockUser {
    clock: Arc<Clock>,
}

#[module(providers = [ClockUser])]
pub struct ClockUserModule;

#[corvis::test]
async fn a_global_module_can_re_export_a_module() {
    let module = compile_ok(Test::create_testing_module().import::<CoreModule>().import::<ClockUserModule>()).await;
    let user = module.get::<ClockUser>().unwrap();
    assert!(Arc::ptr_eq(&user.clock, &module.get::<Clock>().unwrap()));
}

// ---------------------------------------------------------------- precedence and sharing

#[injectable]
pub struct AuditProbe {
    audit: Arc<AuditService>,
}

#[module(global, providers = [AuditService, AuditProbe], exports = [AuditService])]
pub struct ProbedAuditModule;

#[module(providers = [AuditService, ChatService])]
pub struct ShadowingChatModule;

#[injectable]
pub struct OtherService {
    audit: Arc<AuditService>,
}

#[module(providers = [OtherService])]
pub struct OtherModule;

#[corvis::test]
async fn an_own_provider_shadows_the_global_export() {
    let module =
        compile_ok(Test::create_testing_module().import::<ProbedAuditModule>().import::<ShadowingChatModule>().import::<OtherModule>())
            .await;
    let global = module.get::<AuditProbe>().unwrap().audit.id;
    let chat = module.get::<ChatService>().unwrap().audit.id;
    let other = module.get::<OtherService>().unwrap().audit.id;
    assert_ne!(chat, global, "ChatModule must use its own AuditService");
    assert_eq!(other, global, "other modules must use the global one");
}

#[corvis::test]
async fn consumers_in_different_modules_share_the_global_instance() {
    let module = compile_ok(Test::create_testing_module().import::<AuditModule>().import::<ChatModule>().import::<OtherModule>()).await;
    let chat = module.get::<ChatService>().unwrap();
    let other = module.get::<OtherService>().unwrap();
    assert!(Arc::ptr_eq(&chat.audit, &other.audit));
}

// ---------------------------------------------------------------- DynamicModule::global

#[injectable]
pub struct MailerService;

#[module(providers = [MailerService], exports = [MailerService])]
pub struct MailerModule;

#[injectable]
pub struct MailerUser {
    mailer: Arc<MailerService>,
}

#[module(providers = [MailerUser])]
pub struct MailerUserModule;

#[corvis::test]
async fn global_true_makes_a_non_global_module_global() {
    let module = compile_ok(
        Test::create_testing_module().import_dynamic(DynamicModule::new::<MailerModule>().global(true)).import::<MailerUserModule>(),
    )
    .await;
    let user = module.get::<MailerUser>().unwrap();
    assert!(Arc::ptr_eq(&user.mailer, &module.get::<MailerService>().unwrap()));
}

#[corvis::test]
async fn without_global_a_non_global_module_stays_private() {
    let err =
        compile_err(Test::create_testing_module().import_dynamic(DynamicModule::new::<MailerModule>()).import::<MailerUserModule>()).await;
    assert!(matches!(err, DiError::UnknownDependencies { .. }), "{err:?}");
}

#[injectable]
pub struct AuditUser {
    _audit: Arc<AuditService>,
}

#[module(providers = [AuditUser])]
pub struct AuditUserModule;

#[corvis::test]
async fn global_false_makes_a_global_module_non_global() {
    let err = compile_err(
        Test::create_testing_module().import_dynamic(DynamicModule::new::<AuditModule>().global(false)).import::<AuditUserModule>(),
    )
    .await;
    match err {
        DiError::UnknownDependencies { token, .. } => assert_eq!(token, "AuditService"),
        other => panic!("expected UnknownDependencies, got {other:?}"),
    }
}

#[corvis::test]
async fn without_global_the_attribute_applies() {
    compile_ok(Test::create_testing_module().import_dynamic(DynamicModule::new::<AuditModule>()).import::<AuditUserModule>()).await;
}

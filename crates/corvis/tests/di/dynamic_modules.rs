//! `core/dynamic-modules`: a user-written `DynamicModule` (the `for_root` pattern of the docs).
//!
//! Dynamic modules that import each other are covered by `dynamic_import_cycles`.

use std::future::Future;

use corvis::prelude::*;
use corvis::testing::{Test, request};
use corvis::{DiError, ResolveCtx};

use crate::{compile_err, compile_ok};

#[derive(Clone)]
pub struct MailerOptions {
    pub from: String,
    pub host: String,
}

pub const MAILER_OPTIONS: &str = "MAILER_OPTIONS";

#[injectable]
pub struct MailerService {
    #[inject(MAILER_OPTIONS)]
    options: Arc<MailerOptions>,
}

impl MailerService {
    pub fn sender(&self) -> &str {
        &self.options.from
    }
}

#[controller("mailer-debug")]
pub struct MailerDebugController {
    mailer: Arc<MailerService>,
}

#[routes]
impl MailerDebugController {
    #[get]
    async fn sender(&self) -> String {
        self.mailer.sender().to_string()
    }
}

pub struct Settings {
    from: String,
}

#[module(providers = [MailerService], exports = [MailerService])]
pub struct MailerModule;

fn options(from: &str) -> MailerOptions {
    MailerOptions { from: from.into(), host: "smtp.local".into() }
}

impl MailerModule {
    pub fn for_root(options: MailerOptions) -> DynamicModule {
        DynamicModule::new::<Self>().provider(Provider::use_value(MAILER_OPTIONS, options))
    }

    pub fn for_root_debug(options: MailerOptions, debug: bool) -> DynamicModule {
        let mut module = Self::for_root(options);
        if debug {
            module = module.controller(ControllerDef::of::<MailerDebugController>());
        }
        module
    }

    pub fn for_root_exporting(options: MailerOptions) -> DynamicModule {
        Self::for_root(options).export(MAILER_OPTIONS)
    }

    pub fn for_root_invalid_export(options: MailerOptions) -> DynamicModule {
        Self::for_root(options).export("MISSING")
    }

    pub fn for_root_async<F, Fut>(factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = corvis::Result<MailerOptions>> + Send + 'static,
    {
        DynamicModule::new::<Self>().provider(Provider::use_factory(MAILER_OPTIONS, factory))
    }

    pub fn for_root_async_importing<F, Fut>(factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = corvis::Result<MailerOptions>> + Send + 'static,
    {
        Self::for_root_async(factory).import(ModuleImport::of::<SettingsModule>())
    }
}

async fn options_from_settings(ctx: ResolveCtx) -> corvis::Result<MailerOptions> {
    let settings = ctx.get::<Settings>().await?;
    Ok(options(&settings.from))
}

// ------------------------------------------------------------------ static and dynamic parts

#[injectable]
pub struct SignupService {
    mailer: Arc<MailerService>,
}

#[module(imports = [MailerModule::for_root(options("no-reply@tertulia.dev"))], providers = [SignupService])]
pub struct UsersModule;

#[corvis::test]
async fn the_static_and_dynamic_parts_are_combined() {
    let module = compile_ok(Test::create_testing_module().import::<UsersModule>()).await;
    let signup = module.get::<SignupService>().unwrap();
    assert_eq!(signup.mailer.sender(), "no-reply@tertulia.dev");
    assert_eq!(signup.mailer.options.host, "smtp.local");
}

#[module(imports = [MailerModule::for_root_debug(options("debug@tertulia.dev"), true)])]
pub struct DebugOnModule;

#[module(imports = [MailerModule::for_root_debug(options("debug@tertulia.dev"), false)])]
pub struct DebugOffModule;

#[corvis::test]
async fn a_controller_added_by_the_dynamic_part_is_served() {
    let module = compile_ok(Test::create_testing_module().import::<DebugOnModule>()).await;
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    let res = request(&app).get("/mailer-debug").send().await;
    res.expect_status(200).expect_text("debug@tertulia.dev");
}

#[corvis::test]
async fn without_the_controller_its_routes_are_not_served() {
    let module = compile_ok(Test::create_testing_module().import::<DebugOffModule>()).await;
    let mut app = module.create_corvis_application();
    app.init().await.unwrap();
    request(&app).get("/mailer-debug").send().await.expect_status(404);
}

// ------------------------------------------------------------------ privacy

#[injectable]
pub struct TokenOptionsReader {
    #[inject(MAILER_OPTIONS)]
    options: Arc<MailerOptions>,
}

#[module(imports = [MailerModule::for_root(options("a@x"))], providers = [TokenOptionsReader])]
pub struct PrivateOptionsModule;

#[corvis::test]
async fn options_added_by_the_dynamic_part_are_private() {
    let err = compile_err(Test::create_testing_module().import::<PrivateOptionsModule>()).await;
    match err {
        DiError::UnknownDependencies { requester, token, .. } => {
            assert_eq!(requester, "TokenOptionsReader");
            assert_eq!(token, "MAILER_OPTIONS");
        }
        other => panic!("expected UnknownDependencies, got {other:?}"),
    }
}

#[module(imports = [MailerModule::for_root_exporting(options("exported@x"))], providers = [TokenOptionsReader])]
pub struct ExportedOptionsModule;

#[corvis::test]
async fn the_dynamic_part_can_export_what_it_adds() {
    let module = compile_ok(Test::create_testing_module().import::<ExportedOptionsModule>()).await;
    assert_eq!(module.get::<TokenOptionsReader>().unwrap().options.from, "exported@x");
}

#[module(imports = [MailerModule::for_root_invalid_export(options("a@x"))])]
pub struct InvalidExportModule;

#[corvis::test]
async fn exporting_something_the_module_does_not_provide_fails_at_startup() {
    let err = compile_err(Test::create_testing_module().import::<InvalidExportModule>()).await;
    match err {
        DiError::InvalidExport { module, token } => {
            assert_eq!(module, "MailerModule");
            assert_eq!(token, "MISSING");
        }
        other => panic!("expected InvalidExport, got {other:?}"),
    }
}

// ------------------------------------------------------------------ instances

#[injectable]
pub struct UsersMailer {
    mailer: Arc<MailerService>,
}

#[injectable]
pub struct BillingMailer {
    mailer: Arc<MailerService>,
}

#[module(imports = [MailerModule::for_root(options("users@x"))], providers = [UsersMailer])]
pub struct FirstImporterModule;

#[module(imports = [MailerModule::for_root(options("billing@x"))], providers = [BillingMailer])]
pub struct SecondImporterModule;

#[corvis::test]
async fn each_dynamic_import_is_a_separate_module_instance() {
    let module = compile_ok(Test::create_testing_module().import::<FirstImporterModule>().import::<SecondImporterModule>()).await;
    let users = module.get::<UsersMailer>().unwrap();
    let billing = module.get::<BillingMailer>().unwrap();
    assert!(!Arc::ptr_eq(&users.mailer, &billing.mailer));
    assert_eq!(users.mailer.sender(), "users@x");
    assert_eq!(billing.mailer.sender(), "billing@x");
}

#[injectable]
pub struct GreetingUser {
    #[inject("GREETING")]
    greeting: Arc<String>,
}

#[module(providers = [Provider::use_value("GREETING", "static".to_string()), GreetingUser], exports = [GreetingUser])]
pub struct StaticGreetingModule;

#[corvis::test]
async fn a_provider_of_the_dynamic_part_overrides_a_static_one_with_the_same_token() {
    let module = compile_ok(
        Test::create_testing_module()
            .import_dynamic(DynamicModule::new::<StaticGreetingModule>().provider(Provider::use_value("GREETING", "dynamic".to_string()))),
    )
    .await;
    assert_eq!(*module.get::<GreetingUser>().unwrap().greeting, "dynamic");

    let module = compile_ok(Test::create_testing_module().import::<StaticGreetingModule>()).await;
    assert_eq!(*module.get::<GreetingUser>().unwrap().greeting, "static");
}

// ------------------------------------------------------------------ factory context

#[module(providers = [Provider::value(Settings { from: "settings-module".into() })], exports = [Token::of::<Settings>()])]
pub struct SettingsModule;

#[module(global, providers = [Provider::value(Settings { from: "global-settings".into() })], exports = [Token::of::<Settings>()])]
pub struct GlobalSettingsModule;

#[module(
    imports = [MailerModule::for_root_async(options_from_settings)],
    providers = [Provider::value(Settings { from: "importer".into() }), SignupService],
)]
pub struct ImporterSettingsModule;

#[corvis::test]
async fn a_factory_does_not_see_the_providers_of_the_importing_module() {
    let err = compile_err(Test::create_testing_module().import::<ImporterSettingsModule>()).await;
    match err {
        DiError::NotFound { token } => assert_eq!(token, "Settings"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[module(
    imports = [MailerModule::for_root_async_importing(options_from_settings)],
    providers = [Provider::value(Settings { from: "importer".into() }), SignupService],
)]
pub struct ImportingSettingsModule;

#[corvis::test]
async fn a_factory_sees_the_exports_of_the_imports_of_the_dynamic_module() {
    let module = compile_ok(Test::create_testing_module().import::<ImportingSettingsModule>()).await;
    assert_eq!(module.get::<SignupService>().unwrap().mailer.sender(), "settings-module");
}

#[module(imports = [MailerModule::for_root_async(options_from_settings)], providers = [SignupService])]
pub struct GlobalSettingsUserModule;

#[corvis::test]
async fn a_factory_sees_the_exports_of_global_modules() {
    let module = compile_ok(Test::create_testing_module().import::<GlobalSettingsModule>().import::<GlobalSettingsUserModule>()).await;
    assert_eq!(module.get::<SignupService>().unwrap().mailer.sender(), "global-settings");
}

//! Modules: `@Module({ imports, controllers, providers, exports })`, dynamic modules
//! (`forRoot()` / `forFeature()` / `register()`), global modules and re-exports.

use std::any::TypeId;

use super::injectable::Injectable;
use super::provider::{Instance, Provider};
use super::token::Token;
use crate::common::middleware::MiddlewareConsumer;
use crate::router::controller::ControllerDef;

/// Implemented by `#[module(...)]`.
pub trait Module: Injectable {
    fn definition() -> ModuleDefinition;
}

/// Implemented by modules that configure middleware (`configure(consumer)`).
///
/// ```ignore
/// impl CorvisModule for AppModule {
///     fn configure(&self, consumer: &mut MiddlewareConsumer) {
///         consumer.apply::<LoggerMiddleware>().for_routes("cats");
///     }
/// }
/// ```
pub trait CorvisModule {
    fn configure(&self, consumer: &mut MiddlewareConsumer);
}

pub type ConfigureFn = fn(&Instance, &mut MiddlewareConsumer);

/// The static metadata of a module.
pub struct ModuleDefinition {
    pub name: String,
    pub type_id: TypeId,
    pub imports: Vec<ModuleImport>,
    pub providers: Vec<Provider>,
    pub controllers: Vec<ControllerDef>,
    pub exports: Vec<ExportItem>,
    pub global: bool,
    pub module_provider: Option<Provider>,
    pub configure: Option<ConfigureFn>,
}

impl ModuleDefinition {
    /// Definition for the module class `M`.
    pub fn new<M: 'static>(name: impl Into<String>) -> Self {
        ModuleDefinition {
            name: name.into(),
            type_id: TypeId::of::<M>(),
            imports: Vec::new(),
            providers: Vec::new(),
            controllers: Vec::new(),
            exports: Vec::new(),
            global: false,
            module_provider: None,
            configure: None,
        }
    }

    pub fn imports(mut self, imports: Vec<ModuleImport>) -> Self {
        self.imports.extend(imports);
        self
    }
    pub fn providers(mut self, providers: Vec<Provider>) -> Self {
        self.providers.extend(providers);
        self
    }
    pub fn controllers(mut self, controllers: Vec<ControllerDef>) -> Self {
        self.controllers.extend(controllers);
        self
    }
    pub fn exports(mut self, exports: Vec<ExportItem>) -> Self {
        self.exports.extend(exports);
        self
    }
    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }
    pub fn module_provider(mut self, provider: Provider) -> Self {
        self.module_provider = Some(provider);
        self
    }
    pub fn configure(mut self, configure: Option<ConfigureFn>) -> Self {
        self.configure = configure;
        self
    }
}

/// Something that can be placed in `imports: [...]`.
pub enum ModuleImport {
    Static { type_id: TypeId, definition: fn() -> ModuleDefinition },
    Dynamic(DynamicModule),
}

impl ModuleImport {
    pub fn of<M: Module>() -> Self {
        ModuleImport::Static { type_id: TypeId::of::<M>(), definition: M::definition }
    }
}

impl From<DynamicModule> for ModuleImport {
    fn from(m: DynamicModule) -> Self {
        ModuleImport::Dynamic(m)
    }
}

/// A dynamic module, as returned by `ConfigModule::for_root(...)`.
/// Its metadata is merged with the module's static `#[module]` metadata.
pub struct DynamicModule {
    pub(crate) base: fn() -> ModuleDefinition,
    pub(crate) imports: Vec<ModuleImport>,
    pub(crate) providers: Vec<Provider>,
    pub(crate) controllers: Vec<ControllerDef>,
    pub(crate) exports: Vec<ExportItem>,
    pub(crate) global: Option<bool>,
}

impl DynamicModule {
    pub fn new<M: Module>() -> Self {
        DynamicModule {
            base: M::definition,
            imports: Vec::new(),
            providers: Vec::new(),
            controllers: Vec::new(),
            exports: Vec::new(),
            global: None,
        }
    }
    pub fn import(mut self, import: impl Into<ModuleImport>) -> Self {
        self.imports.push(import.into());
        self
    }
    pub fn imports(mut self, imports: Vec<ModuleImport>) -> Self {
        self.imports.extend(imports);
        self
    }
    pub fn provider(mut self, provider: Provider) -> Self {
        self.providers.push(provider);
        self
    }
    pub fn providers(mut self, providers: Vec<Provider>) -> Self {
        self.providers.extend(providers);
        self
    }
    pub fn controller(mut self, controller: ControllerDef) -> Self {
        self.controllers.push(controller);
        self
    }
    pub fn export(mut self, export: impl Into<ExportItem>) -> Self {
        self.exports.push(export.into());
        self
    }
    pub fn exports(mut self, exports: Vec<ExportItem>) -> Self {
        self.exports.extend(exports);
        self
    }
    /// `global: true` – registers the module in the global scope.
    pub fn global(mut self, global: bool) -> Self {
        self.global = Some(global);
        self
    }
}

/// Something that can be placed in `exports: [...]`: a provider token or a module (re-export).
#[derive(Clone, Debug)]
pub enum ExportItem {
    Token(Token),
    Module(TypeId, &'static str),
}

impl From<Token> for ExportItem {
    fn from(t: Token) -> Self {
        ExportItem::Token(t)
    }
}
impl From<&str> for ExportItem {
    fn from(s: &str) -> Self {
        ExportItem::Token(Token::named(s))
    }
}
impl From<String> for ExportItem {
    fn from(s: String) -> Self {
        ExportItem::Token(Token::named(s))
    }
}
impl From<&Provider> for ExportItem {
    fn from(p: &Provider) -> Self {
        ExportItem::Token(p.token.clone())
    }
}

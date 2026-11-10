//! `PassportModule`: the default strategy of `AuthGuard::default()`.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::di::{DynamicModule, Module, ModuleDefinition, Provider, Token};

/// Options of [`PassportModule::register`], the equivalent of `PassportModule.register({ defaultStrategy })`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassportModuleOptions {
    pub(crate) default_strategies: Vec<String>,
}

impl PassportModuleOptions {
    pub fn new() -> Self {
        Self::default()
    }

    /// The strategy `AuthGuard::default()` runs.
    pub fn default_strategy(mut self, name: impl Into<String>) -> Self {
        self.default_strategies = vec![name.into()];
        self
    }

    /// The strategies `AuthGuard::default()` tries in order (`defaultStrategy: ['jwt', 'apikey']`).
    pub fn default_strategies<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.default_strategies = names.into_iter().map(Into::into).collect();
        self
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Import `PassportModule::register(..)` to configure the default strategy. Strategies themselves are registered
/// with [`Provider::strategy`] in any module, and the [`Authenticator`](super::Authenticator) needs no import.
#[derive(Default)]
pub struct PassportModule;
crate::default_injectable!(PassportModule);

impl Module for PassportModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<PassportModule>("PassportModule").module_provider(Provider::class::<PassportModule>())
    }
}

impl PassportModule {
    /// `PassportModule.register({ defaultStrategy: 'jwt' })`. Two registrations with different defaults stop startup.
    pub fn register(options: PassportModuleOptions) -> DynamicModule {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        DynamicModule::new::<PassportModule>().provider(Provider::use_value(Token::named(format!("PASSPORT_OPTIONS:{n}")), options))
    }
}

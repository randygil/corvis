//! Lifecycle events: `OnModuleInit`, `OnApplicationBootstrap`,
//! `OnModuleDestroy`, `BeforeApplicationShutdown` and `OnApplicationShutdown`.
//!
//! Implement any of these traits on a provider, controller or module (with
//! `#[async_trait]`) and corvis will detect and call them automatically.

use async_trait::async_trait;

/// Called once the host module's dependencies have been resolved.
#[async_trait]
pub trait OnModuleInit: Send + Sync {
    async fn on_module_init(&self) -> crate::Result<()>;
}

/// Called once all modules have been initialized, but before listening for connections.
#[async_trait]
pub trait OnApplicationBootstrap: Send + Sync {
    async fn on_application_bootstrap(&self) -> crate::Result<()>;
}

/// Called after a termination signal (e.g. `SIGTERM`) has been received or `app.close()` was called.
#[async_trait]
pub trait OnModuleDestroy: Send + Sync {
    async fn on_module_destroy(&self) -> crate::Result<()>;
}

/// Called after all `on_module_destroy()` handlers have completed; once complete,
/// all existing connections will be closed.
#[async_trait]
pub trait BeforeApplicationShutdown: Send + Sync {
    async fn before_application_shutdown(&self, signal: Option<String>) -> crate::Result<()>;
}

/// Called after connections close.
#[async_trait]
pub trait OnApplicationShutdown: Send + Sync {
    async fn on_application_shutdown(&self, signal: Option<String>) -> crate::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookKind {
    OnModuleInit,
    OnApplicationBootstrap,
    OnModuleDestroy,
    BeforeApplicationShutdown,
    OnApplicationShutdown,
}

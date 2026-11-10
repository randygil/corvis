//! `SessionModule`: the global module that provides the `SessionManager` and installs `SessionMiddleware`.

use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;

use super::manager::SessionManager;
use super::middleware::SessionMiddleware;
use super::options::SessionOptions;
use crate::di::{DiError, DynamicModule, Injectable, Module, ModuleDefinition, Provider, ResolveCtx, Token};
use crate::logger::Logger;

/// `SessionModule::for_root(options)`: server-side sessions for every route of the application (global module, the
/// port of `app.use(session({ .. }))` with `passport.session()`).
///
/// ```ignore
/// #[module(
///     imports = [SessionModule::for_root(
///         SessionOptions::with_secrets([config.get("SESSION_SECRET")?])?
///             .cookie(SessionCookie::default().max_age(Duration::from_secs(7 * 24 * 3600))),
///     )],
///     providers = [UsersService, Provider::strategy::<SessionStrategy<UserSerializer>>()],
/// )]
/// pub struct AppModule;
/// ```
///
/// The middleware belongs to this module, so it runs after the middleware registered with
/// `app.use_middleware(..)`, which therefore do not see the session; register
/// [`SessionMiddleware`] globally instead when one needs to.
///
/// Invalid options (a short secret, a limit out of range, an unsafe cookie) stop `init()`.
pub struct SessionModule {
    manager: Arc<SessionManager>,
}

impl Injectable for SessionModule {
    fn construct(ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
        async move { Ok(SessionModule { manager: ctx.get::<SessionManager>().await? }) }.boxed()
    }

    fn hooks() -> crate::di::Hooks<Self> {
        crate::di::Hooks {
            on_module_init: Some(|module| {
                async move {
                    let log = Logger::new("SessionModule");
                    log.log(module.manager.describe_cookie());
                    for note in &module.manager.notes {
                        log.warn(note);
                    }
                    Ok(())
                }
                .boxed()
            }),
            ..crate::di::Hooks::default()
        }
    }
}

impl Module for SessionModule {
    fn definition() -> ModuleDefinition {
        ModuleDefinition::new::<SessionModule>("SessionModule").module_provider(Provider::class::<SessionModule>()).configure(Some(
            |_, consumer| {
                consumer.apply::<SessionMiddleware>().for_routes("*");
            },
        ))
    }
}

fn manager_provider<F, Fut>(options: F) -> Provider
where
    F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = crate::Result<SessionOptions>> + Send + 'static,
{
    Provider::use_factory(Token::of::<SessionManager>(), move |ctx| {
        let options = options(ctx);
        async move { Ok(SessionManager::new(options.await?)?) }
    })
}

impl SessionModule {
    /// `SessionModule.forRoot(options)`, global.
    pub fn for_root(options: SessionOptions) -> DynamicModule {
        Self::module(manager_provider(move |_| {
            let options = options.clone();
            async move { Ok(options) }
        }))
    }

    /// `SessionModule.forRootAsync({ useFactory })`, global. The factory can resolve other providers (the
    /// configuration service) and returns the options, or an error that stops the application.
    pub fn for_root_async<F, Fut>(factory: F) -> DynamicModule
    where
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<SessionOptions>> + Send + 'static,
    {
        Self::module(manager_provider(factory))
    }

    fn module(manager: Provider) -> DynamicModule {
        DynamicModule::new::<SessionModule>().provider(manager).export(Token::of::<SessionManager>()).global(true)
    }
}

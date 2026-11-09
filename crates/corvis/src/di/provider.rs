//! Providers: class providers, value providers, factory providers and aliases.
//! Equivalent to `{ provide, useClass | useValue | useFactory | useExisting }`.

use std::any::Any;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;

use super::error::DiError;
use super::injectable::{Dependencies, Hooks, Injectable, ResolveCtx};
use super::token::Token;
use crate::common::filter::ExceptionFilter;
use crate::common::guard::CanActivate;
use crate::common::interceptor::CorvisInterceptor;
use crate::common::pipe::PipeTransform;

/// A type-erased provider instance. Internally always holds an `Arc<T>`.
pub type Instance = Arc<dyn Any + Send + Sync>;

pub(crate) fn erase<T: ?Sized + Send + Sync + 'static>(value: Arc<T>) -> Instance {
    Arc::new(value)
}

pub(crate) fn downcast<T: ?Sized + Send + Sync + 'static>(instance: &Instance) -> Option<Arc<T>> {
    instance.downcast_ref::<Arc<T>>().cloned()
}

/// Injection scope of a provider.
///
/// New scopes may be added, so match it with a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Scope {
    /// A single instance shared across the entire application (singleton).
    #[default]
    Default,
    /// A new, dedicated instance is created for each consumer.
    Transient,
    /// At most one instance per HTTP request (and per module that registers the provider), built on first use and
    /// released when the request ends.
    ///
    /// The instances live in a store that belongs to the request, never in the container: they are not created at
    /// startup, `ModuleRef::get` cannot see them and lifecycle hooks are not called on them. A request-scoped provider
    /// may depend on singletons, transient providers and other request-scoped providers, and may inject the current
    /// [`Request`](crate::Request) as a field. Singletons, controllers and gateways cannot depend on it (startup
    /// error); consume it per request with `ModuleRef::resolve_for`.
    Request,
}

/// What a provider factory returns: the instance consumers receive and, when that instance is a view of a
/// concrete class (`use_class_as`), the concrete `Arc<T>` that hooks, handlers and gateways run against.
pub(crate) struct Built {
    pub instance: Instance,
    pub hook_target: Option<Instance>,
}

impl From<Instance> for Built {
    fn from(instance: Instance) -> Self {
        Built { instance, hook_target: None }
    }
}

pub(crate) type Factory = Arc<dyn Fn(ResolveCtx) -> BoxFuture<'static, Result<Built, DiError>> + Send + Sync>;
pub(crate) type ErasedHook = Arc<dyn Fn(Instance) -> BoxFuture<'static, crate::Result<()>> + Send + Sync>;
pub(crate) type ErasedSignalHook = Arc<dyn Fn(Instance, Option<String>) -> BoxFuture<'static, crate::Result<()>> + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct ErasedHooks {
    pub on_module_init: Option<ErasedHook>,
    pub on_application_bootstrap: Option<ErasedHook>,
    pub on_module_destroy: Option<ErasedHook>,
    pub before_application_shutdown: Option<ErasedSignalHook>,
    pub on_application_shutdown: Option<ErasedSignalHook>,
    pub handlers: Option<ErasedExplorer>,
    // Only read by the WebSocket runtime; set by every provider so the hook table stays the same.
    #[cfg_attr(not(feature = "websockets"), allow(dead_code))]
    pub gateway: Option<ErasedGateway>,
}

pub(crate) type ErasedGateway = Arc<dyn Fn(Instance) -> Option<crate::discovery::GatewayInfo> + Send + Sync>;

pub(crate) type ErasedExplorer = Arc<dyn Fn(Instance) -> Vec<crate::discovery::DiscoveredHandler> + Send + Sync>;

type TypedHook<T> = fn(Arc<T>) -> BoxFuture<'static, crate::Result<()>>;
type TypedSignalHook<T> = fn(Arc<T>, Option<String>) -> BoxFuture<'static, crate::Result<()>>;

impl ErasedHooks {
    pub(crate) fn from_typed<T: Send + Sync + 'static>(hooks: Hooks<T>) -> Self {
        fn wrap<T: Send + Sync + 'static>(f: Option<TypedHook<T>>) -> Option<ErasedHook> {
            f.map(|f| {
                Arc::new(move |inst: Instance| match downcast::<T>(&inst) {
                    Some(t) => f(t),
                    None => async { Ok(()) }.boxed(),
                }) as ErasedHook
            })
        }
        fn wrap_sig<T: Send + Sync + 'static>(f: Option<TypedSignalHook<T>>) -> Option<ErasedSignalHook> {
            f.map(|f| {
                Arc::new(move |inst: Instance, sig: Option<String>| match downcast::<T>(&inst) {
                    Some(t) => f(t, sig),
                    None => async { Ok(()) }.boxed(),
                }) as ErasedSignalHook
            })
        }
        ErasedHooks {
            on_module_init: wrap(hooks.on_module_init),
            on_application_bootstrap: wrap(hooks.on_application_bootstrap),
            on_module_destroy: wrap(hooks.on_module_destroy),
            before_application_shutdown: wrap_sig(hooks.before_application_shutdown),
            on_application_shutdown: wrap_sig(hooks.on_application_shutdown),
            handlers: hooks.handlers.map(|f| {
                Arc::new(move |inst: Instance| match downcast::<T>(&inst) {
                    Some(t) => f(t),
                    None => Vec::new(),
                }) as ErasedExplorer
            }),
            gateway: hooks.gateway.map(|f| Arc::new(move |inst: Instance| downcast::<T>(&inst).map(f)) as ErasedGateway),
        }
    }
}

/// Global enhancer registered through the DI container
/// (`APP_GUARD`, `APP_INTERCEPTOR`, `APP_PIPE`, `APP_FILTER`).
///
/// `cast` turns the provider's instance into the enhancer. It accepts the concrete class `T` and the
/// trait object itself, which is what a testing override stores. `override_token` is the token a
/// testing module's `override_guard::<T>()` (and siblings) registers its mock under.
#[derive(Clone)]
pub(crate) enum GlobalEnhancer {
    Guard { cast: fn(&Instance) -> Option<Arc<dyn CanActivate>>, override_token: Token },
    Interceptor { cast: fn(&Instance) -> Option<Arc<dyn CorvisInterceptor>>, override_token: Token },
    Pipe { cast: fn(&Instance) -> Option<Arc<dyn PipeTransform>>, override_token: Token },
    Filter { cast: fn(&Instance) -> Option<Arc<dyn ExceptionFilter>>, override_token: Token },
}

impl GlobalEnhancer {
    /// Token under which a testing override of this enhancer is registered.
    pub(crate) fn override_token(&self) -> &Token {
        match self {
            GlobalEnhancer::Guard { override_token, .. }
            | GlobalEnhancer::Interceptor { override_token, .. }
            | GlobalEnhancer::Pipe { override_token, .. }
            | GlobalEnhancer::Filter { override_token, .. } => override_token,
        }
    }

    /// `guard`, `interceptor`, `pipe` or `filter`.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            GlobalEnhancer::Guard { .. } => "guard",
            GlobalEnhancer::Interceptor { .. } => "interceptor",
            GlobalEnhancer::Pipe { .. } => "pipe",
            GlobalEnhancer::Filter { .. } => "filter",
        }
    }
}

/// What a provider is known to depend on without running it. An alias (`use_existing`) declares its target
/// through `Provider::alias_of` instead.
#[derive(Clone, Debug)]
pub(crate) enum DeclaredDeps {
    /// Factory providers and classes that do not declare their fields: the dependencies are code.
    Unknown,
    /// The injected fields of a class (empty for a value).
    Fields(Dependencies),
}

impl DeclaredDeps {
    fn of_class<T: Injectable>() -> Self {
        T::dependencies().map_or(DeclaredDeps::Unknown, DeclaredDeps::Fields)
    }
}

/// A provider definition. Build one with [`Provider::class`], [`Provider::use_value`],
/// [`Provider::use_factory`], [`Provider::use_class`] or [`Provider::use_existing`].
#[derive(Clone)]
pub struct Provider {
    pub(crate) token: Token,
    pub(crate) name: String,
    pub(crate) factory: Factory,
    pub(crate) scope: Scope,
    pub(crate) hooks: ErasedHooks,
    pub(crate) enhancer: Option<GlobalEnhancer>,
    /// Set by `use_existing`: the token this provider is an alias of.
    pub(crate) alias_of: Option<Token>,
    /// What the provider is known to depend on, for the startup validation of transient providers.
    pub(crate) deps: DeclaredDeps,
}

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider").field("token", &self.token).field("scope", &self.scope).finish()
    }
}

impl Provider {
    fn new(token: Token, name: String, factory: Factory) -> Self {
        Provider {
            token,
            name,
            factory,
            scope: Scope::Default,
            hooks: ErasedHooks::default(),
            enhancer: None,
            alias_of: None,
            deps: DeclaredDeps::Unknown,
        }
    }

    /// Standard class provider: `providers: [CatsService]`.
    pub fn class<T: Injectable>() -> Self {
        Self::use_class::<T>(Token::of::<T>())
    }

    /// `{ provide: token, useClass: T }`. The consumer must request `Arc<T>`.
    pub fn use_class<T: Injectable>(token: impl Into<Token>) -> Self {
        let token = token.into();
        let factory: Factory = Arc::new(|ctx: ResolveCtx| {
            async move {
                let value = T::construct(ctx).await?;
                Ok(erase(Arc::new(value)).into())
            }
            .boxed()
        });
        let mut p = Self::new(token, crate::di::token::short_type_name(std::any::type_name::<T>()), factory);
        p.scope = T::scope();
        p.hooks = ErasedHooks::from_typed(T::hooks());
        p.deps = DeclaredDeps::of_class::<T>();
        p
    }

    /// `{ provide: token, useClass: T }` exposed as a trait object `U`
    /// (e.g. `Provider::use_class_as::<PgRepo, dyn Repo>("REPO", |r| r)`). The hooks of `T` (lifecycle hooks,
    /// `#[handlers]`, gateway metadata) run on the concrete instance, as with [`Provider::use_class`].
    pub fn use_class_as<T: Injectable, U: ?Sized + Send + Sync + 'static>(token: impl Into<Token>, cast: fn(Arc<T>) -> Arc<U>) -> Self {
        let token = token.into();
        let factory: Factory = Arc::new(move |ctx: ResolveCtx| {
            async move {
                let concrete = Arc::new(T::construct(ctx).await?);
                Ok(Built { instance: erase(cast(concrete.clone())), hook_target: Some(erase(concrete)) })
            }
            .boxed()
        });
        let mut p = Self::new(token, crate::di::token::short_type_name(std::any::type_name::<T>()), factory);
        p.scope = T::scope();
        p.hooks = ErasedHooks::from_typed(T::hooks());
        p.deps = DeclaredDeps::of_class::<T>();
        p
    }

    /// `{ provide: token, useValue: value }`. The consumer requests `Arc<T>`.
    pub fn use_value<T: Send + Sync + 'static>(token: impl Into<Token>, value: T) -> Self {
        Self::use_value_arc(token, Arc::new(value))
    }

    /// Value provider keyed by its own type (handy for mocks): `Provider::value(MyConfig { .. })`.
    pub fn value<T: Send + Sync + 'static>(value: T) -> Self {
        Self::use_value(Token::of::<T>(), value)
    }

    /// `{ provide: token, useValue }` with an already shared (possibly unsized) value,
    /// e.g. `Arc<dyn Repo>`.
    pub fn use_value_arc<T: ?Sized + Send + Sync + 'static>(token: impl Into<Token>, value: Arc<T>) -> Self {
        let token = token.into();
        let instance = erase(value);
        let name = token.display_name();
        let factory: Factory = Arc::new(move |_ctx| {
            let instance = instance.clone();
            async move { Ok(instance.into()) }.boxed()
        });
        let mut p = Self::new(token, name, factory);
        p.deps = DeclaredDeps::Fields(Dependencies { requester: "", names: &[], items: Vec::new() });
        p
    }

    /// `{ provide: token, useFactory: async (deps) => value, inject: [...] }`.
    /// Dependencies are obtained from the [`ResolveCtx`] given to the factory.
    pub fn use_factory<T, F, Fut>(token: impl Into<Token>, factory: F) -> Self
    where
        T: Send + Sync + 'static,
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<T>> + Send + 'static,
    {
        Self::use_factory_arc(token, move |ctx| {
            let fut = factory(ctx);
            async move { fut.await.map(Arc::new) }
        })
    }

    /// Like [`Provider::use_factory`] but the factory returns an `Arc<T>` (allows trait objects).
    pub fn use_factory_arc<T, F, Fut>(token: impl Into<Token>, factory: F) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
        F: Fn(ResolveCtx) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = crate::Result<Arc<T>>> + Send + 'static,
    {
        let token = token.into();
        let name = token.display_name();
        let token_for_err = token.clone();
        let factory = Arc::new(factory);
        let f: Factory = Arc::new(move |ctx: ResolveCtx| {
            let factory = factory.clone();
            let token = token_for_err.clone();
            async move {
                match factory(ctx).await {
                    Ok(v) => Ok(erase(v).into()),
                    Err(e) => match e.downcast::<DiError>() {
                        Ok(di) => Err(di),
                        Err(e) => Err(DiError::Factory { token: token.display_name(), cause: Arc::new(e) }),
                    },
                }
            }
            .boxed()
        });
        Self::new(token, name, f)
    }

    /// `{ provide: alias, useExisting: existing }`.
    pub fn use_existing(token: impl Into<Token>, existing: impl Into<Token>) -> Self {
        let token = token.into();
        let existing = existing.into();
        let name = token.display_name();
        let target = existing.clone();
        let f: Factory = Arc::new(move |ctx: ResolveCtx| {
            let existing = existing.clone();
            async move { ctx.resolve_instance(existing).await.map(Built::from) }.boxed()
        });
        let mut p = Self::new(token, name, f);
        p.alias_of = Some(target);
        p
    }

    /// Change the injection scope of the provider.
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    pub fn token(&self) -> &Token {
        &self.token
    }

    /// `{ provide: APP_GUARD, useClass: T }` – a global guard with dependency injection.
    pub fn app_guard<T: Injectable + CanActivate>() -> Self {
        let mut p = Self::use_class::<T>(Token::named(format!("APP_GUARD:{}", std::any::type_name::<T>())));
        p.enhancer = Some(GlobalEnhancer::Guard {
            cast: |i| downcast::<T>(i).map(|t| t as Arc<dyn CanActivate>).or_else(|| downcast::<dyn CanActivate>(i)),
            override_token: crate::testing::enhancer_override_token::<T>(),
        });
        p
    }

    /// `{ provide: APP_INTERCEPTOR, useClass: T }`.
    pub fn app_interceptor<T: Injectable + CorvisInterceptor>() -> Self {
        let mut p = Self::use_class::<T>(Token::named(format!("APP_INTERCEPTOR:{}", std::any::type_name::<T>())));
        p.enhancer = Some(GlobalEnhancer::Interceptor {
            cast: |i| downcast::<T>(i).map(|t| t as Arc<dyn CorvisInterceptor>).or_else(|| downcast::<dyn CorvisInterceptor>(i)),
            override_token: crate::testing::enhancer_override_token::<T>(),
        });
        p
    }

    /// `{ provide: APP_PIPE, useClass: T }`.
    pub fn app_pipe<T: Injectable + PipeTransform>() -> Self {
        let mut p = Self::use_class::<T>(Token::named(format!("APP_PIPE:{}", std::any::type_name::<T>())));
        p.enhancer = Some(GlobalEnhancer::Pipe {
            cast: |i| downcast::<T>(i).map(|t| t as Arc<dyn PipeTransform>).or_else(|| downcast::<dyn PipeTransform>(i)),
            override_token: crate::testing::enhancer_override_token::<T>(),
        });
        p
    }

    /// `{ provide: APP_FILTER, useClass: T }`.
    pub fn app_filter<T: Injectable + ExceptionFilter>() -> Self {
        let mut p = Self::use_class::<T>(Token::named(format!("APP_FILTER:{}", std::any::type_name::<T>())));
        p.enhancer = Some(GlobalEnhancer::Filter {
            cast: |i| downcast::<T>(i).map(|t| t as Arc<dyn ExceptionFilter>).or_else(|| downcast::<dyn ExceptionFilter>(i)),
            override_token: crate::testing::enhancer_override_token::<T>(),
        });
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain;
    impl Injectable for Plain {
        fn construct(_ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
            async { Ok(Plain) }.boxed()
        }
    }

    struct Declared;
    impl Injectable for Declared {
        fn construct(_ctx: ResolveCtx) -> BoxFuture<'static, Result<Self, DiError>> {
            async { Ok(Declared) }.boxed()
        }
        fn dependencies() -> Option<Dependencies> {
            Some(Dependencies { requester: "Declared", names: &[], items: Vec::new() })
        }
    }

    #[test]
    fn every_constructor_records_what_it_knows_about_dependencies() {
        assert!(matches!(Provider::class::<Plain>().deps, DeclaredDeps::Unknown));
        assert!(matches!(Provider::class::<Declared>().deps, DeclaredDeps::Fields(_)));
        assert!(matches!(Provider::use_class::<Declared>("TOKEN").deps, DeclaredDeps::Fields(_)));
        assert!(matches!(Provider::use_class_as::<Declared, Declared>("TOKEN", |d| d).deps, DeclaredDeps::Fields(_)));
        assert!(matches!(Provider::use_value("TOKEN", 1u8).deps, DeclaredDeps::Fields(ref d) if d.items.is_empty()));
        assert!(matches!(Provider::use_factory("TOKEN", |_ctx| async { Ok(1u8) }).deps, DeclaredDeps::Unknown));
        let alias = Provider::use_existing("ALIAS", "TOKEN");
        assert!(matches!(alias.deps, DeclaredDeps::Unknown));
        assert_eq!(alias.alias_of, Some(Token::from("TOKEN")));
    }
}

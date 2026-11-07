//! References to enhancers (guards, interceptors, pipes, filters, middleware) used in
//! decorators: either a class resolved through DI (`#[use_guards(AuthGuard)]`) or an
//! instance (`#[use_guards(RolesGuard::new("admin"))]`).

use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::di::{DiError, Injectable, ResolveCtx, Token};

pub enum EnhancerRef<T: ?Sized> {
    Instance(Arc<T>),
    Lazy { token: Token, factory: fn(ResolveCtx) -> BoxFuture<'static, Result<Arc<T>, DiError>> },
}

impl<T: ?Sized> Clone for EnhancerRef<T> {
    fn clone(&self) -> Self {
        match self {
            EnhancerRef::Instance(i) => EnhancerRef::Instance(i.clone()),
            EnhancerRef::Lazy { token, factory } => EnhancerRef::Lazy { token: token.clone(), factory: *factory },
        }
    }
}

impl<T: ?Sized + Send + Sync + 'static> EnhancerRef<T> {
    pub fn instance(value: Arc<T>) -> Self {
        EnhancerRef::Instance(value)
    }

    pub(crate) async fn resolve(&self, ctx: ResolveCtx) -> Result<Arc<T>, DiError> {
        match self {
            EnhancerRef::Instance(i) => Ok(i.clone()),
            EnhancerRef::Lazy { factory, .. } => factory(ctx).await,
        }
    }
}

macro_rules! enhancer_ctor {
    ($fn_of:ident, $fn_inst:ident, $alias:ident, $tr:path) => {
        pub type $alias = EnhancerRef<dyn $tr>;

        /// Reference to an enhancer class resolved through dependency injection.
        pub fn $fn_of<E: Injectable + $tr>() -> $alias {
            EnhancerRef::Lazy {
                token: Token::of::<E>(),
                factory: |ctx: ResolveCtx| {
                    async move {
                        let ov = crate::testing::enhancer_override_token::<E>();
                        if let Some(o) = ctx.get_optional::<dyn $tr>(ov).await? {
                            return Ok(o);
                        }
                        let e: Arc<E> = ctx.get_or_construct::<E>().await?;
                        Ok(e as Arc<dyn $tr>)
                    }
                    .boxed()
                },
            }
        }

        /// Reference to an already constructed enhancer instance.
        pub fn $fn_inst<E: $tr>(value: E) -> $alias {
            EnhancerRef::Instance(Arc::new(value) as Arc<dyn $tr>)
        }
    };
}

enhancer_ctor!(guard_of, guard_instance, GuardRef, crate::common::guard::CanActivate);
enhancer_ctor!(interceptor_of, interceptor_instance, InterceptorRef, crate::common::interceptor::CorvisInterceptor);
enhancer_ctor!(pipe_of, pipe_instance, PipeRef, crate::common::pipe::PipeTransform);
enhancer_ctor!(filter_of, filter_instance, FilterRef, crate::common::filter::ExceptionFilter);
enhancer_ctor!(middleware_of, middleware_instance, MiddlewareRef, crate::common::middleware::CorvisMiddleware);

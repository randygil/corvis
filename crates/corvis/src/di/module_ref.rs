//! `ModuleRef`: programmatic access to the container.

use std::sync::Arc;

use super::container::{Container, ModuleId};
use super::error::DiError;
use super::injectable::{Injectable, ResolveCtx};
use super::provider::downcast;
use super::request_scope::RequestCtx;
use super::token::Token;

#[derive(Clone)]
pub struct ModuleRef {
    pub(crate) container: Arc<Container>,
    pub(crate) module: ModuleId,
}

impl ModuleRef {
    /// Retrieves an existing instance (`moduleRef.get(CatsService, { strict: false })`).
    /// Looks in the current module context first, then in the whole application.
    pub fn get<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.get_token::<T>(Token::of::<T>())
    }

    /// Retrieves an existing instance by token.
    pub fn get_token<T: ?Sized + Send + Sync + 'static>(&self, token: impl Into<Token>) -> Result<Arc<T>, DiError> {
        let token = token.into();
        let instance = match self.container.lookup(self.module, &token) {
            Some((owner, _)) => self.container.instance(owner, &token),
            None => None,
        }
        .or_else(|| self.container.find_instance(&token))
        .ok_or_else(|| DiError::NotFound { token: token.display_name() })?;
        downcast::<T>(&instance)
            .ok_or_else(|| DiError::TypeMismatch { token: token.display_name(), expected: std::any::type_name::<T>().to_string() })
    }

    /// Retrieves an instance by token as constructor injection would see it: providers of the
    /// current module, exports of its imports and global modules (no application-wide fallback).
    pub fn get_token_in_context<T: ?Sized + Send + Sync + 'static>(&self, token: impl Into<Token>) -> Result<Arc<T>, DiError> {
        let token = token.into();
        let instance = self
            .container
            .lookup(self.module, &token)
            .and_then(|(owner, _)| self.container.instance(owner, &token))
            .ok_or_else(|| DiError::NotFound { token: token.display_name() })?;
        downcast::<T>(&instance)
            .ok_or_else(|| DiError::TypeMismatch { token: token.display_name(), expected: std::any::type_name::<T>().to_string() })
    }

    /// Retrieves an instance only from the current module (`{ strict: true }`).
    pub fn get_strict<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        let token = Token::of::<T>();
        let instance = self.container.instance(self.module, &token).ok_or_else(|| DiError::NotFound { token: token.display_name() })?;
        downcast::<T>(&instance)
            .ok_or_else(|| DiError::TypeMismatch { token: token.display_name(), expected: std::any::type_name::<T>().to_string() })
    }

    /// Resolves a provider (creating a new instance for transient providers).
    pub async fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.ctx().get::<T>().await
    }

    /// Resolves a provider of any scope for a request that is being handled: the stored instance of a singleton, a new
    /// instance of a transient provider and, for a request-scoped provider, the instance of that request (created on
    /// first use). The provider must be visible from this module, as for constructor injection.
    ///
    /// The request must be one the application is dispatching (the one a handler, guard, interceptor, pipe, filter or
    /// middleware receives, or a clone of it while the request is in flight). Otherwise, for example with a request
    /// built by hand, after the response has been sent, or in a WebSocket message handler, it fails with
    /// [`DiError::OutsideRequest`]. The application only keeps a per-request store when at least one provider is
    /// request-scoped.
    ///
    /// ```ignore
    /// #[get("/me")]
    /// async fn me(&self, #[req] req: Request) -> String {
    ///     let ctx = self.module_ref.resolve_for::<RequestContext>(&req).await.expect("request-scoped provider");
    ///     ctx.user_id()
    /// }
    /// ```
    pub async fn resolve_for<T: ?Sized + Send + Sync + 'static>(&self, request: &crate::http::Request) -> Result<Arc<T>, DiError> {
        self.resolve_token_for::<T>(Token::of::<T>(), request).await
    }

    /// Like [`ModuleRef::resolve_for`], for a provider registered under a custom token.
    pub async fn resolve_token_for<T: ?Sized + Send + Sync + 'static>(
        &self,
        token: impl Into<Token>,
        request: &crate::http::Request,
    ) -> Result<Arc<T>, DiError> {
        let token = token.into();
        let Some(request) = RequestCtx::of(request) else {
            return Err(DiError::OutsideRequest { token: token.display_name(), chain: String::new() });
        };
        let mut ctx = self.ctx();
        ctx.request = Some(request);
        ctx.get_token::<T>(token).await
    }

    /// Dynamically instantiates a class that was not registered as a provider
    /// (`moduleRef.create(CatsFactory)`).
    pub async fn create<T: Injectable>(&self) -> Result<T, DiError> {
        T::construct(self.ctx()).await
    }

    fn ctx(&self) -> ResolveCtx {
        ResolveCtx {
            container: self.container.clone(),
            module: self.module,
            requester: Arc::from("ModuleRef"),
            deps: &[],
            // Inside a constructor the resolution continues its path, so a cycle through `ModuleRef` is detected.
            path: super::container::current_resolution_path(),
            request: None,
        }
    }

    pub fn module_name(&self) -> String {
        self.container.module_name(self.module)
    }
}

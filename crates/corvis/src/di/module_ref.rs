//! `ModuleRef`: programmatic access to the container.

use std::sync::Arc;

use super::container::{Container, ModuleId};
use super::error::DiError;
use super::injectable::{Injectable, ResolveCtx};
use super::provider::downcast;
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
        downcast::<T>(&instance).ok_or_else(|| DiError::TypeMismatch {
            token: token.display_name(),
            expected: std::any::type_name::<T>().to_string(),
        })
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
        downcast::<T>(&instance).ok_or_else(|| DiError::TypeMismatch {
            token: token.display_name(),
            expected: std::any::type_name::<T>().to_string(),
        })
    }

    /// Retrieves an instance only from the current module (`{ strict: true }`).
    pub fn get_strict<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        let token = Token::of::<T>();
        let instance = self
            .container
            .instance(self.module, &token)
            .ok_or_else(|| DiError::NotFound { token: token.display_name() })?;
        downcast::<T>(&instance).ok_or_else(|| DiError::TypeMismatch {
            token: token.display_name(),
            expected: std::any::type_name::<T>().to_string(),
        })
    }

    /// Resolves a provider (creating a new instance for transient providers).
    pub async fn resolve<T: ?Sized + Send + Sync + 'static>(&self) -> Result<Arc<T>, DiError> {
        self.ctx().get::<T>().await
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
            path: Arc::new(Vec::new()),
        }
    }

    pub fn module_name(&self) -> String {
        self.container.module_name(self.module)
    }
}

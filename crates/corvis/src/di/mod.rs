//! Dependency injection: tokens, providers, modules and the IoC container.

pub mod container;
pub mod error;
pub mod injectable;
pub mod module;
pub mod module_ref;
pub mod provider;
pub mod token;

pub use container::{Container, ModuleId};
pub use error::DiError;
#[doc(hidden)]
pub use injectable::{Dependencies, Dependency};
pub use injectable::{Hooks, InjectField, Injectable, ResolveCtx};
pub use module::{ConfigureFn, CorvisModule, DynamicModule, ExportItem, Module, ModuleDefinition, ModuleImport};
pub use module_ref::ModuleRef;
pub use provider::{Instance, Provider, Scope};
pub use token::Token;

use std::sync::Arc;

/// Errors raised by the dependency injection container. The messages mirror
/// the usual wording of dependency injection containers in the Node.js ecosystem.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DiError {
    #[error(
        "Corvis can't resolve dependencies of the {requester} ({deps}). Please make sure that the argument {token} at index [{index}] is available in the {module} context.\n\nPotential solutions:\n- Is {module} a valid module?\n- If {token} is a provider, is it part of the current {module}?\n- If {token} is exported from a separate @Module, is that module imported within {module}?\n  #[module(imports = [ /* the Module containing {token} */ ])]"
    )]
    UnknownDependencies { requester: String, deps: String, token: String, index: usize, module: String },

    #[error("Corvis could not find {token} element (this provider does not exist in the current context)")]
    NotFound { token: String },

    #[error(
        "A circular dependency has been detected: {chain}. Please, make sure that each side of a bidirectional relationship is decorated with \"forwardRef()\" (or refactor the dependency graph)."
    )]
    Circular { chain: String },

    #[error("The provider {token} was found but it is not of the requested type {expected}")]
    TypeMismatch { token: String, expected: String },

    #[error(
        "Corvis cannot export a provider/module that is not a part of the currently processed module ({module}). Please verify whether the exported {token} is available in this particular context."
    )]
    InvalidExport { module: String, token: String },

    #[error("Error while instantiating {token}: {cause}")]
    Factory { token: String, cause: Arc<anyhow::Error> },

    #[error("{0}")]
    Other(String),
}

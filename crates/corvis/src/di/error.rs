use std::sync::Arc;

/// Errors raised by the dependency injection container.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DiError {
    #[error(
        "Corvis can't resolve dependencies of the {requester} ({deps}). Please make sure that the argument {token} at index [{index}] is available in the {module} context.\n\nPotential solutions:\n- Is {module} a valid module?\n- If {token} is a provider, is it part of the current {module}?\n- If {token} is exported from a separate #[module], is that module imported within {module}?\n  #[module(imports = [ /* the Module containing {token} */ ])]"
    )]
    UnknownDependencies { requester: String, deps: String, token: String, index: usize, module: String },

    #[error("Corvis could not find {token} element (this provider does not exist in the current context)")]
    NotFound { token: String },

    #[error(
        "A circular dependency has been detected: {chain}. Break the cycle: move the shared logic into a third provider, resolve one side lazily with ModuleRef (not in its constructor), or communicate through EventEmitter. See \"Circular dependencies\" in the docs."
    )]
    Circular { chain: String },

    /// Dynamic modules import each other (directly or through others), so scanning would never end. It is raised when
    /// one module class imports the same class dynamically a second time on one import chain. A graph that repeats
    /// such an edge but would still end is rejected too. `chain` names the modules of the cycle in order.
    #[error(
        "A circular import between dynamic modules has been detected: {chain}. Each dynamic import (for example for_root()) creates a new module instance, so this cycle never ends. Move what both modules need into a separate module that both import, or remove one of the two imports."
    )]
    CircularImport { chain: String },

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

#[cfg(test)]
mod tests {
    use super::DiError;

    #[test]
    fn circular_import_message_names_the_chain_and_the_dynamic_import() {
        let text = DiError::CircularImport { chain: "AModule -> BModule -> AModule".into() }.to_string();
        assert!(text.contains("AModule -> BModule -> AModule"));
        assert!(text.contains("dynamic import"));
        assert!(!text.contains("forwardRef"));
    }
}

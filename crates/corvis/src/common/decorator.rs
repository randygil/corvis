//! Custom decorators: reusable bundles of metadata (`applyDecorators(...)` and decorator
//! factories in decorator-based frameworks).
//!
//! A [`CustomDecorator`] is a list of `(key, value)` metadata entries. It can be attached to a
//! controller (below `#[controller]`) or to a route handler (inside `#[routes]`) with
//! `#[apply_decorators(expr, ...)]`:
//!
//! ```ignore
//! fn admin_only() -> CustomDecorator {
//!     CustomDecorator::new().set_metadata("roles", ["admin"]).set_metadata("audit", true)
//! }
//!
//! #[routes]
//! impl UsersController {
//!     #[delete(":id")]
//!     #[apply_decorators(admin_only())]
//!     fn remove(&self, #[param("id")] id: String) {}
//! }
//! ```
//!
//! Attributes whose name starts with `api_` (`#[api_tags("cats")]`, `#[api_operation(...)]`,
//! ...) are forwarded by `#[controller]` / `#[routes]` to a function-like macro with the same
//! name (`api_tags!("cats")`) that must evaluate to a `CustomDecorator`. This is how
//! `corvis-swagger` implements its OpenAPI decorators (`api_tags!`, `api_property!`, ...).
//!
//! Entries keep their declaration order (top to bottom). Readers that need the TypeScript decorator
//! application order (bottom to top) can iterate them in reverse.

use serde::Serialize;
use serde_json::Value;

/// A reusable set of metadata entries (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomDecorator {
    entries: Vec<(String, Value)>,
}

impl CustomDecorator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `SetMetadata(key, value)`.
    pub fn set_metadata(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        self.entries.push((key.into(), serde_json::to_value(value).unwrap_or(Value::Null)));
        self
    }

    /// Appends the entries of another decorator (`applyDecorators(a, b)`).
    pub fn and(mut self, other: impl Into<CustomDecorator>) -> Self {
        self.entries.extend(other.into().entries);
        self
    }

    /// The metadata entries, in declaration order.
    pub fn entries(&self) -> &[(String, Value)] {
        &self.entries
    }

    pub fn into_entries(self) -> Vec<(String, Value)> {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl From<Vec<CustomDecorator>> for CustomDecorator {
    fn from(list: Vec<CustomDecorator>) -> Self {
        apply_decorators(list)
    }
}

impl<const N: usize> From<[CustomDecorator; N]> for CustomDecorator {
    fn from(list: [CustomDecorator; N]) -> Self {
        apply_decorators(list)
    }
}

impl From<(String, Value)> for CustomDecorator {
    fn from(entry: (String, Value)) -> Self {
        CustomDecorator { entries: vec![entry] }
    }
}

/// `applyDecorators(...decorators)`: combines several decorators into one.
pub fn apply_decorators<I: IntoIterator<Item = CustomDecorator>>(decorators: I) -> CustomDecorator {
    let mut out = CustomDecorator::new();
    for d in decorators {
        out.entries.extend(d.entries);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose() {
        let a = CustomDecorator::new().set_metadata("roles", ["admin"]);
        let b = CustomDecorator::new().set_metadata("public", false);
        let c = apply_decorators([a.clone(), b.clone()]);
        assert_eq!(c.entries().len(), 2);
        assert_eq!(c.entries()[0], ("roles".to_string(), serde_json::json!(["admin"])));
        assert_eq!(a.and(b), c);
        assert!(CustomDecorator::new().is_empty());
    }
}

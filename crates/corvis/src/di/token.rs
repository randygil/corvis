//! Injection tokens: a class reference, a string or a symbol.

use std::any::TypeId;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Identifies a provider inside the dependency injection container.
///
/// * `Token::of::<T>()` – a class token (like using the class itself as the token).
/// * `Token::named("CONFIG")` – a string token (like `@Inject('CONFIG')`).
#[derive(Clone)]
pub enum Token {
    Type { id: TypeId, name: &'static str },
    Named(Arc<str>),
}

impl Token {
    pub fn of<T: ?Sized + 'static>() -> Self {
        Token::Type { id: TypeId::of::<T>(), name: std::any::type_name::<T>() }
    }

    pub fn named(name: impl AsRef<str>) -> Self {
        Token::Named(Arc::from(name.as_ref()))
    }

    /// Human readable name, e.g. `CatsService` or `"CONFIG_OPTIONS"`.
    pub fn display_name(&self) -> String {
        match self {
            Token::Type { name, .. } => short_type_name(name),
            Token::Named(n) => n.to_string(),
        }
    }

    pub fn is_type<T: ?Sized + 'static>(&self) -> bool {
        matches!(self, Token::Type { id, .. } if *id == TypeId::of::<T>())
    }
}

impl PartialEq for Token {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Token::Type { id: a, .. }, Token::Type { id: b, .. }) => a == b,
            (Token::Named(a), Token::Named(b)) => a == b,
            _ => false,
        }
    }
}
impl Eq for Token {}

impl Hash for Token {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Token::Type { id, .. } => {
                0u8.hash(state);
                id.hash(state)
            }
            Token::Named(n) => {
                1u8.hash(state);
                n.hash(state)
            }
        }
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl From<&str> for Token {
    fn from(s: &str) -> Self {
        Token::named(s)
    }
}
impl From<String> for Token {
    fn from(s: String) -> Self {
        Token::named(s)
    }
}
impl From<&String> for Token {
    fn from(s: &String) -> Self {
        Token::named(s)
    }
}
impl From<&Token> for Token {
    fn from(t: &Token) -> Self {
        t.clone()
    }
}

/// Strips module paths from a `std::any::type_name` output.
/// `my_app::cats::CatsService` -> `CatsService`,
/// `alloc::sync::Arc<dyn my_app::Repo>` -> `Arc<dyn Repo>`.
pub fn short_type_name(full: &str) -> String {
    let mut out = String::with_capacity(full.len());
    let mut segment = String::new();
    let mut chars = full.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_alphanumeric() || c == '_' {
            segment.push(c);
        } else if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            segment.clear();
        } else {
            out.push_str(&segment);
            segment.clear();
            out.push(c);
        }
    }
    out.push_str(&segment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names() {
        assert_eq!(short_type_name("my_app::cats::CatsService"), "CatsService");
        assert_eq!(short_type_name("alloc::sync::Arc<dyn my::Repo>"), "Arc<dyn Repo>");
        assert_eq!(short_type_name("u32"), "u32");
    }

    #[test]
    fn token_eq() {
        assert_eq!(Token::of::<u32>(), Token::of::<u32>());
        assert_ne!(Token::of::<u32>(), Token::of::<u64>());
        assert_eq!(Token::from("A"), Token::named("A"));
    }
}

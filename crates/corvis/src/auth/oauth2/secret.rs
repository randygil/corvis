//! Wrappers that keep a token or a client secret out of `Debug` output, logs and JSON.

use subtle::ConstantTimeEq;

/// An access, refresh or ID token. It has no `Display` and no `Serialize`, so a token cannot reach a JSON response, a
/// session or a log line by accident: the application calls [`expose`](OAuth2Token::expose) on purpose.
///
/// The framework stores no token. If the application stores one (to call the provider's API later), it should keep it
/// encrypted at rest and tied to the user record, never in a cookie or in the session of the browser.
#[derive(Clone)]
pub struct OAuth2Token(String);

impl OAuth2Token {
    pub(crate) fn new(value: String) -> Self {
        OAuth2Token(value)
    }

    /// The token as the provider sent it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

/// Compares in time that depends on the lengths only, never on where two tokens differ.
impl PartialEq for OAuth2Token {
    fn eq(&self, other: &Self) -> bool {
        ct_eq(self.0.as_bytes(), other.0.as_bytes())
    }
}

impl Eq for OAuth2Token {}

/// Prints no value.
impl std::fmt::Debug for OAuth2Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OAuth2Token(..)")
    }
}

/// The client secret issued by the provider: never printed, never part of an error.
#[derive(Clone)]
pub(crate) struct ClientSecret(String);

impl ClientSecret {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        ClientSecret(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSecret(..)")
    }
}

/// Compares two byte strings in time that depends on their lengths only, never on where they differ.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_holds_the_value() {
        let token = OAuth2Token::new("tok-SECRET-123".into());
        let secret = ClientSecret::new("client-SECRET-456");
        assert_eq!(token.expose(), "tok-SECRET-123");
        for printed in [format!("{token:?}"), format!("{token:#?}"), format!("{secret:?}")] {
            assert!(!printed.contains("SECRET"), "{printed}");
        }
    }

    // `OAuth2Token` has no `Serialize` and no `Display`: these helpers fail to resolve if one is ever added.
    trait NotSerialize<A> {
        fn check() {}
    }
    impl<T: ?Sized> NotSerialize<()> for T {}
    struct Invalid;
    impl<T: ?Sized + serde::Serialize> NotSerialize<Invalid> for T {}

    #[test]
    fn a_token_is_not_serializable() {
        // Ambiguous (and so a compile error) if `OAuth2Token: Serialize`.
        <OAuth2Token as NotSerialize<_>>::check();
        <ClientSecret as NotSerialize<_>>::check();
    }

    #[test]
    fn tokens_compare_by_value() {
        let token = |s: &str| OAuth2Token::new(s.to_string());
        assert_eq!(token("tok-1"), token("tok-1"));
        assert_ne!(token("tok-1"), token("tok-2"));
        assert_ne!(token("tok-1"), token("tok-10"), "different lengths");
        assert_ne!(token(""), token("a"));
        assert_eq!(token(""), token(""));
    }

    #[test]
    fn constant_time_comparison() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"", b"a"));
        assert!(ct_eq(b"", b""));
    }
}

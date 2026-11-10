//! Signed cookies: values the client can read but not alter, in the format of `cookie-signature` 1.0.7 with the `s:`
//! prefix of `cookie-parser` and `express-session`.
//!
//! ```ignore
//! let keys = CookieKeys::new([config.get("COOKIE_SECRET")?, config.get("COOKIE_SECRET_OLD")?])?;
//! res.cookie("prefs", &keys.sign("dark"), CookieOptions::new().http_only(true));
//! let prefs: Option<String> = req.signed_cookie("prefs", &keys);
//! ```

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// The shortest secret accepted (RFC 2104 asks for a key at least as long as the hash output).
pub const MIN_SECRET_BYTES: usize = 32;

/// The most secrets (the current one and the retired ones still accepted).
pub const MAX_SECRETS: usize = 8;

/// The longest signed input verified. Longer input is refused before any HMAC is computed.
pub const MAX_SIGNED_BYTES: usize = 4096;

/// Base64 (standard alphabet, no padding) of a SHA-256 HMAC.
const SIGNATURE_LEN: usize = 43;

/// Why [`CookieKeys::new`] refused its secrets. The messages hold positions and lengths, never a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CookieKeysError {
    #[error("no cookie secret was given: at least one secret of {MIN_SECRET_BYTES} bytes is required")]
    NoSecrets,
    #[error("more than {MAX_SECRETS} cookie secrets were given: at most {MAX_SECRETS} are accepted")]
    TooMany,
    #[error("the cookie secret at position {index} has {len} bytes: the minimum is {MIN_SECRET_BYTES} bytes")]
    TooShort { index: usize, len: usize },
}

/// The secrets that sign and verify cookie values: the first one signs, every one verifies, so a secret can be
/// rotated by putting the new one first and removing the old one once its cookies have expired.
///
/// `Debug` prints the number of secrets only.
#[derive(Clone)]
pub struct CookieKeys {
    secrets: Arc<[Vec<u8>]>,
}

impl std::fmt::Debug for CookieKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CookieKeys({} secret(s))", self.secrets.len())
    }
}

#[cfg(test)]
thread_local! {
    /// How many HMACs this thread computed, so a test can show that a refused input cost none.
    static HMAC_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn signature(secret: &[u8], value: &str) -> String {
    #[cfg(test)]
    HMAC_RUNS.with(|c| c.set(c.get() + 1));
    // `new_from_slice` accepts a key of any length for HMAC.
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC takes keys of any length");
    mac.update(value.as_bytes());
    STANDARD_NO_PAD.encode(mac.finalize().into_bytes())
}

impl CookieKeys {
    /// 1 to 8 secrets of at least 32 bytes each. The first signs; every one verifies.
    pub fn new<I, S>(secrets: I) -> Result<Self, CookieKeysError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<[u8]>,
    {
        let mut list: Vec<Vec<u8>> = Vec::new();
        for (index, secret) in secrets.into_iter().enumerate() {
            let secret = secret.as_ref();
            if list.len() >= MAX_SECRETS {
                return Err(CookieKeysError::TooMany);
            }
            if secret.len() < MIN_SECRET_BYTES {
                return Err(CookieKeysError::TooShort { index, len: secret.len() });
            }
            list.push(secret.to_vec());
        }
        if list.is_empty() {
            return Err(CookieKeysError::NoSecrets);
        }
        Ok(CookieKeys { secrets: list.into() })
    }

    /// `s:` + `value` + `.` + the signature of `value` with the first secret: what
    /// `'s:' + require('cookie-signature').sign(value, secret)` returns.
    pub fn sign(&self, value: &str) -> String {
        format!("s:{value}.{}", signature(&self.secrets[0], value))
    }

    /// The value of a string produced by [`sign`](Self::sign) with any of the secrets, `None` for anything else
    /// (no `s:` prefix, no `.`, wrong signature, longer than 4096 bytes). Every secret is tried, without stopping at
    /// the first match, and signatures are compared in constant time.
    pub fn unsign(&self, input: &str) -> Option<String> {
        self.verify(input).map(|(value, _)| value.to_string())
    }

    /// `true` when the first (current) secret verifies `input`: `false` for a value signed with a retired secret,
    /// which the caller can sign again.
    pub fn verified_with_current(&self, input: &str) -> bool {
        self.verify(input).is_some_and(|(_, current)| current)
    }

    /// The value and whether the current secret verified it.
    pub(crate) fn verify<'a>(&self, input: &'a str) -> Option<(&'a str, bool)> {
        if input.len() > MAX_SIGNED_BYTES {
            return None;
        }
        let rest = input.strip_prefix("s:")?;
        let dot = rest.rfind('.')?;
        let (value, given) = (&rest[..dot], &rest.as_bytes()[dot + 1..]);
        // The re-encoded signature has one fixed length: anything else cannot match, whatever the secret.
        if given.len() != SIGNATURE_LEN {
            return None;
        }
        let mut any = subtle::Choice::from(0);
        let mut current = subtle::Choice::from(0);
        for (i, secret) in self.secrets.iter().enumerate() {
            let expected = signature(secret, value);
            let eq = expected.as_bytes().ct_eq(given);
            any |= eq;
            if i == 0 {
                current = eq;
            }
        }
        bool::from(any).then_some((value, bool::from(current)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0123456789abcdef0123456789abcdef";
    const B: &str = "fedcba9876543210fedcba9876543210";

    fn keys(list: &[&str]) -> CookieKeys {
        CookieKeys::new(list.iter().copied()).unwrap()
    }

    #[test]
    fn a_signature_computed_by_hand() {
        // `cookie-signature` 1.0.7: sign('hello', A), computed with Node.
        let signed = keys(&[A]).sign("hello");
        assert_eq!(signed, "s:hello.22i0e2PCOX6seFAlscnCEDrgbCiZgjSng7PrZFPRrXA");
        assert_eq!(keys(&[A]).unsign(&signed).as_deref(), Some("hello"));
    }

    #[test]
    fn short_secrets_are_refused_without_printing_them() {
        let e = CookieKeys::new(["short"]).unwrap_err();
        assert_eq!(e, CookieKeysError::TooShort { index: 0, len: 5 });
        let text = e.to_string();
        assert!(text.contains("32") && text.contains('5') && text.contains("position 0"), "{text}");
        assert!(!text.contains("short"), "{text}");
        let e = CookieKeys::new([A, "tiny"]).unwrap_err();
        assert_eq!(e, CookieKeysError::TooShort { index: 1, len: 4 });
    }

    #[test]
    fn the_number_of_secrets_is_bounded() {
        assert_eq!(CookieKeys::new(Vec::<&str>::new()).unwrap_err(), CookieKeysError::NoSecrets);
        assert!(CookieKeys::new(vec![A; 8]).is_ok());
        assert!(matches!(CookieKeys::new(vec![A; 9]).unwrap_err(), CookieKeysError::TooMany));
    }

    #[test]
    fn debug_output_holds_no_secret() {
        let printed = format!("{:?}", keys(&[A, B]));
        assert!(!printed.contains(A) && !printed.contains(B) && !printed.contains("0123"), "{printed}");
    }

    #[test]
    fn tampering_is_refused() {
        let k = keys(&[A]);
        let signed = k.sign("value.with.dots");
        assert_eq!(k.unsign(&signed).as_deref(), Some("value.with.dots"));
        // One changed character in the value, in the signature, a truncation, padding and an extra byte.
        let mut changed = signed.clone().into_bytes();
        changed[3] ^= 1;
        assert_eq!(k.unsign(&String::from_utf8(changed).unwrap()), None);
        let last = signed.len() - 1;
        let mut changed = signed.clone().into_bytes();
        changed[last] = if changed[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(k.unsign(&String::from_utf8(changed).unwrap()), None);
        assert_eq!(k.unsign(&signed[..last]), None);
        assert_eq!(k.unsign(&format!("{signed}=")), None);
        assert_eq!(k.unsign(&format!("{signed}A")), None);
    }

    #[test]
    fn unsigned_input_is_refused() {
        let k = keys(&[A]);
        assert_eq!(k.unsign("plain"), None);
        assert_eq!(k.unsign(""), None);
        assert_eq!(k.unsign("s:"), None);
        assert_eq!(k.unsign("s:nodot"), None);
        let signed = k.sign("v");
        assert_eq!(k.unsign(signed.strip_prefix("s:").unwrap()), None, "the prefix is required");
    }

    #[test]
    fn rotation_signs_with_the_first_and_verifies_with_all() {
        let old = keys(&[A]).sign("v");
        let rotated = keys(&[B, A]);
        assert_eq!(rotated.unsign(&old).as_deref(), Some("v"));
        assert!(!rotated.verified_with_current(&old));
        let fresh = rotated.sign("v");
        assert!(rotated.verified_with_current(&fresh));
        assert_eq!(keys(&[B]).unsign(&fresh).as_deref(), Some("v"), "signed with the new secret");
        assert_eq!(keys(&[B]).unsign(&old), None, "a removed secret verifies nothing");
    }

    #[test]
    fn an_oversized_input_costs_no_hmac() {
        let k = keys(&[A]);
        let before = HMAC_RUNS.with(|c| c.get());
        let big = format!("s:{}", "a".repeat(5000));
        assert_eq!(k.unsign(&big), None);
        assert_eq!(HMAC_RUNS.with(|c| c.get()), before);
        // The bound is on the whole input, the signature included.
        let at_limit = format!("s:{}.{}", "a".repeat(MAX_SIGNED_BYTES - 2 - 1 - SIGNATURE_LEN), "A".repeat(SIGNATURE_LEN));
        assert_eq!(at_limit.len(), MAX_SIGNED_BYTES);
        let _ = k.unsign(&at_limit);
        assert!(HMAC_RUNS.with(|c| c.get()) > before);
    }

    #[test]
    fn every_secret_is_computed_whatever_matches() {
        let k = keys(&[A, B, A]);
        let signed = k.sign("v");
        let before = HMAC_RUNS.with(|c| c.get());
        assert!(k.unsign(&signed).is_some());
        assert_eq!(HMAC_RUNS.with(|c| c.get()) - before, 3);
    }

    #[test]
    fn utf8_and_empty_values_round_trip() {
        let k = keys(&[A]);
        for v in ["", "caf\u{e9}", "a.b.c", "\u{1f600}"] {
            assert_eq!(k.unsign(&k.sign(v)).as_deref(), Some(v));
        }
    }
}

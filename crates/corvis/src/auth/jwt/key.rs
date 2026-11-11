//! Keys, key families and the algorithm allow-list.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey};

use crate::http::Request;

/// The kinds of key. A token is only checked against a key of the family its algorithm belongs to, so a public key can
/// never be used as an HMAC secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KeyFamily {
    Hmac,
    Rsa,
    Ec,
    Ed,
}

impl KeyFamily {
    pub(crate) fn name(self) -> &'static str {
        match self {
            KeyFamily::Hmac => "HMAC",
            KeyFamily::Rsa => "RSA",
            KeyFamily::Ec => "EC",
            KeyFamily::Ed => "Ed25519",
        }
    }
}

/// The family an algorithm needs a key of.
pub(crate) fn family_of(alg: Algorithm) -> KeyFamily {
    match alg {
        Algorithm::HS256 | Algorithm::HS384 | Algorithm::HS512 => KeyFamily::Hmac,
        Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512 | Algorithm::PS256 | Algorithm::PS384 | Algorithm::PS512 => KeyFamily::Rsa,
        Algorithm::ES256 | Algorithm::ES384 => KeyFamily::Ec,
        Algorithm::EdDSA => KeyFamily::Ed,
    }
}

/// The algorithms accepted for a key family when none are configured, as node-jsonwebtoken 9 derives them from the key
/// type. `jsonwebtoken` 9 has no ES512. Ed25519 has none: it must be asked for with `algorithms([EdDSA])`.
pub(crate) fn default_algorithms(family: KeyFamily) -> &'static [Algorithm] {
    match family {
        KeyFamily::Hmac => &[Algorithm::HS256, Algorithm::HS384, Algorithm::HS512],
        KeyFamily::Rsa => &[Algorithm::RS256, Algorithm::RS384, Algorithm::RS512, Algorithm::PS256, Algorithm::PS384, Algorithm::PS512],
        KeyFamily::Ec => &[Algorithm::ES256, Algorithm::ES384],
        KeyFamily::Ed => &[],
    }
}

/// The algorithms a token may use with a key of `family`: the configured list, or the default of the family, narrowed
/// to the family.
pub(crate) fn allowed_for(family: KeyFamily, configured: Option<&[Algorithm]>) -> Vec<Algorithm> {
    match configured {
        Some(list) => list.iter().copied().filter(|a| family_of(*a) == family).collect(),
        None => default_algorithms(family).to_vec(),
    }
}

#[derive(Clone)]
enum Material {
    Secret(Vec<u8>),
    Pem(Vec<u8>),
}

#[cfg(test)]
thread_local! {
    /// How many `JwtKey`s were parsed on this thread, so a test can show that a cached key is parsed once.
    pub(crate) static KEY_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The key a token is verified with: an HMAC secret or a public key. It prints its family and size, never its bytes.
///
/// The key is parsed the first time a token is verified with it, and every clone shares the result: a key provider that
/// hands back clones of a cached `JwtKey` costs no parse per request.
#[derive(Clone)]
pub struct JwtKey {
    family: KeyFamily,
    material: Material,
    parsed: Arc<OnceLock<Result<DecodingKey, String>>>,
}

impl std::fmt::Debug for JwtKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = match &self.material {
            Material::Secret(b) | Material::Pem(b) => b.len(),
        };
        write!(f, "JwtKey {{ family: {}, bytes: {len} }}", self.family.name())
    }
}

impl JwtKey {
    /// An HMAC secret (`HS256`, `HS384`, `HS512`). It must be at least 32 bytes long: the strategy does not start with
    /// a shorter one (RFC 7518 section 3.2).
    pub fn secret(secret: impl Into<Vec<u8>>) -> Self {
        Self::of(KeyFamily::Hmac, Material::Secret(secret.into()))
    }

    /// An RSA public key in PEM (`RS*` and `PS*`).
    pub fn rsa_pem(pem: impl Into<Vec<u8>>) -> Self {
        Self::of(KeyFamily::Rsa, Material::Pem(pem.into()))
    }

    /// An EC public key in PEM (`ES256`, `ES384`).
    pub fn ec_pem(pem: impl Into<Vec<u8>>) -> Self {
        Self::of(KeyFamily::Ec, Material::Pem(pem.into()))
    }

    /// An Ed25519 public key in PEM. It needs an explicit `algorithms([EdDSA])`.
    pub fn ed_pem(pem: impl Into<Vec<u8>>) -> Self {
        Self::of(KeyFamily::Ed, Material::Pem(pem.into()))
    }

    fn of(family: KeyFamily, material: Material) -> Self {
        JwtKey { family, material, parsed: Arc::new(OnceLock::new()) }
    }

    pub(crate) fn family(&self) -> KeyFamily {
        self.family
    }

    /// The length of an HMAC secret.
    pub(crate) fn secret_len(&self) -> Option<usize> {
        match &self.material {
            Material::Secret(b) => Some(b.len()),
            Material::Pem(_) => None,
        }
    }

    /// The key as the verification code needs it, parsed on the first call and shared by every clone. The error never
    /// carries key material.
    pub(crate) fn decoding_key(&self) -> Result<&DecodingKey, &str> {
        self.parsed
            .get_or_init(|| {
                #[cfg(test)]
                KEY_PARSES.with(|c| c.set(c.get() + 1));
                let invalid = |what: &str| format!("the {what} key is not valid PEM");
                match (&self.material, self.family) {
                    (Material::Secret(secret), _) => Ok(DecodingKey::from_secret(secret)),
                    (Material::Pem(pem), KeyFamily::Rsa) => DecodingKey::from_rsa_pem(pem).map_err(|_| invalid("RSA")),
                    (Material::Pem(pem), KeyFamily::Ec) => DecodingKey::from_ec_pem(pem).map_err(|_| invalid("EC")),
                    (Material::Pem(pem), KeyFamily::Ed) => DecodingKey::from_ed_pem(pem).map_err(|_| invalid("Ed25519")),
                    (Material::Pem(_), KeyFamily::Hmac) => Err("an HMAC key must be a secret".into()),
                }
            })
            .as_ref()
            .map_err(String::as_str)
    }
}

/// Chooses the key for each request (`secretOrKeyProvider`), for rotating keys or one key per issuer. It receives the
/// request and the raw token, which is not verified yet: use its `iss` or `kid` only to pick a key, never to trust it.
/// It runs for every request, so cache what it fetches, and cache the `JwtKey` itself rather than its PEM: clones of
/// one `JwtKey` share its parsed key, while a `JwtKey` built per request is parsed per request. An error fails the
/// authentication (401).
///
/// ```ignore
/// struct ByIssuer { keys: HashMap<String, JwtKey> }
///
/// #[async_trait]
/// impl JwtKeyProvider for ByIssuer {
///     async fn key(&self, _req: &Request, raw_token: &str) -> anyhow::Result<JwtKey> {
///         let iss = unverified_issuer(raw_token).ok_or_else(|| anyhow::anyhow!("no issuer"))?;
///         self.keys.get(&iss).cloned().ok_or_else(|| anyhow::anyhow!("unknown issuer"))
///     }
/// }
/// ```
#[async_trait]
pub trait JwtKeyProvider: Send + Sync + 'static {
    async fn key(&self, req: &Request, raw_token: &str) -> anyhow::Result<JwtKey>;
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSA_PUBLIC: &str = include_str!("../../../tests/fixtures/jwt_rsa_public.pem");

    #[test]
    fn default_lists_follow_the_key_family() {
        assert_eq!(default_algorithms(KeyFamily::Hmac).len(), 3);
        assert_eq!(default_algorithms(KeyFamily::Rsa).len(), 6);
        assert_eq!(default_algorithms(KeyFamily::Ec), &[Algorithm::ES256, Algorithm::ES384]);
        assert!(default_algorithms(KeyFamily::Ed).is_empty());
    }

    #[test]
    fn allowed_algorithms_are_narrowed_to_the_family() {
        let mixed = [Algorithm::HS256, Algorithm::RS256, Algorithm::ES256, Algorithm::EdDSA];
        assert_eq!(allowed_for(KeyFamily::Hmac, Some(&mixed)), vec![Algorithm::HS256]);
        assert_eq!(allowed_for(KeyFamily::Rsa, Some(&mixed)), vec![Algorithm::RS256]);
        assert_eq!(allowed_for(KeyFamily::Ed, Some(&mixed)), vec![Algorithm::EdDSA]);
        assert!(allowed_for(KeyFamily::Rsa, Some(&[Algorithm::HS256])).is_empty());
        assert_eq!(allowed_for(KeyFamily::Hmac, None).len(), 3);
    }

    #[test]
    fn keys_parse_and_report_errors_without_key_material() {
        assert!(JwtKey::rsa_pem(RSA_PUBLIC).decoding_key().is_ok());
        let bad = JwtKey::rsa_pem("-----BEGIN PUBLIC KEY-----\nSECRETMATERIAL\n-----END PUBLIC KEY-----");
        let error = bad.decoding_key().err().unwrap();
        assert!(!error.contains("SECRETMATERIAL"), "{error}");
        assert!(JwtKey::ec_pem("nope").decoding_key().is_err());
        assert!(JwtKey::ed_pem("nope").decoding_key().is_err());
        assert!(JwtKey::secret("x").decoding_key().is_ok());
        assert_eq!(JwtKey::secret("abc").secret_len(), Some(3));
        assert_eq!(JwtKey::rsa_pem(RSA_PUBLIC).secret_len(), None);
    }

    #[test]
    fn a_key_does_not_print_its_bytes() {
        let printed = format!("{:?}", JwtKey::secret("super-secret-value-that-is-long-enough!"));
        assert!(!printed.contains("super-secret"), "{printed}");
        assert!(printed.contains("HMAC"));
    }
}

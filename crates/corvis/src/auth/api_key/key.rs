//! Key types and constant-time comparison.
//!
//! Every comparison here is a comparison of two SHA-256 digests with `subtle`, never of the keys themselves: the
//! time taken depends neither on the position of the first different byte nor on the length of the stored key, and a
//! stored key that is only a digest never exists in clear in memory.

use std::fmt;

use sha2::{Digest, Sha256};
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};

/// The shortest secret [`ExpectedApiKey::new`] accepts, in bytes.
pub const MIN_SECRET_BYTES: usize = 32;

/// The longest secret [`ExpectedApiKey::new`] accepts, the same bound as the strategy's `max_key_bytes` limit.
pub const MAX_SECRET_BYTES: usize = 4096;

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    digest.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Compares two byte strings in constant time: both are hashed with SHA-256 and the digests are compared with
/// `subtle`. Neither the position of the first different byte nor the lengths are revealed by the time taken (the
/// time grows with the length of each input, which the caller already knows or bounds). Use it for any secret a
/// strategy of your own has to compare; never `==`.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    digest(a).ct_eq(&digest(b)).into()
}

/// The API key a request presented, after the strategy checked its length and characters. It holds the key and its
/// SHA-256 digest. There is no `Display`, `Serialize`, `Clone` or `PartialEq`: an `==` between keys would be the
/// comparison that leaks through timing, and the redacted `Debug` keeps a derived `Debug` of your own types from
/// writing the key to a log.
///
/// Look keys up by [`ApiKey::sha256_hex`] or by a public id ([`ApiKey::id`]); call [`ApiKey::expose`] only to hand
/// the key to something that needs the text.
pub struct ApiKey {
    value: String,
    digest: [u8; 32],
}

impl ApiKey {
    /// Wraps a key. The strategy builds its keys after the limits of its options; build one yourself only in tests
    /// of your validator.
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let digest = digest(value.as_bytes());
        ApiKey { value, digest }
    }

    /// The key itself. Do not log it, put it in an error message, a response or the auth info.
    pub fn expose(&self) -> &str {
        &self.value
    }

    /// Compares the key with `expected` in constant time (see [`constant_time_eq`]).
    pub fn ct_eq(&self, expected: impl AsRef<[u8]>) -> bool {
        self.digest.ct_eq(&digest(expected.as_ref())).into()
    }

    /// The SHA-256 digest of the key.
    pub fn sha256(&self) -> [u8; 32] {
        self.digest
    }

    /// The SHA-256 digest of the key as lowercase hexadecimal: store this, not the key, and look the presented key
    /// up with it (an indexed column). A digest of a random key of 32 bytes or more cannot be turned back into the
    /// key, and what an index lookup leaks by timing is a prefix of that digest, which does not help to find a key.
    pub fn sha256_hex(&self) -> String {
        hex(&self.digest)
    }

    /// The public id at the start of a key written as `<id><delimiter><secret>` (`ck_live_3f9a.<secret>`): the text
    /// before the first `delimiter`, when it is 1 to 64 characters of `A-Z a-z 0-9 _ -`. The id is not secret, so it
    /// is safe to log and to use as a database key; a key without a valid id gives `None`, which a validator treats
    /// as an unknown key. Look the record up by id, then compare the digest with
    /// [`ExpectedApiKey::matches_or_dummy`].
    pub fn id(&self, delimiter: char) -> Option<&str> {
        let (id, _) = self.value.split_once(delimiter)?;
        let valid = (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        valid.then_some(id)
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Why a configured key was refused. The message never contains the secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ApiKeyConfigError {
    /// The secret is shorter than [`MIN_SECRET_BYTES`].
    #[error("API key secret must be at least {MIN_SECRET_BYTES} bytes (got {got})")]
    TooShort { got: usize },
    /// The secret is longer than [`MAX_SECRET_BYTES`].
    #[error("API key secret must be at most {MAX_SECRET_BYTES} bytes (got {got})")]
    TooLong { got: usize },
    /// The digest is not 64 hexadecimal characters.
    #[error("an API key digest must be 64 hexadecimal characters (got {got} characters)")]
    BadDigest { got: usize },
}

/// A fixed digest that no key is known to hash to, compared when a lookup found no record.
const DUMMY_DIGEST: [u8; 32] = [
    0x6b, 0x1f, 0x3a, 0xd8, 0x52, 0xe0, 0x97, 0x44, 0xc1, 0x0e, 0x7d, 0xa3, 0x85, 0x29, 0xb6, 0xf4, 0x11, 0xcc, 0x5e, 0x70, 0x93, 0x08,
    0xea, 0x2d, 0x4b, 0xf7, 0x60, 0x1a, 0x9c, 0x35, 0xd2, 0x8e,
];

/// A key the application expects: only its SHA-256 digest is kept, so the secret does not stay in memory in clear
/// after startup. Build one from a secret (at least 32 bytes) or from a stored digest.
///
/// ```ignore
/// let expected = ExpectedApiKey::new(config.get("SERVICE_API_KEY").unwrap_or_default())?;
/// // in the validator: `expected.matches(key)`
/// ```
#[derive(Clone)]
pub struct ExpectedApiKey {
    digest: [u8; 32],
}

impl ExpectedApiKey {
    /// An expected key from its secret. A secret under 32 bytes, or over 4096, is an error: generate keys of at
    /// least 32 random bytes. Returning the error from a provider factory stops the application at startup.
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, ApiKeyConfigError> {
        let secret = secret.as_ref();
        if secret.len() < MIN_SECRET_BYTES {
            return Err(ApiKeyConfigError::TooShort { got: secret.len() });
        }
        if secret.len() > MAX_SECRET_BYTES {
            return Err(ApiKeyConfigError::TooLong { got: secret.len() });
        }
        Ok(ExpectedApiKey { digest: digest(secret) })
    }

    /// An expected key from the SHA-256 digest of the key as 64 hexadecimal characters (either case), what
    /// [`ApiKey::sha256_hex`] returns and a database column can hold.
    pub fn from_sha256_hex(digest: &str) -> Result<Self, ApiKeyConfigError> {
        let bytes = digest.as_bytes();
        if bytes.len() != 64 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(ApiKeyConfigError::BadDigest { got: digest.chars().count() });
        }
        let nibble = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
        let mut out = [0u8; 32];
        for (i, pair) in bytes.chunks(2).enumerate() {
            out[i] = nibble(pair[0]) << 4 | nibble(pair[1]);
        }
        Ok(ExpectedApiKey { digest: out })
    }

    /// `true` when `key` is this key, compared in constant time.
    pub fn matches(&self, key: &ApiKey) -> bool {
        key.digest.ct_eq(&self.digest).into()
    }

    /// `true` when `expected` is a record and `key` is its key. When there is no record (an unknown id) the key is
    /// compared with a fixed digest instead and the answer is `false`, so an unknown key costs the same work as a
    /// wrong one:
    ///
    /// ```ignore
    /// let record = db.find_key(key.id('.')?).await?;                       // None for an unknown id
    /// let stored = record.as_ref().map(|r| &r.expected);                   // ExpectedApiKey from the stored digest
    /// if !ExpectedApiKey::matches_or_dummy(stored, key) { return Ok(None); }
    /// ```
    ///
    /// The lookup itself (the database round trip) still differs for an unknown id; keep ids short and non-secret.
    pub fn matches_or_dummy(expected: Option<&ExpectedApiKey>, key: &ApiKey) -> bool {
        let stored = expected.map_or(DUMMY_DIGEST, |e| e.digest);
        let present = Choice::from(u8::from(expected.is_some()));
        (key.digest.ct_eq(&stored) & present).into()
    }
}

impl fmt::Debug for ExpectedApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExpectedApiKey(<redacted>)")
    }
}

/// A few expected keys, each with a value (the owner of the key, its scopes): for keys from configuration, not for
/// thousands of keys (look those up by digest in a database, see [`ApiKey::sha256_hex`]).
pub struct ApiKeySet<T> {
    entries: Vec<(ExpectedApiKey, T)>,
    #[cfg(test)]
    comparisons: std::sync::atomic::AtomicUsize,
}

impl<T> Default for ApiKeySet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ApiKeySet<T> {
    pub fn new() -> Self {
        ApiKeySet {
            entries: Vec::new(),
            #[cfg(test)]
            comparisons: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Adds a key. When two entries hold the same key, [`lookup`](ApiKeySet::lookup) returns the first one.
    pub fn insert(&mut self, key: ExpectedApiKey, value: T) {
        self.entries.push((key, value));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The value of the entry whose key is `key`. Every entry is compared with no early exit and the entry is picked
    /// with constant-time selection, so the time depends on how many keys the set holds, not on which one matched.
    pub fn lookup(&self, key: &ApiKey) -> Option<&T> {
        let mut found = Choice::from(0);
        let mut index = 0u64;
        for (i, (expected, _)) in self.entries.iter().enumerate() {
            #[cfg(test)]
            self.comparisons.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let equal = key.digest.ct_eq(&expected.digest);
            index.conditional_assign(&(i as u64), equal & !found);
            found |= equal;
        }
        bool::from(found).then(|| &self.entries[index as usize].1)
    }

    #[cfg(test)]
    fn comparisons(&self) -> usize {
        self.comparisons.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl<T> fmt::Debug for ApiKeySet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKeySet({} keys, <redacted>)", self.entries.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "test-key-000000000000000000000000000001";

    #[test]
    fn comparison_results() {
        let key = ApiKey::new(KEY);
        assert!(key.ct_eq(KEY));
        assert!(!key.ct_eq("Xest-key-000000000000000000000000000001"));
        assert!(!key.ct_eq("test-key-000000000000000000000000000002"));
        assert!(!key.ct_eq(&KEY[..KEY.len() - 1]));
        assert!(!key.ct_eq(""));
        assert!(!key.ct_eq(format!("{KEY}0")));
        assert!(constant_time_eq(KEY.as_bytes(), KEY.as_bytes()));
        assert!(constant_time_eq(b"", b""));
        assert!(!constant_time_eq(KEY.as_bytes(), b""));
        assert!(!constant_time_eq(b"a", b"b"));
    }

    #[test]
    fn the_digest_is_the_lowercase_hex_sha256() {
        let key = ApiKey::new("abc");
        assert_eq!(key.sha256_hex(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hex(&key.sha256()), key.sha256_hex());
        let stored = ExpectedApiKey::from_sha256_hex(&key.sha256_hex()).unwrap();
        assert!(stored.matches(&key));
        assert!(stored.matches(&ApiKey::new("abc")));
        assert!(!stored.matches(&ApiKey::new("abd")));
        let upper = ExpectedApiKey::from_sha256_hex(&key.sha256_hex().to_uppercase()).unwrap();
        assert!(upper.matches(&key));
    }

    #[test]
    fn debug_output_has_no_key_material() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            key: ApiKey,
            expected: ExpectedApiKey,
            set: ApiKeySet<&'static str>,
        }
        let mut set = ApiKeySet::new();
        set.insert(ExpectedApiKey::new(KEY).unwrap(), "owner-secret-name");
        let holder = Holder { key: ApiKey::new(KEY), expected: ExpectedApiKey::new(KEY).unwrap(), set };
        for text in [format!("{:?}", holder.key), format!("{holder:?}"), format!("{holder:#?}")] {
            assert!(text.contains("<redacted>"), "{text}");
            for start in 0..KEY.len() - 3 {
                assert!(!text.contains(&KEY[start..start + 4]), "{text}");
            }
            assert!(!text.contains("owner-secret-name"), "{text}");
        }
        assert!(!format!("{:?}", holder.key).contains(&holder.key.sha256_hex()));
    }

    #[test]
    fn a_short_configured_key_is_refused_without_the_secret_in_the_message() {
        let secret = "short-secret-12";
        let e = ExpectedApiKey::new(secret).unwrap_err();
        assert_eq!(e, ApiKeyConfigError::TooShort { got: secret.len() });
        let text = e.to_string();
        assert_eq!(text, "API key secret must be at least 32 bytes (got 15)");
        assert!(!text.contains(secret) && !format!("{e:?}").contains(secret));
        assert!(ExpectedApiKey::new("").is_err());
        let e = ExpectedApiKey::new(vec![b'x'; MAX_SECRET_BYTES + 1]).unwrap_err();
        assert_eq!(e, ApiKeyConfigError::TooLong { got: MAX_SECRET_BYTES + 1 });
    }

    #[test]
    fn a_secret_of_32_bytes_is_accepted_and_matches_only_itself() {
        let secret = "k".repeat(32);
        assert_eq!(secret.len(), MIN_SECRET_BYTES);
        let expected = ExpectedApiKey::new(&secret).unwrap();
        assert!(expected.matches(&ApiKey::new(secret.clone())));
        assert!(!expected.matches(&ApiKey::new("k".repeat(31))));
        assert!(!expected.matches(&ApiKey::new("k".repeat(33))));
        assert!(ExpectedApiKey::new("k".repeat(31)).is_err());
        assert!(ExpectedApiKey::new(vec![b'x'; MAX_SECRET_BYTES]).is_ok());
    }

    #[test]
    fn digests_must_be_64_hexadecimal_characters() {
        for bad in ["", "ab", &"g".repeat(64), &"a".repeat(63), &"a".repeat(65), &format!("{}é", "a".repeat(63))] {
            let e = ExpectedApiKey::from_sha256_hex(bad).unwrap_err();
            assert!(matches!(e, ApiKeyConfigError::BadDigest { .. }), "{bad}");
            assert!(!e.to_string().contains(bad) || bad.is_empty());
        }
        assert!(ExpectedApiKey::from_sha256_hex(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn matches_or_dummy_is_false_without_a_record() {
        let key = ApiKey::new(KEY);
        let record = ExpectedApiKey::new(KEY).unwrap();
        assert!(ExpectedApiKey::matches_or_dummy(Some(&record), &key));
        assert!(!ExpectedApiKey::matches_or_dummy(Some(&record), &ApiKey::new("other-key-0000000000000000000000000001")));
        assert!(!ExpectedApiKey::matches_or_dummy(None, &key));
        // The dummy digest itself cannot be presented as a key: it is a digest, not a preimage.
        assert!(!ExpectedApiKey::matches_or_dummy(None, &ApiKey::new(hex(&DUMMY_DIGEST))));
    }

    #[test]
    fn ids_are_the_public_part_before_the_delimiter() {
        assert_eq!(ApiKey::new("ck_live-3f9a.secretsecretsecret").id('.'), Some("ck_live-3f9a"));
        assert_eq!(ApiKey::new("nodelimiter").id('.'), None);
        assert_eq!(ApiKey::new(".secret").id('.'), None);
        assert_eq!(ApiKey::new(format!("{}.x", "a".repeat(65))).id('.'), None);
        assert_eq!(ApiKey::new(format!("{}.x", "a".repeat(64))).id('.').map(str::len), Some(64));
        assert_eq!(ApiKey::new("bad id.secret").id('.'), None);
        assert_eq!(ApiKey::new("é.secret").id('.'), None);
        assert_eq!(ApiKey::new("a.b.c").id('.'), Some("a"));
    }

    fn set_of_five() -> ApiKeySet<usize> {
        let mut set = ApiKeySet::new();
        for i in 0..5 {
            set.insert(ExpectedApiKey::new(format!("{i}{}", "k".repeat(40))).unwrap(), i);
        }
        set
    }

    #[test]
    fn every_entry_is_compared() {
        let set = set_of_five();
        assert_eq!(set.lookup(&ApiKey::new(format!("0{}", "k".repeat(40)))), Some(&0));
        assert_eq!(set.comparisons(), 5);
        assert_eq!(set.lookup(&ApiKey::new("unknown")), None);
        assert_eq!(set.comparisons(), 10);
        assert_eq!(set.lookup(&ApiKey::new(format!("4{}", "k".repeat(40)))), Some(&4));
        assert_eq!(set.comparisons(), 15);
        assert_eq!(set.len(), 5);
        assert!(ApiKeySet::<u8>::new().is_empty());
        assert_eq!(ApiKeySet::<u8>::new().lookup(&ApiKey::new("x")), None);
    }

    #[test]
    fn two_entries_with_the_same_key_return_the_first() {
        let mut set = ApiKeySet::new();
        set.insert(ExpectedApiKey::new(KEY).unwrap(), "first");
        set.insert(ExpectedApiKey::new("other-key-0000000000000000000000000001").unwrap(), "middle");
        set.insert(ExpectedApiKey::new(KEY).unwrap(), "second");
        assert_eq!(set.lookup(&ApiKey::new(KEY)), Some(&"first"));
        assert_eq!(set.lookup(&ApiKey::new("other-key-0000000000000000000000000001")), Some(&"middle"));
    }
}

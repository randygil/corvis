//! Session ids: 15 bytes from the operating system's CSPRNG, encoded as unpadded base64url (20 characters).
//!
//! The id is the session's bearer credential: it is checked for shape before any lookup and never logged.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;

/// Length of a session id in characters (Node's `base64id` length).
pub const SID_LEN: usize = 20;

/// Generates a session id with 120 random bits.
pub fn generate() -> String {
    let mut bytes = [0u8; 15];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `true` when `sid` has the shape of a generated id: exactly 20 base64url characters.
pub fn has_valid_shape(sid: &str) -> bool {
    sid.len() == SID_LEN && sid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_differ_and_have_the_shape_of_node_s() {
        let mut seen = HashSet::new();
        for _ in 0..100_000 {
            let id = generate();
            assert_eq!(id.len(), 20);
            assert!(id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{id}");
            assert!(has_valid_shape(&id));
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn the_shape_check_refuses_everything_else() {
        assert!(!has_valid_shape(""));
        assert!(!has_valid_shape("short"));
        assert!(!has_valid_shape(&"a".repeat(21)));
        assert!(!has_valid_shape(&"a".repeat(1 << 20)));
        assert!(!has_valid_shape("aaaaaaaaaaaaaaaaaaa="));
        assert!(!has_valid_shape("aaaaaaaaaaaaaaaaaaa+"));
        assert!(!has_valid_shape("aaaaaaaaaaaaaaaaaaé"));
        assert!(has_valid_shape("aaaaaaaaaaaaaaaaaa-_"));
    }
}

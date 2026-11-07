//! Id generators (`cuid()`, `cuid(2)`, `nanoid()`, `ulid()`) and small helpers.

use std::sync::atomic::{AtomicU32, Ordering};

const BASE36: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

fn random_u128() -> u128 {
    uuid::Uuid::new_v4().as_u128()
}

fn base36(mut n: u128, width: usize) -> String {
    let mut out = Vec::new();
    while n > 0 {
        out.push(BASE36[(n % 36) as usize]);
        n /= 36;
    }
    while out.len() < width {
        out.push(b'0');
    }
    out.reverse();
    let s = String::from_utf8(out).unwrap();
    if width > 0 && s.len() > width { s[s.len() - width..].to_string() } else { s }
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn fingerprint() -> u128 {
    static FP: std::sync::OnceLock<u128> = std::sync::OnceLock::new();
    *FP.get_or_init(|| {
        let pid = std::process::id() as u128;
        let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
        let h: u128 = host.bytes().fold(host.len() as u128 + 36, |a, b| a + b as u128);
        (pid % 1296) * 1296 + (h % 1296)
    })
}

/// cuid v1 (`c` + timestamp + counter + fingerprint + random): 25 lowercase characters.
pub fn cuid() -> String {
    let ts = chrono::Utc::now().timestamp_millis() as u128;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed) as u128 % (36u128.pow(4));
    let r = random_u128();
    format!("c{}{}{}{}", base36(ts, 8), base36(count, 4), base36(fingerprint(), 4), base36(r, 8))
}

/// cuid2: 24 lowercase characters starting with a letter.
pub fn cuid2() -> String {
    let r = random_u128();
    let first = (b'a' + (r % 26) as u8) as char;
    let rest = base36(random_u128() ^ (chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u128), 23);
    format!("{first}{rest}")
}

/// nanoid with the default URL-safe alphabet.
pub fn nanoid(len: usize) -> String {
    const ALPHABET: &[u8] = b"useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";
    let mut out = String::with_capacity(len);
    let mut r = random_u128();
    let mut bits = 128;
    for _ in 0..len {
        if bits < 6 {
            r = random_u128();
            bits = 128;
        }
        out.push(ALPHABET[(r & 63) as usize] as char);
        r >>= 6;
        bits -= 6;
    }
    out
}

/// ULID: 48-bit timestamp + 80 random bits, Crockford base32 (26 characters).
pub fn ulid() -> String {
    const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let ts = (chrono::Utc::now().timestamp_millis() as u128) & ((1u128 << 48) - 1);
    let rnd = random_u128() & ((1u128 << 80) - 1);
    let mut n = (ts << 80) | rnd;
    let mut out = [b'0'; 26];
    for i in (0..26).rev() {
        out[i] = CROCKFORD[(n & 31) as usize];
        n >>= 5;
    }
    String::from_utf8(out.to_vec()).unwrap()
}

/// Standard base64 (with or without padding).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Standard base64 with padding.
pub fn base64_encode(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in b.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids() {
        assert_eq!(cuid().len(), 25);
        assert!(cuid().starts_with('c'));
        assert_eq!(cuid2().len(), 24);
        assert_eq!(nanoid(21).len(), 21);
        assert_eq!(ulid().len(), 26);
        assert_eq!(base64_decode(&base64_encode(b"hello!?")).unwrap(), b"hello!?");
    }
}

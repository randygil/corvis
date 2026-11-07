---
status: unavailable
---

# Encryption and hashing

Corvis does not include encryption or password hashing utilities. The algorithms it uses
internally (SHA-1 for `ETag`s, SHA-256 for the throttler keys) are not part of its public
API.

The alternative is to use the Rust ecosystem's crates directly and wrap them in a provider, so you can
inject them where needed and replace them in tests.

## Password hashing

For passwords use a slow, salted derivation function, such as Argon2 (the `argon2` crate) or bcrypt
(the `bcrypt` crate). Never use a fast hash (SHA-256, MD5) for this.

```toml
[dependencies]
argon2 = "0.5"
```

```rust
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use corvis::prelude::*;

#[injectable]
pub struct HashingService;

impl HashingService {
    pub fn hash(&self, password: &str) -> corvis::Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| corvis::anyhow::anyhow!(e.to_string()))?;
        Ok(hash.to_string())
    }

    pub fn verify(&self, password: &str, stored: &str) -> bool {
        PasswordHash::new(stored)
            .map(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
            .unwrap_or(false)
    }
}
```

Argon2 is expensive on purpose. If the handler serves many login requests, run it off
the runtime threads with `corvis::tokio::task::spawn_blocking`.

## Symmetric encryption

To encrypt data that you later need to read (third-party tokens, sensitive fields) use an
authenticated cipher such as AES-256-GCM (the `aes-gcm` crate) or ChaCha20-Poly1305 (the `chacha20poly1305` crate). Read the
key from configuration with `ConfigService` in a factory (`Provider::use_factory`) and do not
write it in the code. See [Configuration](/application/configuration) and
[Custom providers](/fundamentals/custom-providers).

## Signed tokens

If what you need is for the client not to be able to alter a piece of data (and not that it cannot read it), a JWT
signed with `JwtService` may be enough. See [Authentication](/security/authentication).

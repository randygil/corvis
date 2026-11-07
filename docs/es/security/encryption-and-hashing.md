---
status: no-disponible
---

# Cifrado y hashing

Corvis no incluye utilidades de cifrado ni de hashing de contraseñas. Los algoritmos que usa por
dentro (SHA-1 para los `ETag`, SHA-256 para las claves del throttler) no forman parte de su API
pública.

La alternativa es usar directamente los crates del ecosistema Rust y envolverlos en un provider, para
inyectarlos donde hagan falta y poder sustituirlos en los tests.

## Hashing de contraseñas

Para contraseñas usa una función de derivación lenta y con sal, como Argon2 (crate `argon2`) o bcrypt
(crate `bcrypt`). Nunca uses un hash rápido (SHA-256, MD5) para esto.

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

Argon2 es costoso a propósito. Si el handler atiende muchas peticiones de login, ejecútalo fuera de
los hilos del runtime con `corvis::tokio::task::spawn_blocking`.

## Cifrado simétrico

Para cifrar datos que luego necesitas leer (tokens de terceros, campos sensibles) usa un cifrado
autenticado como AES-256-GCM (crate `aes-gcm`) o ChaCha20-Poly1305 (crate `chacha20poly1305`). Lee la
clave de la configuración con `ConfigService` en una fábrica (`Provider::use_factory`) y no la
escribas en el código. Ver [Configuración](/es/application/configuration) y
[Providers personalizados](/es/fundamentals/custom-providers).

## Tokens firmados

Si lo que necesitas es que el cliente no pueda alterar un dato (y no que no pueda leerlo), un JWT
firmado con `JwtService` puede bastar. Ver [Autenticación](/es/security/authentication).

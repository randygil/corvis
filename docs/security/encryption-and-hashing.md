---
status: partial
---

# Encryption and hashing

Corvis includes **password hashing** (`PasswordHasher`, behind the `auth-local` cargo feature). It does not include general encryption: the algorithms it uses internally (SHA-1 for `ETag`s, SHA-256 for the throttler keys) are not part of its public API, and for symmetric encryption you use the Rust ecosystem's crates directly (see [below](#symmetric-encryption)).

## Password hashing

Turn the feature on in `Cargo.toml`:

```toml
[dependencies]
corvis = { version = "0.1", features = ["auth-local"] }
```

`PasswordHasher` creates **argon2id** hashes for new passwords, verifies argon2 and **bcrypt** hashes (including the ones `bcryptjs` made), and limits what a login can cost. Register it like any provider and inject it where you hash or verify:

```rust
use corvis::auth::local::{PasswordError, PasswordHasher};
use corvis::prelude::*;

#[module(providers = [Provider::class::<PasswordHasher>(), UsersService])]
pub struct UsersModule;

#[injectable]
pub struct UsersService {
    hasher: Arc<PasswordHasher>,
    repository: Arc<UsersRepository>,
}

impl UsersService {
    pub async fn create(&self, username: &str, password: &str) -> Result<User, PasswordError> {
        let password_hash = self.hasher.hash(password).await?;   // $argon2id$v=19$m=19456,t=2,p=1$...
        // store `password_hash`, never the password
        Ok(self.repository.insert(username, password_hash).await)
    }
}
```

`Provider::class::<PasswordHasher>()` uses the default options. For others, build it yourself and register the value:

```rust
use corvis::auth::local::{Argon2Params, PasswordHasher, PasswordHasherOptions};

let hasher = PasswordHasher::new(PasswordHasherOptions::argon2id(Argon2Params {
    memory_kib: 65536,
    iterations: 3,
    parallelism: 4,
}))?;
// in the module: Provider::use_value(Token::of::<PasswordHasher>(), hasher)
```

`PasswordHasher::new` hashes one dummy password (see [unknown users](#unknown-users)), so create it once at startup.

| Method | What it does |
|---|---|
| `hash(password).await` | A PHC string for argon2id, or a `$2b$` hash when configured for bcrypt |
| `verify(password, &stored).await` | `Ok(true)`, `Ok(false)`, or an error for a stored hash that is malformed, unsupported or over the limits. The algorithm comes from the stored hash |
| `verify_or_dummy(password, Option<&str>).await` | `verify` when there is a stored hash, and one dummy verification when there is none |
| `verify_dummy(password).await` | One verification against a dummy hash; always `false` |
| `needs_rehash(&stored)` | `true` when the hash should be replaced (no key derivation runs) |
| `hash_blocking`, `verify_blocking`, `verify_or_dummy_blocking` | The same, for code outside an async context. They block the calling thread |

### Defaults

| Setting | Default | Why |
|---|---|---|
| Algorithm | argon2id, version 19 | RFC 9106 recommends argon2id |
| Memory, iterations, parallelism | 19456 KiB, 2, 1 | The OWASP Password Storage Cheat Sheet minimum, and the default of the `argon2` crate |
| Salt, output | 16 bytes, 32 bytes | RFC 9106; the salt comes from the operating system's generator |
| bcrypt cost (when chosen) | 10 | The `bcryptjs` default |
| `max_password_bytes` | 1024 | A long password cannot be used to burn CPU; a hash of one is refused with `PasswordError::TooLong`, and verifying one answers `false` without any key derivation |
| `max_concurrent` | `available_parallelism()` | At most this many key derivations run at once, each on a blocking thread, never on an async worker; the others wait for a slot. Each default derivation uses about 19 MiB |
| Stored hash length | 512 bytes | A PHC string with these parameters is about 100 bytes |
| `max_bcrypt_cost` | 14 | A planted `$2b$31$` hash would cost 2^31 rounds |
| argon2 limits when verifying | 262144 KiB, 16 iterations, parallelism 16 | A planted hash cannot ask for gigabytes or minutes |

Every stored hash is checked against the limits **before** any key derivation. A hash over a limit is an error (`PasswordError::OverLimit { what }` says which one); raise the limit with `VerifyLimits` if a legacy database has costlier hashes:

```rust
use corvis::auth::local::{PasswordHasherOptions, VerifyLimits};

PasswordHasherOptions::default().verify_limits(VerifyLimits::default().max_bcrypt_cost(15))
```

#### A floor for new hashes

The parameters of new hashes have a floor: argon2id with at least 19456 KiB and 2 iterations (or 47104 KiB and 1), bcrypt cost at least 10. `PasswordHasher::new` returns an error for anything weaker, unless the options say `.allow_weak_parameters()`, which logs a warning. Use that in tests only, where a hash of 8 KiB makes a suite fast. Parameters above the verification limits are an error too, because the hasher could not verify its own hashes.

### Keeping bcrypt hashes

A database with `bcryptjs` hashes keeps working: verification reads the prefix (`$2a$`, `$2b$`, `$2y$` or `$argon2...`), whatever the configured algorithm is. The results are those of `bcryptjs.compareSync` for every password, including non-ASCII ones, ones with a NUL character and ones over 72 bytes (bcrypt only reads the first 72 bytes, and so does `bcryptjs`). The behavior is checked against `bcryptjs` 3.0.3 in both directions with recorded results.

To move users to argon2id as they log in, check `needs_rehash` after a successful verification:

```rust
if hasher.verify(password, &user.password_hash).await? {
    if hasher.needs_rehash(&user.password_hash) {
        users.update_hash(user.id, hasher.hash(password).await?).await;
    }
    // ... the login succeeds
}
```

`needs_rehash` is `true` for another algorithm or argon2 variant, lower memory, iterations, output or salt length, a lower bcrypt cost, and for a hash that cannot be read. While the Node side still has to read new hashes (a migration in progress), configure bcrypt instead: `PasswordHasherOptions::bcrypt(10)` creates `$2b$` hashes that `bcryptjs.compareSync` accepts.

::: warning One divergence from bcryptjs
`bcryptjs` silently truncates passwords over 72 bytes when it hashes them, so two long passwords with the same first 72 bytes get hashes that accept each other. A `PasswordHasher` configured for bcrypt refuses to hash a password over 72 bytes (`PasswordError::TooLong`) instead. Verification still follows `bcryptjs`, so existing hashes keep working. Argon2id has no such limit below `max_password_bytes`.
:::

### Unknown users

A login that answers faster for an unknown username tells an attacker which usernames exist. `verify_or_dummy` spends one real verification either way, against a dummy hash made when the hasher was created:

```rust
let user = users.find_by_username(username).await;
let ok = hasher
    .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
    .await
    .map_err(|_| InternalServerErrorException::empty())?;
let user = user.filter(|_| ok);
```

When the stored hash is refused (malformed, unsupported, over a limit), the dummy verification runs before the error is returned, so a broken row does not answer faster either. The remaining timing differences are the ones you control: stored hashes with older parameters or another algorithm verify at their own cost (`needs_rehash` closes that gap over time, and `.dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 10 })` matches a database that is mostly bcrypt), and your own user lookup may take a different time for a missing row.

Argon2 costs memory and time on purpose: a flood of logins queues behind `max_concurrent` slots. Limit login attempts (the [throttler](/security/rate-limiting) and the planned login lockout) so a flood cannot keep the slots busy.

The comparison of the derived values is constant-time: `argon2` and `password-hash` compare outputs with `subtle`, and so does `bcrypt`. `PasswordError` and the `Debug` output never contain a password or a hash.

The `local` strategy uses this hasher; see [Passport](/recipes/passport#local-strategy-username-and-password) and [Authentication](/security/authentication).

## Symmetric encryption

To encrypt data that you later need to read (third-party tokens, sensitive fields) use an
authenticated cipher such as AES-256-GCM (the `aes-gcm` crate) or ChaCha20-Poly1305 (the `chacha20poly1305` crate). Read the
key from configuration with `ConfigService` in a factory (`Provider::use_factory`) and do not
write it in the code. See [Configuration](/application/configuration) and
[Custom providers](/fundamentals/custom-providers).

## Signed tokens

If what you need is for the client not to be able to alter a piece of data (and not that it cannot read it), a JWT
signed with `JwtService` may be enough. See [Authentication](/security/authentication).

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
| `verify_or_dummy(password, Option<&str>).await` | `verify` when there is a stored hash, and one dummy verification when there is none. A stored hash that is refused (an empty string, `$2x$`, over a limit) is `Ok(false)` after one dummy verification, like an unknown user |
| `verify_dummy(password).await` | One verification against a dummy hash; `Ok(false)` |
| `needs_rehash(&stored)` | `true` when the hash should be replaced (no key derivation runs) |
| `hash_blocking`, `verify_blocking`, `verify_or_dummy_blocking`, `verify_dummy_blocking` | The same, for synchronous code. They block the calling thread; see [blocking threads](#blocking-threads) |

### Defaults

| Setting | Default | Why |
|---|---|---|
| Algorithm | argon2id, version 19 | RFC 9106 recommends argon2id |
| Memory, iterations, parallelism | 19456 KiB, 2, 1 | The OWASP Password Storage Cheat Sheet minimum, and the default of the `argon2` crate |
| Salt, output | 16 bytes, 32 bytes | RFC 9106; the salt comes from the operating system's generator |
| bcrypt cost (when chosen) | 10 | The `bcryptjs` default |
| `max_password_bytes` | 1024 | A long password cannot be used to burn CPU; a hash of one is refused with `PasswordError::TooLong`, and verifying one answers `false` without any key derivation |
| `max_concurrent` | `available_parallelism()` | At most this many key derivations run at once, each on a blocking thread, never on an async worker; the others wait for a slot. Each default derivation uses about 19 MiB |
| `max_waiting` | `32 * max_concurrent` (at least 1) | How many calls may wait for a slot. The next one fails at once with `PasswordError::Busy`, without any key derivation |
| `acquire_timeout` | 10 s (1 ms to 300 s) | The longest a call waits for a slot; then it fails with `PasswordError::Busy` |
| Stored hash length | 512 bytes | A PHC string with these parameters is about 100 bytes |
| `max_bcrypt_cost` | 14 | A planted `$2b$31$` hash would cost 2^31 rounds |
| argon2 limits when verifying | 65536 KiB, 8 iterations, parallelism 16 | A hash planted in the database costs what it asks for on every login attempt; these caps keep that to a fraction of a second and 64 MiB. They are raised to the hasher's own parameters when those are higher, so its own hashes always verify |

Every stored hash is checked against the limits **before** any key derivation. A hash over a limit is an error from `verify` (`PasswordError::OverLimit { what }` says which one) and a refused login from `verify_or_dummy`; raise the limit with `VerifyLimits` if a legacy database has costlier hashes:

```rust
use corvis::auth::local::{PasswordHasherOptions, VerifyLimits};

PasswordHasherOptions::default().verify_limits(VerifyLimits::default().max_bcrypt_cost(15))
```

The limits have upper bounds: 1048576 KiB of argon2 memory, 64 argon2 iterations and bcrypt cost 20. A `VerifyLimits` above one stops `PasswordHasher::new` with an error, and so do hashing parameters above them. At startup the hasher logs a warning when `max_concurrent` times the largest argon2 memory a stored hash may ask for exceeds 1 GiB: lower `max_concurrent` or the limits.

#### A floor for new hashes

The parameters of new hashes have a floor: argon2id with at least 19456 KiB and 2 iterations (or 47104 KiB and 1), bcrypt cost at least 10. `PasswordHasher::new` returns an error for anything weaker, unless the options say `.allow_weak_parameters()`, which logs a warning. Use that in tests only, where a hash of 8 KiB makes a suite fast. The verification limits are raised to the parameters of the hasher's own hashes (and of its dummy hash), so it can always verify what it made.

### Keeping bcrypt hashes

A database with `bcryptjs` hashes keeps working: verification reads the prefix (`$2a$`, `$2b$`, `$2y$` or `$argon2...`), whatever the configured algorithm is. The results are those of `bcryptjs.compareSync` for every password, including non-ASCII ones, ones with a NUL character and ones over 72 bytes (bcrypt only reads the first 72 bytes, and so does `bcryptjs`). The behavior is checked against `bcryptjs` 3.0.3 in both directions with recorded results.

To move users to argon2id as they log in, verify with `verify_or_dummy` and check `needs_rehash` after a successful verification:

```rust
let ok = hasher
    .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
    .await
    .map_err(PasswordError::into_http_exception)?;
if let Some(user) = user.filter(|_| ok) {
    if hasher.needs_rehash(&user.password_hash) {
        let new_hash = hasher.hash(password).await.map_err(PasswordError::into_http_exception)?;
        users.update_hash(user.id, new_hash).await;
    }
    // ... the login succeeds
}
```

::: danger During a migration, match the dummy hash to the stored hashes
The dummy verification runs with the **configured** algorithm (argon2id by default). While the database still holds bcrypt hashes, a known user with a bcrypt hash verifies faster or slower than the dummy (about 27 ms against 45 ms at cost 10, 179 ms at cost 12, on one machine), so response times tell known usernames from unknown ones. Set `dummy_algorithm` to the **algorithm and cost of the hashes actually stored**, for example `.dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 10 })`, and rehash on login so the old hashes disappear; remove the option when they have. The hasher logs a warning, once, the first time it verifies a stored hash that differs from the dummy hash in algorithm or cost.
:::

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
    .map_err(PasswordError::into_http_exception)?;
let user = user.filter(|_| ok);
```

When the stored hash is refused (malformed, unsupported, over a limit: an empty string for an account created through social login, a `$2x$` hash from an old import) the answer is `Ok(false)` after the dummy verification, so the login is the same `401` as for an unknown user and a broken row does not answer faster or differently. One warning per occurrence names the reason (never the hash, the username or the password), so a broken row is still visible in the logs. `verify` keeps returning the refusal as an error.

The remaining timing differences are the ones you control. Stored hashes with another algorithm or cost verify at their own price: during a migration, [match the dummy hash](#keeping-bcrypt-hashes) to them with `dummy_algorithm`, and let `needs_rehash` close the gap over time. Your own user lookup may take a different time for a missing row.

### A busy hasher

Argon2 costs memory and time on purpose, and a flood of logins queues behind the `max_concurrent` slots. The queue is bounded: at most `max_waiting` calls wait, each for at most `acquire_timeout`. A call that finds the queue full, or waits too long, fails with `PasswordError::Busy` without any key derivation. A login validator turns that into `503 Service Unavailable`, the same for every user so it reveals nothing about the account, with `PasswordError::into_http_exception` (any other error becomes an empty `500`):

```rust
.map_err(PasswordError::into_http_exception)?
```

Also limit login attempts (the [throttler](/security/rate-limiting) and the planned login lockout) so a flood does not keep the slots busy in the first place.

### Blocking threads

`hash_blocking`, `verify_blocking` and the other `*_blocking` methods are for synchronous code, and a synchronous handler is exactly that: it runs on an async worker thread. They are safe there. Inside a multi-thread Tokio runtime they run through `tokio::task::block_in_place`, which hands the worker's other tasks to another thread while the hashing runs. Outside any runtime they just block. Inside a **current-thread** runtime (`#[tokio::test]` without `flavor = "multi_thread"`, for example) they return `PasswordError::CurrentThreadRuntime` instead of risking a deadlock: use the async methods there.

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

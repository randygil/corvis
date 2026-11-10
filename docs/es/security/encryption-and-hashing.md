---
status: partial
---

# Cifrado y hashing

Corvis incluye **hashing de contraseñas** (`PasswordHasher`, con la feature de cargo `auth-local`). No incluye cifrado general: los algoritmos que usa por dentro (SHA-1 para los `ETag`, SHA-256 para las claves del throttler) no forman parte de su API pública, y para el cifrado simétrico usas directamente los crates del ecosistema Rust (ver [más abajo](#cifrado-simetrico)).

## Hashing de contraseñas

Activa la feature en `Cargo.toml`:

```toml
[dependencies]
corvis = { version = "0.1", features = ["auth-local"] }
```

`PasswordHasher` crea hashes **argon2id** para las contraseñas nuevas, verifica hashes argon2 y **bcrypt** (incluidos los que hizo `bcryptjs`) y limita lo que puede costar un login. Regístralo como cualquier provider e inyéctalo donde hagas o verifiques hashes:

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
        // guarda `password_hash`, nunca la contraseña
        Ok(self.repository.insert(username, password_hash).await)
    }
}
```

`Provider::class::<PasswordHasher>()` usa las opciones por defecto. Para otras, constrúyelo tú y registra el valor:

```rust
use corvis::auth::local::{Argon2Params, PasswordHasher, PasswordHasherOptions};

let hasher = PasswordHasher::new(PasswordHasherOptions::argon2id(Argon2Params {
    memory_kib: 65536,
    iterations: 3,
    parallelism: 4,
}))?;
// en el módulo: Provider::use_value(Token::of::<PasswordHasher>(), hasher)
```

`PasswordHasher::new` calcula el hash de una contraseña de relleno (ver [usuarios desconocidos](#usuarios-desconocidos)), así que créalo una sola vez al arrancar.

| Método | Qué hace |
|---|---|
| `hash(password).await` | Una cadena PHC para argon2id, o un hash `$2b$` si está configurado para bcrypt |
| `verify(password, &stored).await` | `Ok(true)`, `Ok(false)`, o un error si el hash guardado está mal formado, no es compatible o supera los límites. El algoritmo sale del hash guardado |
| `verify_or_dummy(password, Option<&str>).await` | `verify` cuando hay hash guardado, y una verificación de relleno cuando no lo hay |
| `verify_dummy(password).await` | Una verificación contra un hash de relleno; siempre `false` |
| `needs_rehash(&stored)` | `true` cuando el hash debería sustituirse (no ejecuta ninguna derivación de clave) |
| `hash_blocking`, `verify_blocking`, `verify_or_dummy_blocking` | Lo mismo, para código fuera de un contexto async. Bloquean el hilo que llama |

### Valores por defecto

| Ajuste | Por defecto | Motivo |
|---|---|---|
| Algoritmo | argon2id, versión 19 | RFC 9106 recomienda argon2id |
| Memoria, iteraciones, paralelismo | 19456 KiB, 2, 1 | El mínimo de la guía de OWASP para almacenar contraseñas, y el valor por defecto del crate `argon2` |
| Sal, salida | 16 bytes, 32 bytes | RFC 9106; la sal sale del generador del sistema operativo |
| Coste de bcrypt (si se elige) | 10 | El valor por defecto de `bcryptjs` |
| `max_password_bytes` | 1024 | Una contraseña larga no sirve para quemar CPU; hashearla da `PasswordError::TooLong`, y verificarla responde `false` sin ninguna derivación de clave |
| `max_concurrent` | `available_parallelism()` | Como máximo esta cantidad de derivaciones de clave a la vez, cada una en un hilo bloqueante y nunca en un worker async; las demás esperan un hueco. Cada derivación por defecto usa unos 19 MiB |
| Longitud del hash guardado | 512 bytes | Una cadena PHC con estos parámetros mide unos 100 bytes |
| `max_bcrypt_cost` | 14 | Un hash `$2b$31$` plantado costaría 2^31 rondas |
| Límites de argon2 al verificar | 262144 KiB, 16 iteraciones, paralelismo 16 | Un hash plantado no puede pedir gigabytes ni minutos |

Todo hash guardado se comprueba contra los límites **antes** de cualquier derivación de clave. Un hash que supera un límite es un error (`PasswordError::OverLimit { what }` dice cuál); sube el límite con `VerifyLimits` si una base de datos heredada tiene hashes más costosos:

```rust
use corvis::auth::local::{PasswordHasherOptions, VerifyLimits};

PasswordHasherOptions::default().verify_limits(VerifyLimits::default().max_bcrypt_cost(15))
```

#### Un mínimo para los hashes nuevos

Los parámetros de los hashes nuevos tienen un mínimo: argon2id con al menos 19456 KiB y 2 iteraciones (o 47104 KiB y 1), bcrypt con coste al menos 10. `PasswordHasher::new` devuelve un error para algo más débil, salvo que las opciones indiquen `.allow_weak_parameters()`, que registra una advertencia. Úsalo solo en tests, donde un hash de 8 KiB hace rápida la suite. Los parámetros por encima de los límites de verificación también son un error, porque el hasher no podría verificar sus propios hashes.

### Conservar los hashes bcrypt

Una base de datos con hashes de `bcryptjs` sigue funcionando: la verificación lee el prefijo (`$2a$`, `$2b$`, `$2y$` o `$argon2...`), sea cual sea el algoritmo configurado. Los resultados son los de `bcryptjs.compareSync` para cualquier contraseña, incluidas las no ASCII, las que llevan un carácter NUL y las de más de 72 bytes (bcrypt solo lee los primeros 72 bytes, y `bcryptjs` también). El comportamiento está comprobado contra `bcryptjs` 3.0.3 en ambos sentidos con resultados grabados.

Para pasar a los usuarios a argon2id cuando inician sesión, consulta `needs_rehash` tras una verificación correcta:

```rust
if hasher.verify(password, &user.password_hash).await? {
    if hasher.needs_rehash(&user.password_hash) {
        users.update_hash(user.id, hasher.hash(password).await?).await;
    }
    // ... el login es correcto
}
```

`needs_rehash` es `true` para otro algoritmo o variante de argon2, menos memoria, iteraciones, longitud de salida o de sal, un coste de bcrypt menor, y para un hash que no se puede leer. Mientras el lado Node tenga que leer los hashes nuevos (una migración en curso), configura bcrypt: `PasswordHasherOptions::bcrypt(10)` crea hashes `$2b$` que `bcryptjs.compareSync` acepta.

::: warning Una diferencia con bcryptjs
`bcryptjs` trunca en silencio las contraseñas de más de 72 bytes al hashearlas, de modo que dos contraseñas largas con los mismos 72 primeros bytes obtienen hashes que se aceptan entre sí. Un `PasswordHasher` configurado para bcrypt se niega a hashear una contraseña de más de 72 bytes (`PasswordError::TooLong`). La verificación sigue a `bcryptjs`, así que los hashes existentes siguen funcionando. Argon2id no tiene ese límite por debajo de `max_password_bytes`.
:::

### Usuarios desconocidos

Un login que responde más rápido para un nombre de usuario desconocido le dice a un atacante qué usuarios existen. `verify_or_dummy` gasta una verificación real en ambos casos, contra un hash de relleno hecho al crear el hasher:

```rust
let user = users.find_by_username(username).await;
let ok = hasher
    .verify_or_dummy(password, user.as_ref().map(|u| u.password_hash.as_str()))
    .await
    .map_err(|_| InternalServerErrorException::empty())?;
let user = user.filter(|_| ok);
```

Cuando se rechaza el hash guardado (mal formado, no compatible, sobre un límite), la verificación de relleno se ejecuta antes de devolver el error, así que una fila rota tampoco responde más rápido. Las diferencias de tiempo que quedan son las que controlas tú: los hashes guardados con parámetros antiguos u otro algoritmo se verifican a su propio coste (`needs_rehash` cierra esa brecha con el tiempo, y `.dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 10 })` se ajusta a una base de datos mayormente bcrypt), y tu propia búsqueda de usuario puede tardar distinto cuando la fila no existe.

Argon2 cuesta memoria y tiempo a propósito: una avalancha de logins hace cola en los `max_concurrent` huecos. Limita los intentos de login (el [throttler](/es/security/rate-limiting) y el bloqueo de login previsto) para que una avalancha no mantenga los huecos ocupados.

La comparación de los valores derivados es de tiempo constante: `argon2` y `password-hash` comparan las salidas con `subtle`, y `bcrypt` también. `PasswordError` y la salida de `Debug` nunca contienen una contraseña ni un hash.

La estrategia `local` usa este hasher; ver [Passport](/es/recipes/passport#estrategia-local-usuario-y-contrasena) y [Autenticación](/es/security/authentication).

## Cifrado simétrico

Para cifrar datos que luego necesitas leer (tokens de terceros, campos sensibles) usa un cifrado
autenticado como AES-256-GCM (el crate `aes-gcm`) o ChaCha20-Poly1305 (el crate `chacha20poly1305`). Lee la
clave de la configuración con `ConfigService` en una factory (`Provider::use_factory`) y no la
escribas en el código. Ver [Configuración](/es/application/configuration) y
[Providers personalizados](/es/fundamentals/custom-providers).

## Tokens firmados

Si lo que necesitas es que el cliente no pueda alterar un dato (y no que no pueda leerlo), puede bastar un JWT
firmado con `JwtService`. Ver [Autenticación](/es/security/authentication).

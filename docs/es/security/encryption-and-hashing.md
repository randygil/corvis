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
| `verify_or_dummy(password, Option<&str>).await` | `verify` cuando hay hash guardado, y una verificación de relleno cuando no lo hay. Un hash guardado rechazado (una cadena vacía, `$2x$`, sobre un límite) da `Ok(false)` tras una verificación de relleno, como un usuario desconocido |
| `verify_dummy(password).await` | Una verificación contra un hash de relleno; `Ok(false)` |
| `needs_rehash(&stored)` | `true` cuando el hash debería sustituirse (no ejecuta ninguna derivación de clave) |
| `hash_blocking`, `verify_blocking`, `verify_or_dummy_blocking`, `verify_dummy_blocking` | Lo mismo, para código síncrono. Bloquean el hilo que llama; ver [hilos bloqueantes](#hilos-bloqueantes) |

### Valores por defecto

| Ajuste | Por defecto | Motivo |
|---|---|---|
| Algoritmo | argon2id, versión 19 | RFC 9106 recomienda argon2id |
| Memoria, iteraciones, paralelismo | 19456 KiB, 2, 1 | El mínimo de la guía de OWASP para almacenar contraseñas, y el valor por defecto del crate `argon2` |
| Sal, salida | 16 bytes, 32 bytes | RFC 9106; la sal sale del generador del sistema operativo |
| Coste de bcrypt (si se elige) | 10 | El valor por defecto de `bcryptjs` |
| `max_password_bytes` | 1024 | Una contraseña larga no sirve para quemar CPU; hashearla da `PasswordError::TooLong`, y verificarla responde `false` sin ninguna derivación de clave |
| `max_concurrent` | `available_parallelism()` | Como máximo esta cantidad de derivaciones de clave a la vez, cada una en un hilo bloqueante y nunca en un worker async; las demás esperan un hueco. Cada derivación por defecto usa unos 19 MiB |
| `max_waiting` | `32 * max_concurrent` (al menos 1) | Cuántas llamadas pueden esperar un hueco. La siguiente falla de inmediato con `PasswordError::Busy`, sin ninguna derivación de clave |
| `acquire_timeout` | 10 s (de 1 ms a 300 s) | Lo máximo que una llamada espera un hueco; después falla con `PasswordError::Busy` |
| Longitud del hash guardado | 512 bytes | Una cadena PHC con estos parámetros mide unos 100 bytes |
| `max_bcrypt_cost` | 14 | Un hash `$2b$31$` plantado costaría 2^31 rondas |
| Límites de argon2 al verificar | 65536 KiB, 8 iteraciones, paralelismo 16 | Un hash plantado en la base de datos cuesta lo que pide en cada intento de login; estos topes lo dejan en una fracción de segundo y 64 MiB. Se suben a los parámetros del propio hasher cuando estos son mayores, así que sus hashes siempre se verifican |

Todo hash guardado se comprueba contra los límites **antes** de cualquier derivación de clave. Un hash que supera un límite es un error de `verify` (`PasswordError::OverLimit { what }` dice cuál) y un login rechazado en `verify_or_dummy`; sube el límite con `VerifyLimits` si una base de datos heredada tiene hashes más costosos:

```rust
use corvis::auth::local::{PasswordHasherOptions, VerifyLimits};

PasswordHasherOptions::default().verify_limits(VerifyLimits::default().max_bcrypt_cost(15))
```

Los límites tienen cotas superiores: 1048576 KiB de memoria de argon2, 64 iteraciones de argon2 y coste 20 de bcrypt. Un `VerifyLimits` por encima de alguna detiene `PasswordHasher::new` con un error, igual que unos parámetros de hashing por encima de ellas. Al arrancar, el hasher registra una advertencia cuando `max_concurrent` multiplicado por la mayor memoria de argon2 que puede pedir un hash guardado supera 1 GiB: baja `max_concurrent` o los límites.

#### Un mínimo para los hashes nuevos

Los parámetros de los hashes nuevos tienen un mínimo: argon2id con al menos 19456 KiB y 2 iteraciones (o 47104 KiB y 1), bcrypt con coste al menos 10. `PasswordHasher::new` devuelve un error para algo más débil, salvo que las opciones indiquen `.allow_weak_parameters()`, que registra una advertencia. Úsalo solo en tests, donde un hash de 8 KiB hace rápida la suite. Los límites de verificación se suben a los parámetros de los hashes del propio hasher (y de su hash de relleno), así que siempre puede verificar lo que hizo.

### Conservar los hashes bcrypt

Una base de datos con hashes de `bcryptjs` sigue funcionando: la verificación lee el prefijo (`$2a$`, `$2b$`, `$2y$` o `$argon2...`), sea cual sea el algoritmo configurado. Los resultados son los de `bcryptjs.compareSync` para cualquier contraseña, incluidas las no ASCII, las que llevan un carácter NUL y las de más de 72 bytes (bcrypt solo lee los primeros 72 bytes, y `bcryptjs` también). El comportamiento está comprobado contra `bcryptjs` 3.0.3 en ambos sentidos con resultados grabados.

Para pasar a los usuarios a argon2id cuando inician sesión, verifica con `verify_or_dummy` y consulta `needs_rehash` tras una verificación correcta:

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
    // ... el login es correcto
}
```

::: danger En una migración, iguala el hash de relleno a los hashes guardados
La verificación de relleno se ejecuta con el algoritmo **configurado** (argon2id por defecto). Mientras la base de datos siga teniendo hashes bcrypt, un usuario conocido con un hash bcrypt se verifica más rápido o más lento que el relleno (unos 27 ms frente a 45 ms con coste 10, 179 ms con coste 12, en una máquina), y los tiempos de respuesta distinguen los usuarios conocidos de los desconocidos. Define `dummy_algorithm` con el **algoritmo y el coste de los hashes realmente guardados**, por ejemplo `.dummy_algorithm(DummyAlgorithm::Bcrypt { cost: 10 })`, y vuelve a hashear al iniciar sesión para que los hashes viejos desaparezcan; quita la opción cuando ya no queden. El hasher registra una advertencia, una sola vez, la primera vez que verifica un hash guardado que difiere del hash de relleno en algoritmo o coste.
:::

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
    .map_err(PasswordError::into_http_exception)?;
let user = user.filter(|_| ok);
```

Cuando se rechaza el hash guardado (mal formado, no compatible, sobre un límite: una cadena vacía en una cuenta creada con login social, un hash `$2x$` de una importación antigua) la respuesta es `Ok(false)` tras la verificación de relleno, así que el login da el mismo `401` que un usuario desconocido y una fila rota no responde más rápido ni distinto. Una advertencia por cada caso nombra el motivo (nunca el hash, el usuario ni la contraseña), de modo que una fila rota sigue visible en los logs. `verify` sigue devolviendo el rechazo como un error.

Las diferencias de tiempo que quedan son las que controlas tú. Los hashes guardados con otro algoritmo o coste se verifican a su propio precio: en una migración, [iguala el hash de relleno](#conservar-los-hashes-bcrypt) a ellos con `dummy_algorithm`, y deja que `needs_rehash` cierre la brecha con el tiempo. Tu propia búsqueda de usuario puede tardar distinto cuando la fila no existe.

### Un hasher ocupado

Argon2 cuesta memoria y tiempo a propósito, y una avalancha de logins hace cola en los `max_concurrent` huecos. La cola tiene un límite: esperan como máximo `max_waiting` llamadas, cada una como mucho `acquire_timeout`. Una llamada que encuentra la cola llena, o espera demasiado, falla con `PasswordError::Busy` sin ninguna derivación de clave. Un validador de login lo convierte en `503 Service Unavailable`, igual para todos los usuarios para no revelar nada de la cuenta, con `PasswordError::into_http_exception` (cualquier otro error se convierte en un `500` vacío):

```rust
.map_err(PasswordError::into_http_exception)?
```

Limita además los intentos de login (el [throttler](/es/security/rate-limiting) y el bloqueo de login previsto) para que una avalancha no mantenga los huecos ocupados desde el principio.

### Hilos bloqueantes

`hash_blocking`, `verify_blocking` y los demás métodos `*_blocking` son para código síncrono, y un handler síncrono es justo eso: se ejecuta en un hilo worker async. Allí son seguros. Dentro de un runtime Tokio multi-hilo se ejecutan con `tokio::task::block_in_place`, que pasa las demás tareas del worker a otro hilo mientras dura el hashing. Fuera de cualquier runtime simplemente bloquean. Dentro de un runtime de **un solo hilo** (por ejemplo `#[tokio::test]` sin `flavor = "multi_thread"`) devuelven `PasswordError::CurrentThreadRuntime` en lugar de arriesgar un bloqueo permanente: allí usa los métodos async.

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

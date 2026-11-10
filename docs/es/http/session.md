---
status: documented
---

# Sesión

Las sesiones de servidor funcionan como `express-session`: el navegador guarda una cookie firmada con un identificador y
los datos viven en un almacén en el servidor. La cookie y los registros guardados usan el mismo formato que
`express-session` 1.19.0, así que una aplicación Node y una de Corvis que comparten secreto y almacén pueden leer las
sesiones de la otra (ver [Lectura cruzada con una app Node](#lectura-cruzada-con-una-app-node)).

## Activarlas

Las sesiones necesitan la feature de cargo `auth-session`:

```toml
corvis = { version = "0.1", features = ["auth-session"] }
```

Todo lo que sigue está en `corvis::auth::session`, salvo `CookieKeys`, que está en `corvis::http` (ver
[Cookies](/es/http/cookies#cookies-firmadas)).

## Configuración

`SessionModule` es un módulo dinámico global. Impórtalo una vez:

```rust
use corvis::auth::session::{SameSite, SessionCookie, SessionModule, SessionOptions};
use std::time::Duration;

fn session_options() -> SessionOptions {
    SessionOptions::with_secrets([std::env::var("SESSION_SECRET").unwrap_or_default()])
        .expect("SESSION_SECRET must be at least 32 bytes")
        .name("connect.sid")
        .cookie(SessionCookie::default().same_site(SameSite::Strict))
        .idle_timeout(Duration::from_secs(24 * 3600))
        .absolute_timeout(Duration::from_secs(7 * 24 * 3600))
}

#[module(
    imports = [SessionModule::for_root(session_options())],
    controllers = [CartController],
)]
pub struct AppModule;
```

`SessionOptions::with_secrets([..])` y `SessionOptions::new(CookieKeys::new([..])?)` devuelven un error cuando un secreto
no es válido (ver [Secretos y rotación](#secretos-y-rotacion)). Para leer el secreto de la configuración usa
`for_root_async`, que recibe un `ResolveCtx`:

```rust
SessionModule::for_root_async(|ctx: ResolveCtx| async move {
    let config = ctx.get::<ConfigService>().await?;
    Ok(SessionOptions::with_secrets([config.get::<String>("SESSION_SECRET").unwrap_or_default()])?)
})
```

Todas las opciones:

| Método | Por defecto | Significado |
|---|---|---|
| `.name("connect.sid")` | `connect.sid` | nombre de la cookie |
| `.cookie(SessionCookie::..)` | ver [La cookie](#la-cookie) | atributos de la cookie |
| `.idle_timeout(d)` | 24 h | tiempo sin peticiones tras el cual termina la sesión |
| `.absolute_timeout(d)` / `.no_absolute_timeout()` | 7 días | vida máxima de una sesión desde que se generó su identificador |
| `.rolling(bool)` | `false` | enviar la cookie en cada respuesta |
| `.resave(bool)` | `false` | escribir la sesión en el almacén aunque no se haya modificado |
| `.save_uninitialized(bool)` | `false` | guardar una sesión nueva en la que nadie escribió |
| `.keep_session_info_on_login(bool)` | `false` | conservar los datos de la sesión en `log_in` y `log_out` |
| `.max_session_bytes(n)` | 16 KiB | límite de tamaño de un registro serializado |
| `.store_timeout(d)` | 5 s | límite para cada llamada al almacén |
| `.store(almacen)` | `MemorySessionStore::new()` | dónde se guardan las sesiones |

Las opciones no válidas detienen `init()` con un error que nombra la opción y nunca el secreto. La línea de log de
arranque indica los atributos de la cookie, y se registran avisos para `secure(false)`, `http_only(false)`,
`no_absolute_timeout()` y para el almacén en memoria.

## La API de `Session`

`Request::session()` devuelve un `Option<Session>`: `None` cuando ningún módulo de sesión se ejecuta para la petición
(por ejemplo, una petición fuera del `Path` de la cookie). La API es síncrona: el middleware hace todo el acceso al
almacén antes de los guards y después del handler.

```rust
use corvis::auth::session::Session;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Cart {
    items: Vec<String>,
}

#[controller("cart")]
pub struct CartController;

#[routes]
impl CartController {
    #[post("add")]
    fn add(&self, #[req] req: Request, #[body] dto: AddItemDto) -> HttpResult<Cart> {
        let session = req.session().ok_or_else(|| InternalServerErrorException::new("no session"))?;
        let mut cart: Cart = session.get("cart").unwrap_or_default();
        cart.items.push(dto.item);
        session.set("cart", &cart).map_err(HttpException::internal)?;
        Ok(cart)
    }
}
```

| Método | Qué hace |
|---|---|
| `id()` | el identificador de la sesión |
| `is_new()` | la sesión fue creada por esta petición |
| `get::<T>(key)` | un valor deserializado como `T`, o `None` |
| `get_value(key)` | el valor JSON de una clave |
| `data()` | todos los valores |
| `set(key, value)` | guarda un valor. Falla con `SessionError` para las claves reservadas `cookie` y `__corvis`, o para un valor que no se serializa |
| `remove(key)`, `clear()` | quitan una clave o todas |
| `regenerate()` | una sesión nueva y vacía con un identificador nuevo; el registro anterior se destruye |
| `regenerate_keeping_data()` | un identificador nuevo y los mismos datos. Llámalo cuando cambian los privilegios y la sesión debe continuar |
| `destroy()` | el registro se destruye al terminar la petición, se borra la cookie y se descartan los datos. Los valores escritos después inician una sesión nueva |

`regenerate()` es el de `express-session`. El `Debug` de una `Session` no imprime ni el identificador ni los datos.

## La cookie

La cookie se llama `connect.sid` y su valor es `s:<id>.<firma>`, codificado como URI, firmado con el primer secreto y
verificado con todos. El identificador son 24 bytes aleatorios del generador del sistema operativo, en base64url (32
caracteres); nunca se deriva de datos del usuario. Un identificador que se verifica pero que el almacén no conoce nunca
se adopta: se genera uno nuevo.

`SessionCookie::default()` da `Path=/; HttpOnly; Secure; SameSite=Lax`, sin `Domain` y sin expiración:

| Atributo | Por defecto | Por qué |
|---|---|---|
| `HttpOnly` | activo | un script (y por tanto un XSS) no puede leer el identificador |
| `Secure` | activo | el navegador no envía la cookie por una conexión sin cifrar. El `Set-Cookie` de una respuesta emitida por HTTP plano sí viaja en claro, y con él su contenido |
| `SameSite` | `Lax` | el navegador no envía la cookie en `POST` y `fetch` entre sitios. No es protección CSRF por sí sola, ver [CSRF](/es/security/csrf) |
| `Path` | `/` | toda la aplicación. Una petición cuya ruta no empieza por el `Path` no tiene sesión |
| `Domain` | ninguno | la cookie va solo al host que la emitió, no a los subdominios |
| expiración | ninguna | cookie de sesión del navegador. El servidor termina las sesiones con los tiempos límite de abajo |

Cámbialos con `SessionCookie`: `.secure(bool)`, `.http_only(bool)`, `.same_site(SameSite::Strict)`,
`.without_same_site()`, `.max_age(Duration)`, `.domain("example.com")` y `.path("/app")`. Con `max_age` la cookie lleva
`Expires` y nunca `Max-Age`, como el original, y nunca más tarde que el límite absoluto de la sesión.

La cookie `Secure` se envía en cada respuesta: el framework no puede saber el transporte, y un proxy inverso suele
terminar TLS. Los navegadores difieren en si guardan una cookie `Secure` desde `http://localhost`.

Prefiere el prefijo `__Host-` (`.name("__Host-sid")`). Un navegador rechaza esa cookie salvo que sea `Secure`, tenga
`Path=/` y no tenga `Domain`, y sobre todo impide que un subdominio hermano la plante. Corvis valida esas reglas al
arrancar para `__Host-` y para `__Secure-`, sin distinguir mayúsculas, como los comparan los navegadores (`__host-sid` con
un `Domain` no arranca).

::: warning Cookie tossing
Sin el prefijo `__Host-`, un subdominio hermano (o cualquier sitio que pueda ejecutar código en uno) puede plantar para
todo el dominio una cookie con el nombre de la sesión. Dos cookies con ese nombre no cargan ninguna sesión (ver abajo),
así que la cookie plantada cierra la sesión de la víctima mientras viva, y desde ese navegador no podrá iniciar sesión
de nuevo hasta que se elimine. Usa `__Host-`.
:::

::: warning El nombre de la cookie no forma parte de la firma
Como en el original, la firma cubre el identificador y nada más. Un valor firmado con el mismo secreto para otra cookie
se verifica como valor de la cookie de sesión, y la cookie de sesión reenviada con el nombre de otra cookie firmada que
el código de la aplicación lee con `signed_cookie` entrega a ese código el identificador de sesión. Nunca firmes tus
propias cookies con los secretos de la sesión: usa para la aplicación unos `CookieKeys` propios (ver
[Cookies](/es/http/cookies)).
:::

### Desarrollo en HTTP plano

En local, sin TLS, desactiva `Secure` y nada más:

```rust
SessionOptions::with_secrets([secret])?.cookie(SessionCookie::default().secure(false))
```

Se registra un aviso al arrancar. Hazlo desde la configuración, no en código que llegue a producción.

### Cookies inválidas y repetidas

Las cookies inválidas (ausentes, sin `s:`, con firma incorrecta, con un identificador que no son 1 a 128 bytes de ASCII
visible, o de más de 4096 bytes tras decodificar) se ignoran, sin acceso al almacén y sin error. Si la petición trae
**dos o más** cookies con el nombre de la sesión (por ejemplo una plantada desde un subdominio hermano, también entre
varias cabeceras `Cookie`), todas se ignoran y la petición corre con una sesión nueva. Es distinto de `req.cookie()`,
donde gana la primera.

## Secretos y rotación

- Un secreto debe tener al menos 32 bytes; uno más corto detiene el arranque con `the session secret at position 0 has
  N bytes: the minimum is 32 bytes`. Los errores nombran la posición y la longitud, nunca el secreto.
- De 1 a 8 secretos. El primero firma, todos verifican.
- Genera uno con `openssl rand -hex 32`, mantenlo fuera del repositorio y nunca lo registres en logs.

Para rotar: pon el secreto nuevo primero y conserva el viejo.

```rust
SessionOptions::with_secrets([new_secret, old_secret])?
```

Conserva el viejo hasta que pase la vida más larga de una cookie (el tiempo límite absoluto, 7 días por defecto) y
entonces quítalo. Durante la rotación una cookie firmada con un secreto retirado se acepta, y la respuesta la vuelve a
enviar firmada con el nuevo.

## Tiempos límite

Cada registro lleva `__corvis: { createdAt, lastAccess }` (milisegundos) junto al objeto `cookie` y los datos. El
middleware aplica los tiempos límite a partir de esas marcas en cada carga, haga lo que haga el almacén con el ttl:

- **Tiempo de inactividad** (24 h): tiempo desde el último acceso.
- **Tiempo absoluto** (7 días): tiempo desde que se generó el identificador. Login, logout y `regenerate` inician un
  nuevo periodo. `.no_absolute_timeout()` lo quita y registra un aviso.
- Un `cookie.expires` en el pasado también termina la sesión.

Un registro vencido se destruye y la petición recibe una sesión nueva; si llegó una cookie válida, se le indica al
navegador que la descarte. `rolling` solo extiende el tiempo de inactividad y la cookie: nunca extiende una sesión más
allá del tiempo absoluto (la expiración de la cookie también queda limitada por él).

Un registro sin `__corvis` (escrito por el original) se acepta y recibe ambas marcas en su primera carga. Las marcas del
futuro cuentan como ahora, y las marcas inutilizables destruyen el registro.

El ttl que reciben los almacenes es el menor entre el tiempo de inactividad, el tiempo absoluto restante y la vida
restante de la cookie (al menos 1 s). Cada llamada al almacén está acotada por `store_timeout`; un vencimiento es el
[mismo 500](#comportamiento-ante-fallos) que un error del almacén.

## Límites

| Límite | Por defecto | Rango |
|---|---|---|
| secretos | ninguno | de 1 a 8, cada uno de al menos 32 bytes |
| entrada firmada | | como máximo 4096 bytes |
| identificador | 24 bytes aleatorios (32 caracteres) | se acepta: 1 a 128 bytes de ASCII visible |
| nombre, ruta y dominio de la cookie | `connect.sid`, `/`, ninguno | como máximo 128, 1024 y 253 bytes |
| `idle_timeout` | 24 h | de 60 s hasta el tiempo absoluto |
| `absolute_timeout` | 7 días | como máximo 400 días |
| `max_age` de la cookie | ninguno | como máximo el tiempo absoluto |
| `max_session_bytes` | 16 KiB | de 1 KiB a 1 MiB |
| profundidad de anidamiento de un registro | 100 niveles | el registro es el nivel 1, así que un valor anida 99 niveles |
| `store_timeout` | 5 s | de 100 ms a 60 s |
| `max_sessions` del almacén en memoria | 100 000 | al menos 1 |
| `max_total_bytes` del almacén en memoria | 256 MiB | al menos 1 |
| `max_sessions_per_user` del almacén en memoria | 16 | al menos 1 |

Un registro guardado mayor que `max_session_bytes` (escrito por otro escritor de un almacén compartido) no es una sesión
al leerlo: no se carga, su identificador no se adopta y el registro no se elimina. Un aviso, como máximo una vez por
minuto y sin identificadores, lo reporta.

Un registro anidado a más de 100 niveles (contando el objeto que contiene la sesión) se rechaza al guardarlo, con el
[mismo 500](#comportamiento-ante-fallos) que una sesión mayor que `max_session_bytes`, de modo que el cambio del handler
no se pierda en silencio más tarde: un parser lee como máximo 128 niveles, así que un registro más profundo se podría
escribir y no volver a leer. Un registro guardado más profundo que el límite no es una sesión al leerlo, igual que uno
demasiado grande. Un registro que no se puede interpretar en absoluto lo destruye el almacén en memoria y no es una
sesión, con un aviso como máximo una vez por minuto.

Otras combinaciones que detienen el arranque: `SameSite=None` sin `Secure`, un nombre `__Host-` sin `Secure`, con
`Domain` o con un `Path` distinto de `/`, un nombre `__Secure-` sin `Secure`, y un nombre, ruta o dominio de cookie
inválido.

## El almacén en memoria

`MemorySessionStore` es el predeterminado. Es para un solo proceso y para desarrollo:

```rust
.store(MemorySessionStore::new().max_sessions(100_000).max_total_bytes(256 << 20).max_sessions_per_user(16))
```

- Las sesiones se pierden al reiniciar la aplicación y no se comparten entre instancias. El arranque registra un aviso.
- Los registros se guardan serializados, y el límite de bytes cuenta la asignación de cada registro, su identificador, la
  clave de su usuario y una sobrecarga fija por entrada (640 bytes), de modo que `max_total_bytes` acota la memoria que
  usa el almacén, sea cual sea la forma de los datos (un arreglo de números pequeños cuesta lo que cuesta su texto). Una
  prueba con un asignador contador lo comprueba. Las tablas hash devuelven su espacio cuando contienen menos de la cuarta
  parte, de modo que una ráfaga de sesiones que ya terminó no deja nada atrás.
- Nada proporcional al tamaño de un registro ocurre mientras se mantiene el bloqueo del almacén: una carga clona una
  referencia e interpreta el texto fuera del bloqueo, y una petición que no cambió nada solo reemplaza un fragmento
  pequeño (`cookie` y `__corvis`) de la entrada. Las sesiones grandes de un usuario no ralentizan a los demás.
- Los registros vencidos se eliminan sin tarea en segundo plano: al cargar y unos pocos en cada escritura.
- **Por usuario:** como máximo `max_sessions_per_user` sesiones (16 por defecto), identificadas por el
  `passport.user` serializado. Una sesión nueva de un usuario que está en el límite expulsa la sesión más antigua de ese
  mismo usuario, nunca la de otro: una cuenta que inicia sesión una y otra vez no puede cerrar la sesión de las demás.
- **Cuando el almacén está lleno** expulsa primero la sesión **anónima** (sin `passport.user`) menos usada recientemente
  y después la sesión más antigua del usuario con más sesiones. Una avalancha de sesiones anónimas llena el almacén con
  sus propios registros pero no cierra la sesión de nadie. Se registra un aviso limitado en frecuencia con el conteo, sin
  identificadores. Ante una avalancha de muchas cuentas, los usuarios que tienen varias sesiones pierden primero las más
  antiguas.
- Un registro mayor que `max_total_bytes` es un error.

Los demás almacenes necesitan su propia política: un almacén compartido debe acotar por sí mismo las sesiones de un
usuario (el contrato de `SessionStore` no lo hace).

Por defecto solo una ruta que escribe en la sesión crea una, así que una visita anónima no cuesta nada. Si una ruta
escribe para visitantes anónimos, límitale la frecuencia con [`ThrottlerModule`](/es/security/rate-limiting). Limita
también la ruta de login: cada login crea una sesión.

Los almacenes Redis y de ORM todavía no son parte del framework; están planificados como trabajo posterior. Mientras
tanto, escribe el almacén tú mismo.

## Escribir un almacén

Implementa `SessionStore`, un trait asíncrono:

| Método | Contrato |
|---|---|
| `load(id) -> Result<Option<Value>, _>` | el registro, o `None` |
| `create(id, &record, ttl)` | debe fallar con `SessionStoreError::Exists`, dejando el registro, cuando el identificador existe |
| `update(id, &record, ttl) -> Result<bool, _>` | `Ok(false)` sin escribir nada cuando el registro no existe |
| `touch(id, &record, ttl) -> Result<bool, _>` | **obligatorio, sin implementación por defecto.** Debe renovar solo la expiración y las claves `cookie` y `__corvis`, nunca los datos: la copia de `record` se leyó al empezar la petición y puede estar desactualizada, así que una petición sin modificaciones no debe sobrescribir lo que guardó otra. `Ok(false)` sin escribir nada cuando el registro no existe |
| `destroy(id) -> Result<bool, _>` | `Ok(true)` cuando se eliminó un registro, `Ok(false)` cuando ya no estaba (no es un error). El middleware lo usa para saber que otra petición terminó la sesión |
| `in_process() -> bool` | `false` por defecto; `true` hace que el arranque registre el aviso de memoria |
| `validate() -> Result<(), String>` | se comprueba al arrancar |

`create` y `update` deben ser atómicos (`SET NX` y `SET XX` en Redis, una sentencia condicional en SQL): comprobar que el
registro existe y escribir después en dos pasos reabre la ventana que permite a una petición lenta deshacer un logout.

Los errores son `SessionStoreError::{Exists, Timeout, Backend(String)}`; construye el último con
`SessionStoreError::backend(msg)` y mantén identificadores y datos fuera del mensaje.

Los registros son el JSON de `express-session`: un objeto `cookie` (`originalMaxAge`, `expires` como cadena ISO o
`null`, `secure`, `httpOnly`, `domain` si está definido, `path`, `sameSite` en minúsculas si está definido), las claves
de datos, y `passport: { user: <serializado> }` para un usuario con sesión iniciada. Es el formato que usan los almacenes
al estilo `connect-redis`.

## Reglas de guardado

Son las reglas de `express-session` 1.19.0 para las mismas opciones.

- **Identificador nuevo:** `create` cuando la sesión se modificó o `save_uninitialized` está activo, y con él `Set-Cookie`.
- **Identificador existente:** `update` cuando se modificó o `resave` está activo; en otro caso `touch`.
- **`Set-Cookie`** en una sesión existente cuando `rolling` está activo, cuando la cookie tiene `max_age` y la sesión se
  modificó, o cuando la cookie recibida solo se verificó con un secreto retirado (se firma de nuevo con el actual).

"Modificada" significa que los datos difieren de lo cargado o que el identificador cambió; asignar a una clave el valor
que ya tiene no es una modificación. La escritura en el almacén se espera antes de devolver la respuesta. Si la petición
termina en error antes de que exista una respuesta (por ejemplo un `404`), una sesión existente se actualiza o se toca,
pero una nueva no se guarda.

Los valores por defecto `resave(false)` y `save_uninitialized(false)` difieren del original, donde ambos son `true`: una
visita anónima no crea nada.

## Comportamiento ante fallos

Un guardado que falla (un error del almacén, un vencimiento, una sesión mayor que `max_session_bytes` o un fallo del
generador de identificadores) convierte la respuesta en un `500` simple, idéntico para toda sesión y sin cookie:

```json
{"statusCode":500,"message":"Internal server error"}
```

Los efectos secundarios del handler ya ocurrieron. Un fallo del almacén al cargar es el mismo `500`. Se registra un
aviso como máximo una vez por minuto por motivo, sin identificadores, valores de cookie, datos ni el mensaje propio del
almacén.

Concurrencia: entre peticiones concurrentes de una misma sesión gana la última escritura, pero la destrucción es
definitiva. `update` y `touch` nunca crean un registro, y `create` nunca sobrescribe (una colisión se reintenta una vez
con un identificador nuevo). Una petición lenta que empezó antes de un logout (o de `destroy()`) no puede devolver la
sesión a la vida: cuando la petición termina, una sesión que cargó y cuyo registro ya no existe (la destrucción que puso
en cola no encontró nada, o `update` / `touch` no encontraron registro) se trata como destruida. No se crea ni se
actualiza nada, haya hecho la petición lo que haya hecho con la sesión (`regenerate_keeping_data()`, `regenerate()`, un
cambio de datos), y la cookie se borra. Si el cliente se desconecta a mitad de la petición, los identificadores en cola
para destrucción (logout, `regenerate`, `destroy`) igualmente se destruyen.

Tres casos están decididos a propósito:

- **Un login sobre una sesión anónima que no conserva nada de ella** (el valor por defecto: los datos de antes del
  login se descartan) crea la sesión nueva aunque el registro anterior ya no exista. De lo contrario, quien inunde el
  almacén hasta expulsar la sesión anónima de la víctima podría hacer que el login responda `200` sin poner cookie. El
  usuario acaba de demostrar quién es y no se recupera nada de la sesión vieja; el identificador viejo se destruye
  igualmente. Con `keep_session_info_on_login(true)`, `regenerate_keeping_data()` y los guardados simples se aplica la
  regla anterior. También se aplica a un login sobre una sesión que **ya tenía usuario**: ese login no es una prueba
  nueva de identidad (un handler puede llamar a `log_in` con el mismo usuario que la sesión restauró), así que si la
  petición A cierra sesión con la cookie C0 mientras la petición B, con C0, inicia sesión otra vez, la sesión queda
  cerrada y el usuario vuelve a iniciar sesión.
- **Un logout no termina a un sucesor.** Un logout que cargó la sesión X no termina una sesión Y que otra petición
  concurrente creó regenerando X. Es el comportamiento del original; cerrarlo exigiría una marca "movida a" en el
  registro (una posible tarea futura).
- **Un cliente que se desconecta durante un login hecho por el guard** hace que la sesión anterior se destruya y no se
  restaure: el login se descarta y también el identificador al que reemplazaba, la dirección segura.

## Login y logout

Iniciar sesión mediante una sesión necesita un `SessionSerializer`, que decide qué se guarda del usuario y cómo
recuperarlo, y un `SessionStrategy<S>`, registrado como la estrategia llamada `"session"` (`SESSION_STRATEGY`):

```rust
use corvis::auth::session::{SessionSerializer, SessionStrategy};

#[injectable]
pub struct UserSerializer {
    users: Arc<UsersService>,
}

#[async_trait]
impl SessionSerializer for UserSerializer {
    type User = Account;

    async fn serialize_user(&self, user: &Account) -> Result<Value, HttpException> {
        Ok(json!({ "id": user.id }))
    }

    async fn deserialize_user(&self, _req: &Request, stored: Value) -> Result<Option<Account>, HttpException> {
        let Some(id) = stored.get("id").and_then(Value::as_u64) else { return Ok(None) };
        Ok(self.users.find(id).await) // None cuando el usuario ya no existe
    }
}

// providers = [UsersService, Provider::strategy::<SessionStrategy<UserSerializer>>()]
```

**`serialize_user` debe ser estable entre logins.** El almacén en memoria identifica su límite por usuario
(`max_sessions_per_user`) por el usuario serializado. Si el valor incluye una marca de tiempo, un token aleatorio o
cualquier cosa que cambie de un login al siguiente, cada login parece un usuario nuevo y el límite nunca se aplica.
Guarda el identificador del usuario y nada que cambie.

Un segundo `SessionStrategy` detiene el arranque. En cada petición cuya sesión tiene `passport.user`, el middleware llama
a `deserialize_user` antes de los guards:

- `Some(user)`: el usuario tipado y su JSON se ponen en la petición (`User<T>` y `Request::user()`).
- `None`: `passport.user` se quita de la sesión guardada y la petición es anónima.
- `Err(e)`: `e` es la respuesta y el handler no se ejecuta.

Un usuario cuyo JSON es `null`, `false`, `0` o `""` nunca se restaura ni se guarda; iniciar sesión con uno es un `500` con
un mensaje fijo. Envuelve esos identificadores en un struct, como hace el ejemplo de arriba.

La ruta de login usa la estrategia `"local"` (ver
[Passport](/es/recipes/passport#estrategia-local-usuario-y-contrasena)) con `.log_in()`:

```rust
#[controller("auth")]
pub struct AuthController {
    authenticator: Arc<Authenticator>,
}

#[routes]
impl AuthController {
    #[post("login")]
    #[use_guards(AuthGuard::new("local").log_in())]
    fn login(&self, user: User<Account>) -> Account {
        user.0
    }

    #[get("me")]
    #[use_guards(AuthGuard::new("session"))]
    fn me(&self, user: User<Account>) -> Account {
        user.0
    }

    #[post("logout")]
    #[http_code(204)]
    async fn logout(&self, #[req] req: Request) -> HttpResult<()> {
        self.authenticator.log_out(&req).await
    }
}
```

`Authenticator::log_in(&req, &AuthenticatedUser::new(user)?)` hace lo mismo desde tu propio código
(`AuthGuard::new("local").log_in()` lo llama tras una autenticación exitosa). Este:

- genera un identificador nuevo y destruye el registro anterior, antes de crear el nuevo;
- descarta los datos previos al login, salvo con `keep_session_info_on_login(true)`;
- guarda el usuario serializado en `passport.user` y establece el usuario de la petición.

Reemplazar el identificador evita la fijación de sesión. `Authenticator::log_out(&req)` quita el usuario, genera un
identificador nuevo, destruye el registro anterior, descarta los datos salvo `keep_session_info_on_login` y limpia el
usuario de la petición. Una sesión que todavía no tiene registro no se regenera: cerrar sesión de forma anónima no crea
nada.

Ambos fallan con un `500` y un mensaje fijo, sin guardar nada, cuando la petición no tiene sesión o no hay ningún
`SessionStrategy` registrado. Un login fallido (`401`) deja la sesión y su identificador sin cambios.

**Un login hecho por el guard solo se conserva cuando la petición no es rechazada.** Si un guard posterior deniega, un pipe
o la validación fallan, el handler devuelve un `Err` o entra en pánico, o falla un interceptor, el login se descarta: la
sesión con la que llegó la petición, si la había, queda como estaba y no se emite ninguna cookie con la sesión iniciada.
El router anota el rechazo en el momento en que ocurre, antes de que un [filtro de excepciones](/es/exception-filters) o
un interceptor puedan cambiar la respuesta, así que lo que responda un filtro no importa: un filtro que convierte el
`403` en una redirección a `/mfa`, o en una página de "acceso denegado" con estado `200`, no conserva el login. Lo mismo
vale cuando un interceptor externo recupera el fallo de uno interno y cuando el handler se descarta antes de terminar
(un interceptor de timeout). Un interceptor que responde por sí mismo **sin** llamar al handler y retorna con normalidad
ocupa el lugar del handler: su login se conserva.

```rust
#[use_guards(AuthGuard::new("local").log_in(), SecondFactorGuard)] // si el segundo guard deniega no queda login,
                                                                   // responda lo que responda un filtro
```

Una petición cancelada antes de que exista su respuesta (el cliente se desconecta, un timeout) también descarta el
login: la sesión anterior se destruye en lugar de restaurarse, la dirección segura.

Un handler que **devuelve con normalidad** conserva el login, sea cual sea el estado que fije, incluida una redirección
que emita él mismo. Por eso un handler que implementa un segundo paso no debe usar `.log_in()` en el guard del primer
paso: autentica con `AuthGuard::new("local")` (sin `.log_in()`) y llama a `Authenticator::log_in` cuando el último paso
haya pasado. Un login hecho con `Authenticator::log_in` desde un handler es decisión del handler y se mantiene sea cual
sea el estado.

`AuthGuard::new("local").log_in()` es un error de arranque cuando no hay ningún `SessionStrategy` registrado (el error
nombra la ruta) y cuando el guard nombra la estrategia `"session"` (`cannot be combined`). Sin `.log_in()` un `AuthGuard`
nunca escribe en la sesión.

## Guards y el usuario de la sesión

- El middleware restaura el usuario en **toda ruta que cubre el módulo**, incluidas las rutas sin `AuthGuard` y las
  rutas `#[public]`: `Request::user()` y `User<T>` lo ven ahí, igual que `passport.session()` establece `req.user` en
  todas partes.
- `AuthGuard::new("session")` tiene éxito con el usuario restaurado (lo lee de la sesión, no de `Request::user()`) y
  responde `401 {"message":"Unauthorized","statusCode":401}` a una petición anónima. La estrategia del original siempre
  pasa.
- Otro `AuthGuard` que tenga éxito reemplaza el usuario de la petición para esa petición.
- Un guard opcional (`.optional()`), o una estrategia que pasa, limpia el usuario de la petición cuando no encuentra
  credenciales, como `req.user = null` en el original. Así, `AuthGuard::new("jwt").optional()` oculta el usuario de la
  sesión en esa ruta. Usa `AuthGuard::chain(["session", "jwt"]).optional()` (sesión primero) para conservarlo y aun así
  aceptar tokens.
- Los guards con `.skip_public()` dejan en paz al usuario de la sesión en las rutas públicas.

::: warning El usuario de la sesión es visible en todas las rutas
El middleware restaura el usuario también en las rutas sin `AuthGuard`. Un guard escrito a mano que lee `req.user()` (un
guard de roles, por ejemplo) autentica entonces solo por la cookie en esas rutas, sin comprobar nada de la petición:

```rust
// Autenticado solo por la cookie de sesión en cualquier ruta a la que se asocie, incluido un POST de formulario entre sitios.
ctx.request().user().and_then(|u| u.get("role").cloned()) == Some(json!("admin"))
```

Asocia también las rutas que cambian estado a `AuthGuard::new("session")`, y protégelas de las peticiones entre sitios:
la cookie de sesión es una credencial ambiental. Ver [protección CSRF](/es/security/csrf), que cubre también el CSRF de
login: `SameSite=Lax` no lo detiene.
:::

Los upgrades de WebSocket y los handshakes de Socket.IO se responden antes del middleware HTTP, así que
`Request::session()` no existe ahí y una cookie de sesión no autentica un socket. Autentica en `handle_connection` de otra
forma (ver [Gateways](/es/websockets/gateways)).

## Orden del middleware

El middleware de `SessionModule` es middleware de módulo, así que se ejecuta después del middleware registrado con
`app.use_middleware(..)`, que por tanto no ve la sesión. Para ejecutarlo primero, regístralo tú con el
`Arc<SessionManager>`:

```rust
app.use_middleware(SessionMiddleware::new(manager));
```

Una petición que ya tiene sesión pasa sin cambios por el segundo registro. El middleware se aplica a todas las rutas,
como `app.use(session())`. Una petición sin cookie válida no cuesta acceso al almacén, y los archivos estáticos servidos
por el router también lo ejecutan cuando traen una cookie.

## Lectura cruzada con una app Node

Con el mismo nombre de cookie (`connect.sid`), el mismo secreto y los mismos registros del almacén, cada lado lee las
sesiones del otro: `passport.user` contiene el usuario serializado, y la clave extra `__corvis` es un dato más para Node.
La cookie de Corvis lleva `Expires` y no `Max-Age`, como la de Node.

Está probado contra `express-session` 1.19.0, `passport` 0.7.0, `cookie-signature` 1.0.7 y `cookie-parser` 1.4.7 con
fixtures grabados (`crates/corvis/tests/fixtures/auth/session.mjs` y `signed_cookies.mjs`).

## Diferencias con el original

| `express-session` | Corvis |
|---|---|
| sin `SameSite` ni `Secure` por defecto | `SameSite=Lax` y `Secure` |
| un login por el guard se mantiene aunque un guard posterior o el handler rechacen la petición | se descarta cuando el pipeline rechaza la petición (un guard que deniega, un pipe que falla, un error o pánico del handler), responda lo que responda un filtro; un handler que devuelve con normalidad lo conserva |
| una petición que sobrevive a un logout puede volver a guardar la sesión | una sesión cuyo registro ya no existe al terminar la petición se trata como destruida y la cookie se borra |
| `MemoryStore` conserva todas las sesiones de una cuenta | como máximo `max_sessions_per_user` (16) en el almacén en memoria |
| `resave` y `save_uninitialized` son `true` por defecto, con un aviso | ambos `false` |
| sin claves extra | una clave `__corvis` con marcas de tiempo |
| sin tiempo de inactividad ni absoluto | tiempos de inactividad (24 h) y absoluto (7 días) |
| sin límite de tamaño | 16 KiB por registro |
| `store.set` crea o sobrescribe | `update` y `touch` nunca vuelven a crear un registro |
| la cookie conserva la firma con la que llegó | se firma de nuevo con el secreto nuevo tras una rotación |
| una cookie `Secure` no se envía por HTTP plano | se envía en toda conexión |
| secretos de cualquier longitud | se rechazan los de menos de 32 bytes |
| `MemoryStore` crece sin límite | acotado, con expulsión |
| cookies de sesión repetidas: gana la primera | todas se ignoran |
| no se envía nada tras `destroy()` o la expiración | se borra la cookie |
| cerrar sesión de una sesión anónima la regenera | no crea nada |
| la estrategia de sesión siempre pasa; las apps escriben `isAuthenticated()` | `AuthGuard::new("session")` falla para anónimos |
| un guardado fallido es el error del almacén | un `500` simple |
| llamadas al almacén sin límite de tiempo | acotadas por `store_timeout` |
| las cookies firmadas JSON con `j:` se decodifican | no se decodifican |
| opciones `genid`, `unset`, `proxy`, ediciones por petición de `req.session.cookie`, `store.all`, `store.clear`, `store.length` | no disponibles |

El original nunca adopta un identificador que su almacén no conoce, y Corvis se comporta igual.

## Qué no cubre

- **WebSockets.** Ver arriba: no hay sesión en un socket.
- **Tokens CSRF.** La cookie es `SameSite=Lax` por defecto, lo que ayuda pero no reemplaza una comprobación. Ver
  [CSRF](/es/security/csrf).
- **Almacenes Redis y de ORM.** Llegan después; el formato de registro es compatible con los almacenes al estilo
  `connect-redis`.

## Varias instancias

El almacén en memoria pertenece a un proceso: con varias instancias (o un reinicio en cada despliegue) los usuarios
pierden la sesión o ven una distinta en cada petición. Ejecuta una sola instancia, o usa un almacén compartido que
implementes con `SessionStore`. Todas las instancias deben usar los mismos secretos.

Si solo necesitas saber quién es el usuario, un token firmado con `JwtService` no necesita almacén. Ver
[Autenticación](/es/security/authentication).

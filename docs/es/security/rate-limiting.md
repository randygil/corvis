---
status: documented
---

# Límite de peticiones

Limitar cuántas peticiones puede hacer un cliente en una ventana de tiempo protege la API de abusos y
de ataques de fuerza bruta. Corvis lo resuelve con `ThrottlerModule`, que guarda los contadores, y
`ThrottlerGuard`, que los consulta en cada petición.

## Configuración

Importa el módulo con uno o más límites y registra el guard. `ThrottlerOptions::new(ttl, limit)`
recibe la ventana en **milisegundos** y el número máximo de peticiones por ventana:

```rust
use corvis::prelude::*;

#[module(
    imports = [
        ThrottlerModule::for_root(vec![ThrottlerOptions::new(60_000, 100)]),
        AuthModule,
        CatsModule,
    ],
    providers = [
        Provider::app_guard::<ThrottlerGuard>(),
        Provider::app_guard::<AuthGuard>(),
    ],
)]
pub struct AppModule;
```

`ThrottlerModule::for_root` es global: no hace falta importarlo en cada módulo. Con
`Provider::app_guard` el guard se aplica a todas las rutas de los controladores; también puedes
aplicarlo solo a un controlador o a un método con `#[use_guards(ThrottlerGuard)]`.

Cuando se supera el límite, la petición recibe:

```json
{ "statusCode": 429, "message": "ThrottlerException: Too Many Requests" }
```

## Cómo se cuenta

- La clave de cada contador combina el nombre del límite, la IP del cliente (`req.ip()`), el
  controlador y el handler. Cada ruta tiene su propio contador por cliente. La IP se normaliza como en la
  librería original: una dirección IPv4 se usa tal cual, una IPv6 mapeada a IPv4 (`::ffff:1.2.3.4`) cuenta como su
  forma IPv4, `::1` queda sola y cualquier otra dirección IPv6 cuenta como su prefijo /64 (ver
  [Clientes IPv6](#clientes-ipv6)).
- La ventana es deslizante: cada petición cuenta durante `ttl` milisegundos desde el momento en que se
  hizo, y el conteo de una petición es el número de accesos de esa clave en los últimos `ttl`. Un cliente
  no puede enviar el doble del límite en el borde de una ventana fija.
- La petición que lleva el conteo por encima del límite bloquea la clave (ver [Bloqueo](#bloqueo)). Las
  peticiones rechazadas mientras la clave está bloqueada no se cuentan ni alargan el bloqueo.
- Con varios límites en el vector, se comprueban en orden ascendente de `ttl` (los empates mantienen el
  orden dado) y el primero que rechaza la petición detiene la revisión: los límites siguientes no cuentan
  esa petición. Para distinguirlos, `ThrottlerOptions` tiene un campo público `name` (`"default"` por defecto):

  ```rust
  ThrottlerModule::for_root(vec![
      ThrottlerOptions { name: "short".into(), ttl: 1_000, limit: 3 },
      ThrottlerOptions { name: "long".into(), ttl: 60_000, limit: 100 },
  ])
  ```

## Bloqueo

Una clave que supera su límite se bloquea durante la **duración del bloqueo**, contada desde el momento
del bloqueo: toda petición en ese tiempo recibe `429` con `Retry-After` igual al tiempo que falta. La
primera petición después del bloqueo vuelve a contar desde uno. La duración del bloqueo es el `ttl` del
throttler (después de su override de `ttl`) salvo que la configures:

- Para un throttler, en el storage, por nombre: `ThrottlerStorage::new(...).with_block_duration("default", 300_000)`
  (milisegundos). Un nombre que no coincide con ningún throttler provoca un panic al construir el storage.
- Para una ruta o un controlador, con `throttle("nombre").block_duration(ms)` (milisegundos, para ese throttler;
  ver [Sobrescribir un throttler](#sobrescribir-un-throttler)); tiene prioridad sobre el valor del storage.
- `0` desactiva el bloqueo: una petición se rechaza mientras la clave ya tenga `limit` accesos en los
  últimos `ttl` y no se cuenta; `Retry-After` es el tiempo hasta que vence el más antiguo de esos accesos.

Un límite de `0` rechaza todas las peticiones, incluida la primera después de un bloqueo. La librería
original (6.7.1) deja pasar una petición cada vez que termina un bloqueo; Corvis no, igual que el código
actual de la librería.

Esto cambia el comportamiento de las apps que ya usaban el guard: el bloqueo ahora dura una duración
completa desde el momento del bloqueo en lugar del resto de una ventana fija, y los accesos se cuentan en
una ventana deslizante.

## Cambiar el límite por ruta

Las rutas y los controladores sobrescriben las opciones de un throttler, como lo hace
`@Throttle({ short: { limit, ttl, blockDuration } })` en la librería original. El guard lee estas claves de
metadata, en el método o en el controlador:

| Constante o helper | Efecto |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` omite el throttler llamado `default`, como un `@SkipThrottle()` sin argumentos |
| `skip_throttle([..])`, `skip_throttle_key(nombre)` | Omiten throttlers por nombre (ver [Omitir throttlers](#omitir-throttlers)) |
| `throttle(nombre)` | Sobrescribe un throttler: `.limit(n)`, `.ttl(ms)`, `.block_duration(ms)` (`0`: sin bloqueo), `.tracker(id)`, `.key_generator(id)` |
| `throttle_key(OPCION, nombre)` | La clave de metadata de una opción de un throttler, para `#[set_metadata]` (`THROTTLER:LIMIT` seguido del nombre) |
| `THROTTLE_LIMIT`, `THROTTLE_TTL`, `THROTTLE_BLOCK_DURATION` | Las claves sin nombre: la entrada del throttler llamado `default` solamente |
| `THROTTLE_TRACKER`, `THROTTLE_KEY_GENERATOR` | Prefijos de las claves del tracker y del generador de claves; no tienen forma sin nombre |

### Sobrescribir un throttler

`throttle(nombre)` devuelve un builder que se convierte en un `CustomDecorator`, así que va en
`#[apply_decorators]` sobre un controlador o una ruta (`@Throttle({ short: { limit: 1 }, long: { limit: 100 } })`):

```rust
#[routes]
impl AuthController {
    #[post("login")]
    #[apply_decorators(throttle("short").limit(1), throttle("long").limit(5).ttl(120_000))]
    async fn sign_in(&self, #[body] dto: SignInDto) -> HttpResult<Value> {
        // ...
    }

    #[get("health")]
    #[set_metadata(SKIP_THROTTLE, true)]
    fn health(&self) -> &'static str {
        "ok"
    }
}
```

- Un override cambia el throttler nombrado y ningún otro: con `short` (3 cada 30 s) y `long` (5 cada 60 s),
  `throttle("short").limit(1)` deja `long` en 5.
- Cada opción se resuelve por separado, primero el handler y después el controlador, y gana el primer valor presente.
  Un handler que solo define `limit` conserva el `ttl` del controlador para ese throttler. Una opción que nadie define
  vuelve a la configuración del throttler (y la duración del bloqueo a `with_block_duration` y luego al `ttl` efectivo).
- Un `null` JSON cuenta como ausente: en un handler pasa al valor del controlador, que puede ser más permisivo que el
  del propio throttler, así que no lo uses para endurecer una ruta.
- Los handlers de gateways no tienen `#[apply_decorators]`; usa `#[set_metadata(throttle_key(THROTTLE_LIMIT, "short"), 1)]`.
- Las claves sin nombre `THROTTLE_LIMIT`, `THROTTLE_TTL` y `THROTTLE_BLOCK_DURATION` son la entrada del throttler
  `default`, igual que el `SKIP_THROTTLE` sin nombre: `#[set_metadata(THROTTLE_LIMIT, 5)]` es
  `throttle("default").limit(5)`. Si un destino tiene ambas, gana la entrada con nombre.
- Los valores se validan. `limit` debe ser un entero de 0 a 4.294.967.295, `ttl` y la duración del bloqueo enteros no
  negativos (milisegundos), y un tracker o generador de claves el nombre de una función registrada. Cualquier otra cosa
  (un texto como `"5"`, un número negativo, una fracción) responde `500` con el cuerpo estándar y registra
  `Invalid "limit" for the "short" throttler: ...`; el guard no vuelve a un valor más permisivo. El builder solo recibe
  enteros, así que esto solo puede ocurrir con un `#[set_metadata]` escrito a mano.
- Un nombre que ningún throttler tiene (`throttle("shrot")`), o una clave sin nombre cuando ningún throttler se llama
  `default`, responde `500` en cada petición de esa ruta y registra un error por ruta con el nombre de la ruta, los
  nombres desconocidos y los configurados. La librería original ignora esos overrides; Corvis falla cerrado porque un
  nombre mal escrito dejaría la ruta con el límite global, más permisivo, sin ninguna señal.

::: warning Cambio de comportamiento
Antes, `THROTTLE_LIMIT`, `THROTTLE_TTL` y `THROTTLE_BLOCK_DURATION` se aplicaban a todos los throttlers: un
`THROTTLE_LIMIT` de 5 sin nombre subía también un throttler `short` de 3 a 5, y un `THROTTLE_TTL` sin nombre daba a todos
los throttlers la misma ventana. Ahora se aplican solo al throttler llamado `default`. En una app cuyos throttlers
tienen todos otros nombres, un override sin nombre hace que esa ruta responda `500` (el guard registra un error por cada ruta
así); reemplázalo por `throttle("nombre")` para cada throttler al que iba dirigido.
:::

### Trackers y generadores de claves

Por defecto el tracker es la IP normalizada del cliente y la clave del contador combina el controlador, el handler y el
tracker. La librería original permite reemplazar ambos (`getTracker` y `generateKey`), y Corvis también:

```rust
use corvis::modules::throttler::normalize_ip;
use corvis::prelude::*;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)])
        // el getTracker del original: async, recibe el ExecutionContext; un Err se devuelve tal cual
        .with_tracker(|ctx| async move {
            let request = ctx.request();
            Ok(match request.header("x-api-key") {
                Some(key) => format!("key:{key}"),
                None => normalize_ip(&request.ip(), 64),
            })
        })
        // el generateKey(context, tracker, throttlerName) del original: síncrono
        .with_key_generator(|_ctx, tracker, name| format!("{name}:shared:{tracker}")),
)
```

- `with_tracker` y `with_key_generator` definen los hooks de nivel de módulo. Se ejecutan por cada throttler que no
  se omite, una vez por petición, así que mantenlos baratos y sin efectos secundarios.
- Una ruta elige otra función para un throttler por nombre, porque la metadata guarda JSON y no closures: regístrala
  con `with_named_tracker("by-email", f)` o `with_named_key_generator("shared", f)` y selecciónala con
  `throttle("login").tracker("by-email")` o `.key_generator("shared")`. Un id que no está registrado responde `500` (la
  ruta no queda sin límite). Registrar un id dos veces provoca un panic.
- Para cada throttler el guard usa la función de la ruta, después el hook del módulo y después el predeterminado.
- Un tracker propio reemplaza por completo al predeterminado, incluida la agrupación IPv6: llama tú a `normalize_ip`,
  como en el ejemplo.
- El nombre del throttler siempre forma parte de la clave guardada. Un generador que ignora el nombre (`|_, tracker, _|
  tracker.to_string()`) comparte un contador entre las rutas que lo usan, que es para lo que sirve, pero nunca entre throttlers. La clave
  guardada también registra su origen (el predeterminado, el generador del módulo o un generador registrado por id), así
  que las claves de orígenes distintos nunca se encuentran: un cliente no puede fabricar un tracker que caiga en el
  contador predeterminado de otra ruta. La
  librería original comparte la ventana de reporte y el bloqueo entre dos throttlers que obtienen la misma clave, así que
  su `X-RateLimit-Reset-long` muestra la ventana del otro throttler y un bloqueo de `long` hace que `short` responda
  `Retry-After-short`; Corvis mantiene los valores de cada throttler. La prueba diferencial registra este caso y
  comprueba que Corvis difiere de él.
- Un generador no debería devolver un texto que el cliente controla por completo (el valor de una cabecera): antepón un
  prefijo constante, como en `format!("shared:{tracker}")`. Las rutas que usan el mismo generador comparten contadores,
  así que un cliente que elige el tracker elige cuál de esos contadores golpea.

::: warning Trackers elegidos por el cliente
Un tracker que lee una cabecera o un campo del cuerpo deja que el cliente elija su contador, así que un cliente que rota
el valor obtiene un límite nuevo cada vez. Cada valor rotado además crea un contador, así que un cliente que envía
muchos valores llena `max_entries` y expulsa los contadores de otros clientes (lo que los reinicia). Mantén un throttler
basado en la IP junto al de cuenta (el throttler `default` cuenta por IP, uno con nombre cuenta por cuenta en la ruta de
login con `.tracker(..)`), pero ten en cuenta que solo frena esto si se comprueba primero: los throttlers se comprueban
en orden ascendente de `ttl` y el primero que rechaza detiene la revisión, así que el throttler por IP necesita el `ttl`
más corto; si no, el throttler de cuenta cuenta el acceso antes de que el de IP lo rechace. Normaliza el identificador de
cuenta (por ejemplo, un email en minúsculas) para que las variantes de escritura no tengan contadores separados, y
recuerda que un límite por cuenta puede usarse para bloquear a una víctima a propósito: cualquiera puede enviar su
identificador. Un tracker que devuelve un solo valor para todas las peticiones anónimas las pone todas en un contador, y un cliente puede agotarlo para todos.
:::

### Omitir throttlers

Las omisiones se deciden por throttler, como en la librería original. `SKIP_THROTTLE` solo omite el throttler llamado `default`; los demás throttlers siguen contando la petición y enviando sus cabeceras. Se omiten por nombre con `skip_throttle`, que devuelve un `CustomDecorator` (`@SkipThrottle({ short: true, long: true })`):

```rust
#[get("status")]
#[apply_decorators(skip_throttle(["short", "long"]))]
fn status(&self) -> &'static str {
    "ok"
}
```

Los nombres deben coincidir con `ThrottlerOptions::name`; un nombre que ningún throttler tiene no omite nada. Un throttler omitido no cuenta la petición ni pone cabeceras `X-RateLimit-*` ni `Retry-After`.

Para cada throttler el guard mira primero el método y después el controlador, y gana el primer valor que encuentra. Un método puede deshacer la omisión de un controlador para un throttler con `false`, usando `skip_throttle_key(nombre)` (`"THROTTLER:SKIP"` seguido del nombre, como lo guarda el original); para `default` también sirve `SKIP_THROTTLE` con `false`:

```rust
#[controller("accounts")]
#[apply_decorators(skip_throttle(["short"]))]
pub struct AccountsController;

#[routes]
impl AccountsController {
    #[post("login")]
    #[set_metadata(skip_throttle_key("short"), false)]
    fn login(&self) { /* ... */ }
}
```

Los handlers de gateway usan las mismas claves con `#[set_metadata(skip_throttle_key("short"), true)]`, porque no tienen `#[apply_decorators]`.

::: warning Cambio de comportamiento
Antes, `SKIP_THROTTLE` omitía todos los throttlers. Una app cuyos throttlers tienen todos otros nombres (`short`, `long`) y una ruta con `SKIP_THROTTLE` ahora limita esa ruta. El guard registra una advertencia por cada ruta así en su primera petición, con el nombre de la ruta; reemplaza la omisión por `skip_throttle(["short", "long"])` para conservar el comportamiento anterior.
:::

## Almacenamiento

Los contadores viven en memoria, en `ThrottlerStorage`. Guarda el `sha256` de cada clave y sus accesos
agrupados en como máximo 64 cubos de `ttl / 64`: los accesos cuyo vencimiento cae en el mismo cubo se cuentan
juntos y vencen juntos, así que una clave usa la misma memoria sea cual sea su límite y cuantas peticiones
reciba. Las claves sin accesos vivos, con la ventana terminada y sin bloqueo se borran en cada petición.

Una clave que supera su límite pasa a una tabla aparte y compacta de claves bloqueadas (sus accesos se
descartan: no se usan durante el bloqueo). Las dos tablas tienen su propio tope, así que una inundación de
claves bloqueadas no puede desplazar los contadores de otros clientes:

- Contadores: 100 000 claves como máximo por defecto (`with_max_entries`). El peor caso es de unos 1,8 KB por
  clave (64 cubos de 24 bytes más el estado fijo de la clave), unos 180 MB con el valor por defecto. Cuando la
  tabla se llena se descartan los contadores que antes quedan inactivos, lo que los reinicia. Bajo un ataque
  con más claves distintas que el tope, se pueden reiniciar los contadores de clientes no bloqueados, así que
  dimensiona el tope según tu tráfico.
- Claves bloqueadas: tantas como `with_max_entries` salvo que uses `with_max_blocked`. Una clave bloqueada
  ocupa unos 300 bytes, unos 30 MB con 100 000 claves. Cuando esa tabla se llena se descarta el bloqueo que
  empezó primero, sea cual sea su duración y aunque le quede mucho tiempo, lo que permite a esa clave volver a
  contar; liberar el bloqueo de otro cliente exige por tanto tantos bloqueos nuevos como cabe en la tabla.
  Configura `with_max_blocked` con al menos el número de bloqueos que tu aplicación inicia durante su
  duración de bloqueo más larga: con bloqueos de login de 24 horas, cuenta los bloqueos iniciados en 24 horas.

Juntas, el peor caso con los valores por defecto es de unos 200 MB (unos 170 MB de contadores más unos 30 MB
de bloqueos). Para cambiar los topes:

```rust
use corvis::ThrottlerStorage;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)])
        .with_max_entries(500_000)
        .with_max_blocked(200_000),
)
```

Diferencia conocida con la librería original: cuando un cliente envía accesos más juntos que `ttl / 64` (unos
0,9 s en una ventana de 60 s), comparten cubo y vencen juntos, así que `X-RateLimit-Remaining` y
`X-RateLimit-Reset` pueden diferir hasta `ttl / 64`. Las peticiones que se rechazan y `Retry-After` no se
ven afectados. Con accesos más espaciados los valores son idénticos.

Dos throttlers con el mismo nombre comparten contadores y duraciones de bloqueo; se registra una advertencia
al construir el storage.

Al ser memoria del proceso, cada instancia de la aplicación cuenta por separado. No hay un
almacenamiento compartido (Redis u otro) incluido.

## Límites desde otro provider

`ThrottlerModule::for_root_async(fábrica)` construye los límites con una fábrica que puede resolver otros providers, como `ConfigService`. La fábrica devuelve un `Vec<ThrottlerOptions>` o un `ThrottlerStorage` configurado; el módulo es global, igual que `for_root`:

```rust
ThrottlerModule::for_root_async(|ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let limit = config.get_or("THROTTLE_LIMIT", 10u32);
    Ok(vec![ThrottlerOptions::new(60_000, limit)])
})
```

Si la fábrica falla, la aplicación no arranca (`Error while instantiating ThrottlerStorage: ...`).

## Clientes IPv6

Una conexión de hosting o doméstica suele recibir un /64 completo (2^64 direcciones). Si cada dirección contara como
un cliente distinto, un cliente podría obtener un límite nuevo con cada dirección que rote. Por eso `ThrottlerGuard`
agrupa a los clientes IPv6 por prefijo, de 64 bits por defecto, como hace la librería original desde 6.7.0
(`ipv6SubnetPrefix`). Los clientes de un mismo /64 comparten sus contadores, como los clientes IPv4 detrás de un NAT.

Cambia la longitud con `ThrottlerStorage::with_ipv6_subnet_prefix`. Vale para todos los throttlers y funciona con
`for_root_storage` y con fábricas de `for_root_async` que devuelven un storage:

```rust
use corvis::modules::throttler::ThrottlerStorage;

// Asignaciones mayores: muchos proveedores entregan un /56 o un /48 a cada cliente.
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 10)]).with_ipv6_subnet_prefix(56),
)
```

- `128` mantiene separada cada dirección IPv6 (la forma de desactivar la agrupación). Las direcciones mapeadas a IPv4
  siguen contando como IPv4 y `::1` queda sola. Los valores por encima de 128 cuentan como 128.
- `56` o `48` dificultan la rotación en endpoints como el login, pero los clientes que comparten ese prefijo también
  comparten el límite.
- Un prefijo menor de 32 bits se acepta, pero deja una parte grande del espacio de direcciones en un solo contador,
  así que se registra una advertencia una vez al configurarlo.

El hosting compartido es el caso donde agrupar perjudica: si un proveedor da a cada VM una porción de un mismo /64
(a menudo un /128 por VM), todas las VM, y con ellas todos tus clientes que viven allí, comparten un contador. Si tus
clientes pueden estar en esas direcciones, usa `with_ipv6_subnet_prefix(128)` o calcula tú la clave con un guard que
llame a `record_hit`.

Donde Corvis se aparta de la librería original (seguridad sobre paridad), `normalize_ip` devuelve la dirección IPv4 del
cliente para las direcciones que la contienen, sea cual sea el prefijo:

- NAT64 `64:ff9b::/96`. Un traductor sin estado (NAT46, SIIT-DC) muestra a cada cliente IPv4 como
  `64:ff9b::a.b.c.d`; el /64 los pondría a todos en un solo contador. Un despliegue con su propio prefijo de
  traducción no se reconoce: allí usa `with_ipv6_subnet_prefix(128)` o un tracker propio.
- 6to4 `2002::/16` (la IPv4 son los bits 16 a 47; una sola IPv4 es dueña de un /48 entero de estas) y Teredo
  `2001::/32` (la IPv4 del cliente son los últimos 32 bits, invertidos).
- La forma obsoleta IPv4-compatible `::a.b.c.d`, salvo `::` y `::1`.

Una dirección con puerto (`203.0.113.5:54321`, `[2001:db8:1:2::1]:443`) o una IPv6 entre corchetes
(`[2001:db8:1:2::1]`) cuenta como la dirección sola; algunos proxies escriben el puerto en `X-Forwarded-For` y cada
conexión tendría su propio contador. El resto de la salida del original no cambia y se comprueba contra él.

`req.ip()`, `req.ips()` y `#[ip]` siguen devolviendo la dirección completa; solo cambia la clave del throttler. Un
guard propio que llame a `ThrottlerStorage::record_hit` puede construir la misma clave de cliente con
`corvis::modules::throttler::normalize_ip(&req.ip(), prefijo)`, un port del `normalizeIp` original. Lo que no sea una
dirección IP, con o sin puerto (ver [Detrás de un proxy](#detras-de-un-proxy)) se devuelve sin cambios, y el identificador de zona
(`%eth0`) se descarta en las direcciones válidas.

## Detrás de un proxy

La clave usa `req.ip()`, que por defecto es la dirección del socket. Detrás de un proxy inverso todas
las peticiones vendrían de la IP del proxy y compartirían contador. Configura `trust proxy` para que
`req.ip()` lea `X-Forwarded-For`:

```rust
app.set_trust_proxy(1); // un proxy delante
```

El ajuste también aplica a los mensajes de los gateways: un `ThrottlerGuard` en un gateway usa el `ip()` de la
petición del handshake, que resuelve `X-Forwarded-For` igual (ver [Gateways](/es/websockets/gateways)).

Usa `true` (confiar en todos los saltos) solo si todas las peticiones pasan por un proxy que
reescribe `X-Forwarded-For`: si no, un cliente puede elegir su IP y saltarse el límite. Ver
[Adaptadores HTTP](/es/http/performance).

El contador usa la forma normalizada de la entrada de cliente que resuelve `trust proxy`. Lo que no sea una dirección
IP, ni una dirección con puerto, se conserva tal cual, así que un cliente que puede elegir su entrada de `X-Forwarded-For` sigue eligiendo su
contador.

## Cabeceras de respuesta

Igual que la librería original, `ThrottlerGuard` le dice al cliente cómo va. Por cada throttler que cuenta una petición y la deja pasar envía:

| Cabecera | Valor |
|---|---|
| `X-RateLimit-Limit` | El límite efectivo (después del override de la ruta) |
| `X-RateLimit-Remaining` | El límite menos los accesos contados en los últimos `ttl`, nunca menos de 0 |
| `X-RateLimit-Reset` | Segundos hasta que termina la ventana de reporte de la clave, redondeados hacia arriba |

Cuando un throttler bloquea la petición, el `429` lleva `Retry-After` (segundos hasta que termina el bloqueo, al menos 1) y ninguna cabecera `X-RateLimit-*` de ese throttler. Las cabeceras de los throttlers revisados antes se mantienen en el `429`. Los throttlers omitidos no envían ninguna.

Para un throttler que no se llama `default`, los nombres de las cabeceras terminan en `-<nombre>`: con los throttlers `short` y `long` de arriba, una respuesta lleva `X-RateLimit-Limit-short`, `X-RateLimit-Remaining-long`, `Retry-After-long`, etc. El nombre del throttler pasa a ser parte del nombre de una cabecera, así que usa un token simple (letras, dígitos y `-`) que no revele nada interno; un nombre que no sea un nombre de cabecera válido hace que sus cabeceras se descarten.

Las cabeceras vienen activadas por defecto y le cuentan a cualquier cliente cada límite, su ventana y los nombres de los throttlers. Para ocultarlas, sin cambiar el conteo ni el bloqueo, se desactivan en el storage:

```rust
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_headers(false),
)
```

Los navegadores solo dejan que una página lea estas cabeceras si la configuración de CORS las expone (`Access-Control-Expose-Headers` en `CorsOptions`). `Retry-After` y los valores de `X-RateLimit-*` siguen a la librería original para las mismas peticiones. Un mensaje de WebSocket se cuenta pero no recibe cabeceras. Otros guards e interceptors pueden poner cabeceras con `ctx.switch_to_http().set_response_header` (ver [Guards](/es/guards#poner-cabeceras-de-respuesta)).

## Limitaciones

- Un `ThrottlerGuard` global también cuenta los mensajes de los gateways de WebSocket, por IP del handshake y por handler, así que una conexión duradera puede agotar su límite. `app.apply_global_enhancers_to_gateways(false)` deja fuera a los gateways. Los listeners `corvis::ws` planos nunca se cuentan.

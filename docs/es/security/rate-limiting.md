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
  controlador y el handler. Cada ruta tiene su propio contador por cliente.
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
throttler (después de `THROTTLE_TTL`) salvo que la configures:

- Para un throttler, en el storage, por nombre: `ThrottlerStorage::new(...).with_block_duration("default", 300_000)`
  (milisegundos). Un nombre que no coincide con ningún throttler provoca un panic al construir el storage.
- Para una ruta o un controlador, con la metadata `THROTTLE_BLOCK_DURATION` (milisegundos, para todos los
  throttlers); tiene prioridad sobre el valor del storage.
- `0` desactiva el bloqueo: una petición se rechaza mientras la clave ya tenga `limit` accesos en los
  últimos `ttl` y no se cuenta; `Retry-After` es el tiempo hasta que vence el más antiguo de esos accesos.

Un límite de `0` rechaza todas las peticiones, incluida la primera después de un bloqueo. La librería
original (6.7.1) deja pasar una petición cada vez que termina un bloqueo; Corvis no, igual que el código
actual de la librería.

Esto cambia el comportamiento de las apps que ya usaban el guard: el bloqueo ahora dura una duración
completa desde el momento del bloqueo en lugar del resto de una ventana fija, y los accesos se cuentan en
una ventana deslizante.

## Cambiar el límite por ruta

El guard lee cuatro claves de metadata, en el método o en el controlador:

| Constante | Efecto |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` omite el límite |
| `THROTTLE_LIMIT` | Cambia el número máximo de peticiones |
| `THROTTLE_TTL` | Cambia la ventana, en milisegundos |
| `THROTTLE_BLOCK_DURATION` | Cambia la duración del bloqueo, en milisegundos (`0`: sin bloqueo) |

```rust
#[routes]
impl AuthController {
    #[post("login")]
    #[set_metadata(THROTTLE_LIMIT, 5)]
    #[set_metadata(THROTTLE_TTL, 60_000)]
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

`THROTTLE_LIMIT`, `THROTTLE_TTL` y `THROTTLE_BLOCK_DURATION` se aplican a todos los límites configurados, no a uno por nombre.

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
  ocupa unos 120 bytes, unos 12 MB con el valor por defecto. Cuando esa tabla se llena se descarta el
  bloqueo que termina antes, lo que permite a esa clave volver a contar.

Juntas, el peor caso con los valores por defecto es de unos 190 MB. Para cambiar los topes:

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

## Detrás de un proxy

La clave usa `req.ip()`, que por defecto es la dirección del socket. Detrás de un proxy inverso todas
las peticiones vendrían de la IP del proxy y compartirían contador. Configura `trust proxy` para que
`req.ip()` lea `X-Forwarded-For`:

```rust
app.set_trust_proxy(1); // un proxy delante
```

Usa `true` (confiar en todos los saltos) solo si todas las peticiones pasan por un proxy que
reescribe `X-Forwarded-For`: si no, un cliente puede elegir su IP y saltarse el límite. Ver
[Adaptadores HTTP](/es/http/performance).

## Cabeceras de respuesta

Igual que la librería original, `ThrottlerGuard` le dice al cliente cómo va. Por cada throttler que cuenta una petición y la deja pasar envía:

| Cabecera | Valor |
|---|---|
| `X-RateLimit-Limit` | El límite efectivo (después de `THROTTLE_LIMIT`) |
| `X-RateLimit-Remaining` | El límite menos los accesos contados en los últimos `ttl`, nunca menos de 0 |
| `X-RateLimit-Reset` | Segundos hasta que termina la ventana de reporte de la clave, redondeados hacia arriba |

Cuando un throttler bloquea la petición, el `429` lleva `Retry-After` (segundos hasta que termina el bloqueo, al menos 1) y ninguna cabecera `X-RateLimit-*` de ese throttler. Las cabeceras de los throttlers revisados antes se mantienen en el `429`. Las rutas con `SKIP_THROTTLE` no reciben ninguna.

Para un throttler que no se llama `default`, los nombres de las cabeceras terminan en `-<nombre>`: con los throttlers `short` y `long` de arriba, una respuesta lleva `X-RateLimit-Limit-short`, `X-RateLimit-Remaining-long`, `Retry-After-long`, etc. El nombre del throttler pasa a ser parte del nombre de una cabecera, así que usa un token simple (letras, dígitos y `-`) que no revele nada interno; un nombre que no sea un nombre de cabecera válido hace que sus cabeceras se descarten.

Las cabeceras vienen activadas por defecto y le cuentan a cualquier cliente cada límite, su ventana y los nombres de los throttlers. Para ocultarlas, sin cambiar el conteo ni el bloqueo, se desactivan en el storage:

```rust
ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_headers(false),
)
```

Los navegadores solo dejan que una página lea estas cabeceras si la configuración de CORS las expone (`Access-Control-Expose-Headers` en `CorsOptions`). `Retry-After` y los valores de `X-RateLimit-*` siguen a la librería original para las mismas peticiones. Un mensaje de WebSocket se cuenta pero no recibe cabeceras. Otros guards e interceptors pueden poner cabeceras con `ctx.switch_to_http().set_response_header` (ver [Guards](/es/guards#poner-cabeceras-de-respuesta)).

## Limitaciones

- `SKIP_THROTTLE` omite todos los throttlers, mientras que el `@SkipThrottle()` original solo omite el throttler llamado `default` salvo que se listen los demás.
- Un `ThrottlerGuard` global también cuenta los mensajes de los gateways de WebSocket, por IP del handshake y por handler, así que una conexión duradera puede agotar su límite. `app.apply_global_enhancers_to_gateways(false)` deja fuera a los gateways. Los listeners `corvis::ws` planos nunca se cuentan.

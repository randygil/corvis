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
- La ventana es fija: el primer acceso crea el contador con su vencimiento y las peticiones
  siguientes lo incrementan hasta que vence.
- Con varios límites en el vector, se comprueban todos y basta con que uno se supere. Para
  distinguirlos, `ThrottlerOptions` tiene un campo público `name` (`"default"` por defecto):

  ```rust
  ThrottlerModule::for_root(vec![
      ThrottlerOptions { name: "short".into(), ttl: 1_000, limit: 3 },
      ThrottlerOptions { name: "long".into(), ttl: 60_000, limit: 100 },
  ])
  ```

## Cambiar el límite por ruta

El guard lee tres claves de metadata, en el método o en el controlador:

| Constante | Efecto |
|---|---|
| `SKIP_THROTTLE` | `#[set_metadata(SKIP_THROTTLE, true)]` omite el límite |
| `THROTTLE_LIMIT` | Cambia el número máximo de peticiones |
| `THROTTLE_TTL` | Cambia la ventana, en milisegundos |

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

`THROTTLE_LIMIT` y `THROTTLE_TTL` se aplican a todos los límites configurados, no a uno por nombre.

## Almacenamiento

Los contadores viven en memoria, en `ThrottlerStorage`. Guarda `sha256` de cada clave, borra los
contadores vencidos en cada petición y no supera 100 000 entradas; al llenarse descarta el que antes
vence. Para cambiar ese tope:

```rust
use corvis::ThrottlerStorage;

ThrottlerModule::for_root_storage(
    ThrottlerStorage::new(vec![ThrottlerOptions::new(60_000, 100)]).with_max_entries(500_000),
)
```

Al ser memoria del proceso, cada instancia de la aplicación cuenta por separado. No hay un
almacenamiento compartido (Redis u otro) incluido.

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

## Limitaciones

- No se envían cabeceras informativas (`Retry-After`, `X-RateLimit-*`).
- `ThrottlerGuard` no se aplica a los gateways de WebSocket aunque esté registrado como global.

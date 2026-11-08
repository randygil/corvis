---
status: documented
---

# Health checks

Un orquestador (Kubernetes, un balanceador, un monitor externo) necesita saber si una instancia está sana para enviarle tráfico o reiniciarla. `TerminusModule` ofrece `HealthCheckService`, que ejecuta una lista de indicadores y arma una respuesta estándar: `200` si todos están bien y `503` si alguno falla.

## Registrar el módulo

```rust
#[module(
    imports = [TerminusModule],
    controllers = [AppController],
)]
pub struct AppModule;
```

`TerminusModule` es un módulo estático (se importa por su nombre, sin `for_root`) que registra y exporta `HealthCheckService`.

## Un endpoint de salud

`examples/cats-app` expone `/health` en su controlador raíz:

```rust
#[controller]
#[set_metadata(IS_PUBLIC_KEY, true)]
pub struct AppController {
    app_service: Arc<AppService>,
    health: Arc<HealthCheckService>,
}

#[routes]
impl AppController {
    #[get("health")]
    #[version(VERSION_NEUTRAL)]
    async fn health(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                corvis::MemoryHealthIndicator::check_rss("memory_rss", 512 * 1024 * 1024),
                HealthIndicator::new("app", || async { Ok(json!({ "uptime": "ok" })) }),
            ])
            .await
    }
}
```

Dos detalles de ese ejemplo: la metadata `IS_PUBLIC_KEY` hace que el guard global de autenticación de la app deje pasar la ruta, y `VERSION_NEUTRAL` la mantiene accesible sin prefijo de versión. Un health check que exige token o versión suele ser un problema para el orquestador.

## Formato de la respuesta

Todos los indicadores se ejecutan en paralelo, cada uno con un [límite de tiempo](#limites-de-tiempo). Con todos arriba, la respuesta es `200`:

```json
{
  "status": "ok",
  "info": { "app": { "status": "up", "uptime": "ok" } },
  "error": {},
  "details": { "app": { "status": "up", "uptime": "ok" } }
}
```

Si alguno falla, `check` devuelve un `Err` con estado `503 Service Unavailable` y el mismo cuerpo con `"status": "error"`; los indicadores caídos aparecen en `error` con `"status": "down"` y los detalles que hayan devuelto.

## Indicadores

### Propios

`HealthIndicator::new(clave, closure)` recibe una closure asíncrona que devuelve `Ok(detalles)` si está arriba y `Err(detalles)` si está abajo; los detalles son un objeto JSON que se combina con `status`. Un indicador para la base de datos con `corvis-orm`:

```rust
#[controller("health")]
pub struct HealthController {
    health: Arc<HealthCheckService>,
    db: Arc<OrmService>,
}

#[routes]
impl HealthController {
    #[get]
    async fn check(&self) -> HttpResult<Value> {
        let db = self.db.clone();
        self.health
            .check(vec![HealthIndicator::new("database", move || {
                let db = db.clone();
                async move {
                    match db.query_raw("SELECT 1", vec![]).await {
                        Ok(_) => Ok(json!({})),
                        Err(e) => Err(json!({ "message": e.to_string() })),
                    }
                }
            })])
            .await
    }
}
```

La closure tiene que ser `'static`, por eso se clona el `Arc` antes de moverlo dentro.

### Incluidos

| Indicador | Qué comprueba |
|---|---|
| `corvis::HttpHealthIndicator::ping_check(clave, "http://host:puerto/ruta")` | Envía un `GET` HTTP y solo está `up` con una respuesta 2xx (se siguen hasta 5 redirecciones). `https://` requiere la feature `health-tls` de `corvis` |
| `corvis::HttpHealthIndicator::tcp_check(clave, "host:puerto")` | Que se pueda abrir una conexión TCP a esa dirección. No envía ninguna petición: úsalo con servicios sin HTTP, como Redis o SMTP |
| `corvis::MemoryHealthIndicator::check_rss(clave, bytes)` | Que la memoria residente del proceso no supere el umbral. Mide en Linux, Windows y macOS; donde no puede medir, informa `down` con un mensaje en lugar de un `up` silencioso |

Cuando `ping_check` falla, la entrada en `error` tiene un `message` y, si el servidor respondió, `statusCode` y `statusText`:

```json
{ "status": "down", "message": "Request failed with status code 500", "statusCode": 500, "statusText": "Internal Server Error" }
```

`check_rss` informa `{ "status": "down", "message": "Used rss exceeded the set threshold" }` por encima del umbral, y `up` sin campos adicionales en caso contrario.

::: warning Migrar desde `ping_check("host:puerto")`
`ping_check` antes solo abría una conexión TCP. Ahora recibe una URL y hace una petición HTTP, y un valor sin `http://` ni `https://` se informa `down` con un mensaje que nombra `tcp_check`. Reemplaza `ping_check(clave, "host:puerto")` por `tcp_check(clave, "host:puerto")` para conservar el comportamiento anterior.
:::

Para URLs `https://`, activa la feature en `Cargo.toml`:

```toml
corvis = { version = "0.1", features = ["health-tls"] }
```

Sin ella, una URL `https://` se informa `down` con un mensaje que nombra la feature, y no se envía ninguna petición.

## Límites de tiempo

Cada indicador se ejecuta con un límite de 5 segundos. Uno que no termina a tiempo se informa `down` con `"message": "timeout of <ms>ms exceeded"`, y los demás indicadores se siguen informando, así que una dependencia colgada no puede colgar el endpoint de salud. Los indicadores corren de forma concurrente: el tiempo de respuesta lo acota el mayor límite, no la suma.

Cambia el límite de un indicador con `timeout`:

```rust
use std::time::Duration;

HttpHealthIndicator::ping_check("api", "http://127.0.0.1:8080/ready").timeout(Duration::from_millis(500))
```

Un orquestador cuya sonda expira antes de 5 segundos necesita un límite menor aquí. El valor por defecto no se puede cambiar para toda la aplicación.

## Límites

- No hay indicadores incluidos para bases de datos, disco o servicios concretos: se escriben con `HealthIndicator::new`.

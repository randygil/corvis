---
status: documentado
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

Todos los indicadores se ejecutan en paralelo. Con todos arriba, la respuesta es `200`:

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
| `corvis::HttpHealthIndicator::ping_check(clave, "host:puerto")` | Que se pueda abrir una conexión TCP a esa dirección en menos de 3 segundos. No hace una petición HTTP |
| `corvis::MemoryHealthIndicator::check_rss(clave, bytes)` | Que la memoria residente del proceso no supere el umbral. Lee `/proc/self/statm`, así que solo mide en Linux; en otros sistemas siempre informa `up` sin datos |

## Límites

- No hay indicadores incluidos para bases de datos, disco o servicios concretos: se escriben con `HealthIndicator::new`.
- `check` no aplica un tiempo máximo a los indicadores propios. Si uno puede colgarse, envuélvelo con `tokio::time::timeout` dentro de la closure.

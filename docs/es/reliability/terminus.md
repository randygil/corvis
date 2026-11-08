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
#[public]
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

Dos detalles de ese ejemplo: el marcador `#[public]` hace que el guard global de autenticación de la app deje pasar la ruta, y `VERSION_NEUTRAL` la mantiene accesible sin prefijo de versión. Un health check que exige token o versión suele ser un problema para el orquestador.

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

### Redirecciones

`ping_check` sigue hasta 5 redirecciones, pero solo dentro del mismo origen (mismo esquema, host y puerto; una subida de `http` a `https` del mismo host también cuenta). Una redirección a otro origen no se sigue: el indicador queda `down` con `statusCode` `302` (o el estado recibido) y un mensaje que lo dice. Una redirección de `https` a `http` nunca se sigue. Esto difiere de la librería original, que sigue redirecciones a cualquier sitio (valor por defecto de axios), porque la redirección la controla el servidor destino y podría apuntar la comprobación a una dirección interna.

Cambia el comportamiento con `ping_check_with` y `PingOptions`:

```rust
use corvis::{HttpHealthIndicator, PingOptions};

HttpHealthIndicator::ping_check_with(
    "api",
    "https://api.example.com/health",
    PingOptions::new().max_redirects(0), // 0 = no seguir ninguna redirección
);

HttpHealthIndicator::ping_check_with(
    "cdn",
    "https://example.com/",
    PingOptions::new().follow_cross_origin(true),
);
```

Incluso con `follow_cross_origin(true)`, una redirección a una dirección de loopback, link-local o privada (`127.0.0.1`, `10.x`, `192.168.x`, `169.254.x`, `100.64.x`, `localhost`, otros rangos no públicos, los equivalentes IPv6 y las formas IPv6 que contienen una IPv4, como `::ffff:127.0.0.1`) se rechaza salvo que la URL configurada ya apuntara a una. Solo se reconocen IP literales y `localhost`: los nombres de host no se resuelven.

Los mensajes sobre URLs inválidas y redirecciones son texto fijo: la URL configurada (que podría contener credenciales) y la cabecera `Location` de la respuesta nunca se copian a la respuesta de salud.

### Exposición de detalles

Los detalles `down` de un indicador van a la respuesta de salud tal cual: `ping_check` y `tcp_check` pueden incluir el `host:puerto` de destino y el texto de error del sistema operativo (por ejemplo `connect ECONNREFUSED 10.0.0.5:6379`), y los indicadores propios incluyen lo que devuelvan. Si el endpoint es público, cualquiera puede leerlos: no pongas texto sensible en los detalles o protege la ruta.

Cada llamada al endpoint de salud también abre conexiones salientes: una por cada `ping_check` o `tcp_check` (más las redirecciones), cada una abierta hasta el límite de tiempo del indicador. Un endpoint público que se llama a menudo puede usarse para generar ese tráfico, así que limita su tasa (ver [Límite de peticiones](/es/security/rate-limiting)) o guarda el resultado en caché unos segundos.

## Límites de tiempo

Cada indicador se ejecuta con un límite de 5 segundos. Uno que no termina a tiempo se informa `down` con `"message": "timeout of <ms>ms exceeded"`, y los demás indicadores se siguen informando, así que una dependencia colgada no puede colgar el endpoint de salud. Los indicadores corren de forma concurrente: el tiempo de respuesta lo acota el mayor límite, no la suma.

Cambia el límite de un indicador con `timeout`:

```rust
use std::time::Duration;

HttpHealthIndicator::ping_check("api", "http://127.0.0.1:8080/ready").timeout(Duration::from_millis(500))
```

Un orquestador cuya sonda expira antes de 5 segundos necesita un límite menor aquí. El valor por defecto no se puede cambiar para toda la aplicación.

## Indicadores que entran en pánico

Un indicador que entra en pánico, al empezar su comprobación o mientras corre, se informa `down` con `"message": "health indicator panicked"`, y los demás indicadores se siguen informando. El estado es `503`, como cualquier otro fallo:

```json
{ "status": "down", "message": "health indicator panicked" }
```

El mensaje del pánico no forma parte de la respuesta, porque los endpoints de salud suelen ser públicos y un mensaje puede llevar una cadena de conexión. Se escribe una vez en el log de errores, con la clave del indicador, bajo el contexto `HealthCheckService`.

Esto difiere de la librería original, que no captura un indicador que lanza una excepción y responde un `500` sin cuerpo de salud. Un `500` y un `503` son igual de no saludables para un orquestador, y ahora el informe dice qué indicador falló.

## Límites

- No hay indicadores incluidos para bases de datos, disco o servicios concretos: se escriben con `HealthIndicator::new`.

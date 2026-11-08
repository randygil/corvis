---
status: documented
---

# Despliegue

Una aplicación de Corvis se despliega como un único binario nativo. No hay un runtime que instalar en el servidor: compilas en modo release, copias el ejecutable junto con los archivos que lea en tiempo de ejecución (`.env`, carpetas estáticas, migraciones) y lo ejecutas.

## Compilar para producción

```bash
corvis build --release
# equivalente a: cargo build --release
```

El binario queda en `target/release/<nombre-del-crate>` (`.exe` en Windows). Compila en la misma plataforma de destino (o en un contenedor con la misma) para evitar problemas de compatibilidad de bibliotecas del sistema.

Como Corvis no está publicado todavía en crates.io, el `Cargo.toml` usa una dependencia de ruta (`corvis = { path = "..." }`). Asegúrate de que esa ruta exista en la máquina o la imagen donde compilas; en un `Dockerfile`, copia el código del framework dentro del contexto de construcción.

## Configuración

Lee la configuración del entorno con `ConfigModule` en lugar de fijarla en el código:

```rust
#[module(
    imports = [ConfigModule::for_root(ConfigModuleOptions { is_global: true, ..Default::default() })],
    /* ... */
)]
pub struct AppModule;
```

`ConfigModule` carga `.env` si existe y después las variables del proceso. En producción lo habitual es pasar las variables desde el orquestador (systemd, Docker, Kubernetes) y no desplegar el `.env`. Para fallar al arrancar si falta algo, usa la opción `validate` o, como en `examples/tertulia`, devuelve un error desde la factory de un módulo asíncrono:

```rust
JwtModule::register_async(true, |ctx| async move {
    let config = ctx.get::<ConfigService>().await?;
    let secret = config.get::<String>("JWT_SECRET").filter(|s| !s.trim().is_empty());
    let secret = secret.ok_or_else(|| corvis::anyhow::anyhow!("JWT_SECRET is required"))?;
    Ok(JwtModuleOptions::new(secret))
}),
```

Ver [Configuración](/es/application/configuration).

## Dirección y puerto

```rust
let port: u16 = app.get::<ConfigService>()?.get_or("PORT", 3000);
app.listen(port).await
```

| Argumento de `listen` | Dirección |
|---|---|
| `3000` (un número) | `0.0.0.0:3000`: todas las interfaces |
| `"127.0.0.1:3000"` | Solo local |
| `"0.0.0.0:3000"`, `"[::]:3000"` | Todas las interfaces IPv4 o IPv6 |
| Un `SocketAddr` | Tal cual |

Dentro de un contenedor necesitas escuchar en todas las interfaces (`0.0.0.0`), no en `127.0.0.1`.

## Detrás de un proxy inverso

Corvis sirve HTTP/1.1 y HTTP/2 sin cifrar; **no termina TLS**. En producción ponlo detrás de un proxy inverso o balanceador (nginx, Caddy, Traefik, el de tu proveedor) que atienda HTTPS.

Detrás de un proxy, la IP del socket es la del proxy. Para que `req.ip()`, `#[ip]` y el `ThrottlerGuard` vean la del cliente, indica en quién confiar:

```rust
use corvis::TrustProxy;

app.set_trust_proxy(1usize);                                         // un proxy delante
app.set_trust_proxy(TrustProxy::parse("loopback, 10.0.0.0/8")?);    // direcciones o subredes
```

No uses `set_trust_proxy(true)` salvo que **todas** las peticiones pasen por un proxy que reescribe `X-Forwarded-For`: si no, cualquier cliente puede elegir su IP. Ver [Límite de peticiones](/es/security/rate-limiting).

## Límites del servidor

Los valores por defecto siguen a los de Node.js y protegen contra clientes lentos:

```rust
use std::time::Duration;
use corvis::ServerOptions;

app.set_server_options(
    ServerOptions::new()
        .headers_timeout(Duration::from_secs(30))    // por defecto 60 s
        .request_timeout(Duration::from_secs(120))   // por defecto 300 s (responde 408)
        .keep_alive_timeout(Duration::from_secs(10)) // por defecto 5 s
        .max_connections(10_000)                     // por defecto sin tope
        .max_header_size(16 * 1024),                 // por defecto 32 KiB (responde 431)
);
```

`ServerOptions::h2c(false)` desactiva HTTP/2 sin TLS si tu proxy no lo usa. Llama a `set_server_options` antes de `listen`. Ver [Conexiones keep-alive](/es/faq/keep-alive-connections).

## Apagado ordenado

```rust
let mut app = CorvisFactory::create::<AppModule>().await?;
app.enable_shutdown_hooks();
app.listen(port).await
```

Con `enable_shutdown_hooks()`, al recibir `SIGINT` (Ctrl+C) o `SIGTERM` (en Unix) el servidor deja de aceptar conexiones, espera hasta 10 segundos a que terminen las peticiones en curso y ejecuta `on_module_destroy`, `before_application_shutdown` y `on_application_shutdown` (ver [Eventos del ciclo de vida](/es/fundamentals/lifecycle-events)). Es lo que esperan los orquestadores al reemplazar una instancia. En Windows solo se escucha Ctrl+C.

## Health checks

Expón un endpoint para que el orquestador sepa si la instancia está viva, con `TerminusModule` y `HealthCheckService`:

```rust
use corvis::MemoryHealthIndicator;

#[controller("health")]
pub struct HealthController {
    health: Arc<HealthCheckService>,
}

#[routes]
impl HealthController {
    #[get]
    async fn check(&self) -> HttpResult<Value> {
        self.health
            .check(vec![
                HealthIndicator::new("database", || async { Ok(json!({})) }),
                MemoryHealthIndicator::check_rss("memory_rss", 300 * 1024 * 1024),
            ])
            .await
    }
}

#[module(imports = [TerminusModule], controllers = [HealthController])]
pub struct HealthModule;
```

Ver [Health checks](/es/reliability/terminus).

## Logs

El logger por defecto escribe en la consola con colores. En producción, si un agregador procesa los logs, conviene el formato JSON:

```rust
use corvis::ConsoleLogger;

app.use_logger(ConsoleLogger::new().json(true));
```

Ver [Logging](/es/application/logger).

## Contenedores

Un `Dockerfile` de dos etapas típico (ajusta las rutas a la ubicación del framework en tu contexto de construcción):

```dockerfile
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p mi-api

FROM debian:bookworm-slim
COPY --from=build /src/target/release/mi-api /usr/local/bin/mi-api
ENV PORT=3000
EXPOSE 3000
CMD ["mi-api"]
```

Las aplicaciones de Corvis requieren la edición 2024 de Rust (Rust 1.85 o posterior). Si usas `corvis-orm` con PostgreSQL y TLS, la feature por defecto `tls-rustls` no necesita OpenSSL en la imagen final.

## Lista de comprobación

- Compilado con `--release`.
- Variables de entorno definidas y validadas al arrancar.
- `enable_shutdown_hooks()` activado.
- TLS terminado en el proxy y `set_trust_proxy` configurado si hay proxy.
- Rutas de Swagger, archivos estáticos y WebSockets planos protegidos por separado si no son públicos: los guards globales no los cubren.
- Un endpoint de health check.

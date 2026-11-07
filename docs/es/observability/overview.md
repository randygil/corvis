---
status: parcial
---

# Visión general

Observar una aplicación en producción significa poder responder qué está haciendo, qué tan rápido y por qué falla. Corvis no trae una plataforma de observabilidad (SDK de trazas, métricas, panel ni integración con proveedores de APM). Lo que sí ofrece son las piezas básicas sobre las que montar la tuya:

| Necesidad | Qué hay en Corvis |
|---|---|
| Logs estructurados | `ConsoleLogger::new().json(true)` escribe una línea JSON por entrada; `LoggerService` permite enviar los logs a donde quieras (ver [Logging](/es/application/logger)) |
| Duración de cada petición | `corvis::LoggingInterceptor` registra método, URL, estado y milisegundos; o un [interceptor](/es/interceptors) propio que mida y publique la métrica |
| Errores no controlados | Un [filtro de excepciones](/es/exception-filters) global ve cada error antes de que se responda; `exception.is_internal()` distingue los errores inesperados |
| Estado de la instancia | `TerminusModule` y `HealthCheckService` (ver [Health checks](/es/reliability/terminus)) |

Las demás páginas de esta sección describen funciones que todavía no existen en Corvis y, cuando aplica, cómo cubrir la necesidad con estas piezas.

## Ejemplo: un interceptor de métricas

```rust
use std::time::Instant;

#[injectable]
pub struct MetricsInterceptor {
    #[init(Logger::new("Metrics"))]
    logger: Logger,
}

#[async_trait]
impl CorvisInterceptor for MetricsInterceptor {
    async fn intercept(&self, ctx: &ExecutionContext, next: CallHandler) -> HandlerResult {
        let started = Instant::now();
        let route = format!("{}.{}", ctx.get_class().name, ctx.get_handler().name);
        let result = next.handle().await;
        let status = match &result {
            Ok(r) => r.status().map(|s| s.as_u16()).unwrap_or(200),
            Err(e) => e.get_status(),
        };
        self.logger.log(format!("{route} {status} {}us", started.elapsed().as_micros()));
        result
    }
}
```

Regístralo como global con `Provider::app_interceptor::<MetricsInterceptor>()` en el módulo raíz. En lugar de escribir un log, puedes publicar el valor en el sistema de métricas que uses.

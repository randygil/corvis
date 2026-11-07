---
status: no-disponible
---

# Resiliencia

Corvis no incluye utilidades de resiliencia como reintentos con espera exponencial, circuit breakers o bulkheads.

## Alternativa

- **Tiempo máximo por petición**: `corvis::TimeoutInterceptor` corta el handler cuando supera un plazo y responde `408 Request Timeout`. Por defecto son 5 segundos; `TimeoutInterceptor::millis(2_000)` o `TimeoutInterceptor::new(Duration)` lo cambian. Se aplica con `#[use_interceptors(...)]` o como interceptor global (ver [Interceptors](/interceptors)).
- **Tiempos del servidor**: `ServerOptions` limita cuánto se espera por las cabeceras y el cuerpo de cada petición (ver [Conexiones keep-alive](/faq/keep-alive-connections)).
- **Reintentos**: escríbelos en tu servicio con `tokio::time::sleep` o con un crate de reintentos de Rust; si la operación es una llamada a la base de datos, combínalo con una transacción de [corvis-orm](/data/corvis-orm).

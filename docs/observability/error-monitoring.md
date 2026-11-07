---
status: no-disponible
---

# Monitoreo de errores

No hay integración con servicios de monitoreo de errores.

## Alternativa

Registra un [filtro de excepciones](/exception-filters) global que reporte los errores inesperados y delegue la respuesta en el comportamiento por defecto:

```rust
pub struct ReportingFilter;

#[async_trait]
impl ExceptionFilter for ReportingFilter {
    async fn catch(&self, exception: HttpException, _host: &ArgumentsHost) -> Response {
        if exception.is_internal() {
            Logger::new("Errors").error(exception.message());
            // Envía aquí el error al servicio que uses.
        }
        BaseExceptionFilter::handle(&exception)
    }
}

// main.rs
app.use_global_filters(ReportingFilter);
```

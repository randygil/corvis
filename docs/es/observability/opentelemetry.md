---
status: unavailable
---

# OpenTelemetry

No hay integración con OpenTelemetry: Corvis no exporta trazas, métricas ni logs en ese formato.

## Alternativa

Puedes inicializar los crates de OpenTelemetry para Rust en tu `main` y crear spans desde un [interceptor](/es/interceptors) global, que tiene acceso a la petición (`ctx.request()`) y al resultado del handler. Corvis no lo hace por ti ni propaga el contexto.

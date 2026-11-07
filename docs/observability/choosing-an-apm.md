---
status: no-disponible
---

# Elegir un APM

Corvis no tiene integraciones con proveedores de APM, así que no hay una recomendación específica.

## Alternativa

Cualquier APM que acepte logs JSON por la salida estándar funciona con `ConsoleLogger::new().json(true)`. Para trazas, elige uno que tenga SDK para Rust y conéctalo desde tu código (ver [OpenTelemetry](/observability/opentelemetry)).

---
status: no-disponible
---

# Instrumentación manual

No existe una API de Corvis para crear spans o métricas propias dentro de un servicio.

## Alternativa

Registra los eventos importantes con `Logger` (en modo JSON para poder procesarlos) y mide duraciones con `std::time::Instant` dentro de tus servicios o en un [interceptor](/interceptors). Si necesitas spans reales, usa un crate de trazas de Rust en tu código; Corvis no lo integra por ti.

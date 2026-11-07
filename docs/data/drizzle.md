---
status: no-disponible
---

# Drizzle

Drizzle es un ORM de TypeScript y no tiene integración con Corvis. No hay un módulo que registre su cliente en el contenedor.

## Alternativa

Para consultas tipadas cercanas a SQL en Rust, registra un pool de `sqlx` como provider con `Provider::use_factory` (ver [Visión general](/data/overview)). Si prefieres un ORM con modelos y relaciones, usa [corvis-orm](/data/corvis-orm).

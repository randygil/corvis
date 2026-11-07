---
status: no-disponible
---

# Claves de idempotencia

Corvis no tiene soporte incorporado para claves de idempotencia (guardar la respuesta de una petición identificada por una cabecera `Idempotency-Key` y repetirla si llega de nuevo).

## Alternativa

Se puede construir con piezas existentes:

- Un [interceptor](/es/interceptors) lee la cabecera con `ctx.request()`, busca una respuesta guardada y, si no existe, ejecuta `next.handle()` y guarda el resultado.
- Para guardar las respuestas en un solo proceso sirve `CacheManager` (ver [Caché](/es/data/caching)). Si tienes varias instancias, guarda la clave en la base de datos con una restricción única (`#[unique]` en un modelo de [corvis-orm](/es/data/corvis-orm)), para que dos peticiones simultáneas no se ejecuten ambas.

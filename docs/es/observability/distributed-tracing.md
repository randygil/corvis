---
status: no-disponible
---

# Trazas distribuidas

Corvis no propaga contexto de trazas entre servicios (cabeceras como `traceparent`) ni crea spans por petición.

## Alternativa

Un [middleware](/es/middleware) o un [interceptor](/es/interceptors) puede leer la cabecera de traza entrante con `req.header("traceparent")` y guardarla en la petición con `req.insert(valor)` para que los handlers la incluyan en sus logs y en las llamadas salientes.

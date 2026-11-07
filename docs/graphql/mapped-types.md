---
status: no-disponible
---

# Tipos mapeados

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay tipos mapeados para GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- Los schemas de `corvis::z` se derivan con `.partial()`, `.pick(..)`, `.omit(..)` y `.extend(..)` (ver [Tipos mapeados de OpenAPI](/openapi/mapped-types)).

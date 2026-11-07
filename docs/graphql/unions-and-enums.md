---
status: no-disponible
---

# Uniones y enums

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay uniones ni enums GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- `z::union`, `z::discriminated_union` y `z::enum_` validan esas formas en JSON (ver [Validación](/application/validation)).

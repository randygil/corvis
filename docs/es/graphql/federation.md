---
status: no-disponible
---

# Federación

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay federación de esquemas GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).

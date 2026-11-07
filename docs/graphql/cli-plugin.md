---
status: no-disponible
---

# Plugin de la CLI

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

La CLI `corvis` no tiene plugin de GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- Ver lo que sí hace la CLI en [CLI](/cli/overview).

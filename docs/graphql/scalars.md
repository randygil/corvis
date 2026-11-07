---
status: no-disponible
---

# Escalares

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay escalares GraphQL personalizados.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- La forma de los datos la definen tus tipos con `serde` y, si los validas, un schema de `corvis::z` (`z::iso::datetime()`, `z::uuid()`, ...).

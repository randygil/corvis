---
status: no-disponible
---

# Compartir modelos

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay modelos GraphQL que compartir.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).
- Un mismo `struct` con `serde` sirve de DTO, de modelo de `corvis-orm` y de schema de OpenAPI (`#[derive(ApiSchema)]`).

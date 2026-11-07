---
status: no-disponible
---

# Extensiones

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay extensiones de campos GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).
- Los metadatos por ruta se declaran con `#[set_metadata(clave, valor)]` y se leen con `Reflector` (ver [Contexto de ejecución](/es/fundamentals/execution-context)).

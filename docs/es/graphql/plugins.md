---
status: no-disponible
---

# Plugins

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay plugins de servidor GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).
- Los puntos de extensión de Corvis son el [middleware](/es/middleware), los [interceptors](/es/interceptors) y los [eventos del ciclo de vida](/es/fundamentals/lifecycle-events).

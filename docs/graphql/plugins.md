---
status: no-disponible
---

# Plugins

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No hay plugins de servidor GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- Los puntos de extensión de Corvis son el [middleware](/middleware), los [interceptors](/interceptors) y los [eventos del ciclo de vida](/fundamentals/lifecycle-events).

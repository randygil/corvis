---
status: no-disponible
---

# Suscripciones

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No existen suscripciones GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).
- Para empujar datos al cliente en tiempo real usa un [gateway de WebSockets](/es/websockets/gateways) o [Server-Sent Events](/es/http/server-sent-events).

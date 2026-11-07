---
status: no-disponible
---

# Suscripciones

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No existen suscripciones GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/controllers) REST y documenta el contrato con [OpenAPI](/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/application/validation).
- Para empujar datos al cliente en tiempo real usa un [gateway de WebSockets](/websockets/gateways) o [Server-Sent Events](/http/server-sent-events).

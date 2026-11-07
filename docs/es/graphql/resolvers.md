---
status: no-disponible
---

# Resolvers

Corvis no incluye soporte para GraphQL: no hay módulo, resolvers, generación de esquema ni servidor GraphQL. Las APIs de Corvis se exponen hoy como HTTP con controladores, WebSockets y Server-Sent Events.

No existen resolvers, consultas ni tipos de objeto GraphQL.

## Alternativa

- Para APIs de lectura y escritura, usa [controladores](/es/controllers) REST y documenta el contrato con [OpenAPI](/es/openapi/introduction).
- Para validar y dar forma a las entradas, usa [schemas de `corvis::z`](/es/application/validation).
- Un método de un controlador con `#[get]` cumple el papel de una consulta; uno con `#[post]`, `#[patch]` o `#[delete]`, el de una mutación.

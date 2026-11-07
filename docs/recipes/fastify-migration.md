---
status: parcial
---

# Migrar desde Fastify

Corvis tiene **un solo adaptador HTTP**, construido sobre hyper, y su comportamiento sigue al de Express 5, no al de Fastify. No hay un adaptador que reproduzca la semántica de Fastify (su *body parser*, su validación con JSON Schema, sus hooks o su sistema de plugins), ni una forma de elegir adaptador al crear la aplicación.

Si migras una API escrita con Fastify, la estructura se traduce igual que desde Express (ver [Migrar desde Express](/recipes/express-migration)), y estas son las equivalencias de las piezas propias de Fastify:

| Fastify | Corvis |
|---|---|
| Plugins (`fastify.register`) y encapsulación | [Módulos](/modules) con `imports` y `exports` |
| Decoradores (`fastify.decorate`) | [Providers](/providers) inyectados |
| `schema: { body, querystring, params }` con JSON Schema | DTOs con `zod_dto!` y `ZodValidationPipe`, o `#[derive(Validate)]` y `ValidationPipe` ([Validación](/application/validation)) |
| `schema.response` (serialización) | `ZodSerializerInterceptor` con `zod_response` ([Serialización](/application/serialization)) |
| Hooks `onRequest`, `preHandler` | [Middleware](/middleware), [guards](/guards) e [interceptors](/interceptors) |
| `onSend`, `onResponse` | [Interceptors](/interceptors) |
| `setErrorHandler` | [Filtros de excepción](/exception-filters) |
| `@fastify/swagger` | [`corvis-swagger`](/openapi/introduction) |
| `@fastify/static` | [`ServeStaticModule` o `serve_static`](/recipes/serve-static) |
| `@fastify/multipart` | [`FileInterceptor`](/http/file-upload) |
| `@fastify/cors` | `app.enable_cors(...)` ([CORS](/security/cors)) |

## Diferencias de comportamiento

Como la capa HTTP sigue a Express, algunas respuestas cambian respecto de una app de Fastify:

- **Errores**: los cuerpos de error tienen la forma `{ "message", "error", "statusCode" }` de las excepciones HTTP de Corvis, no el `{ statusCode, code, error, message }` de Fastify.
- **Cuerpos**: solo se parsean `application/json` y `application/x-www-form-urlencoded`; un JSON vacío se interpreta como `{}`.
- **Rutas**: no distinguen mayúsculas y aceptan una barra final opcional.
- **Rendimiento**: el benchmark del repositorio (`bench/`) compara Corvis con una app equivalente de Node.js sobre Express y sobre Fastify; los resultados están en el [README](https://github.com/randygil/corvis#rendimiento).

Si tus clientes dependen del formato exacto de los errores de Fastify, escribe un [filtro de excepción](/exception-filters) global que lo reproduzca.

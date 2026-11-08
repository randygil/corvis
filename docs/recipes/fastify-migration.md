---
status: partial
---

# Migrating from Fastify

Corvis has **a single HTTP adapter**, built on hyper, and its behavior follows Express 5, not Fastify. There is no adapter that reproduces Fastify's semantics (its *body parser*, its JSON Schema validation, its hooks or its plugin system), nor a way to choose an adapter when creating the application.

If you are migrating an API written with Fastify, the structure translates the same way as from Express (see [Migrating from Express](/recipes/express-migration)), and these are the equivalents for Fastify's own pieces:

| Fastify | Corvis |
|---|---|
| Plugins (`fastify.register`) and encapsulation | [Modules](/modules) with `imports` and `exports` |
| Decorators (`fastify.decorate`) | Injected [providers](/providers) |
| `schema: { body, querystring, params }` with JSON Schema | DTOs with `zod_dto!` and `ZodValidationPipe`, or `#[derive(Validate)]` and `ValidationPipe` ([Validation](/application/validation)) |
| `schema.response` (serialization) | `ZodSerializerInterceptor` with `zod_response` ([Serialization](/application/serialization)) |
| `onRequest`, `preHandler` hooks | [Middleware](/middleware), [guards](/guards) and [interceptors](/interceptors) |
| `onSend`, `onResponse` | [Interceptors](/interceptors) |
| `setErrorHandler` | [Exception filters](/exception-filters) |
| `@fastify/swagger` | [`corvis-swagger`](/openapi/introduction) |
| `@fastify/static` | [`ServeStaticModule` or `serve_static`](/recipes/serve-static) |
| `@fastify/multipart` | [`FileInterceptor`](/http/file-upload) |
| `@fastify/cors` | `app.enable_cors(...)` ([CORS](/security/cors)) |

## Behavior differences

Because the HTTP layer follows Express, some responses change compared to a Fastify app:

- **Errors**: error bodies have the `{ "message", "error", "statusCode" }` shape of Corvis HTTP exceptions, not Fastify's `{ statusCode, code, error, message }`.
- **Bodies**: only `application/json` and `application/x-www-form-urlencoded` are parsed; an empty JSON body is interpreted as `{}`.
- **Routes**: they are case-insensitive and accept an optional trailing slash.
- **Performance**: the repository benchmark (`bench/`) compares Corvis with an equivalent Node.js app on Express and on Fastify; the results are in the [README](https://github.com/randygil/corvis#rendimiento).

If your clients depend on the exact format of Fastify's errors, write a global [exception filter](/exception-filters) that reproduces it.

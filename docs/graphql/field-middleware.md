---
status: unavailable
---

# Field middleware

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There is no per-field middleware.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- To transform whole responses, use an [interceptor](/interceptors) or the `corvis::z` serializer (see [Serialization](/application/serialization)).

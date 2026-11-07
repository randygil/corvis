---
status: unavailable
---

# Mapped types

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no mapped types for GraphQL.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- `corvis::z` schemas are derived with `.partial()`, `.pick(..)`, `.omit(..)` and `.extend(..)` (see [OpenAPI mapped types](/openapi/mapped-types)).

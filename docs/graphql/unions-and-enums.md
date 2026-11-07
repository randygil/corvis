---
status: unavailable
---

# Unions and enums

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL unions or enums.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- `z::union`, `z::discriminated_union` and `z::enum_` validate those shapes in JSON (see [Validation](/application/validation)).

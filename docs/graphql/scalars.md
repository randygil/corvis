---
status: unavailable
---

# Scalars

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no custom GraphQL scalars.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- The shape of your data is defined by your `serde` types and, if you validate them, a `corvis::z` schema (`z::iso::datetime()`, `z::uuid()`, ...).

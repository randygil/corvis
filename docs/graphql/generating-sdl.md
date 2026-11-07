---
status: unavailable
---

# Generating SDL

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

No SDL is generated.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- The contract of a Corvis HTTP API is generated as an OpenAPI document with `corvis-swagger` (see [OpenAPI](/openapi/introduction)).

---
status: unavailable
---

# Sharing models

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL models to share.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- A single `serde` `struct` serves as a DTO, as a `corvis-orm` model and as an OpenAPI schema (`#[derive(ApiSchema)]`).

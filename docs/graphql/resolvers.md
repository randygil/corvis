---
status: unavailable
---

# Resolvers

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL resolvers, queries or object types.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- A controller method with `#[get]` plays the role of a query; one with `#[post]`, `#[patch]` or `#[delete]` plays the role of a mutation.

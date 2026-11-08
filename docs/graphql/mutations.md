---
status: unavailable
---

# Mutations

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL mutations.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- Use `#[post]`, `#[put]`, `#[patch]` and `#[delete]` routes with a validated DTO in `#[body]` (see [Pipes](/pipes)).

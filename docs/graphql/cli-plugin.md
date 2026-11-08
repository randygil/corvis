---
status: unavailable
---

# CLI plugin

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

The `corvis` CLI has no GraphQL plugin.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- See what the CLI does offer in [CLI](/cli/overview).

---
status: unavailable
---

# Plugins

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL server plugins.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- Corvis extension points are [middleware](/middleware), [interceptors](/interceptors) and [lifecycle events](/fundamentals/lifecycle-events).

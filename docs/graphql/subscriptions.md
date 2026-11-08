---
status: unavailable
---

# Subscriptions

Corvis does not include GraphQL support: there is no module, no resolvers, no schema generation and no GraphQL server. Corvis APIs are currently exposed over HTTP with controllers, WebSockets and Server-Sent Events.

There are no GraphQL subscriptions.

## Alternative

- For read and write APIs, use REST [controllers](/controllers) and document the contract with [OpenAPI](/openapi/introduction).
- To validate and shape inputs, use [`corvis::z` schemas](/application/validation).
- To push data to the client in real time, use a [WebSocket gateway](/websockets/gateways) or [Server-Sent Events](/http/server-sent-events).

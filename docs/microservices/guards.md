---
status: unavailable
---

# Guards

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

Since there are no message handlers, there are no guards for them either.

## Alternative

Check authorization in the consumer code. [Guards](/guards) exist for HTTP and [WebSockets](/websockets/guards).

---
status: unavailable
---

# Interceptors

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

Since there are no message handlers, there are no interceptors for them either.

## Alternative

Wrap the processing of each message in your own function that measures timings or records results. [Interceptors](/interceptors) exist for HTTP and [WebSockets](/websockets/interceptors).

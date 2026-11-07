---
status: unavailable
---

# Exception filters

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

Since there are no message handlers, there are no exception filters for them either.

## Alternative

In your own consumer, handle errors with `Result` inside the task that processes each message and record them with `Logger` (see [Logging](/application/logger)). [Exception filters](/exception-filters) exist for HTTP.

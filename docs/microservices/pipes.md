---
status: unavailable
---

# Pipes

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

Since there are no message handlers, there are no pipes for them either.

## Alternative

Validate each message with a `corvis::z` schema (`schema.parse(&value)` or `parse_as::<T>(&value)`) before processing it (see [Validation](/application/validation)). [Pipes](/pipes) exist for HTTP and WebSockets.

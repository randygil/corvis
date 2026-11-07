---
status: unavailable
---

# Pre-request hooks

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

Since there are no message handlers, there are no pre-request hooks for them either.

## Alternative

In HTTP, [middleware](/middleware) runs before the rest of the pipeline.

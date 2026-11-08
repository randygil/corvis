---
status: unavailable
---

# Hybrid application

A hybrid application serves HTTP requests and, at the same time, listens for messages from one or more
microservice transports (Redis, NATS, Kafka, etc.). Corvis does not have microservices yet
(see [Microservices](/microservices/basics)), so it has no hybrid applications or a
method to connect transports to an HTTP application either.

What you can combine in a single application:

- **HTTP and WebSockets.** Controllers and `#[websocket_gateway]` coexist in the same
  process and the same port, and share providers. In `examples/tertulia`, `RoomsService` is injected
  into both the gateway and a REST controller, and the controller injects the gateway to push
  announcements over WebSocket.
- **HTTP and background tasks.** `ScheduleModule` (cron, intervals, timeouts) and
  `EventEmitterModule` run inside the same application. See
  [Task scheduling](/application/task-scheduling) and [Events](/application/events).
- **A queue consumer of your own.** You can start a consume loop (with the broker client
  you prefer) in a provider's `on_application_bootstrap`, with `corvis::tokio::spawn`, and stop it
  in `on_application_shutdown`. The handlers it calls do not go through guards, pipes or interceptors.
- **A context without HTTP.** For a separate *worker* process,
  `CorvisFactory::create_application_context::<M>()` creates the DI container without a server. See
  [Standalone applications](/standalone-applications).

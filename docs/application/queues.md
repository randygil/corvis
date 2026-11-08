---
status: unavailable
---

# Queues

Corvis does not yet include a job queue module (producers, consumers, retries and jobs persisted in a broker such as Redis). There are no attributes to declare processors and no queue registry.

## Alternative

- For background work inside the same process, emit an event with `EventEmitter::emit`: each listener runs in its own Tokio task and the HTTP request does not wait (see [Events](/application/events)).
- For periodic work, use `#[interval]` or `#[cron]` from `ScheduleModule` (see [Task scheduling](/application/task-scheduling)).
- For a persistent queue, use the Rust client of the broker you prefer directly and register it as a provider with `Provider::use_factory` (see [Async providers](/fundamentals/async-providers)). Corvis provides no retries or job visibility in that case.

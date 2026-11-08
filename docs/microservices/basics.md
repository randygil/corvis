---
status: unavailable
---

# Overview

Corvis has no microservices layer: there are no transports, no `ClientProxy`, no message patterns (`@MessagePattern`/`@EventPattern`) and no hybrid applications that listen on HTTP and a broker at the same time.

## Alternative

You can consume or publish messages with your broker's Rust client inside a regular provider:

- Create the client with an async factory (`Provider::use_factory`), reading the connection from `ConfigService`.
- Start consuming in `OnApplicationBootstrap` (spawning a task with `corvis::tokio::spawn`) and stop consuming in `BeforeApplicationShutdown` or `OnApplicationShutdown`; call `app.enable_shutdown_hooks()` so they run on SIGINT/SIGTERM.
- For a process that only consumes, without listening on a port, see [Standalone applications](/standalone-applications): a standalone context registers and runs the `#[on_event]`, `#[cron]`, `#[interval]` and `#[timeout]` handlers.
- Within the same process, `EventEmitter` and `#[on_event]` decouple producers and consumers (see [Events](/application/events)).

See [Lifecycle events](/fundamentals/lifecycle-events) and [Async providers](/fundamentals/async-providers).
